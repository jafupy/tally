mod network;
mod prompt;
mod release;

use release::{Release, parse_version};
use semver::Version;
use std::io::{self, IsTerminal, Write};

pub fn check() -> io::Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(io::Error::other)?;
    let mut output = io::stdout().lock();
    writeln!(output, "tally {current}")?;

    let release = network::latest_release()?;
    let latest = parse_version(&release.tag_name)?;
    if latest <= current {
        writeln!(output, "Tally is up to date.")?;
        return Ok(());
    }

    writeln!(output, "\nTally {latest} is available.\n")?;
    writeln!(
        output,
        "{}",
        release
            .body
            .as_deref()
            .unwrap_or("No release notes provided.")
    )?;

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        writeln!(output, "\nRun `tally --version` in a terminal to update.")?;
        return Ok(());
    }

    let mut input = io::stdin().lock();
    if prompt::confirm(&mut input, &mut output)? {
        drop(output);
        network::install(&release)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_versions() {
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_version("v1.2").unwrap(), Version::new(1, 2, 0));
        assert!(parse_version("latest").is_err());
    }
}
