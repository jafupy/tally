mod dir;
mod file;
mod language;
mod output;
mod update;

use dir::scan_directory;
use file::{Batch, parse_file};
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

    /// Number of worker threads. Defaults adaptively to up to 4 workers for directories and 1 for a file.
    #[option(short = 'j', long = "threads")]
    threads: Option<usize>,

    /// Print extra diagnostics, including unknown file formats.
    #[flag(short = 'v', long = "verbose")]
    verbose: bool,

    /// Output results as JSON.
    #[flag(long = "json")]
    json: bool,

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
        update::check().map_err(io::Error::other)?;
        return Ok(());
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
    let sink = file::Sink::new();
    let progress = std::io::stderr().is_terminal().then(|| {
        let (progress_done, done) = mpsc::channel();
        (progress_done, show_progress(Arc::clone(&sink), done))
    });

    if path_is_dir {
        scan_directory(
            &args.path,
            Arc::clone(&sink),
            !args.all,
            threads,
            adaptive_threads,
            verbose,
        );
    } else {
        parse_single_file(&args.path, &sink, verbose)?;
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

fn parse_args() -> Args {
    match Args::parse() {
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
        .map_or(1, usize::from)
        .min(4)
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
        assert!((1..=4).contains(&default_threads(true)));
    }

    #[test]
    fn args_apply_defaults() {
        let args = Args::parse_from(["tally"]).unwrap();

        assert!(!args.all);
        assert!(!args.verbose);
        assert!(!args.json);
        assert!(!args.version);
        assert_eq!(args.threads, None);
        assert_eq!(args.path, PathBuf::from("."));
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
}
