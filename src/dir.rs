use crate::file::{self, Batch};
use ignore::{DirEntry, Error, WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FLUSH_EVERY_FILES: u64 = 512;

pub fn scan_directory(
    path: &Path,
    sink: Arc<file::Sink>,
    ignore_git: bool,
    threads: usize,
    verbose: bool,
) {
    if threads <= 1 {
        scan_directory_serial(path, sink, ignore_git, verbose);
        return;
    }

    let root = path.to_path_buf();
    let mut builder = walk_builder(path, ignore_git);
    builder.threads(threads);
    let walker = builder.build_parallel();

    walker.run(|| {
        let mut worker = ScanWorker {
            root: root.clone(),
            sink: Arc::clone(&sink),
            batch: Batch::default(),
            verbose,
        };

        Box::new(move |entry| worker.visit(entry))
    });
}

fn scan_directory_serial(path: &Path, sink: Arc<file::Sink>, ignore_git: bool, verbose: bool) {
    let mut worker = ScanWorker {
        root: path.to_path_buf(),
        sink,
        batch: Batch::default(),
        verbose,
    };

    for entry in walk_builder(path, ignore_git).build() {
        worker.visit(entry);
    }
}

fn walk_builder(path: &Path, ignore_git: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(path);
    builder
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
}

impl ScanWorker {
    fn visit(&mut self, entry: Result<DirEntry, Error>) -> WalkState {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("failed to read directory entry: {err}");
                return WalkState::Continue;
            }
        };

        if entry.path() == self.root || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            return WalkState::Continue;
        }

        if let Some(stats) = file::parse_file(entry.path(), self.verbose) {
            self.batch.add(stats);

            if self.batch.files() >= FLUSH_EVERY_FILES {
                self.flush();
            }
        }

        WalkState::Continue
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
