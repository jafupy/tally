use crate::file::{Stats, Summary};

pub fn print_summary(summary: &Summary) {
    let mut rows = summary_rows(summary);
    let widths = table_widths(&rows, summary.all);

    print_header(widths);
    for (name, stats) in rows.drain(..) {
        print_row(widths, name, stats);
    }
    print_separator(widths);
    print_row(widths, "Total", summary.all);
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
    let mut languages = summary.languages.clone();
    languages.sort_by_key(|&(language_id, _)| crate::language::get(language_id).name);

    let mut rows = languages
        .into_iter()
        .map(|(language_id, stats)| (crate::language::get(language_id).name, stats))
        .collect::<Vec<_>>();

    if summary.unknown.files > 0 {
        rows.push(("Unknown", summary.unknown));
    }

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
        widths.files = widths.files.max(digits(stats.files));
        widths.lines = widths.lines.max(digits(stats.lines));
        widths.blanks = widths.blanks.max(digits(stats.blanks));
        widths.comments = widths.comments.max(digits(stats.comments));
        widths.code = widths.code.max(digits(stats.code));
    }

    widths
}

fn digits(number: u64) -> usize {
    number.max(1).ilog10() as usize + 1
}

fn print_header(widths: TableWidths) {
    println!(
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
    print_separator(widths);
}

fn print_separator(widths: TableWidths) {
    println!(
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
}

pub fn print_unknown_formats(summary: &Summary) {
    if summary.unknown_formats.is_empty() {
        return;
    }

    eprintln!("\nUnknown file formats:");
    for (format, files) in &summary.unknown_formats {
        eprintln!("  {format:<24} {files:>8}");
    }
}

fn print_row(widths: TableWidths, name: &str, stats: Stats) {
    println!(
        "{:<name_width$} {:>files_width$} {:>lines_width$} {:>blanks_width$} {:>comments_width$} {:>code_width$}",
        name,
        stats.files,
        stats.lines,
        stats.blanks,
        stats.comments,
        stats.code,
        name_width = widths.name,
        files_width = widths.files,
        lines_width = widths.lines,
        blanks_width = widths.blanks,
        comments_width = widths.comments,
        code_width = widths.code,
    );
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
}
