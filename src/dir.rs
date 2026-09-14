mod crawler;

pub(crate) use crawler::ScanReport;

use crate::file::{self, Batch};
use ignore::Error;
use ignore::overrides::Override;
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
    debug: bool,
    overrides: Override,
) -> io::Result<ScanReport> {
    crawler::scan(
        path,
        ignore_git,
        threads,
        adaptive_threads,
        sink,
        debug,
        overrides,
    )
}

fn scan_result(failed: bool) -> io::Result<()> {
    if failed {
        Err(io::Error::other("directory scan was incomplete"))
    } else {
        Ok(())
    }
}

fn report_walk_error(error: &Error) -> bool {
    if error
        .io_error()
        .is_some_and(|error| error.kind() == io::ErrorKind::PermissionDenied)
    {
        eprintln!("skipping inaccessible path: {error}");
        false
    } else {
        eprintln!("failed to read directory entry: {error}");
        true
    }
}

fn report_file_error(path: &Path, error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::PermissionDenied {
        eprintln!("skipping inaccessible file {}: {error}", path.display());
        false
    } else {
        eprintln!("failed to read file {}: {error}", path.display());
        true
    }
}

struct ScanWorker {
    sink: Arc<file::Sink>,
    batch: Batch,
    debug: bool,
    failed: Arc<AtomicBool>,
    buffer: Vec<u8>,
}

impl ScanWorker {
    fn visit_path(&mut self, path: &Path) {
        #[cfg(feature = "debug")]
        let _file_context = crate::trace::file_context(path);
        let _span = trace_span!("visit_path", Some(path));
        let result = file::parse_file_buffered(path, self.debug, &mut self.buffer);
        match result {
            Ok(Some(stats)) => {
                self.batch.add(stats);
                if self.batch.files() >= FLUSH_EVERY_FILES {
                    self.flush();
                }
            }
            Ok(None) => {
                trace_event!("file_skipped", Some(path), serde_json::json!({}));
            }
            Err(error) => {
                trace_event!(
                    "file_error",
                    Some(path),
                    serde_json::json!({"error": error.to_string()})
                );
                if report_file_error(path, &error) {
                    self.failed.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    fn flush(&mut self) {
        let _span = trace_span!("worker_flush", None);
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
            scan_directory(
                &root,
                Arc::clone(&sink),
                ignore_git,
                1,
                false,
                true,
                Override::empty(),
            )
            .unwrap();
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

        scan_directory(
            &root,
            Arc::clone(&sink),
            false,
            4,
            true,
            true,
            Override::empty(),
        )
        .unwrap();

        let summary = sink.snapshot();
        assert_eq!(summary.all.files, 9);
        #[cfg(feature = "debug")]
        assert_eq!(summary.unknown_formats.len(), 9);
        #[cfg(feature = "debug")]
        assert!(summary.unknown_formats.iter().all(|(_, files)| *files == 1));

        fs::remove_dir_all(root).unwrap();
    }
}
