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

struct CounterValues {
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

impl CounterValues {
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

pub(crate) struct Counters {
    values: Option<CounterValues>,
}

impl Counters {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            values: enabled.then(CounterValues::new),
        }
    }

    pub(crate) fn directory_queued(&self, depth: usize) {
        if let Some(values) = &self.values {
            values
                .peak_directory_queue
                .fetch_max(depth, Ordering::Relaxed);
        }
    }

    pub(crate) fn files_queued(&self, count: usize, depth: usize) {
        if let Some(values) = &self.values {
            values.files_queued.fetch_add(count, Ordering::Relaxed);
            values.file_batches.fetch_add(1, Ordering::Relaxed);
            values.peak_file_queue.fetch_max(depth, Ordering::Relaxed);
        }
    }

    pub(crate) fn directory_listed(&self) {
        if let Some(values) = &self.values {
            values.directories_listed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn worker(&self) -> WorkerStats<'_> {
        WorkerStats {
            values: self.values.as_ref(),
            counting: Duration::ZERO,
            listing: Duration::ZERO,
            idle_yields: 0,
        }
    }

    pub(crate) fn snapshot(&self, workers: usize, worker_limit: usize) -> ScanReport {
        self.values.as_ref().map_or(
            ScanReport {
                workers,
                worker_limit,
                ..ScanReport::default()
            },
            |values| values.snapshot(workers, worker_limit),
        )
    }
}

pub(crate) struct WorkerStats<'a> {
    values: Option<&'a CounterValues>,
    counting: Duration,
    listing: Duration,
    idle_yields: u64,
}

impl WorkerStats<'_> {
    pub(crate) fn counting(&mut self) -> PhaseTimer<'_> {
        PhaseTimer {
            start: self.values.map(|_| std::time::Instant::now()),
            total: &mut self.counting,
        }
    }

    pub(crate) fn listing(&mut self) -> PhaseTimer<'_> {
        PhaseTimer {
            start: self.values.map(|_| std::time::Instant::now()),
            total: &mut self.listing,
        }
    }

    pub(crate) fn yielded(&mut self) {
        if self.values.is_some() {
            self.idle_yields += 1;
        }
    }
}

impl Drop for WorkerStats<'_> {
    fn drop(&mut self) {
        if let Some(values) = self.values {
            values
                .counting_nanos
                .fetch_add(self.counting.as_nanos() as u64, Ordering::Relaxed);
            values
                .listing_nanos
                .fetch_add(self.listing.as_nanos() as u64, Ordering::Relaxed);
            values
                .idle_yields
                .fetch_add(self.idle_yields, Ordering::Relaxed);
        }
        trace_event!(worker_exit, None, self.idle_yields);
    }
}

pub(crate) struct PhaseTimer<'a> {
    start: Option<std::time::Instant>,
    total: &'a mut Duration,
}

impl Drop for PhaseTimer<'_> {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            *self.total += start.elapsed();
        }
    }
}
