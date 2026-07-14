use crate::file::{self, Batch};
use ignore::{DirEntry, Error, WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FLUSH_EVERY_FILES: u64 = 512;
const ADAPTIVE_SMALL_FILE_LIMIT: usize = 128;

pub fn scan_directory(
    path: &Path,
    sink: Arc<file::Sink>,
    ignore_git: bool,
    threads: usize,
    adaptive_threads: bool,
    verbose: bool,
) {
    if adaptive_threads && threads > 1 {
        match probe_small_directory(path, ignore_git) {
            DirectoryProbe::Small { files, errors } => {
                for err in errors {
                    eprintln!("failed to read directory entry: {err}");
                }
                scan_file_list(files, sink, verbose);
                return;
            }
            DirectoryProbe::Large => {}
        }
    }

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

enum DirectoryProbe {
    Small {
        files: Vec<PathBuf>,
        errors: Vec<String>,
    },
    Large,
}

fn probe_small_directory(path: &Path, ignore_git: bool) -> DirectoryProbe {
    let mut files = Vec::new();
    let mut errors = Vec::new();

    for entry in walk_builder(path, ignore_git).build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors.push(err.to_string());
                continue;
            }
        };

        if entry.path() == path || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }

        files.push(entry.path().to_path_buf());
        if files.len() > ADAPTIVE_SMALL_FILE_LIMIT {
            return DirectoryProbe::Large;
        }
    }

    DirectoryProbe::Small { files, errors }
}

fn scan_file_list(files: Vec<PathBuf>, sink: Arc<file::Sink>, verbose: bool) {
    let mut batch = Batch::default();

    for path in files {
        match file::parse_file(&path, verbose) {
            Ok(Some(stats)) => batch.add(stats),
            Ok(None) => continue,
            Err(error) => {
                eprintln!("failed to read file {}: {error}", path.display());
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

        match file::parse_file(entry.path(), self.verbose) {
            Ok(Some(stats)) => {
                self.batch.add(stats);
                if self.batch.files() >= FLUSH_EVERY_FILES {
                    self.flush();
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!("failed to read file {}: {error}", entry.path().display()),
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
