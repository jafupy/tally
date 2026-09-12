use super::{ScanWorker, scan_result, walk_builder};
use crate::file::{self, Batch};
use crossbeam_channel::unbounded;
use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SHARDS_PER_PULLER: usize = 2;
const EXPANSIONS_PER_PULLER: usize = 4;
const FAST_START_PULLERS: usize = 3;
const CALIBRATION_INTERVAL: Duration = Duration::from_millis(400);
const MIN_SCALING_EFFICIENCY: f64 = 0.97;
const FILES_PER_JOB: usize = 128;

enum Job {
    Directory(std::path::PathBuf),
    Files(Vec<std::path::PathBuf>),
}

#[cfg(unix)]
fn cpu_seconds() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // getrusage writes the complete rusage value on success.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0.0;
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    user + system
}

#[cfg(not(unix))]
fn cpu_seconds() -> f64 {
    0.0
}

pub(super) fn scan(
    root: &Path,
    ignore_git: bool,
    max_pullers: usize,
    sink: Arc<file::Sink>,
    verbose: bool,
) -> io::Result<()> {
    let failed = Arc::new(AtomicBool::new(false));
    let mut scanner = ScanWorker {
        root: root.to_path_buf(),
        sink: Arc::clone(&sink),
        batch: Batch::default(),
        verbose,
        failed: Arc::clone(&failed),
        buffer: file::read_buffer(),
        completed_bytes: None,
        completed_files: None,
    };
    let mut frontier = VecDeque::from([root.to_path_buf()]);
    let mut file_jobs = VecDeque::new();
    let target_shards = max_pullers.saturating_mul(SHARDS_PER_PULLER);
    let max_expansions = max_pullers.saturating_mul(EXPANSIONS_PER_PULLER);

    for _ in 0..max_expansions {
        if frontier.len() >= target_shards {
            break;
        }
        let Some(dir) = frontier.pop_front() else {
            break;
        };
        let mut builder = walk_builder(&dir, ignore_git);
        if dir != root {
            builder.parents(true);
        }
        builder.max_depth(Some(1));
        let mut files = Vec::with_capacity(FILES_PER_JOB);
        for entry in builder.build() {
            match entry {
                Ok(entry)
                    if entry.path() != dir
                        && entry.file_type().is_some_and(|kind| kind.is_dir()) =>
                {
                    frontier.push_back(entry.into_path());
                }
                Ok(entry)
                    if entry.path() != dir
                        && entry.file_type().is_some_and(|kind| kind.is_file()) =>
                {
                    files.push(entry.into_path());
                    if files.len() == FILES_PER_JOB {
                        file_jobs.push_back(Job::Files(std::mem::take(&mut files)));
                    }
                }
                other => {
                    scanner.visit(other);
                }
            }
        }
        if !files.is_empty() {
            file_jobs.push_back(Job::Files(files));
        }
    }
    drop(scanner);

    if frontier.is_empty() && file_jobs.is_empty() {
        return scan_result(failed.load(Ordering::Relaxed));
    }

    let (shard_sender, shards) = unbounded();
    let (done_sender, done) = unbounded();
    let completed_bytes = Arc::new(AtomicU64::new(0));
    let completed_files = Arc::new(AtomicU64::new(0));
    let shard_count = frontier.len() + file_jobs.len();
    for job in file_jobs
        .into_iter()
        .chain(frontier.into_iter().map(Job::Directory))
    {
        shard_sender.send(job).unwrap();
    }

    let mut pullers: Vec<JoinHandle<()>> = Vec::new();
    let mut completed = 0;
    let mut sampled_at = Instant::now();
    let mut sampled_cpu = cpu_seconds();
    let mut sampled_bytes = 0;
    let mut sampled_files = 0;
    let mut trial_rate = None;
    let mut may_spawn = false;
    let mut scaling_finished = false;
    while completed < shard_count {
        let fast_start = pullers.len() < FAST_START_PULLERS.min(max_pullers);
        let should_spawn = !shards.is_empty()
            && pullers.len() < max_pullers
            && (fast_start || (!scaling_finished && may_spawn));
        if should_spawn {
            may_spawn = false;
            let shards = shards.clone();
            let sink = Arc::clone(&sink);
            let failed = Arc::clone(&failed);
            let done_sender = done_sender.clone();
            let completed_bytes = Arc::clone(&completed_bytes);
            let completed_files = Arc::clone(&completed_files);
            let root = root.to_path_buf();
            pullers.push(std::thread::spawn(move || {
                let mut scanner = ScanWorker {
                    root,
                    sink,
                    batch: Batch::default(),
                    verbose,
                    failed,
                    buffer: file::read_buffer(),
                    completed_bytes: Some(completed_bytes),
                    completed_files: Some(completed_files),
                };
                while let Ok(job) = shards.recv() {
                    match job {
                        Job::Directory(shard) => {
                            let mut builder = walk_builder(&shard, ignore_git);
                            if shard != scanner.root {
                                builder.parents(true);
                            }
                            for entry in builder.build() {
                                scanner.visit(entry);
                            }
                        }
                        Job::Files(files) => {
                            for path in files {
                                scanner.visit_path(&path);
                            }
                        }
                    }
                    let _ = done_sender.send(());
                }
            }));
        }
        if done.recv_timeout(Duration::from_millis(1)).is_ok() {
            completed += 1;
        }

        if !scaling_finished
            && pullers.len() >= FAST_START_PULLERS.min(max_pullers)
            && sampled_at.elapsed() >= CALIBRATION_INTERVAL
        {
            let now = Instant::now();
            let bytes = completed_bytes.load(Ordering::Relaxed);
            let files = completed_files.load(Ordering::Relaxed);
            let elapsed = now.duration_since(sampled_at).as_secs_f64();
            let cpu = cpu_seconds();
            let effort = if cpu > sampled_cpu {
                cpu - sampled_cpu
            } else {
                elapsed
            };
            // Include a per-file cost so tiny files count as useful work too.
            let work = (bytes - sampled_bytes) as f64 + (files - sampled_files) as f64 * 4096.0;
            let rate = work / effort;
            sampled_at = now;
            sampled_cpu = cpu;
            sampled_bytes = bytes;
            sampled_files = files;

            if let Some(previous_rate) = trial_rate.take() {
                let required = previous_rate * MIN_SCALING_EFFICIENCY;
                if rate < required {
                    scaling_finished = true;
                }
            }
            if !scaling_finished && rate > 0.0 && !shards.is_empty() && pullers.len() < max_pullers
            {
                trial_rate = Some(rate);
                may_spawn = true;
            }
        }
    }
    drop(shard_sender);
    for puller in pullers {
        puller.join().unwrap();
    }
    scan_result(failed.load(Ordering::Relaxed))
}
