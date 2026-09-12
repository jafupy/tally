// Prototype: own directory crawler and lock-free MPMC job rings. The ignore
// crate is used only to parse and match ignore patterns, never to walk.
mod rules;
mod workers;

use super::scan_result;
use crate::file;
use crossbeam_queue::ArrayQueue;
use ignore::gitignore::Gitignore;
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
    directories: ArrayQueue<DirectoryJob>,
    files: ArrayQueue<Vec<PathBuf>>,
    pending_directories: AtomicUsize,
    pending_files: AtomicUsize,
}

impl Shared {
    fn push_directory(&self, job: DirectoryJob, local: &mut VecDeque<DirectoryJob>) {
        self.pending_directories.fetch_add(1, Ordering::AcqRel);
        if let Err(job) = self.directories.push(job) {
            local.push_back(job);
        }
    }

    fn push_files(&self, paths: Vec<PathBuf>, local: &mut VecDeque<Vec<PathBuf>>) {
        self.pending_files.fetch_add(paths.len(), Ordering::AcqRel);
        if let Err(paths) = self.files.push(paths) {
            local.push_back(paths);
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
    let rules = extend_rules(&job.path, job.rules, ignore_git, failed);
    let entries = match fs::read_dir(&job.path) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("failed to read directory {}: {error}", job.path.display());
            failed.store(true, Ordering::Relaxed);
            return;
        }
    };
    let mut batch = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                eprintln!(
                    "failed to read directory entry in {}: {error}",
                    job.path.display()
                );
                failed.store(true, Ordering::Relaxed);
                continue;
            }
        };
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                eprintln!("failed to inspect {}: {error}", entry.path().display());
                failed.store(true, Ordering::Relaxed);
                continue;
            }
        };
        let is_dir = kind.is_dir();
        if is_dir && entry.file_name() == ".git" {
            continue;
        }
        if !is_dir && !kind.is_file() {
            continue;
        }
        let path = entry.path();
        if rules
            .as_ref()
            .is_some_and(|rules| rules.ignored(&path, is_dir, global))
            || (rules.is_none() && global.matched(&path, is_dir).is_ignore())
        {
            continue;
        }
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
    verbose: bool,
) -> io::Result<()> {
    let root = root.canonicalize()?;
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
            eprintln!("failed to read global ignore rules: {error}");
            failed.store(true, Ordering::Relaxed);
        }
        global
    } else {
        Gitignore::empty()
    };
    let shared = Arc::new(Shared {
        directories: ArrayQueue::new(DIR_QUEUE_CAPACITY),
        files: ArrayQueue::new(FILE_QUEUE_CAPACITY),
        pending_directories: AtomicUsize::new(1),
        pending_files: AtomicUsize::new(0),
    });
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

    if max_workers == 1 {
        run_single(
            &shared,
            sink,
            Arc::clone(&failed),
            global,
            ignore_git,
            verbose,
            batch_size,
        );
    } else {
        run_shared(
            &shared,
            sink,
            Arc::clone(&failed),
            global,
            ignore_git,
            verbose,
            batch_size,
            max_workers,
            adaptive_threads,
        );
    }
    scan_result(failed.load(Ordering::Relaxed))
}
