mod count;
mod language;
mod output;
mod progress;
mod result;
mod scan;
mod update;
use scan::{scan_directory, scan_file};
use std::{
    io::{self, ErrorKind, IsTerminal},
    path::PathBuf,
    sync::Arc,
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

    /// Path to tally
    #[positional(default = ".")]
    path: PathBuf,
}

fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(err) => {
            match &err {
                argue::Error::Help(help) => println!("{help}"),
                _ => eprintln!("{err}"),
            }
            std::process::exit(err.exit_code());
        }
    };

    if let Err(error) = execute(args) {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("tally: {error}");
        std::process::exit(1);
    }
}

fn execute(args: Args) -> io::Result<()> {
    if args.version {
        update::check()?;
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
    let threads = match args.threads {
        Some(threads) => threads,
        None if path_is_dir => std::thread::available_parallelism()
            .map_or(1, |threads| usize::from(threads).saturating_mul(2))
            .min(8),
        None => 1,
    };
    let adaptive_threads = args.threads.is_none() && path_is_dir;
    let verbose = args.verbose;
    let sink = Arc::new(result::Sink::new());
    let progress = std::io::stderr()
        .is_terminal()
        .then(|| progress::start(Arc::clone(&sink)));

    if path_is_dir {
        scan_directory(
            &args.path,
            Arc::clone(&sink),
            !args.all,
            threads,
            adaptive_threads,
            verbose,
        )?;
    } else {
        scan_file(&args.path, &sink, verbose)?;
    }

    if let Some(progress) = progress {
        progress::stop(progress);
    }

    let summary = sink.snapshot();
    let stdout_color = std::io::stdout().is_terminal();
    let mut stdout = std::io::stdout().lock();
    if args.json {
        output::write_json(&mut stdout, &summary)?;
    } else {
        output::write_summary(&mut stdout, &summary, stdout_color)?;
    }

    if verbose {
        let stderr_color = std::io::stderr().is_terminal();
        output::write_unknown_formats(&mut std::io::stderr().lock(), &summary, stderr_color)?;
    }
    Ok(())
}
