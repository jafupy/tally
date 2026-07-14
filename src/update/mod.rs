mod network;

use semver::Version;
use serde::Deserialize;
use std::io::{self, BufRead, IsTerminal, Write};

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

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

    if prompt_for_update(&mut output)? {
        drop(output);
        network::install(&release)?;
    }
    Ok(())
}

fn prompt_for_update(output: &mut impl Write) -> io::Result<bool> {
    let mut input = io::stdin().lock();
    loop {
        write!(output, "\nUpdate now? [Y/n] ")?;
        output.flush()?;

        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            return Ok(false);
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Please answer yes or no.")?,
        }
    }
}

fn parse_version(tag: &str) -> io::Result<Version> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let version = if version
        .split(['-', '+'])
        .next()
        .unwrap_or(version)
        .matches('.')
        .count()
        == 1
    {
        let suffix = version.find(['-', '+']).unwrap_or(version.len());
        format!("{}.0{}", &version[..suffix], &version[suffix..])
    } else {
        version.to_owned()
    };

    Version::parse(&version)
        .map_err(|error| io::Error::other(format!("invalid release tag {tag:?}: {error}")))
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
