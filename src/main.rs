#[macro_use]
mod trace_macros;

mod cli;
mod debug;
mod diff;
mod file;
mod language;
mod output;
mod progress;
mod result;
mod scan;
#[cfg(feature = "debug")]
mod trace;
mod trace_output;
mod update;

use cli::Args;
use result::{Batch, Sink};
use std::{
    io::{self, ErrorKind, IsTerminal},
    path::Path,
};

fn main() {
    let result = run(cli::parse());
    let result = debug::finish(result);
    if let Err(error) = result {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("tally: {error}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> io::Result<()> {
    let extended = tally_stats::parse(&args.extended)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    if args.version {
        return update::check();
    }

    let metadata = if args.path == Path::new("-") {
        None
    } else {
        Some(std::fs::metadata(&args.path)?)
    };
    if trace_output::TraceOutput::current()?.matches_file(&args.path) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} is tally's trace output", args.path.display()),
        ));
    }
    debug::start(&args)?;

    let input = match metadata {
        Some(metadata) => {
            let input = scan::PathScan::new(&args, metadata)?;
            if !args.diff.is_empty() {
                return input.diff(&args, &extended);
            }
            Some(input)
        }
        None => {
            if args.tracked
                || !args.diff.is_empty()
                || !args.include.is_empty()
                || !args.exclude.is_empty()
            {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "git and path filters cannot be used with stdin",
                ));
            }
            None
        }
    };

    let sink = Sink::new_with_samples(!extended.is_empty());
    let path_scan = input.is_some();
    let progress = path_scan.then(|| progress::Progress::start(&sink));
    let timer = debug::ScanTimer::start(path_scan && args.debug_enabled());

    let report = match input {
        Some(input) => input.count(&args, &sink)?,
        None => {
            let mut batch = Batch::with_samples(!extended.is_empty());
            if let Some(stats) = file::parse_stdin(args.debug_enabled())? {
                batch.add(stats);
            }
            sink.add_batch(&mut batch);
            None
        }
    };
    let timing = timer.finish();
    if path_scan {
        trace_event!(scan_complete, Some(&args.path));
    }
    drop(progress);

    let summary = sink.snapshot();
    if path_scan {
        trace_event!(summary_snapshot, None, summary.all.files, summary.all.lines);
    }
    let _span = trace_span!("output", None);
    let color = io::stdout().is_terminal();
    {
        let mut stdout = io::stdout().lock();
        if args.json {
            output::write_json(&mut stdout, &summary, &extended)?;
        } else {
            output::write_summary(&mut stdout, &summary, color, &extended)?;
        }
    }
    timing.print(report, &summary)
}
