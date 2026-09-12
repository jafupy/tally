use super::{Shared, list_directory};
use crate::dir::ScanWorker;
use crate::file::{self, Batch};
use ignore::gitignore::Gitignore;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

fn new_scanner(sink: Arc<file::Sink>, failed: Arc<AtomicBool>, verbose: bool) -> ScanWorker {
    ScanWorker {
        sink,
        batch: Batch::default(),
        verbose,
        failed,
        buffer: file::read_buffer(),
    }
}

fn count_batch(scanner: &mut ScanWorker, shared: &Shared, paths: Vec<PathBuf>) {
    let count = paths.len();
    for path in paths {
        scanner.visit_path(&path);
    }
    shared.pending_files.fetch_sub(count, Ordering::AcqRel);
}

pub(super) fn run_single(
    shared: &Shared,
    sink: Arc<file::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    verbose: bool,
    batch_size: usize,
) {
    let mut scanner = new_scanner(sink, Arc::clone(&failed), verbose);
    let mut local_directories = VecDeque::new();
    let mut local_files: VecDeque<Vec<PathBuf>> = VecDeque::new();
    while !shared.done() {
        let mut worked = false;
        if let Some(paths) = shared.files.pop().or_else(|| local_files.pop_front()) {
            count_batch(&mut scanner, shared, paths);
            worked = true;
        }
        if let Some(job) = shared
            .directories
            .pop()
            .or_else(|| local_directories.pop_front())
        {
            list_directory(
                job,
                shared,
                &mut local_directories,
                &mut local_files,
                batch_size,
                &global,
                ignore_git,
                &failed,
            );
            shared.pending_directories.fetch_sub(1, Ordering::AcqRel);
            worked = true;
        } else if let Some(paths) = shared.files.pop().or_else(|| local_files.pop_front()) {
            count_batch(&mut scanner, shared, paths);
            worked = true;
        }
        if !worked {
            thread::yield_now();
        }
    }
}

fn spawn_worker(
    shared: &Arc<Shared>,
    sink: &Arc<file::Sink>,
    failed: &Arc<AtomicBool>,
    global: &Gitignore,
    ignore_git: bool,
    verbose: bool,
    batch_size: usize,
) -> thread::JoinHandle<()> {
    let shared = Arc::clone(shared);
    let sink = Arc::clone(sink);
    let failed = Arc::clone(failed);
    let global = global.clone();
    thread::spawn(move || {
        run_single(
            &shared, sink, failed, global, ignore_git, verbose, batch_size,
        )
    })
}

pub(super) fn run_shared(
    shared: &Arc<Shared>,
    sink: Arc<file::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    verbose: bool,
    batch_size: usize,
    max_workers: usize,
    adaptive_threads: bool,
) {
    let mut workers = vec![spawn_worker(
        shared, &sink, &failed, &global, ignore_git, verbose, batch_size,
    )];
    if !adaptive_threads {
        while workers.len() < max_workers {
            workers.push(spawn_worker(
                shared, &sink, &failed, &global, ignore_git, verbose, batch_size,
            ));
        }
    }
    while !shared.done() {
        let queued = shared.directories.len() + shared.files.len();
        if workers.len() < max_workers && queued > workers.len() {
            workers.push(spawn_worker(
                shared, &sink, &failed, &global, ignore_git, verbose, batch_size,
            ));
        }
        thread::park_timeout(Duration::from_micros(100));
    }
    for worker in workers {
        worker.join().unwrap();
    }
}
