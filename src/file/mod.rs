mod sink;

use crate::language::{self, LanguageDef, LanguageId};
use memchr::{memchr, memmem};
pub use sink::{Batch, Sink, Stats, Summary};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const BUFFER_BYTES: usize = 64 * 1024;
const DETECTION_PREFIX_BYTES: usize = 16 * 1024;
const UNKNOWN: LanguageDef = LanguageDef {
    name: "Unknown",
    line_comments: &[],
    block_comments: &[],
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

pub fn parse_file(path: &Path, verbose: bool) -> Option<FileStats> {
    let Ok(mut reader) = open(path) else {
        return None;
    };

    let language_id = {
        let prefix = read_prefix(&mut reader);
        let contents_prefix = text_prefix(prefix)?;
        language::detect_path(path, Some(contents_prefix))
    };

    match language_id {
        Some(language_id) => {
            let language = language::get(language_id);
            let stats = count_lines(reader, language);
            Some(FileStats::Known { language_id, stats })
        }
        None => {
            let stats = count_lines(reader, &UNKNOWN);
            let format = verbose.then(|| unknown_format(path)).flatten();
            Some(FileStats::Unknown { format, stats })
        }
    }
}

fn text_prefix(prefix: &[u8]) -> Option<&str> {
    if memchr(0, prefix).is_some() {
        return None;
    }

    std::str::from_utf8(prefix).ok()
}

fn unknown_format(path: &Path) -> Option<String> {
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        return Some(format!(".{extension}"));
    }

    path.file_name()
        .and_then(|filename| filename.to_str())
        .map(|filename| filename.to_owned())
}

fn open(path: &Path) -> Result<BufReader<File>, ()> {
    File::open(path)
        .map(|file| BufReader::with_capacity(BUFFER_BYTES, file))
        .map_err(|err| {
            eprintln!("failed to open file {}: {err}", path.display());
        })
}

fn read_prefix(reader: &mut BufReader<File>) -> &[u8] {
    let buffer = reader.fill_buf().unwrap_or_default();
    &buffer[..buffer.len().min(DETECTION_PREFIX_BYTES)]
}

fn count_lines(mut reader: BufReader<File>, language: &LanguageDef) -> Stats {
    let mut stats = Stats {
        files: 1,
        ..Stats::default()
    };
    let mut block_comment: Option<&str> = None;
    let mut partial_line = Vec::new();

    loop {
        let consumed = {
            let buffer = match reader.fill_buf() {
                Ok([]) | Err(_) => break,
                Ok(buffer) => buffer,
            };
            let mut start = 0;

            while let Some(relative_end) = memchr(b'\n', &buffer[start..]) {
                let end = start + relative_end + 1;
                let line = &buffer[start..end];

                if partial_line.is_empty() {
                    count_line(line, language, &mut block_comment, &mut stats);
                } else {
                    partial_line.extend_from_slice(line);
                    count_line(&partial_line, language, &mut block_comment, &mut stats);
                    partial_line.clear();
                }

                start = end;
            }

            if start < buffer.len() {
                partial_line.extend_from_slice(&buffer[start..]);
            }

            buffer.len()
        };

        reader.consume(consumed);
    }

    if !partial_line.is_empty() {
        count_line(&partial_line, language, &mut block_comment, &mut stats);
    }

    stats
}

fn count_line(
    line: &[u8],
    language: &LanguageDef,
    block_comment: &mut Option<&str>,
    stats: &mut Stats,
) {
    stats.lines += 1;
    let trimmed = trim_start_ascii(line);

    if trimmed.is_empty() {
        stats.blanks += 1;
        return;
    }

    if let Some(end) = block_comment {
        stats.comments += 1;
        if find_bytes(trimmed, end.as_bytes()).is_some() {
            *block_comment = None;
        }
        return;
    }

    if language
        .line_comments
        .iter()
        .any(|comment| trimmed.starts_with(comment.as_bytes()))
    {
        stats.comments += 1;
        return;
    }

    for &(start, end) in language.block_comments {
        let start = start.as_bytes();
        let end_bytes = end.as_bytes();

        if trimmed.starts_with(start) {
            stats.comments += 1;
            if find_bytes(trimmed, end_bytes).is_none_or(|end_at| end_at < start.len()) {
                *block_comment = Some(end);
            }
            return;
        }
    }

    stats.code += 1;
}

fn trim_start_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }

    &bytes[start..]
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    memmem::find(haystack, needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tally-{}-{name}", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn counts_file_without_trailing_newline() {
        let path = temp_file("no-newline.rs", b"fn main() {}");
        let Some(FileStats::Known { stats, .. }) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.files, 1);
        assert_eq!(stats.lines, 1);
        assert_eq!(stats.code, 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn counts_comments_and_blank_lines() {
        let path = temp_file(
            "comments.rs",
            b"// comment\n\n/* block\nstill block */\nfn main() {}\n",
        );
        let Some(FileStats::Known { stats, .. }) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.lines, 5);
        assert_eq!(stats.comments, 3);
        assert_eq!(stats.blanks, 1);
        assert_eq!(stats.code, 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn counts_lines_spanning_reader_buffers() {
        let path = temp_file(
            "long-line.rs",
            format!("{}\nfn main() {{}}\n", "x".repeat(BUFFER_BYTES + 1)).as_bytes(),
        );
        let Some(FileStats::Known { stats, .. }) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.lines, 2);
        assert_eq!(stats.code, 2);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn skips_binary_files() {
        let path = temp_file("binary.rs", b"fn main() {}\0");

        assert!(parse_file(&path, false).is_none());

        fs::remove_file(path).unwrap();
    }
}
