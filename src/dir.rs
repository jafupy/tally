mod crawler;

use crate::file::{self, Batch};
use std::io;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
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
    crawler::scan(path, ignore_git, threads, adaptive_threads, sink, verbose)
}

fn scan_result(failed: bool) -> io::Result<()> {
    if failed {
        Err(io::Error::other("directory scan was incomplete"))
    } else {
        Ok(())
    }
}

struct ScanWorker {
    sink: Arc<file::Sink>,
    batch: Batch,
    verbose: bool,
    failed: Arc<AtomicBool>,
    buffer: Vec<u8>,
}

impl ScanWorker {
    fn visit_path(&mut self, path: &Path) {
        let result = file::parse_file_buffered(path, self.verbose, &mut self.buffer);
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
            let sink = file::Sink::new();
            scan_directory(&root, Arc::clone(&sink), ignore_git, 1, false, true).unwrap();
            assert_eq!(sink.snapshot().all.files, 2);
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
