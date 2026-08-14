use crate::file::{self, Batch};
use ignore::{DirEntry, Error, WalkBuilder, WalkState};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const FLUSH_EVERY_FILES: u64 = 512;
const ADAPTIVE_SMALL_FILE_LIMIT: usize = 128;

pub fn scan_directory(
    path: &Path,
    sink: Arc<file::Sink>,
    ignore_git: bool,
    threads: usize,
    adaptive_threads: bool,
    verbose: bool,
) -> io::Result<()> {
    if adaptive_threads && threads > 1 {
        match probe_small_directory(path, ignore_git) {
            DirectoryProbe::Small { files, errors } => {
                let mut failed = !errors.is_empty();
                for err in errors {
                    eprintln!("failed to read directory entry: {err}");
                }
                failed |= !scan_file_list(files, sink, verbose);
                return scan_result(failed);
            }
            DirectoryProbe::Large => {}
        }
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

        match file::parse_file_buffered(entry.path(), self.verbose, &mut self.buffer) {
            Ok(Some(stats)) => {
                self.batch.add(stats);
                if self.batch.files() >= FLUSH_EVERY_FILES {
                    self.flush();
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("failed to read file {}: {error}", entry.path().display());
                self.failed.store(true, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
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
}
