use crate::{
    file,
    result::{Batch, Sink},
    trace_output,
};
use ignore::overrides::{Override, OverrideBuilder};
use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) fn build_overrides(
    root: &Path,
    includes: &[String],
    excludes: &[String],
) -> io::Result<Override> {
    let mut builder = OverrideBuilder::new(root);
    for pattern in includes {
        builder.add(pattern).map_err(io::Error::other)?;
    }
    for pattern in excludes {
        builder
            .add(&format!("!{pattern}"))
            .map_err(io::Error::other)?;
    }
    builder.build().map_err(io::Error::other)
}

pub(crate) fn git_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    tally_git::tracked_files(root)
}

pub(crate) fn parse_file_list(
    files: Vec<PathBuf>,
    root: &Path,
    overrides: &Override,
    sink: &Sink,
    verbose: bool,
) -> io::Result<()> {
    let trace_output = trace_output::TraceOutput::current()?;
    for path in files {
        if trace_output.matches_file(&path) {
            continue;
        }
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
            && file_is_included(overrides, path.strip_prefix(root).unwrap_or(&path))
        {
            parse_single_file(&path, sink, verbose)?;
        }
    }
    Ok(())
}

// Paths are relative to the override root. Match the same paths as the walker,
// including directory exclusions that prevent it from reaching a file.
pub(crate) fn file_is_included(overrides: &Override, relative_path: &Path) -> bool {
    relative_path
        .ancestors()
        .take_while(|path| !path.as_os_str().is_empty())
        .enumerate()
        .all(|(depth, path)| {
            !overrides
                .matched(overrides.path().join(path), depth > 0)
                .is_ignore()
        })
}

pub(crate) fn parse_single_file(path: &Path, sink: &Sink, debug: bool) -> io::Result<()> {
    let _file_context = trace_context!(path);
    let mut batch = Batch::with_samples(sink.collects_samples());
    if let Some(file_stats) = file::parse_file(path, debug)? {
        batch.add(file_stats);
    }
    sink.record_progress(batch.files());
    sink.add_batch(&mut batch);
    Ok(())
}

pub(crate) fn parse_single_opened_file(
    path: &Path,
    opened_file: std::fs::File,
    sink: &Sink,
    debug: bool,
) -> io::Result<()> {
    let _file_context = trace_context!(path);
    let mut batch = Batch::with_samples(sink.collects_samples());
    if let Some(file_stats) = file::parse_opened_file(path, opened_file, debug)? {
        batch.add(file_stats);
    }
    sink.record_progress(batch.files());
    sink.add_batch(&mut batch);
    Ok(())
}
