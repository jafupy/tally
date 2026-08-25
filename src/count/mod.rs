mod lines;
mod reader;
mod syntax;
use crate::{
    language::{self, LanguageDef, LanguageId},
    result::Stats,
};
use lines::{BUFFER_BYTES, count_lines};
use memchr::memchr;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
const DETECTION_PREFIX_BYTES: usize = 16 * 1024;
const UNKNOWN: LanguageDef = LanguageDef {
    name: "Unknown",
    alphabetical_rank: 0,
    line_comments: &[],
    block_comments: &[],
    quotes: &[],
    comment_candidates: &[],
    block_candidates: &[],
    quote_candidates: &[],
    max_delimiter_len: 2,
};

pub enum FileStats {
    Known {
        language_id: LanguageId,
        stats: Stats,
    },
    Unknown {
        format: Option<String>,
        stats: Stats,
    },
}

pub fn parse_file(path: &Path, verbose: bool) -> io::Result<Option<FileStats>> {
    let mut buffer = vec![0; BUFFER_BYTES];
    parse_file_buffered(path, verbose, &mut buffer)
}

pub fn parse_file_buffered(
    path: &Path,
    verbose: bool,
    buffer: &mut [u8],
) -> io::Result<Option<FileStats>> {
    let mut reader = reader::Reusable::open(File::open(path)?, buffer);

    let language_id = {
        let prefix = read_prefix(&mut reader)?;
        let Some(contents_prefix) = text_prefix(prefix) else {
            return Ok(None);
        };
        language::detect_path(path, Some(contents_prefix))
    };

    match language_id {
        Some(language_id) => {
            let language = language::get(language_id);
            let stats = count_lines(reader, language)?;
            Ok(Some(FileStats::Known { language_id, stats }))
        }
        None => {
            let stats = count_lines(reader, &UNKNOWN)?;
            let format = verbose.then(|| unknown_format(path)).flatten();
            Ok(Some(FileStats::Unknown { format, stats }))
        }
    }
}

fn text_prefix(prefix: &[u8]) -> Option<&str> {
    if memchr(0, prefix).is_some() {
        return None;
    }

    match std::str::from_utf8(prefix) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&prefix[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

fn unknown_format(path: &Path) -> Option<String> {
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        return Some(format!(".{extension}"));
    }

    path.file_name()
        .and_then(|filename| filename.to_str())
        .map(|filename| filename.to_owned())
}

pub fn read_buffer() -> Vec<u8> {
    reader::buffer()
}

fn read_prefix(reader: &mut impl BufRead) -> io::Result<&[u8]> {
    let buffer = reader.fill_buf()?;
    Ok(&buffer[..buffer.len().min(DETECTION_PREFIX_BYTES)])
}
