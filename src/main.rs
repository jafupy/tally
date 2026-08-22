mod diff;
mod dir;
mod file;
mod language;
mod output;
mod update;

use dir::scan_directory;
use file::{Batch, parse_file};
use ignore::overrides::{Override, OverrideBuilder};
use std::{
    io::{self, ErrorKind, IsTerminal},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

#[argue::parser(name = "tally", about = "Count and inspect a codebase")]
#[derive(Debug)]
struct Args {
    /// Print the version and check GitHub for updates.
    #[flag(short = 'V', long = "version")]
    version: bool,

    /// Include files ignored by gitignore rules.
    #[flag(short = 'a', long = "all")]
    all: bool,

    /// Number of worker threads. Defaults adaptively to up to 8 workers for directories and 1 for a file.
    #[option(short = 'j', long = "threads")]
    threads: Option<usize>,

    /// Print extra diagnostics, including unknown file formats.
    #[flag(short = 'v', long = "verbose")]
    verbose: bool,

    /// Output results as JSON.
    #[flag(long = "json")]
    json: bool,

    /// Count added and deleted lines since this git ref.
    #[option(long = "diff")]
    diff: Option<String>,

    /// Count only files known to git.
    #[flag(long = "tracked")]
    tracked: bool,

    /// Include paths matching this glob. May be repeated.
    #[option(long = "include")]
    include: Vec<String>,

    /// Exclude paths matching this glob. May be repeated.
    #[option(long = "exclude")]
    exclude: Vec<String>,

    /// Path to tally
    #[positional(default = ".")]
    path: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("tally: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args = parse_args();
    if args.version {
        update::check()?;
        return Ok(());
    }

    if args.path == Path::new("-") {
        return count_stdin(&args);
    }

    let metadata = std::fs::metadata(&args.path)?;
    let path_is_dir = metadata.is_dir();
    if !path_is_dir && !metadata.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} is not a regular file or directory", args.path.display()),
        ));
    }
    if !path_is_dir {
        std::fs::File::open(&args.path)?;
    }
    let threads = args.threads.unwrap_or_else(|| default_threads(path_is_dir));
    let adaptive_threads = args.threads.is_none() && path_is_dir;
    let verbose = args.verbose;
    let override_root = if path_is_dir {
        args.path.as_path()
    } else {
        args.path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    };
    let overrides = build_overrides(override_root, &args.include, &args.exclude)?;
    if let Some(reference) = &args.diff {
        return diff::count(&args.path, reference, &overrides, args.json);
    }
    let sink = file::Sink::new();
    let progress = std::io::stderr().is_terminal().then(|| {
        let (progress_done, done) = mpsc::channel();
        (progress_done, show_progress(Arc::clone(&sink), done))
    });

    if path_is_dir && args.tracked {
        let files = git_files(&args.path)?;
        parse_file_list(files, &args.path, &overrides, &sink, verbose)?;
    } else if path_is_dir {
        scan_directory(
            &args.path,
            Arc::clone(&sink),
            !args.all,
            threads,
            adaptive_threads,
            verbose,
            overrides,
        )?;
    } else if !args.tracked
        || git_files(override_root)?.iter().any(|path| {
            path == &override_root.join(args.path.strip_prefix(override_root).unwrap_or(&args.path))
        })
    {
        let relative_path = args.path.strip_prefix(override_root).unwrap_or(&args.path);
        if std::fs::symlink_metadata(&args.path)?.file_type().is_file()
            && !overrides.matched(relative_path, false).is_ignore()
        {
            parse_single_file(&args.path, &sink, verbose)?;
        }
    }

    if let Some((progress_done, progress)) = progress {
        let _ = progress_done.send(());
        progress.join().unwrap();
    }

    let summary = sink.snapshot();
    if args.json {
        output::print_json(&summary)?;
    } else {
        output::print_summary(&summary, std::io::stdout().is_terminal())?;
    }

    if verbose {
        output::print_unknown_formats(&summary, std::io::stderr().is_terminal())?;
    }
    Ok(())
}

fn build_overrides(root: &Path, includes: &[String], excludes: &[String]) -> io::Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    for pattern in includes {
        builder.add(pattern).map_err(io::Error::other)?;
    }
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(io::Error::other)?;
    }
    builder.build().map_err(io::Error::other)
}

fn git_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut command = std::process::Command::new("git");
    command.current_dir(root);
    command.args(["ls-files", "-z", "--cached"]);
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| root.join(git_path(path)))
        .collect())
}

fn parse_file_list(
    files: Vec<PathBuf>,
    root: &Path,
    overrides: &Override,
    sink: &file::Sink,
    verbose: bool,
) -> io::Result<()> {
    for path in files {
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
            && !overrides
                .matched(path.strip_prefix(root).unwrap_or(&path), false)
                .is_ignore()
        {
            parse_single_file(&path, sink, verbose)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn git_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn git_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

fn count_stdin(args: &Args) -> io::Result<()> {
    if args.tracked || args.diff.is_some() || !args.include.is_empty() || !args.exclude.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "git and path filters cannot be used with stdin",
        ));
    }
    let sink = file::Sink::new();
    let mut batch = Batch::default();
    if let Some(stats) = file::parse_stdin(args.verbose)? {
        batch.add(stats);
    }
    sink.add_batch(&mut batch);
    let summary = sink.snapshot();
    if args.json {
        output::print_json(&summary)
    } else {
        output::print_summary(&summary, std::io::stdout().is_terminal())
    }
}

fn parse_args() -> Args {
    let mut argv = std::env::args_os().collect::<Vec<_>>();
    if let Some(position) = argv.iter().position(|arg| arg == "-")
        && (position == 1
            || (position + 1 == argv.len()
                && !matches!(
                    argv.get(position.wrapping_sub(1))
                        .and_then(|arg| arg.to_str()),
                    Some("-j" | "--threads" | "--diff" | "--include" | "--exclude")
                )))
    {
        argv.remove(position);
        argv.push("--".into());
        argv.push("-".into());
    }
    match Args::parse_from(argv) {
        Ok(args) => args,
        Err(err) => {
            match &err {
                argue::Error::Help(help) => println!("{help}"),
                _ => eprintln!("{err}"),
            }
            std::process::exit(err.exit_code());
        }
    }
}

fn default_threads(path_is_dir: bool) -> usize {
    if !path_is_dir {
        return 1;
    }

    std::thread::available_parallelism()
        .map_or(1, |threads| usize::from(threads).saturating_mul(2))
        .min(8)
}

fn parse_single_file(path: &Path, sink: &file::Sink, verbose: bool) -> io::Result<()> {
    let mut batch = Batch::default();
    if let Some(file_stats) = parse_file(path, verbose)? {
        batch.add(file_stats);
    }
    sink.record_progress(batch.files());
    sink.add_batch(&mut batch);
    Ok(())
}

fn show_progress(sink: Arc<file::Sink>, done: Receiver<()>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_files = None;

        loop {
            match done.recv_timeout(Duration::from_millis(250)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let files = sink.files();
                    if last_files == Some(files) {
                        continue;
                    }

                    last_files = Some(files);
                    eprint!(
                        "\r\x1b[36mprocessed {} files\x1b[0m",
                        output::format_number(files)
                    );
                }
            }
        }

        eprint!("\r{:<24}\r", "");
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_threads_uses_one_worker_for_a_file() {
        assert_eq!(default_threads(false), 1);
    }

    #[test]
    fn default_threads_caps_directory_workers() {
        assert!((1..=8).contains(&default_threads(true)));
    }

    #[test]
    fn args_apply_defaults() {
        let args = Args::parse_from(["tally"]).unwrap();

        assert!(!args.all);
        assert!(!args.verbose);
        assert!(!args.json);
        assert!(!args.version);
        assert!(!args.tracked);
        assert_eq!(args.diff, None);
        assert!(args.include.is_empty());
        assert!(args.exclude.is_empty());
        assert_eq!(args.threads, None);
        assert_eq!(args.path, PathBuf::from("."));
    }

    #[test]
    fn args_accept_repeated_filters() {
        let args = Args::parse_from([
            "tally",
            "--include",
            "*.rs",
            "--exclude",
            "tests/**",
            "--exclude",
            "*.ts",
        ])
        .unwrap();
        assert_eq!(args.include, ["*.rs"]);
        assert_eq!(args.exclude, ["tests/**", "*.ts"]);
    }

    #[test]
    fn args_parse_flags_options_and_path() {
        let args = Args::parse_from(["tally", "--all", "--json", "-v", "-j", "2", "src"]).unwrap();

        assert!(args.all);
        assert!(args.verbose);
        assert!(args.json);
        assert_eq!(args.threads, Some(2));
        assert_eq!(args.path, PathBuf::from("src"));
    }

    #[test]
    fn args_report_help() {
        let err = Args::parse_from(["tally", "--help"]).unwrap_err();

        assert_eq!(err, argue::Error::Help(Args::HELP));
    }

    #[test]
    fn args_parse_version() {
        let args = Args::parse_from(["tally", "--version"]).unwrap();

        assert!(args.version);
    }

    #[test]
    fn args_accept_stdin_marker() {
        let args = Args::parse_from(["tally", "--", "-"]).unwrap();

        assert_eq!(args.path, PathBuf::from("-"));
    }
}
