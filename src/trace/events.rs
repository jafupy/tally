use super::event;
use serde::Serialize;
use std::path::Path;

macro_rules! events {
    ($( $name:ident, $label:literal, [$($field:ident),*]; )*) => {
        $(pub(crate) fn $name(path: Option<&Path>, $($field: impl Serialize),*) {
            event($label, path, serde_json::json!({ $(stringify!($field): $field),* }));
        })*
    };
}

events! {
    run_error, "run_error", [error];
    run_start, "run_start", [all, json, threads];
    scan_complete, "scan_complete", [];
    summary_snapshot, "summary_snapshot", [files, lines];
    file_skipped, "file_skipped", [];
    file_error, "file_error", [error];
    progress_update, "progress_update", [files];
    sink_batch, "sink_batch", [files, lines];
    file_opened, "file_opened", [];
    prefix_read, "prefix_read", [bytes];
    file_skip, "file_skip", [reason];
    language_detected, "language_detected", [language];
    language_unknown, "language_unknown", [];
    read_chunk, "read_chunk", [bytes];
    directory_enqueue, "directory_enqueue", [];
    file_enqueue, "file_enqueue", [];
    file_batch_enqueue, "file_batch_enqueue", [files];
    directory_rules, "directory_rules", [ignore_git];
    entry_seen, "entry_seen", [directory, file];
    entry_skip, "entry_skip", [reason];
    entry_accept, "entry_accept", [directory];
    root_canonicalized, "root_canonicalized", [];
    root_queued, "root_queued", [];
    ignore_rules_absent, "ignore_rules_absent", [];
    ignore_rules_loaded, "ignore_rules_loaded", [patterns, error];
    exclude_rules_absent, "exclude_rules_absent", [];
    rules_inherited, "rules_inherited", [];
    rules_extended, "rules_extended", [ignore, gitignore, exclude];
    file_batch_start, "file_batch_start", [files];
    file_batch_done, "file_batch_done", [files];
    worker_start, "worker_start", [];
    file_batch_pop, "file_batch_pop", [files, ring_depth];
    directory_pop, "directory_pop", [ring_depth];
    worker_yield, "worker_yield", [directories_pending, files_pending];
    worker_exit, "worker_exit", [idle_yields];
    worker_spawn, "worker_spawn", [number, reason];
    controller_tick, "controller_tick", [queued, workers];
    worker_spawn_detail, "worker_spawn", [number, reason, queued];
}

use crate::{file::FileStats, result::Stats};

pub(crate) fn batch_add_file(path: Option<&Path>, file: &FileStats) {
    let (stats, known) = match file {
        FileStats::Known { stats, .. } => (stats, true),
        FileStats::Unknown { stats, .. } => (stats, false),
    };
    event(
        "batch_add_file",
        path,
        serde_json::json!({"lines": stats.lines, "known": known}),
    );
}

pub(crate) fn file_counted(path: Option<&Path>, stats: &Stats) {
    event(
        "file_counted",
        path,
        serde_json::json!({
            "lines": stats.lines, "code": stats.code, "comments": stats.comments, "blanks": stats.blanks
        }),
    );
}

pub(crate) fn unknown_file_counted(path: Option<&Path>, stats: &Stats, format: &Option<String>) {
    event(
        "file_counted",
        path,
        serde_json::json!({
            "lines": stats.lines, "code": stats.code, "comments": stats.comments, "blanks": stats.blanks,
            "unknown_format": format
        }),
    );
}

pub(crate) fn line_classified(path: Option<&Path>, before: Stats, after: &Stats) {
    if after.lines == before.lines {
        return;
    }
    let class = if after.code > before.code {
        "code"
    } else if after.comments > before.comments {
        "comment"
    } else {
        "blank"
    };
    event(
        "line_classified",
        path,
        serde_json::json!({"line": after.lines, "class": class}),
    );
}

pub(crate) fn plain_line_classified(path: Option<&Path>, line: u64, code: bool) {
    event(
        "line_classified",
        path,
        serde_json::json!({"line": line, "class": if code { "code" } else { "blank" }}),
    );
}

pub(crate) fn files_queued(_: Option<&Path>, paths: &[std::path::PathBuf]) {
    for path in paths {
        file_enqueue(Some(path));
    }
    file_batch_enqueue(None, paths.len());
}
