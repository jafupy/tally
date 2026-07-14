use super::summary_rows;
use crate::file::{Stats, Summary};
use std::io::{self, Write};

pub fn print_summary(summary: &Summary, color: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    let rows = summary_rows(summary);
    let widths = table_widths(&rows, summary.all);

    print_header(&mut output, widths, color)?;
    for (name, stats) in rows {
        print_row(&mut output, widths, name, stats, color, false)?;
    }
    print_separator(&mut output, widths, color)?;
    print_row(&mut output, widths, "Total", summary.all, color, true)
}

#[derive(Clone, Copy)]
struct TableWidths {
    name: usize,
    files: usize,
    lines: usize,
    blanks: usize,
    comments: usize,
    code: usize,
}

fn table_widths(rows: &[(&str, Stats)], total: Stats) -> TableWidths {
    let mut widths = TableWidths {
        name: "Language".len(),
        files: "Files".len(),
        lines: "Lines".len(),
        blanks: "Blank".len(),
        comments: "Comment".len(),
        code: "Code".len(),
    };

    for &(name, stats) in rows.iter().chain([("Total", total)].iter()) {
        widths.name = widths.name.max(name.len());
        widths.files = widths.files.max(format_number(stats.files).len());
        widths.lines = widths.lines.max(format_number(stats.lines).len());
        widths.blanks = widths.blanks.max(format_number(stats.blanks).len());
        widths.comments = widths.comments.max(format_number(stats.comments).len());
        widths.code = widths.code.max(format_number(stats.code).len());
    }
    widths
}

pub fn format_number(number: u64) -> String {
    let digits = number.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group = digits.len() % 3;

    for (index, digit) in digits.bytes().enumerate() {
        if index > 0 && index % 3 == first_group {
            formatted.push(',');
        }
        formatted.push(char::from(digit));
    }
    formatted
}

fn print_header(output: &mut impl Write, widths: TableWidths, color: bool) -> io::Result<()> {
    let line = format!(
        "{:<name$} {:>files$} {:>lines$} {:>blanks$} {:>comments$} {:>code$}",
        "Language",
        "Files",
        "Lines",
        "Blank",
        "Comment",
        "Code",
        name = widths.name,
        files = widths.files,
        lines = widths.lines,
        blanks = widths.blanks,
        comments = widths.comments,
        code = widths.code,
    );
    print_styled(output, &line, color, "\x1b[1;36m")?;
    print_separator(output, widths, color)
}

fn print_separator(output: &mut impl Write, widths: TableWidths, color: bool) -> io::Result<()> {
    let line = format!(
        "{:-<name$} {:-<files$} {:-<lines$} {:-<blanks$} {:-<comments$} {:-<code$}",
        "",
        "",
        "",
        "",
        "",
        "",
        name = widths.name,
        files = widths.files,
        lines = widths.lines,
        blanks = widths.blanks,
        comments = widths.comments,
        code = widths.code,
    );
    print_styled(output, &line, color, "\x1b[2m")
}

pub fn print_unknown_formats(summary: &Summary, color: bool) -> io::Result<()> {
    if summary.unknown_formats.is_empty() {
        return Ok(());
    }

    let mut error = io::stderr().lock();
    if color {
        writeln!(error, "\n\x1b[1;33mUnknown file formats:\x1b[0m")?;
    } else {
        writeln!(error, "\nUnknown file formats:")?;
    }
    for (format, files) in &summary.unknown_formats {
        writeln!(error, "  {format:<24} {:>8}", format_number(*files))?;
    }
    Ok(())
}

fn print_row(
    output: &mut impl Write,
    widths: TableWidths,
    name: &str,
    stats: Stats,
    color: bool,
    total: bool,
) -> io::Result<()> {
    let line = format!(
        "{:<name_width$} {:>files_width$} {:>lines_width$} {:>blanks_width$} {:>comments_width$} {:>code_width$}",
        name,
        format_number(stats.files),
        format_number(stats.lines),
        format_number(stats.blanks),
        format_number(stats.comments),
        format_number(stats.code),
        name_width = widths.name,
        files_width = widths.files,
        lines_width = widths.lines,
        blanks_width = widths.blanks,
        comments_width = widths.comments,
        code_width = widths.code,
    );
    print_styled(
        output,
        &line,
        color,
        if total { "\x1b[1;32m" } else { "\x1b[34m" },
    )
}

fn print_styled(output: &mut impl Write, line: &str, color: bool, style: &str) -> io::Result<()> {
    if color {
        writeln!(output, "{style}{line}\x1b[0m")
    } else {
        writeln!(output, "{line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_have_thousands_separators() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(12_345_678), "12,345,678");
    }
}
