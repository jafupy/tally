mod json;
mod table;
use crate::result::{Stats, Summary};
pub use json::write_json;
pub use table::{format_number, write_summary, write_unknown_formats};

fn summary_rows(summary: &Summary) -> Vec<(&'static str, Stats)> {
    let mut rows = summary
        .languages
        .iter()
        .map(|&(language_id, stats)| (crate::language::get(language_id).name, stats))
        .collect::<Vec<_>>();

    if summary.unknown.files > 0 {
        let at = rows
            .iter()
            .position(|&(name, stats)| {
                stats.code < summary.unknown.code
                    || (stats.code == summary.unknown.code && name > "Unknown")
            })
            .unwrap_or(rows.len());
        rows.insert(at, ("Unknown", summary.unknown));
    }
    rows
}
