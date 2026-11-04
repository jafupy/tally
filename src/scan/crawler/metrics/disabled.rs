pub(crate) type ScanReport = ();
pub(crate) struct Counters;
pub(crate) struct PhaseTimer;
pub(crate) struct WorkerStats<'a>(std::marker::PhantomData<&'a Counters>);

impl Counters {
    pub(crate) fn new(_: bool) -> Self {
        Self
    }
    pub(crate) fn directory_queued(&self, _: usize) {}
    pub(crate) fn files_queued(&self, _: usize, _: usize) {}
    pub(crate) fn directory_listed(&self) {}
    pub(crate) fn worker(&self) -> WorkerStats<'_> {
        WorkerStats(std::marker::PhantomData)
    }
    pub(crate) fn snapshot(&self, _: usize, _: usize) -> ScanReport {}
}

impl WorkerStats<'_> {
    pub(crate) fn counting(&mut self) -> PhaseTimer {
        PhaseTimer
    }
    pub(crate) fn listing(&mut self) -> PhaseTimer {
        PhaseTimer
    }
    pub(crate) fn yielded(&mut self) {}
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {}
}
