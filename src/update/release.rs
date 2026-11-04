use semver::Version;
use serde::Deserialize;
use std::io;

#[derive(Deserialize)]
pub(super) struct Release {
    pub(super) tag_name: String,
    pub(super) body: Option<String>,
    pub(super) assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(super) struct Asset {
    pub(super) name: String,
    pub(super) browser_download_url: String,
    pub(super) digest: Option<String>,
}

pub(super) fn parse_version(tag: &str) -> io::Result<Version> {
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
