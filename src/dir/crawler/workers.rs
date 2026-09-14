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
use std::time::{Duration, Instant};

fn new_scanner(sink: Arc<file::Sink>, failed: Arc<AtomicBool>, debug: bool) -> ScanWorker {
    let batch = Batch::with_samples(sink.collects_samples());
    ScanWorker {
        sink,
        batch,
        debug,
        failed,
        buffer: file::read_buffer(),
    }
}

fn count_batch(
    scanner: &mut ScanWorker,
    shared: &Shared,
    paths: Vec<PathBuf>,
    counting_time: &mut Duration,
) {
    let _span = trace_span!("count_batch", None);
    trace_event!(
        "file_batch_start",
        None,
        serde_json::json!({"files": paths.len()})
    );
    let started = shared.metrics.as_ref().map(|_| Instant::now());
    let count = paths.len();
    for path in paths {
        scanner.visit_path(&path);
    }
    shared.pending_files.fetch_sub(count, Ordering::AcqRel);
    if let Some(started) = started {
        *counting_time += started.elapsed();
    }
    trace_event!("file_batch_done", None, serde_json::json!({"files": count}));
}

pub(super) fn run_single(
    shared: &Shared,
    sink: Arc<file::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    debug: bool,
    batch_size: usize,
) {
    let _span = trace_span!("worker_lifetime", None);
    trace_event!("worker_start", None, serde_json::json!({}));
    let mut scanner = new_scanner(sink, Arc::clone(&failed), debug);
    let mut local_directories = VecDeque::new();
    let mut local_files: VecDeque<Vec<PathBuf>> = VecDeque::new();
    let mut counting_time = Duration::ZERO;
    let mut listing_time = Duration::ZERO;
    let mut idle_yields = 0;
    while !shared.done() {
        let mut worked = false;
        if let Some(paths) = shared.files.pop().or_else(|| local_files.pop_front()) {
            trace_event!(
                "file_batch_pop",
                None,
                serde_json::json!({"files": paths.len(), "ring_depth": shared.files.len()})
            );
            count_batch(&mut scanner, shared, paths, &mut counting_time);
            worked = true;
        }
        if let Some(job) = shared
            .directories
            .pop()
            .or_else(|| local_directories.pop_front())
        {
            trace_event!(
                "directory_pop",
                Some(&job.path),
                serde_json::json!({"ring_depth": shared.directories.len()})
            );
            let started = shared.metrics.as_ref().map(|_| Instant::now());
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
            if let Some(started) = started {
                listing_time += started.elapsed();
            }
            shared.pending_directories.fetch_sub(1, Ordering::AcqRel);
            worked = true;
        } else if let Some(paths) = shared.files.pop().or_else(|| local_files.pop_front()) {
            trace_event!(
                "file_batch_pop",
                None,
                serde_json::json!({"files": paths.len(), "ring_depth": shared.files.len()})
            );
            count_batch(&mut scanner, shared, paths, &mut counting_time);
            worked = true;
        }
        if !worked {
            trace_event!(
                "worker_yield",
                None,
                serde_json::json!({"directories_pending": shared.pending_directories.load(Ordering::Relaxed), "files_pending": shared.pending_files.load(Ordering::Relaxed)})
            );
            if shared.metrics.is_some() {
                idle_yields += 1;
            }
            thread::yield_now();
        }
    }
    if let Some(metrics) = &shared.metrics {
        metrics
            .counting_nanos
            .fetch_add(counting_time.as_nanos() as u64, Ordering::Relaxed);
        metrics
            .listing_nanos
            .fetch_add(listing_time.as_nanos() as u64, Ordering::Relaxed);
        metrics
            .idle_yields
            .fetch_add(idle_yields, Ordering::Relaxed);
    }
    trace_event!(
        "worker_exit",
        None,
        serde_json::json!({"idle_yields": idle_yields})
    );
}

fn spawn_worker(
    shared: &Arc<Shared>,
    sink: &Arc<file::Sink>,
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
    sink: Arc<file::Sink>,
    failed: Arc<AtomicBool>,
    global: Gitignore,
    ignore_git: bool,
    debug: bool,
    batch_size: usize,
    max_workers: usize,
    adaptive_threads: bool,
) -> usize {
    trace_event!(
        "worker_spawn",
        None,
        serde_json::json!({"number": 1, "reason": "initial"})
    );
    let mut workers = vec![spawn_worker(
        shared, &sink, &failed, &global, ignore_git, debug, batch_size,
    )];
    if !adaptive_threads {
        while workers.len() < max_workers {
            trace_event!(
                "worker_spawn",
                None,
                serde_json::json!({"number": workers.len() + 1, "reason": "explicit_threads"})
            );
            workers.push(spawn_worker(
                shared, &sink, &failed, &global, ignore_git, debug, batch_size,
            ));
        }
    }
    while !shared.done() {
        let queued = shared.directories.len() + shared.files.len();
        trace_event!(
            "controller_tick",
            None,
            serde_json::json!({"queued": queued, "workers": workers.len()})
        );
        if workers.len() < max_workers && queued > workers.len() {
            trace_event!(
                "worker_spawn",
                None,
                serde_json::json!({"number": workers.len() + 1, "reason": "queue_pressure", "queued": queued})
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
