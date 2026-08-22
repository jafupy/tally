use crate::file::{self, Batch};
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
    sink: Arc<file::Sink>,
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
        let mut worker = ScanWorker {
            root: root.clone(),
            sink: Arc::clone(&sink),
            batch: Batch::default(),
            verbose,
            failed: Arc::clone(&failed),
            buffer: file::read_buffer(),
        };

        Box::new(move |entry| worker.visit(entry))
    });
    scan_result(failed.load(Ordering::Relaxed))
}

fn scan_directory_adaptive(
    root: &Path,
    build_walker: impl FnOnce() -> WalkParallel,
    sink: Arc<file::Sink>,
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
        let mut worker = AdaptiveWorker {
            scan: ScanWorker {
                root: root.clone(),
                sink: Arc::clone(&sink),
                batch: Batch::default(),
                verbose,
                failed: Arc::clone(&failed),
                buffer: file::read_buffer(),
            },
            state: Arc::clone(&state),
            pending: Vec::with_capacity(PATH_BATCH_SIZE),
        };

        Box::new(move |entry| worker.visit(entry))
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

struct AdaptiveWorker {
    scan: ScanWorker,
    state: Arc<AdaptiveState>,
    pending: Vec<PathBuf>,
}

impl AdaptiveWorker {
    fn visit(&mut self, entry: Result<DirEntry, Error>) -> WalkState {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("failed to read directory entry: {err}");
                self.scan.failed.store(true, Ordering::Relaxed);
                return WalkState::Continue;
            }
        };

        if entry.path() == self.scan.root || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            return WalkState::Continue;
        }

        if self.state.large.load(Ordering::Acquire) {
            self.flush_pending();
            self.scan.visit_path(entry.path());
            return WalkState::Continue;
        }

        self.pending.push(entry.into_path());
        if self.pending.len() == PATH_BATCH_SIZE {
            self.flush_pending();
        }
        WalkState::Continue
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let mut pending = self.state.pending.lock().unwrap();
        if self.state.large.load(Ordering::Relaxed) {
            drop(pending);
            for path in self.pending.drain(..) {
                self.scan.visit_path(&path);
            }
            return;
        }

        pending.append(&mut self.pending);
        if pending.len() > ADAPTIVE_SMALL_FILE_LIMIT {
            self.state.large.store(true, Ordering::Release);
        }
    }
}

impl Drop for AdaptiveWorker {
    fn drop(&mut self) {
        self.flush_pending();
        if !self.state.large.load(Ordering::Acquire) {
            return;
        }

        loop {
            let path = self.state.pending.lock().unwrap().pop();
            let Some(path) = path else { break };
            self.scan.visit_path(&path);
        }
    }
}

fn scan_file_list(files: Vec<PathBuf>, sink: Arc<file::Sink>, verbose: bool) -> bool {
    let mut batch = Batch::default();
    let mut buffer = file::read_buffer();
    let mut succeeded = true;

    for path in files {
        match file::parse_file_buffered(&path, verbose, &mut buffer) {
            Ok(Some(stats)) => batch.add(stats),
            Ok(None) => continue,
            Err(error) => {
                eprintln!("failed to read file {}: {error}", path.display());
                succeeded = false;
                continue;
            }
        }

        if batch.files() >= FLUSH_EVERY_FILES {
            sink.record_progress(batch.files());
            sink.add_batch(&mut batch);
        }
    }

    sink.record_progress(batch.files());
    sink.add_batch(&mut batch);
    succeeded
}

fn scan_directory_serial(
    path: &Path,
    sink: Arc<file::Sink>,
    ignore_git: bool,
    verbose: bool,
) -> io::Result<()> {
    let failed = Arc::new(AtomicBool::new(false));
    let mut worker = ScanWorker {
        root: path.to_path_buf(),
        sink,
        batch: Batch::default(),
        verbose,
        failed: Arc::clone(&failed),
        buffer: file::read_buffer(),
    };

    for entry in walk_builder(path, ignore_git).build() {
        worker.visit(entry);
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

struct ScanWorker {
    root: PathBuf,
    sink: Arc<file::Sink>,
    batch: Batch,
    verbose: bool,
    failed: Arc<AtomicBool>,
    buffer: Vec<u8>,
}

impl ScanWorker {
    fn visit(&mut self, entry: Result<DirEntry, Error>) -> WalkState {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("failed to read directory entry: {err}");
                self.failed.store(true, Ordering::Relaxed);
                return WalkState::Continue;
            }
        };

        if entry.path() == self.root || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            return WalkState::Continue;
        }

        self.visit_path(entry.path());
        WalkState::Continue
    }

    fn visit_path(&mut self, path: &Path) {
        match file::parse_file_buffered(path, self.verbose, &mut self.buffer) {
            Ok(Some(stats)) => {
                self.batch.add(stats);
                if self.batch.files() >= FLUSH_EVERY_FILES {
                    self.flush();
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("failed to read file {}: {error}", path.display());
                self.failed.store(true, Ordering::Relaxed);
            }
        }
    }

    fn flush(&mut self) {
        self.sink.record_progress(self.batch.files());
        self.sink.add_batch(&mut self.batch);
    }
}

impl Drop for ScanWorker {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::AtomicUsize,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn walks_hidden_entries_but_not_git_directories() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tally-walk-{unique}"));
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".dotfile"), b"dotfile").unwrap();
        fs::write(root.join(".hidden/source.rs"), b"fn main() {}\n").unwrap();
        fs::write(root.join(".git/config"), b"config").unwrap();
        fs::write(root.join(".git/objects/data"), b"object").unwrap();

        for ignore_git in [true, false] {
            let paths = walk_builder(&root, ignore_git)
                .build()
                .map(|entry| entry.unwrap().into_path())
                .collect::<Vec<_>>();

            assert!(paths.contains(&root.join(".dotfile")));
            assert!(paths.contains(&root.join(".hidden/source.rs")));
            assert!(!paths.iter().any(|path| path.starts_with(root.join(".git"))));
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adaptive_handoff_processes_every_discovered_file_once() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tally-adaptive-{unique}"));
        fs::create_dir(&root).unwrap();
        for index in 0..=ADAPTIVE_SMALL_FILE_LIMIT {
            fs::write(root.join(format!("source.unique-{index}")), b"one line\n").unwrap();
        }

        let sink = file::Sink::new();

        scan_directory(&root, Arc::clone(&sink), false, 4, true, true).unwrap();

        let summary = sink.snapshot();
        assert_eq!(summary.all.files, (ADAPTIVE_SMALL_FILE_LIMIT + 1) as u64);
        assert_eq!(summary.unknown_formats.len(), ADAPTIVE_SMALL_FILE_LIMIT + 1);
        assert!(summary.unknown_formats.iter().all(|(_, files)| *files == 1));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adaptive_large_directory_consumes_one_walk() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tally-single-walk-{unique}"));
        fs::create_dir(&root).unwrap();
        for index in 0..=ADAPTIVE_SMALL_FILE_LIMIT {
            fs::write(root.join(format!("source-{index}.rs")), b"one line\n").unwrap();
        }

        let walker_builds = AtomicUsize::new(0);
        scan_directory_adaptive(
            &root,
            || {
                walker_builds.fetch_add(1, Ordering::Relaxed);
                let mut builder = walk_builder(&root, false);
                builder.threads(4);
                builder.build_parallel()
            },
            file::Sink::new(),
            false,
        )
        .unwrap();

        assert_eq!(walker_builds.load(Ordering::Relaxed), 1);

        fs::remove_dir_all(root).unwrap();
    }
}
