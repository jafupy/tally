// Prototype: own directory crawler and lock-free MPMC job rings. The ignore
// crate is used only to parse and match ignore patterns, never to walk.
#[cfg(feature = "debug")]
mod metrics;
mod rules;
mod workers;

use super::{report_file_error, scan_result};
use crate::file;
#[cfg(feature = "debug")]
use crate::trace_output::TraceOutput;
use crossbeam_queue::ArrayQueue;
use ignore::gitignore::Gitignore;
use ignore::overrides::Override;
#[cfg(feature = "debug")]
use metrics::Counters;
#[cfg(feature = "debug")]
pub(crate) use metrics::ScanReport;
#[cfg(not(feature = "debug"))]
pub(crate) type ScanReport = ();
use rules::{Rules, extend_rules};
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use workers::{run_shared, run_single};

const DIR_QUEUE_CAPACITY: usize = 8192;
const FILE_QUEUE_CAPACITY: usize = 8192;

struct DirectoryJob {
    path: PathBuf,
    rules: Option<Arc<Rules>>,
}

struct Shared {
    overrides: Override,
    #[cfg(feature = "debug")]
    trace_output: TraceOutput,
    directories: ArrayQueue<DirectoryJob>,
    files: ArrayQueue<Vec<PathBuf>>,
    pending_directories: AtomicUsize,
    pending_files: AtomicUsize,
    #[cfg(feature = "debug")]
    metrics: Option<Counters>,
}

impl Shared {
    fn push_directory(&self, job: DirectoryJob, local: &mut VecDeque<DirectoryJob>) {
        trace_event!("directory_enqueue", Some(&job.path), serde_json::json!({}));
        self.pending_directories.fetch_add(1, Ordering::AcqRel);
        if let Err(job) = self.directories.push(job) {
            trace_event!("directory_spill", Some(&job.path), serde_json::json!({}));
            local.push_back(job);
            #[cfg(feature = "debug")]
            if let Some(metrics) = &self.metrics {
                metrics.directory_spills.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            #[cfg(feature = "debug")]
            if let Some(metrics) = &self.metrics {
                metrics
                    .peak_directory_queue
                    .fetch_max(self.directories.len(), Ordering::Relaxed);
            }
        }
    }

    fn push_files(&self, paths: Vec<PathBuf>, local: &mut VecDeque<Vec<PathBuf>>) {
        #[cfg(feature = "debug")]
        if crate::trace::enabled() {
            for path in &paths {
                trace_event!("file_enqueue", Some(path), serde_json::json!({}));
            }
        }
        trace_event!(
            "file_batch_enqueue",
            None,
            serde_json::json!({"files": paths.len()})
        );
        self.pending_files.fetch_add(paths.len(), Ordering::AcqRel);
        #[cfg(feature = "debug")]
        if let Some(metrics) = &self.metrics {
            metrics
                .files_queued
                .fetch_add(paths.len(), Ordering::Relaxed);
            metrics.file_batches.fetch_add(1, Ordering::Relaxed);
        }
        if let Err(paths) = self.files.push(paths) {
            trace_event!(
                "file_batch_spill",
                None,
                serde_json::json!({"files": paths.len()})
            );
            local.push_back(paths);
            #[cfg(feature = "debug")]
            if let Some(metrics) = &self.metrics {
                metrics.file_spills.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            #[cfg(feature = "debug")]
            if let Some(metrics) = &self.metrics {
                metrics
                    .peak_file_queue
                    .fetch_max(self.files.len(), Ordering::Relaxed);
            }
        }
    }

    fn done(&self) -> bool {
        self.pending_directories.load(Ordering::Acquire) == 0
            && self.pending_files.load(Ordering::Acquire) == 0
    }
}

fn list_directory(
    job: DirectoryJob,
    shared: &Shared,
    local_directories: &mut VecDeque<DirectoryJob>,
    local_files: &mut VecDeque<Vec<PathBuf>>,
    batch_size: usize,
    global: &Gitignore,
    ignore_git: bool,
    failed: &AtomicBool,
) {
    let _span = trace_span!("list_directory", Some(&job.path));
    trace_event!(
        "directory_rules",
        Some(&job.path),
        serde_json::json!({"ignore_git": ignore_git})
    );
    let rules = extend_rules(&job.path, job.rules, ignore_git, failed);
    let entries = match fs::read_dir(&job.path) {
        Ok(entries) => entries,
        Err(error) => {
            if report_file_error(&job.path, &error) {
                failed.store(true, Ordering::Relaxed);
            }
            return;
        }
    };
    #[cfg(feature = "debug")]
    if let Some(metrics) = &shared.metrics {
        metrics.directories_listed.fetch_add(1, Ordering::Relaxed);
    }
    let mut batch = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if report_file_error(&job.path, &error) {
                    failed.store(true, Ordering::Relaxed);
                }
                continue;
            }
        };
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                if report_file_error(&entry.path(), &error) {
                    failed.store(true, Ordering::Relaxed);
                }
                continue;
            }
        };
        let is_dir = kind.is_dir();
        trace_event!(
            "entry_seen",
            Some(&entry.path()),
            serde_json::json!({"directory": is_dir, "file": kind.is_file()})
        );
        if is_dir && entry.file_name() == ".git" {
            trace_event!(
                "entry_skip",
                Some(&entry.path()),
                serde_json::json!({"reason": "git_directory"})
            );
            continue;
        }
        if !is_dir && !kind.is_file() {
            trace_event!(
                "entry_skip",
                Some(&entry.path()),
                serde_json::json!({"reason": "not_regular_file"})
            );
            continue;
        }
        let path = entry.path();
        #[cfg(feature = "debug")]
        if shared.trace_output.matches_entry(&path) {
            trace_event!(
                "entry_skip",
                Some(&path),
                serde_json::json!({"reason": "trace_output"})
            );
            continue;
        }
        let ignored = {
            let _match_span = trace_span!("match_ignore", Some(&path));
            let matched = shared.overrides.matched(&path, is_dir);
            if matched.is_ignore() {
                true
            } else if matched.is_whitelist() {
                false
            } else {
                rules
                    .as_ref()
                    .is_some_and(|rules| rules.ignored(&path, is_dir, global))
                    || (rules.is_none() && global.matched(&path, is_dir).is_ignore())
            }
        };
        if ignored {
            trace_event!(
                "entry_skip",
                Some(&path),
                serde_json::json!({"reason": "ignore_rule"})
            );
            continue;
        }
        trace_event!(
            "entry_accept",
            Some(&path),
            serde_json::json!({"directory": is_dir})
        );
        if is_dir {
            shared.push_directory(
                DirectoryJob {
                    path,
                    rules: rules.clone(),
                },
                local_directories,
            );
        } else {
            batch.push(path);
            if batch.len() == batch_size {
                shared.push_files(std::mem::take(&mut batch), local_files);
            }
        }
    }
    if !batch.is_empty() {
        shared.push_files(batch, local_files);
    }
}

pub(super) fn scan(
    root: &Path,
    ignore_git: bool,
    threads: usize,
    adaptive_threads: bool,
    sink: Arc<file::Sink>,
    debug: bool,
    overrides: Override,
) -> io::Result<ScanReport> {
    let _span = trace_span!("crawler_scan", Some(root));
    let root = root.canonicalize()?;
    trace_event!("root_canonicalized", Some(&root), serde_json::json!({}));
    let failed = Arc::new(AtomicBool::new(false));
    let mut ancestor_rules = None;
    if ignore_git {
        for ancestor in root
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            ancestor_rules = extend_rules(ancestor, ancestor_rules, true, &failed);
        }
    }
    let global = if ignore_git {
        let (global, error) = Gitignore::global();
        if let Some(error) = error {
            if crate::dir::report_walk_error(&error) {
                failed.store(true, Ordering::Relaxed);
            }
        }
        global
    } else {
        Gitignore::empty()
    };
    let shared = Arc::new(Shared {
        overrides,
        #[cfg(feature = "debug")]
        trace_output: TraceOutput::current()?,
        directories: ArrayQueue::new(DIR_QUEUE_CAPACITY),
        files: ArrayQueue::new(FILE_QUEUE_CAPACITY),
        pending_directories: AtomicUsize::new(1),
        pending_files: AtomicUsize::new(0),
        #[cfg(feature = "debug")]
        metrics: debug.then(Counters::new),
    });
    trace_event!("root_queued", Some(&root), serde_json::json!({}));
    shared
        .directories
        .push(DirectoryJob {
            path: root,
            rules: ancestor_rules,
        })
        .ok();
    let max_workers = std::env::var("TALLY_RING_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map_or(threads.max(1), |workers| workers.clamp(1, threads.max(1)));
    let batch_size = std::env::var("TALLY_FILE_BATCH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(32)
        .max(1);

    let workers = if max_workers == 1 {
        run_single(
            &shared,
            sink,
            Arc::clone(&failed),
            global,
            ignore_git,
            debug,
            batch_size,
        );
        1
    } else {
        run_shared(
            &shared,
            sink,
            Arc::clone(&failed),
            global,
            ignore_git,
            debug,
            batch_size,
            max_workers,
            adaptive_threads,
        )
    };
    scan_result(failed.load(Ordering::Relaxed))?;
    #[cfg(not(feature = "debug"))]
    {
        let _ = workers;
        Ok(())
    }
    #[cfg(feature = "debug")]
    Ok(shared.metrics.as_ref().map_or(
        ScanReport {
            workers,
            worker_limit: max_workers,
            ..ScanReport::default()
        },
        |metrics| metrics.snapshot(workers, max_workers),
    ))
}
