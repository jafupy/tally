mod json;
mod table;

use crate::file::{Stats, Summary};

pub use json::print_json;
pub use table::{format_number, print_summary, print_unknown_formats};

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
