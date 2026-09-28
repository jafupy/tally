mod json;
mod table;

use crate::file::{Stats, Summary};
use std::collections::HashMap;
use tally_stats::{Kind, Sample, Values};

pub use json::print_json;
pub use table::{format_number, print_summary, print_unknown_formats};

pub(crate) const DIM_STYLE: &str = "\x1b[2m";

struct ExtendedStats {
    by_language: HashMap<&'static str, Vec<(Kind, Values)>>,
    empty: Vec<(Kind, Values)>,
    total: Vec<(Kind, Values)>,
}

impl ExtendedStats {
    fn new(summary: &Summary, kinds: &[Kind]) -> Self {
        if kinds.is_empty() {
            return Self {
                by_language: HashMap::new(),
                empty: Vec::new(),
                total: Vec::new(),
            };
        }

        let mut groups: HashMap<&'static str, Vec<Sample>> = HashMap::new();
        for &(id, stats) in &summary.samples {
            let language = id
                .map(|id| crate::language::get(id).name)
                .unwrap_or("Unknown");
            groups.entry(language).or_default().push(Sample {
                blanks: stats.blanks,
                comments: stats.comments,
                code: stats.code,
            });
        }

        Self {
            by_language: groups
                .into_iter()
                .map(|(language, samples)| (language, tally_stats::calculate_all(samples, kinds)))
                .collect(),
            empty: tally_stats::calculate_all(std::iter::empty(), kinds),
            total: tally_stats::calculate_all(
                summary.samples.iter().map(|&(_, stats)| Sample {
                    blanks: stats.blanks,
                    comments: stats.comments,
                    code: stats.code,
                }),
                kinds,
            ),
        }
    }

    fn language(&self, name: &str) -> &[(Kind, Values)] {
        self.by_language.get(name).unwrap_or(&self.empty).as_slice()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
            samples: Vec::new(),
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
