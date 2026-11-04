#[cfg(not(feature = "debug"))]
mod disabled;
#[cfg(feature = "debug")]
mod enabled;

#[cfg(not(feature = "debug"))]
pub(crate) use disabled::{Counters, ScanReport, WorkerStats};
#[cfg(feature = "debug")]
pub(crate) use enabled::{Counters, ScanReport, WorkerStats};
