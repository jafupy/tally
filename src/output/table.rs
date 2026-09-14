use super::{calculate_extended, summary_rows};
use crate::file::{Stats, Summary};
use std::io::{self, Write};
use tally_stats::Kind;

pub fn print_summary(summary: &Summary, color: bool, kinds: &[Kind]) -> io::Result<()> {
    let mut output = io::stdout().lock();
    let rows = summary_rows(summary)
        .into_iter()
        .map(|(name, stats)| table_row(summary, name, stats, Some(name), kinds))
        .collect::<Vec<_>>();
    let total = table_row(summary, "Total", summary.all, None, kinds);
    let widths = table_widths(&rows, &total, kinds);

    print_header(&mut output, &widths, kinds, color)?;
    for row in &rows {
        print_row(&mut output, &widths, row, color, false)?;
    }
    print_separator(&mut output, &widths, color)?;
    print_row(&mut output, &widths, &total, color, true)
}

struct TableRow<'a> {
    name: &'a str,
    stats: Stats,
    extra: Vec<String>,
}

fn table_row<'a>(
    summary: &Summary,
    name: &'a str,
    stats: Stats,
    language: Option<&str>,
    kinds: &[Kind],
) -> TableRow<'a> {
    let values = calculate_extended(summary, language, kinds);
    let mut extra = Vec::with_capacity(kinds.len() * 3);
    for metric in 0..3 {
        for (kind, value) in &values {
            let value = match metric {
                0 => value.blanks,
                1 => value.comments,
                _ => value.code,
            };
            extra.push(match value {
                None => "-".to_string(),
                Some(value) if matches!(kind, Kind::Min | Kind::Max) => format_number(value as u64),
                Some(value) => format!("{value:.2}"),
            });
        }
    }
    TableRow { name, stats, extra }
}

struct TableWidths {
    name: usize,
    files: usize,
    lines: usize,
    blanks: usize,
    comments: usize,
    code: usize,
    extra: Vec<usize>,
}

fn table_widths(rows: &[TableRow<'_>], total: &TableRow<'_>, kinds: &[Kind]) -> TableWidths {
    let mut widths = TableWidths {
        name: "Language".len(),
        files: "Files".len(),
        lines: "Lines".len(),
        blanks: "Blank".len(),
        comments: "Comment".len(),
        code: "Code".len(),
        extra: (0..3)
            .flat_map(|_| kinds.iter().map(|kind| kind.label().len()))
            .collect(),
    };

    for row in rows.iter().chain(std::iter::once(total)) {
        let stats = row.stats;
        widths.name = widths.name.max(row.name.len());
        widths.files = widths.files.max(format_number(stats.files).len());
        widths.lines = widths.lines.max(format_number(stats.lines).len());
        widths.blanks = widths.blanks.max(format_number(stats.blanks).len());
        widths.comments = widths.comments.max(format_number(stats.comments).len());
        widths.code = widths.code.max(format_number(stats.code).len());
        for (width, value) in widths.extra.iter_mut().zip(&row.extra) {
            *width = (*width).max(value.len());
        }
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

fn print_header(
    output: &mut impl Write,
    widths: &TableWidths,
    kinds: &[Kind],
    color: bool,
) -> io::Result<()> {
    let mut line = format!(
        "{:<name$} {:>files$} {:>lines$}",
        "Language",
        "Files",
        "Lines",
        name = widths.name,
        files = widths.files,
        lines = widths.lines,
    );
    for (metric, width, group) in [
        ("Blank", widths.blanks, 0),
        ("Comment", widths.comments, 1),
        ("Code", widths.code, 2),
    ] {
        append_group_boundary(&mut line, color, !kinds.is_empty(), "\x1b[1;36m");
        line.push_str(&format!(" {metric:>width$}"));
        append_extended(
            &mut line,
            kinds.iter().map(Kind::label),
            &widths.extra[group * kinds.len()..(group + 1) * kinds.len()],
            color,
            "\x1b[1;36m",
        );
    }
    print_styled(output, &line, color, "\x1b[1;36m")?;
    print_separator(output, widths, color)
}

fn print_separator(output: &mut impl Write, widths: &TableWidths, color: bool) -> io::Result<()> {
    let width = widths.name
        + widths.files
        + widths.lines
        + widths.blanks
        + widths.comments
        + widths.code
        + 5
        + widths.extra.iter().sum::<usize>()
        + widths.extra.len()
        + if widths.extra.is_empty() { 0 } else { 6 };
    let line = "─".repeat(width);
    print_styled(output, &line, color, super::DIM_STYLE)
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
    widths: &TableWidths,
    row: &TableRow<'_>,
    color: bool,
    total: bool,
) -> io::Result<()> {
    let stats = row.stats;
    let base_style = if total { "\x1b[1;32m" } else { "\x1b[34m" };
    let mut line = format!(
        "{:<name_width$} {:>files_width$} {:>lines_width$}",
        row.name,
        format_number(stats.files),
        format_number(stats.lines),
        name_width = widths.name,
        files_width = widths.files,
        lines_width = widths.lines,
    );
    let per_group = row.extra.len() / 3;
    for (metric, width, group) in [
        (stats.blanks, widths.blanks, 0),
        (stats.comments, widths.comments, 1),
        (stats.code, widths.code, 2),
    ] {
        append_group_boundary(&mut line, color, per_group > 0, base_style);
        line.push_str(&format!(" {:>width$}", format_number(metric)));
        append_extended(
            &mut line,
            row.extra[group * per_group..(group + 1) * per_group]
                .iter()
                .cloned(),
            &widths.extra[group * per_group..(group + 1) * per_group],
            color,
            base_style,
        );
    }
    print_styled(output, &line, color, base_style)
}

fn append_group_boundary(line: &mut String, color: bool, extended: bool, base_style: &str) {
    if extended {
        if color {
            line.push_str("\x1b[0;2m");
        }
        line.push_str(" │");
        if color {
            line.push_str("\x1b[0m");
            line.push_str(base_style);
        }
    }
}

fn append_extended(
    line: &mut String,
    values: impl IntoIterator<Item = String>,
    widths: &[usize],
    color: bool,
    base_style: &str,
) {
    if widths.is_empty() {
        return;
    }
    if color {
        line.push_str("\x1b[0;2m");
    }
    for (value, width) in values.into_iter().zip(widths) {
        line.push_str(&format!(" {value:>width$}"));
    }
    if color {
        line.push_str("\x1b[0m");
        line.push_str(base_style);
    }
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
