use serde::Serialize;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const BUFFER_SIZE: usize = 64 * 1024;
static TRACE: OnceLock<Recorder> = OnceLock::new();

thread_local! {
    static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) };
}

struct Recorder {
    path: PathBuf,
    session: u128,
    origin: Instant,
    writer: Mutex<BufWriter<File>>,
    next_thread: AtomicU64,
    failed: AtomicBool,
}

struct Local {
    thread: u64,
    sequence: u64,
    current_file: Option<String>,
    bytes: Vec<u8>,
}

impl Local {
    fn new(recorder: &Recorder) -> Self {
        Self {
            thread: recorder.next_thread.fetch_add(1, Ordering::Relaxed),
            sequence: 0,
            current_file: None,
            bytes: Vec::with_capacity(BUFFER_SIZE),
        }
    }

    fn flush(&mut self, recorder: &Recorder) {
        if self.bytes.is_empty() {
            return;
        }
        let result = recorder.writer.lock().unwrap().write_all(&self.bytes);
        if result.is_err() {
            recorder.failed.store(true, Ordering::Relaxed);
        }
        self.bytes.clear();
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        if let Some(recorder) = TRACE.get() {
            self.flush(recorder);
        }
    }
}

#[derive(Serialize)]
struct Record<'a> {
    session: u128,
    thread: u64,
    sequence: u64,
    wall_unix_ns: u128,
    elapsed_ns: u128,
    clock_count: u64,
    event: &'a str,
    path: Option<String>,
    detail: Value,
}

pub fn start() -> io::Result<PathBuf> {
    let path = std::env::current_dir()?.join(".tallydebug");
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let path = path.canonicalize()?;
    let session = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    TRACE
        .set(Recorder {
            path: path.clone(),
            session,
            origin: Instant::now(),
            writer: Mutex::new(BufWriter::new(file)),
            next_thread: AtomicU64::new(1),
            failed: AtomicBool::new(false),
        })
        .map_err(|_| io::Error::other("trace already started"))?;
    event(
        "session_start",
        None,
        json!({
            "pid": std::process::id(),
            "clock": clock_name(),
            "format": "tally-jsonl-v1"
        }),
    );
    Ok(path)
}

pub fn enabled() -> bool {
    TRACE.get().is_some()
}

pub fn is_output_path(path: &Path) -> bool {
    TRACE.get().is_some_and(|recorder| recorder.path == path)
}

pub fn event(name: &'static str, path: Option<&Path>, detail: Value) {
    let Some(recorder) = TRACE.get() else {
        return;
    };
    let wall_unix_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos());
    let clock_count = clock_count(recorder);
    let elapsed_ns = recorder.origin.elapsed().as_nanos();
    LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        let local = local.get_or_insert_with(|| Local::new(recorder));
        local.sequence += 1;
        let record = Record {
            session: recorder.session,
            thread: local.thread,
            sequence: local.sequence,
            wall_unix_ns,
            elapsed_ns,
            clock_count,
            event: name,
            path: path
                .map(|path| path.display().to_string())
                .or_else(|| local.current_file.clone()),
            detail,
        };
        if serde_json::to_writer(&mut local.bytes, &record).is_err() {
            recorder.failed.store(true, Ordering::Relaxed);
        }
        local.bytes.push(b'\n');
        if local.bytes.len() >= BUFFER_SIZE {
            local.flush(recorder);
        }
    });
}

pub struct FileContext {
    previous: Option<String>,
}

pub fn file_context(path: &Path) -> Option<FileContext> {
    let recorder = TRACE.get()?;
    LOCAL.with(|local| {
        let mut local = local.borrow_mut();
        let local = local.get_or_insert_with(|| Local::new(recorder));
        Some(FileContext {
            previous: local.current_file.replace(path.display().to_string()),
        })
    })
}

impl Drop for FileContext {
    fn drop(&mut self) {
        LOCAL.with(|local| {
            if let Some(local) = local.borrow_mut().as_mut() {
                local.current_file = self.previous.take();
            }
        });
    }
}

pub struct Span {
    name: &'static str,
    path: Option<PathBuf>,
    start: Instant,
    clock_start: u64,
}

pub fn span(name: &'static str, path: Option<&Path>) -> Option<Span> {
    let recorder = TRACE.get()?;
    event("span_start", path, json!({ "name": name }));
    Some(Span {
        name,
        path: path.map(Path::to_path_buf),
        start: Instant::now(),
        clock_start: clock_count(recorder),
    })
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(recorder) = TRACE.get() else {
            return;
        };
        event(
            "span_end",
            self.path.as_deref(),
            json!({
                "name": self.name,
                "duration_ns": self.start.elapsed().as_nanos(),
                "clock_delta": clock_count(recorder).wrapping_sub(self.clock_start)
            }),
        );
    }
}

pub fn finish() -> io::Result<PathBuf> {
    event("session_end", None, json!({}));
    let recorder = TRACE
        .get()
        .ok_or_else(|| io::Error::other("trace not started"))?;
    LOCAL.with(|local| {
        if let Some(local) = local.borrow_mut().as_mut() {
            local.flush(recorder);
        }
    });
    recorder.writer.lock().unwrap().flush()?;
    if recorder.failed.load(Ordering::Relaxed) {
        return Err(io::Error::other("failed to write complete trace"));
    }
    Ok(recorder.path.clone())
}

#[cfg(target_os = "macos")]
fn clock_count(_: &Recorder) -> u64 {
    unsafe extern "C" {
        fn mach_absolute_time() -> u64;
    }
    unsafe { mach_absolute_time() }
}

#[cfg(not(target_os = "macos"))]
fn clock_count(recorder: &Recorder) -> u64 {
    recorder.origin.elapsed().as_nanos() as u64
}

#[cfg(target_os = "macos")]
fn clock_name() -> &'static str {
    "mach_absolute_time"
}

#[cfg(not(target_os = "macos"))]
fn clock_name() -> &'static str {
    "elapsed_nanoseconds"
}
