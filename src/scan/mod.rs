use crate::{
    count,
    result::{Batch, Sink},
};
use ignore::{DirEntry, Error, WalkBuilder, WalkParallel, WalkState};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
const FLUSH_EVERY_FILES: u64 = 512;
const ADAPTIVE_SMALL_FILE_LIMIT: usize = 128;
const PATH_BATCH_SIZE: usize = 32;
pub fn scan_directory(
    path: &Path,
    sink: Arc<Sink>,
    ignore_git: bool,
    threads: usize,
    adaptive_threads: bool,
    verbose: bool,
) -> io::Result<()> {
    if adaptive_threads && threads > 1 {
        let mut builder = walk_builder(path, ignore_git);
        builder.threads(threads);
        return scan_directory_adaptive(path, || builder.build_parallel(), sink, verbose);
    }

    if threads <= 1 {
        return scan_directory_serial(path, sink, ignore_git, verbose);
    }

    let root = path.to_path_buf();
    let failed = Arc::new(AtomicBool::new(false));
    let mut builder = walk_builder(path, ignore_git);
    builder.threads(threads);
    let walker = builder.build_parallel();

    walker.run(|| {
        let root = root.clone();
        let mut worker = worker_batch(Arc::clone(&sink), verbose, Arc::clone(&failed));

        Box::new(move |entry| visit_entry(entry, &root, &mut worker))
    });
    scan_result(failed.load(Ordering::Relaxed))
}

pub fn scan_file(path: &Path, sink: &Sink, verbose: bool) -> io::Result<()> {
    let mut batch = Batch::default();
    if let Some(file) = count::parse_file(path, verbose)? {
        batch.add(file);
    }
    sink.dump(&mut batch);
    Ok(())
}

fn scan_directory_adaptive(
    root: &Path,
    build_walker: impl FnOnce() -> WalkParallel,
    sink: Arc<Sink>,
    verbose: bool,
) -> io::Result<()> {
    let state = Arc::new(AdaptiveState {
        large: AtomicBool::new(false),
        pending: Mutex::new(Vec::new()),
    });
    let failed = Arc::new(AtomicBool::new(false));
    let root = root.to_path_buf();
    let walker = build_walker();

    walker.run(|| {
        let mut worker = AdaptiveScan {
            root: root.clone(),
            files: worker_batch(Arc::clone(&sink), verbose, Arc::clone(&failed)),
            state: Arc::clone(&state),
            pending: Vec::with_capacity(PATH_BATCH_SIZE),
        };

        Box::new(move |entry| visit_adaptive(&mut worker, entry))
    });

    let scan_failed = if state.large.load(Ordering::Acquire) {
        false
    } else {
        let files = std::mem::take(&mut *state.pending.lock().unwrap());
        !scan_file_list(files, sink, verbose)
    };
    scan_result(scan_failed || failed.load(Ordering::Relaxed))
}

struct AdaptiveState {
    large: AtomicBool,
    pending: Mutex<Vec<PathBuf>>,
}

struct AdaptiveScan {
    root: PathBuf,
    files: WorkerBatch,
    state: Arc<AdaptiveState>,
    pending: Vec<PathBuf>,
}

fn visit_adaptive(scan: &mut AdaptiveScan, entry: Result<DirEntry, Error>) -> WalkState {
    let Some(entry) = file_entry(entry, &scan.root, &scan.files.failed) else {
        return WalkState::Continue;
    };

    if scan.state.large.load(Ordering::Acquire) {
        flush_pending(scan);
        process_file(&mut scan.files, entry.path());
        return WalkState::Continue;
    }

    scan.pending.push(entry.into_path());
    if scan.pending.len() == PATH_BATCH_SIZE {
        flush_pending(scan);
    }
    WalkState::Continue
}

fn flush_pending(scan: &mut AdaptiveScan) {
    if scan.pending.is_empty() {
        return;
    }

    let mut pending = scan.state.pending.lock().unwrap();
    if scan.state.large.load(Ordering::Relaxed) {
        drop(pending);
        for path in scan.pending.drain(..) {
            process_file(&mut scan.files, &path);
        }
        return;
    }

    pending.append(&mut scan.pending);
    if pending.len() > ADAPTIVE_SMALL_FILE_LIMIT {
        scan.state.large.store(true, Ordering::Release);
    }
}

impl Drop for AdaptiveScan {
    fn drop(&mut self) {
        flush_pending(self);
        if !self.state.large.load(Ordering::Acquire) {
            return;
        }

        loop {
            let path = self.state.pending.lock().unwrap().pop();
            let Some(path) = path else { break };
            process_file(&mut self.files, &path);
        }
    }
}

fn scan_file_list(files: Vec<PathBuf>, sink: Arc<Sink>, verbose: bool) -> bool {
    let failed = Arc::new(AtomicBool::new(false));
    let mut worker = worker_batch(sink, verbose, Arc::clone(&failed));

    for path in files {
        process_file(&mut worker, &path);
    }

    flush_batch(&mut worker);
    !failed.load(Ordering::Relaxed)
}

fn scan_directory_serial(
    path: &Path,
    sink: Arc<Sink>,
    ignore_git: bool,
    verbose: bool,
) -> io::Result<()> {
    let failed = Arc::new(AtomicBool::new(false));
    let root = path.to_path_buf();
    let mut worker = worker_batch(sink, verbose, Arc::clone(&failed));

    for entry in walk_builder(path, ignore_git).build() {
        visit_entry(entry, &root, &mut worker);
    }
    scan_result(failed.load(Ordering::Relaxed))
}

fn scan_result(failed: bool) -> io::Result<()> {
    if failed {
        Err(io::Error::other("directory scan was incomplete"))
    } else {
        Ok(())
    }
}

fn walk_builder(path: &Path, ignore_git: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(path);
    builder
        .hidden(false)
        .filter_entry(|entry| {
            !entry.file_type().is_some_and(|kind| kind.is_dir()) || entry.file_name() != ".git"
        })
        .git_ignore(ignore_git)
        .git_global(ignore_git)
        .git_exclude(ignore_git)
        .parents(ignore_git);
    builder
}

struct WorkerBatch {
    sink: Arc<Sink>,
    batch: Batch,
    verbose: bool,
    failed: Arc<AtomicBool>,
    buffer: Vec<u8>,
}

fn worker_batch(sink: Arc<Sink>, verbose: bool, failed: Arc<AtomicBool>) -> WorkerBatch {
    WorkerBatch {
        sink,
        batch: Batch::default(),
        verbose,
        failed,
        buffer: count::read_buffer(),
    }
}

fn visit_entry(entry: Result<DirEntry, Error>, root: &Path, worker: &mut WorkerBatch) -> WalkState {
    let Some(entry) = file_entry(entry, root, &worker.failed) else {
        return WalkState::Continue;
    };

    process_file(worker, entry.path());
    WalkState::Continue
}

fn file_entry(
    entry: Result<DirEntry, Error>,
    root: &Path,
    failed: &AtomicBool,
) -> Option<DirEntry> {
    let entry = match entry {
        Ok(entry) => entry,
        Err(err) => {
            eprintln!("failed to read directory entry: {err}");
            failed.store(true, Ordering::Relaxed);
            return None;
        }
    };

    if entry.path() == root || !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return None;
    }
    Some(entry)
}

fn process_file(state: &mut WorkerBatch, path: &Path) {
    match count::parse_file_buffered(path, state.verbose, &mut state.buffer) {
        Ok(Some(stats)) => {
            state.batch.add(stats);
            if state.batch.files() >= FLUSH_EVERY_FILES {
                flush_batch(state);
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("failed to read file {}: {error}", path.display());
            state.failed.store(true, Ordering::Relaxed);
        }
    }
}

fn flush_batch(state: &mut WorkerBatch) {
    state.sink.dump(&mut state.batch);
}

impl Drop for WorkerBatch {
    fn drop(&mut self) {
        flush_batch(self);
    }
}
