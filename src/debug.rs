mod formats;
pub(crate) use formats::UnknownFormats;
#[cfg(feature = "debug")]
mod report;

use crate::{cli::Args, result::Summary, scan::ScanReport};
use std::{io, path::Path};

pub(crate) fn start(args: &Args) -> io::Result<()> {
    #[cfg(feature = "debug")]
    if args.trace_requested() {
        crate::trace::start()?;
    }
    trace_event!(
        run_start,
        Some(&args.path),
        args.all,
        args.json,
        args.threads
    );
    #[cfg(not(feature = "debug"))]
    let _ = args;
    Ok(())
}

pub(crate) fn finish(result: io::Result<()>) -> io::Result<()> {
    #[cfg(feature = "debug")]
    if crate::trace::enabled() {
        return match result {
            Ok(()) => {
                let path = crate::trace::finish()?;
                eprintln!("Trace appended to {}", path.display());
                Ok(())
            }
            Err(error) => {
                trace_event!(run_error, None, error.to_string());
                let _ = crate::trace::finish();
                Err(error)
            }
        };
    }
    result
}

pub(crate) struct ScanTimer {
    #[cfg(feature = "debug")]
    timer: Option<report::Timer>,
}

pub(crate) struct ScanTiming {
    #[cfg(feature = "debug")]
    timing: Option<report::Timing>,
}

impl ScanTimer {
    pub(crate) fn start(enabled: bool) -> Self {
        #[cfg(not(feature = "debug"))]
        let _ = enabled;
        Self {
            #[cfg(feature = "debug")]
            timer: enabled.then(report::Timer::start),
        }
    }

    pub(crate) fn finish(self) -> ScanTiming {
        ScanTiming {
            #[cfg(feature = "debug")]
            timing: self.timer.map(report::Timer::finish),
        }
    }
}

impl ScanTiming {
    pub(crate) fn print(self, scan: Option<ScanReport>, summary: &Summary) -> io::Result<()> {
        #[cfg(feature = "debug")]
        if let Some(timing) = self.timing {
            use std::io::IsTerminal;
            report::print(timing, scan, summary)?;
            crate::output::write_unknown_formats(
                &mut io::stderr().lock(),
                summary,
                io::stderr().is_terminal(),
            )?;
        }
        #[cfg(not(feature = "debug"))]
        let _ = (scan, summary);
        Ok(())
    }
}

pub(crate) fn unknown_format(path: &Path, enabled: bool) -> Option<String> {
    #[cfg(feature = "debug")]
    if enabled {
        if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
            return Some(format!(".{extension}"));
        }
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
    }
    #[cfg(not(feature = "debug"))]
    let _ = (path, enabled);
    None
}
