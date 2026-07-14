use super::Release;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read, Write},
    process::{Command, ExitStatus, Stdio},
};

const REPOSITORY: &str = "jafupy/tally";
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_BINARY_BYTES: usize = 16 * 1024 * 1024;

pub fn latest_release() -> io::Result<Release> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let output = run_download(&url, true, MAX_METADATA_BYTES)?;
    serde_json::from_slice(&output)
        .map_err(|error| io::Error::other(format!("invalid GitHub response: {error}")))
}

pub fn install(release: &Release) -> io::Result<()> {
    let asset_name = platform_asset()?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| io::Error::other(format!("release has no {asset_name} binary")))?;
    let binary = run_download(&asset.browser_download_url, false, MAX_BINARY_BYTES)?;
    verify_digest(&binary, asset.digest.as_deref())?;
    replace_current_executable(&binary)
}

fn platform_asset() -> io::Result<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("tally-mac-arm"),
        ("linux", "x86_64") => Ok("tally-linux-x86_64"),
        (os, arch) => Err(io::Error::other(format!(
            "no release binary for {os}/{arch}"
        ))),
    }
}

fn verify_digest(binary: &[u8], digest: Option<&str>) -> io::Result<()> {
    let Some(expected) = digest.and_then(|digest| digest.strip_prefix("sha256:")) else {
        return Err(io::Error::other("release asset has no SHA-256 digest"));
    };
    let actual = format!("{:x}", Sha256::digest(binary));
    if actual == expected {
        Ok(())
    } else {
        Err(io::Error::other("release asset SHA-256 mismatch"))
    }
}

fn replace_current_executable(binary: &[u8]) -> io::Result<()> {
    let current = env::current_exe()?;
    let temporary = current.with_extension(format!("update-{}", std::process::id()));
    let permissions = fs::metadata(&current)?.permissions();

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(binary)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&temporary, permissions)?;
        fs::rename(&temporary, &current)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn run_download(url: &str, github_headers: bool, max_bytes: usize) -> io::Result<Vec<u8>> {
    let max_bytes_arg = max_bytes.to_string();
    let mut curl = Command::new("curl");
    curl.args([
        "-fsSL",
        "--connect-timeout",
        "10",
        "--max-time",
        "60",
        "--max-filesize",
        &max_bytes_arg,
    ]);
    if github_headers {
        curl.args([
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "-A",
            "tally-update-check",
        ]);
    }
    curl.arg(url);

    let (body, status) = match capture(curl, max_bytes) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut wget = Command::new("wget");
            wget.args([
                "-qO-",
                "--timeout=10",
                "--tries=1",
                "--max-filesize",
                &max_bytes_arg,
            ]);
            if github_headers {
                wget.args([
                    "--header=Accept: application/vnd.github+json",
                    "--header=X-GitHub-Api-Version: 2022-11-28",
                    "--user-agent=tally-update-check",
                ]);
            }
            wget.arg(url);
            capture(wget, max_bytes).map_err(|error| {
                io::Error::other(format!("missing required command: curl or wget ({error})"))
            })?
        }
        Err(error) => return Err(io::Error::other(format!("failed to run curl: {error}"))),
    };

    if status.success() {
        Ok(body)
    } else {
        Err(io::Error::other(format!("download failed with {status}")))
    }
}

fn capture(mut command: Command, max_bytes: usize) -> io::Result<(Vec<u8>, ExitStatus)> {
    let mut child = command.stdout(Stdio::piped()).spawn()?;
    let mut body = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("could not read download"))?
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut body);

    if let Err(error) = read {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if body.len() > max_bytes {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("download exceeded size limit"));
    }
    Ok((body, child.wait()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_sha256_digests() {
        let digest = format!("sha256:{:x}", Sha256::digest(b"tally"));
        assert!(verify_digest(b"tally", Some(&digest)).is_ok());
        assert!(verify_digest(b"tampered", Some(&digest)).is_err());
        assert!(verify_digest(b"tally", None).is_err());
    }

    #[test]
    fn bounds_captured_downloads() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 12345"]);
        assert!(capture(command, 4).is_err());
    }
}
