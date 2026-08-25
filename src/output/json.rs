use super::summary_rows;
use crate::result::{Stats, Summary};
use std::io::{self, Write};
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

pub fn write_json(output: &mut impl Write, summary: &Summary) -> io::Result<()> {
    writeln!(
        output,
        "{}",
        serde_json::to_string_pretty(&json_summary(summary)).expect("summary should serialize")
    )
}
