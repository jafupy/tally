use semver::Version;
use serde::Deserialize;
use std::io;

#[derive(Deserialize)]
pub(super) struct Release {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(super) struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
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
