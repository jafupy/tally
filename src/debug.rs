use crate::dir::ScanReport;
use crate::file::Summary;
use std::io::{self, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct CpuTime {
    user: Duration,
    system: Duration,
}

pub struct Timer {
    start: Instant,
    cpu_start: Option<CpuTime>,
}

pub struct Timing {
    wall: Duration,
    cpu: Option<CpuTime>,
}

impl Timer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
            cpu_start: cpu_time(),
        }
    }

    pub fn finish(self) -> Timing {
        Timing {
            wall: self.start.elapsed(),
            cpu: self.cpu_start.zip(cpu_time()).map(|(start, end)| CpuTime {
                user: end.user.saturating_sub(start.user),
                system: end.system.saturating_sub(start.system),
            }),
        }
    }
}

pub fn print(timing: Timing, scan: Option<ScanReport>, summary: &Summary) -> io::Result<()> {
    let mut error = io::stderr().lock();
    writeln!(error, "\nDebug scan:")?;
    writeln!(
        error,
        "  Wall time:             {:.3} ms",
        timing.wall.as_secs_f64() * 1e3
    )?;
    if let Some(cpu) = timing.cpu {
        let total = cpu.user + cpu.system;
        let cores = total.as_secs_f64() / timing.wall.as_secs_f64();
        writeln!(
            error,
            "  User CPU:              {:.3} ms",
            cpu.user.as_secs_f64() * 1e3
        )?;
        writeln!(
            error,
            "  System CPU:            {:.3} ms",
            cpu.system.as_secs_f64() * 1e3
        )?;
        writeln!(
            error,
            "  Total CPU:             {:.3} ms",
            total.as_secs_f64() * 1e3
        )?;
        writeln!(
            error,
            "  CPU / wall:            {:.1}% ({cores:.2} core equivalents)",
            cores * 100.0
        )?;
    } else {
        writeln!(error, "  CPU time:              unavailable")?;
    }
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    writeln!(error, "  Logical CPUs available: {available}")?;
    if let Some(scan) = scan {
        writeln!(
            error,
            "  Workers started:       {} / {} limit",
            scan.workers, scan.worker_limit
        )?;
        writeln!(
            error,
            "  Directories listed:    {}",
            scan.directories_listed
        )?;
        writeln!(
            error,
            "  Files queued:          {} in {} batches",
            scan.files_queued, scan.file_batches
        )?;
        writeln!(
            error,
            "  Observed dir peak (jobs): {}",
            scan.peak_directory_queue
        )?;
        writeln!(
            error,
            "  Observed file peak (batches): {}",
            scan.peak_file_queue
        )?;
        writeln!(
            error,
            "  Queue overflows:       {} dirs, {} file batches",
            scan.directory_spills, scan.file_spills
        )?;
        writeln!(
            error,
            "  Counting worker time:  {:.3} ms",
            scan.counting_time.as_secs_f64() * 1e3
        )?;
        writeln!(
            error,
            "  Listing worker time:   {:.3} ms",
            scan.listing_time.as_secs_f64() * 1e3
        )?;
        writeln!(error, "  Idle worker yields:    {}", scan.idle_yields)?;
        writeln!(
            error,
            "  Worker times include I/O waits and overlap across threads."
        )?;
    } else {
        writeln!(error, "  Scanner:               main thread")?;
    }
    writeln!(error, "  Files counted:         {}", summary.all.files)?;
    writeln!(error, "  Lines counted:         {}", summary.all.lines)?;
    Ok(())
}

#[cfg(unix)]
fn cpu_time() -> Option<CpuTime> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    let duration = |time: libc::timeval| {
        Duration::from_secs(time.tv_sec as u64) + Duration::from_micros(time.tv_usec as u64)
    };
    Some(CpuTime {
        user: duration(usage.ru_utime),
        system: duration(usage.ru_stime),
    })
}

#[cfg(not(unix))]
fn cpu_time() -> Option<CpuTime> {
    None
}
