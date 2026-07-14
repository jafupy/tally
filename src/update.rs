use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use semver::Version;
use serde::Deserialize;
use std::{
    env,
    io::{self, IsTerminal, Write},
    process::{Command, Stdio},
};

const REPOSITORY: &str = "jafupy/tally";
const INSTALL_URL: &str = "https://jafupy.com/tally.sh";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
}

pub fn check() -> Result<(), String> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|error| error.to_string())?;
    println!("tally {current}");

    let release = latest_release()?;
    let latest = parse_version(&release.tag_name)?;
    if latest <= current {
        println!("Tally is up to date.");
        return Ok(());
    }

    println!("\nTally {latest} is available.\n");
    println!(
        "{}",
        release
            .body
            .as_deref()
            .unwrap_or("No release notes provided.")
    );

    if !io::stdin().is_terminal() {
        println!("\nRun `curl -fsSL {INSTALL_URL} | sh` to update.");
        return Ok(());
    }

    if prompt_for_update()? {
        install()?;
    }

    Ok(())
}

fn prompt_for_update() -> Result<bool, String> {
    print!("\nUpdate now? [Y/n] ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    enable_raw_mode().map_err(|error| error.to_string())?;
    let raw_mode = RawMode;

    loop {
        let Event::Key(key) = event::read().map_err(|error| error.to_string())? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if let Some(answer) = accepts_update(key) {
            drop(raw_mode);
            println!();
            return Ok(answer);
        }
    }
}

struct RawMode;

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn accepts_update(key: KeyEvent) -> Option<bool> {
    match key {
        KeyEvent {
            code: KeyCode::Enter,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('y' | 'Y'),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } => Some(true),
        KeyEvent {
            code: KeyCode::Esc, ..
        }
        | KeyEvent {
            code: KeyCode::Char('n' | 'N'),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('c' | 'C'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
        | KeyEvent {
            code: KeyCode::Char('d' | 'D'),
            modifiers: KeyModifiers::CONTROL,
            ..
        } => Some(false),
        _ => None,
    }
}

fn latest_release() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let output = match Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "-A",
            "tally-update-check",
            &url,
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Command::new("wget")
            .args([
                "-qO-",
                "--timeout=10",
                "--tries=1",
                "--header=Accept: application/vnd.github+json",
                "--header=X-GitHub-Api-Version: 2022-11-28",
                "--user-agent=tally-update-check",
                &url,
            ])
            .output()
            .map_err(|error| format!("missing required command: curl or wget ({error})"))?,
        Err(error) => return Err(format!("failed to run curl: {error}")),
    };

    if !output.status.success() {
        return Err(format!("GitHub returned {}", output.status));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid GitHub response: {error}"))
}

fn install() -> Result<(), String> {
    let mut download = match Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            INSTALL_URL,
        ])
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(download) => download,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Command::new("wget")
            .args(["-qO-", "--timeout=10", "--tries=1", INSTALL_URL])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("missing required command: curl or wget ({error})"))?,
        Err(error) => return Err(format!("failed to run curl: {error}")),
    };
    let script = download.stdout.take().ok_or("could not read installer")?;
    let status = Command::new("sh")
        .stdin(script)
        .status()
        .map_err(|error| format!("failed to run installer: {error}"))?;
    let download_status = download.wait().map_err(|error| error.to_string())?;

    if !download_status.success() {
        return Err(format!("installer download failed with {download_status}"));
    }
    if !status.success() {
        return Err(format!("installer failed with {status}"));
    }
    Ok(())
}

fn parse_version(tag: &str) -> Result<Version, String> {
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

    Version::parse(&version).map_err(|error| format!("invalid release tag {tag:?}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixed_release_version() {
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn parses_two_part_release_version() {
        assert_eq!(parse_version("v1.2").unwrap(), Version::new(1, 2, 0));
    }

    #[test]
    fn rejects_non_version_release_tag() {
        assert!(parse_version("latest").is_err());
    }

    #[test]
    fn update_key_decisions_are_safe() {
        assert_eq!(accepts_update(KeyCode::Enter.into()), Some(true));
        assert_eq!(accepts_update(KeyCode::Char('y').into()), Some(true));
        assert_eq!(accepts_update(KeyCode::Esc.into()), Some(false));
        assert_eq!(accepts_update(KeyCode::Char('n').into()), Some(false));
        assert_eq!(
            accepts_update(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(false)
        );
        assert_eq!(
            accepts_update(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(false)
        );
        assert_eq!(accepts_update(KeyCode::Char('x').into()), None);
    }
}
