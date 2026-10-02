use crate::{cli::Args, diff, result::Sink, scan};
use ignore::overrides::Override;
use std::{
    fs::{File, Metadata},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tally_stats::Kind;

enum Input {
    Directory,
    File(File),
}

pub(crate) struct PathScan {
    input: Input,
    filter_root: PathBuf,
    canonical_root: PathBuf,
    overrides: Override,
    threads: usize,
    adaptive_threads: bool,
}

impl PathScan {
    pub(crate) fn new(args: &Args, metadata: Metadata) -> io::Result<Self> {
        let path_is_dir = metadata.is_dir();
        let input = if path_is_dir {
            Input::Directory
        } else if metadata.is_file() {
            Input::File(File::open(&args.path)?)
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a regular file or directory", args.path.display()),
            ));
        };
        let threads = args.threads.unwrap_or_else(|| default_threads(path_is_dir));
        let adaptive_threads = args.threads.is_none() && path_is_dir;
        let filter_root = if path_is_dir {
            args.path.as_path()
        } else {
            args.path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
        }
        .to_path_buf();
        let canonical_root = filter_root.canonicalize()?;
        let overrides = scan::build_overrides(&canonical_root, &args.include, &args.exclude)?;
        Ok(Self {
            input,
            filter_root,
            canonical_root,
            overrides,
            threads,
            adaptive_threads,
        })
    }

    pub(crate) fn diff(self, args: &Args, extended: &[Kind]) -> io::Result<()> {
        if !extended.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "-x cannot be used with --diff",
            ));
        }
        if args.diff.len() > 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--diff may be specified at most twice",
            ));
        }
        diff::count(
            &args.path,
            &args.diff[0],
            args.diff.get(1).map(String::as_str),
            &self.overrides,
            args.json,
            self.threads,
            self.adaptive_threads,
        )
    }

    pub(crate) fn count(
        self,
        args: &Args,
        sink: &Arc<Sink>,
    ) -> io::Result<Option<scan::ScanReport>> {
        let debug = args.debug_enabled();
        match self.input {
            Input::Directory if args.tracked => {
                let files = scan::git_files(&args.path)?;
                scan::parse_file_list(files, &args.path, &self.overrides, sink, debug)?;
            }
            Input::Directory => {
                return scan::scan_directory(
                    &self.canonical_root,
                    Arc::clone(sink),
                    !args.all,
                    self.threads,
                    self.adaptive_threads,
                    debug,
                    self.overrides,
                )
                .map(Some);
            }
            Input::File(file) => {
                if args.tracked && !file_is_tracked(&args.path, &self.filter_root)? {
                    return Ok(None);
                }
                let relative_path = args
                    .path
                    .strip_prefix(&self.filter_root)
                    .unwrap_or(&args.path);
                if std::fs::symlink_metadata(&args.path)?.file_type().is_file()
                    && scan::file_is_included(&self.overrides, relative_path)
                {
                    scan::parse_single_opened_file(&args.path, file, sink, debug)?;
                }
            }
        }
        Ok(None)
    }
}

fn file_is_tracked(path: &Path, root: &Path) -> io::Result<bool> {
    let relative_path = path.strip_prefix(root).unwrap_or(path);
    let selected = root.join(relative_path);
    Ok(scan::git_files(root)?.iter().any(|path| path == &selected))
}

fn default_threads(path_is_dir: bool) -> usize {
    if path_is_dir {
        std::thread::available_parallelism().map_or(1, usize::from)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_threads_uses_one_worker_for_a_file() {
        assert_eq!(default_threads(false), 1);
    }

    #[test]
    fn default_threads_uses_available_parallelism_for_a_directory() {
        assert_eq!(
            default_threads(true),
            std::thread::available_parallelism().map_or(1, usize::from)
        );
    }
}
