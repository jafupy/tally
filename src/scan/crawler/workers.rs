use super::{Shared, list_directory, metrics::WorkerStats};
use crate::scan::ScanWorker;
use crate::{
    file,
    result::{self, Batch},
};
use ignore::gitignore::Gitignore;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

fn new_scanner(sink: Arc<result::Sink>, failed: Arc<AtomicBool>, debug: bool) -> ScanWorker {
    ScanWorker {
        batch: Batch::with_samples(sink.collects_samples()),
        sink,
        debug,
        failed,
        buffer: file::read_buffer(),
    }
}

fn count_batch(
    scanner: &mut ScanWorker,
    shared: &Shared,
    paths: Vec<PathBuf>,
    metrics: &mut WorkerStats<'_>,
) {
    let _span = trace_span!("count_batch", None);
    trace_event!(file_batch_start, None, paths.len());
    let timer = metrics.counting();
    let count = paths.len();
    for path in paths {
        scanner.visit_path(&path);
    }
    shared.pending_files.fetch_sub(count, Ordering::AcqRel);
    drop(timer);
    trace_event!(file_batch_done, None, count);
}

pub(super) fn run_single(
    shared: &Shared,
    sink: Arc<result::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    debug: bool,
    batch_size: usize,
) {
    let _span = trace_span!("worker_lifetime", None);
    trace_event!(worker_start, None);
    let mut scanner = new_scanner(sink, Arc::clone(&failed), debug);
    let mut metrics = shared.metrics.worker();
    while !shared.done() {
        let mut worked = false;
        if let Some(paths) = shared.files.pop() {
            trace_event!(file_batch_pop, None, paths.len(), shared.files.len());
            count_batch(&mut scanner, shared, paths, &mut metrics);
            worked = true;
        }
        if let Some(job) = shared.directories.pop() {
            trace_event!(directory_pop, Some(&job.path), shared.directories.len());
            {
                let _timer = metrics.listing();
                list_directory(job, shared, batch_size, &global, ignore_git, &failed);
            }
            shared.pending_directories.fetch_sub(1, Ordering::AcqRel);
            worked = true;
        } else if let Some(paths) = shared.files.pop() {
            trace_event!(file_batch_pop, None, paths.len(), shared.files.len());
            count_batch(&mut scanner, shared, paths, &mut metrics);
            worked = true;
        }
        if !worked {
            trace_event!(
                worker_yield,
                None,
                shared.pending_directories.load(Ordering::Relaxed),
                shared.pending_files.load(Ordering::Relaxed)
            );
            metrics.yielded();
            thread::yield_now();
        }
    }
}

fn spawn_worker(
    shared: &Arc<Shared>,
    sink: &Arc<result::Sink>,
    failed: &Arc<AtomicBool>,
    global: &Gitignore,
    ignore_git: bool,
    debug: bool,
    batch_size: usize,
) -> thread::JoinHandle<()> {
    let shared = Arc::clone(shared);
    let sink = Arc::clone(sink);
    let failed = Arc::clone(failed);
    let global = global.clone();
    thread::spawn(move || run_single(&shared, sink, failed, global, ignore_git, debug, batch_size))
}

pub(super) fn run_shared(
    shared: &Arc<Shared>,
    sink: Arc<result::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    debug: bool,
    batch_size: usize,
    max_workers: usize,
    adaptive_threads: bool,
) -> usize {
    trace_event!(worker_spawn, None, 1, "initial");
    let mut workers = vec![spawn_worker(
        shared, &sink, &failed, &global, ignore_git, debug, batch_size,
    )];
    if !adaptive_threads {
        while workers.len() < max_workers {
            trace_event!(worker_spawn, None, workers.len() + 1, "explicit_threads");
            workers.push(spawn_worker(
                shared, &sink, &failed, &global, ignore_git, debug, batch_size,
            ));
        }
    }
    while !shared.done() {
        let queued = shared.directories.len() + shared.files.len();
        trace_event!(controller_tick, None, queued, workers.len());
        if workers.len() < max_workers && queued > workers.len() {
            trace_event!(
                worker_spawn_detail,
                None,
                workers.len() + 1,
                "queue_pressure",
                queued
            );
            workers.push(spawn_worker(
                shared, &sink, &failed, &global, ignore_git, debug, batch_size,
            ));
        }
        let _park_span = trace_span!("controller_park", None);
        thread::park_timeout(Duration::from_micros(100));
    }
    let worker_count = workers.len();
    for worker in workers {
        worker.join().unwrap();
    }
    worker_count
}
