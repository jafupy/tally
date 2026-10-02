#[macro_use]
mod trace_macros;

mod cli;
#[cfg(feature = "debug")]
mod debug;
mod diff;
mod file;
mod language;
mod output;
mod progress;
mod result;
mod run;
mod scan;
#[cfg(feature = "debug")]
mod trace;
mod trace_output;
mod update;

use std::io::ErrorKind;

fn main() {
    if let Err(error) = run::execute(cli::parse()) {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("tally: {error}");
        std::process::exit(1);
    }
}
