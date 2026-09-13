mod direct_dynamic;

use crate::file::{self, Batch};
use ignore::{DirEntry, Error, WalkBuilder, WalkState};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const FLUSH_EVERY_FILES: u64 = 512;

pub fn scan_directory(
    path: &Path,
    sink: Arc<file::Sink>,
    ignore_git: bool,
    threads: usize,
    adaptive_threads: bool,
    verbose: bool,
) -> io::Result<()> {
    if adaptive_threads && threads > 1 {
        // Sub-walks would also load .ignore rules above the requested root,
        // which -a's ordinary walk does not do.
        let ancestor_ignore = !ignore_git
            && path.canonicalize().ok().is_some_and(|root| {
                root.ancestors()
                    .skip(1)
                    .any(|dir| dir.join(".ignore").exists())
            });
        if ancestor_ignore {
            return scan_directory(path, sink, ignore_git, threads.min(3), false, verbose);
        }
        return direct_dynamic::scan(path, ignore_git, threads, sink, verbose);
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
            completed_bytes: None,
            completed_files: None,
        };

        Box::new(move |entry| worker.visit(entry))
    });
    scan_result(failed.load(Ordering::Relaxed))
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
        completed_bytes: None,
        completed_files: None,
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
        .parents(ignore_git)
        .require_git(false);
    builder
}

struct ScanWorker {
    root: PathBuf,
    sink: Arc<file::Sink>,
    batch: Batch,
    verbose: bool,
    failed: Arc<AtomicBool>,
    buffer: Vec<u8>,
    completed_bytes: Option<Arc<AtomicU64>>,
    completed_files: Option<Arc<AtomicU64>>,
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
        let result = match &self.completed_bytes {
            Some(bytes) => {
                file::parse_file_buffered_with_progress(path, self.verbose, &mut self.buffer, bytes)
            }
            None => file::parse_file_buffered(path, self.verbose, &mut self.buffer),
        };
        if let Some(files) = &self.completed_files {
            files.fetch_add(1, Ordering::Relaxed);
        }
        match result {
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
    fn adaptive_scan_processes_every_discovered_file_once() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tally-adaptive-{unique}"));
        fs::create_dir(&root).unwrap();
        for index in 0..9 {
            fs::write(root.join(format!("source.unique-{index}")), b"one line\n").unwrap();
        }

        let sink = file::Sink::new();

        scan_directory(&root, Arc::clone(&sink), false, 4, true, true).unwrap();

        let summary = sink.snapshot();
        assert_eq!(summary.all.files, 9);
        assert_eq!(summary.unknown_formats.len(), 9);
        assert!(summary.unknown_formats.iter().all(|(_, files)| *files == 1));

        fs::remove_dir_all(root).unwrap();
    }
}
