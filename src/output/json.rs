use super::summary_rows;
use crate::file::{Stats, Summary};

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
