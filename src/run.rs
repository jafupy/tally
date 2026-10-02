mod path;

use crate::{
    cli::Args,
    file, output, progress,
    result::{Batch, Sink, Summary},
    trace_output, update,
};
use std::{
    io::{self, ErrorKind, IsTerminal},
    path::Path,
};
use tally_stats::Kind;

pub(crate) fn execute(args: Args) -> io::Result<()> {
    let result = run_command(args);
    #[cfg(feature = "debug")]
    let result = result.and_then(|()| {
        if crate::trace::enabled() {
            let path = crate::trace::finish()?;
            eprintln!("Trace appended to {}", path.display());
        }
        Ok(())
    });
    #[cfg(feature = "debug")]
    if let Err(error) = &result
        && crate::trace::enabled()
    {
        trace_event!(
            "run_error",
            None,
            serde_json::json!({"error": error.to_string()})
        );
        let _ = crate::trace::finish();
    }
    result
}

fn run_command(args: Args) -> io::Result<()> {
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
    #[cfg(feature = "debug")]
    if args.trace_requested() {
        crate::trace::start()?;
    }
    trace_event!(
        "run_start",
        Some(&args.path),
        serde_json::json!({
            "all": args.all, "json": args.json, "threads": args.threads
        })
    );

    match metadata {
        None => count_stdin(&args, &extended),
        Some(metadata) => count_path(&args, metadata, &extended),
    }
}

fn count_path(args: &Args, metadata: std::fs::Metadata, extended: &[Kind]) -> io::Result<()> {
    let input = path::PathScan::new(args, metadata)?;
    if !args.diff.is_empty() {
        return input.diff(args, extended);
    }

    let sink = Sink::new_with_samples(!extended.is_empty());
    let progress = progress::Progress::start(&sink);
    #[cfg(feature = "debug")]
    let timer = args.debug_enabled().then(crate::debug::Timer::start);

    let report = input.count(args, &sink)?;
    #[cfg(feature = "debug")]
    let timing = timer.map(crate::debug::Timer::finish);
    trace_event!("scan_complete", Some(&args.path), serde_json::json!({}));
    drop(progress);

    let summary = sink.snapshot();
    trace_event!(
        "summary_snapshot",
        None,
        serde_json::json!({
            "files": summary.all.files, "lines": summary.all.lines
        })
    );
    let _span = trace_span!("output", None);
    write_output(args.json, &summary, extended)?;

    #[cfg(feature = "debug")]
    if let Some(timing) = timing {
        crate::debug::print(timing, report, &summary)?;
        output::write_unknown_formats(
            &mut io::stderr().lock(),
            &summary,
            io::stderr().is_terminal(),
        )?;
    }
    #[cfg(not(feature = "debug"))]
    let _ = report;
    Ok(())
}

fn count_stdin(args: &Args, extended: &[Kind]) -> io::Result<()> {
    if args.tracked || !args.diff.is_empty() || !args.include.is_empty() || !args.exclude.is_empty()
    {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "git and path filters cannot be used with stdin",
        ));
    }
    let sink = Sink::new_with_samples(!extended.is_empty());
    let mut batch = Batch::with_samples(!extended.is_empty());
    if let Some(stats) = file::parse_stdin(args.debug_enabled())? {
        batch.add(stats);
    }
    sink.add_batch(&mut batch);
    write_output(args.json, &sink.snapshot(), extended)
}

fn write_output(json: bool, summary: &Summary, extended: &[Kind]) -> io::Result<()> {
    let color = io::stdout().is_terminal();
    let mut output = io::stdout().lock();
    if json {
        output::write_json(&mut output, summary, extended)
    } else {
        output::write_summary(&mut output, summary, color, extended)
    }
}
