use crate::file::{Stats, Summary};

pub fn print_summary(summary: &Summary, color: bool) {
    let rows = summary_rows(summary);
    let widths = table_widths(&rows, summary.all);

    print_header(widths, color);
    for (name, stats) in rows {
        print_row(widths, name, stats, color, false);
    }
    print_separator(widths, color);
    print_row(widths, "Total", summary.all, color, true);
}

#[derive(serde::Serialize)]
struct JsonSummary {
    languages: Vec<JsonLanguage>,
    total: JsonStats,
}

#[derive(serde::Serialize)]
struct JsonLanguage {
    language: &'static str,
    #[serde(flatten)]
    stats: JsonStats,
}

#[derive(serde::Serialize)]
struct JsonStats {
    files: u64,
    lines: u64,
    comments: u64,
    blanks: u64,
    code: u64,
}

impl From<Stats> for JsonStats {
    fn from(stats: Stats) -> Self {
        Self {
            files: stats.files,
            lines: stats.lines,
            comments: stats.comments,
            blanks: stats.blanks,
            code: stats.code,
        }
    }
}

fn json_summary(summary: &Summary) -> JsonSummary {
    JsonSummary {
        languages: summary_rows(summary)
            .into_iter()
            .map(|(language, stats)| JsonLanguage {
                language,
                stats: stats.into(),
            })
            .collect(),
        total: summary.all.into(),
    }
}

pub fn print_json(summary: &Summary) {
    println!(
        "{}",
        serde_json::to_string_pretty(&json_summary(summary)).expect("summary should serialize")
    );
}

fn summary_rows(summary: &Summary) -> Vec<(&'static str, Stats)> {
    let mut rows = summary
        .languages
        .iter()
        .map(|&(language_id, stats)| (crate::language::get(language_id).name, stats))
        .collect::<Vec<_>>();

    if summary.unknown.files > 0 {
        rows.push(("Unknown", summary.unknown));
    }

    rows.sort_by(|(left_name, left), (right_name, right)| {
        right
            .code
            .cmp(&left.code)
            .then_with(|| left_name.cmp(right_name))
    });
    rows
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

fn print_header(widths: TableWidths, color: bool) {
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
    print_styled(&line, color, "\x1b[1;36m");
    print_separator(widths, color);
}

fn print_separator(widths: TableWidths, color: bool) {
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
    print_styled(&line, color, "\x1b[2m");
}

pub fn print_unknown_formats(summary: &Summary, color: bool) {
    if summary.unknown_formats.is_empty() {
        return;
    }

    if color {
        eprintln!("\n\x1b[1;33mUnknown file formats:\x1b[0m");
    } else {
        eprintln!("\nUnknown file formats:");
    }
    for (format, files) in &summary.unknown_formats {
        eprintln!("  {format:<24} {:>8}", format_number(*files));
    }
}

fn print_row(widths: TableWidths, name: &str, stats: Stats, color: bool, total: bool) {
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
    print_styled(&line, color, if total { "\x1b[1;32m" } else { "\x1b[34m" });
}

fn print_styled(line: &str, color: bool, style: &str) {
    if color {
        println!("{style}{line}\x1b[0m");
    } else {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_summary_has_language_rows_and_total() {
        let stats = Stats {
            files: 1,
            lines: 3,
            blanks: 1,
            comments: 1,
            code: 1,
        };
        let summary = Summary {
            all: stats,
            unknown: stats,
            unknown_formats: Vec::new(),
            languages: Vec::new(),
        };

        let value = serde_json::to_value(json_summary(&summary)).unwrap();

        assert_eq!(value["languages"][0]["language"], "Unknown");
        assert_eq!(value["languages"][0]["files"], 1);
        assert_eq!(value["total"]["lines"], 3);
    }

    #[test]
    fn numbers_have_thousands_separators() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(12_345_678), "12,345,678");
    }

    #[test]
    fn summary_rows_are_ordered_by_code() {
        let summary = Summary {
            all: Stats::default(),
            unknown: Stats {
                files: 1,
                code: 200,
                ..Stats::default()
            },
            unknown_formats: Vec::new(),
            languages: vec![
                (
                    crate::language::LanguageId(0),
                    Stats {
                        files: 1,
                        code: 100,
                        ..Stats::default()
                    },
                ),
                (
                    crate::language::LanguageId(1),
                    Stats {
                        files: 1,
                        code: 300,
                        ..Stats::default()
                    },
                ),
            ],
        };

        let code_counts = summary_rows(&summary)
            .into_iter()
            .map(|(_, stats)| stats.code)
            .collect::<Vec<_>>();

        assert_eq!(code_counts, [300, 200, 100]);
    }
}
