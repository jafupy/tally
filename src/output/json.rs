use super::{calculate_extended, summary_rows};
use crate::file::{Stats, Summary};
use std::collections::BTreeMap;
use std::io::{self, Write};
use tally_stats::{Kind, Values};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    extended: Option<BTreeMap<String, Values>>,
}

impl From<Stats> for JsonStats {
    fn from(stats: Stats) -> Self {
        Self {
            files: stats.files,
            lines: stats.lines,
            comments: stats.comments,
            blanks: stats.blanks,
            code: stats.code,
            extended: None,
        }
    }
}

fn json_summary(summary: &Summary, kinds: &[Kind]) -> JsonSummary {
    JsonSummary {
        languages: summary_rows(summary)
            .into_iter()
            .map(|(language, stats)| JsonLanguage {
                language,
                stats: with_extended(summary, Some(language), stats, kinds),
            })
            .collect(),
        total: with_extended(summary, None, summary.all, kinds),
    }
}

fn with_extended(
    summary: &Summary,
    language: Option<&str>,
    stats: Stats,
    kinds: &[Kind],
) -> JsonStats {
    let mut json: JsonStats = stats.into();
    if !kinds.is_empty() {
        json.extended = Some(
            calculate_extended(summary, language, kinds)
                .into_iter()
                .map(|(kind, values)| (kind.name(), values))
                .collect(),
        );
    }
    json
}

pub fn print_json(summary: &Summary, kinds: &[Kind]) -> io::Result<()> {
    writeln!(
        io::stdout().lock(),
        "{}",
        serde_json::to_string_pretty(&json_summary(summary, kinds))
            .expect("summary should serialize")
    )
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
            samples: Vec::new(),
        };

        let value = serde_json::to_value(json_summary(&summary, &[])).unwrap();

        assert_eq!(value["languages"][0]["language"], "Unknown");
        assert_eq!(value["languages"][0]["files"], 1);
        assert_eq!(value["total"]["lines"], 3);
    }
}
