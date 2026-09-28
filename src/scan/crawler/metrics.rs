use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct ScanReport {
    pub workers: usize,
    pub worker_limit: usize,
    pub directories_listed: usize,
    pub files_queued: usize,
    pub file_batches: usize,
    pub peak_directory_queue: usize,
    pub peak_file_queue: usize,
    pub directory_spills: usize,
    pub file_spills: usize,
    pub counting_time: Duration,
    pub listing_time: Duration,
    pub idle_yields: u64,
}

pub(super) struct Counters {
    pub directories_listed: AtomicUsize,
    pub files_queued: AtomicUsize,
    pub file_batches: AtomicUsize,
    pub peak_directory_queue: AtomicUsize,
    pub peak_file_queue: AtomicUsize,
    pub directory_spills: AtomicUsize,
    pub file_spills: AtomicUsize,
    pub counting_nanos: AtomicU64,
    pub listing_nanos: AtomicU64,
    pub idle_yields: AtomicU64,
}

impl Counters {
    pub fn new() -> Self {
        Self {
            directories_listed: AtomicUsize::new(0),
            files_queued: AtomicUsize::new(0),
            file_batches: AtomicUsize::new(0),
            peak_directory_queue: AtomicUsize::new(1),
            peak_file_queue: AtomicUsize::new(0),
            directory_spills: AtomicUsize::new(0),
            file_spills: AtomicUsize::new(0),
            counting_nanos: AtomicU64::new(0),
            listing_nanos: AtomicU64::new(0),
            idle_yields: AtomicU64::new(0),
        }
    }

    pub fn snapshot(&self, workers: usize, worker_limit: usize) -> ScanReport {
        let load = |counter: &AtomicUsize| counter.load(Ordering::Relaxed);
        ScanReport {
            workers,
            worker_limit,
            directories_listed: load(&self.directories_listed),
            files_queued: load(&self.files_queued),
            file_batches: load(&self.file_batches),
            peak_directory_queue: load(&self.peak_directory_queue),
            peak_file_queue: load(&self.peak_file_queue),
            directory_spills: load(&self.directory_spills),
            file_spills: load(&self.file_spills),
            counting_time: Duration::from_nanos(self.counting_nanos.load(Ordering::Relaxed)),
            listing_time: Duration::from_nanos(self.listing_nanos.load(Ordering::Relaxed)),
            idle_yields: self.idle_yields.load(Ordering::Relaxed),
        }
    }
}
