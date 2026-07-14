mod sink;

use crate::language::{self, LanguageDef, LanguageId};
use memchr::{memchr, memmem};
pub use sink::{Batch, Sink, Stats, Summary};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
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

pub fn parse_file(path: &Path, verbose: bool) -> io::Result<Option<FileStats>> {
    let mut reader = open(path)?;

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

fn open(path: &Path) -> io::Result<BufReader<File>> {
    File::open(path).map(|file| BufReader::with_capacity(BUFFER_BYTES, file))
}

fn read_prefix(reader: &mut impl BufRead) -> io::Result<&[u8]> {
    let buffer = reader.fill_buf()?;
    Ok(&buffer[..buffer.len().min(DETECTION_PREFIX_BYTES)])
}

fn count_lines(mut reader: impl BufRead, language: &LanguageDef) -> io::Result<Stats> {
    let mut stats = Stats {
        files: 1,
        ..Stats::default()
    };
    let mut block_comment: Option<&str> = None;
    let mut partial_line = Vec::new();

    loop {
        let consumed = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
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

    Ok(stats)
}

fn count_line<'a>(
    line: &[u8],
    language: &'a LanguageDef,
    block_comment: &mut Option<&'a str>,
    stats: &mut Stats,
) {
    stats.lines += 1;
    let trimmed = trim_start_ascii(line);

    if trimmed.is_empty() {
        stats.blanks += 1;
        return;
    }

    if block_comment.is_some() {
        stats.comments += 1;
        update_block_comment(trimmed, language, block_comment);
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

        if trimmed.starts_with(start) {
            stats.comments += 1;
            *block_comment = Some(end);
            update_block_comment(&trimmed[start.len()..], language, block_comment);
            return;
        }
    }

    stats.code += 1;
}

fn update_block_comment<'a>(
    mut remainder: &[u8],
    language: &'a LanguageDef,
    block_comment: &mut Option<&'a str>,
) {
    loop {
        if let Some(end) = *block_comment {
            let Some(end_at) = find_bytes(remainder, end.as_bytes()) else {
                return;
            };
            remainder = trim_start_ascii(&remainder[end_at + end.len()..]);
            *block_comment = None;
        }

        let Some(&(start, end)) = language
            .block_comments
            .iter()
            .find(|(start, _)| remainder.starts_with(start.as_bytes()))
        else {
            return;
        };
        remainder = &remainder[start.len()..];
        *block_comment = Some(end);
    }
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
    use std::{fs, io::Read, path::PathBuf};

    struct FailingReader {
        first_line_pending: bool,
    }

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read failed"))
        }
    }

    impl BufRead for FailingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.first_line_pending {
                Ok(b"code\n")
            } else {
                Err(io::Error::other("read failed"))
            }
        }

        fn consume(&mut self, _amount: usize) {
            self.first_line_pending = false;
        }
    }

    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tally-{}-{name}", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn counts_file_without_trailing_newline() {
        let path = temp_file("no-newline.rs", b"fn main() {}");
        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
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
        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.lines, 5);
        assert_eq!(stats.comments, 3);
        assert_eq!(stats.blanks, 1);
        assert_eq!(stats.code, 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn tracks_a_new_block_comment_after_one_closes() {
        let path = temp_file("adjacent-comments.css", b"/*\n*/ /*\ninside\n*/\n");
        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
            panic!("expected CSS file stats");
        };

        assert_eq!(stats.lines, 4);
        assert_eq!(stats.comments, 4);
        assert_eq!(stats.code, 0);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn counts_lines_spanning_reader_buffers() {
        let path = temp_file(
            "long-line.rs",
            format!("{}\nfn main() {{}}\n", "x".repeat(BUFFER_BYTES + 1)).as_bytes(),
        );
        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.lines, 2);
        assert_eq!(stats.code, 2);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn skips_binary_files() {
        let path = temp_file("binary.rs", b"fn main() {}\0");

        assert!(parse_file(&path, false).unwrap().is_none());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn accepts_utf8_split_at_the_detection_boundary() {
        let mut contents = vec![b'x'; DETECTION_PREFIX_BYTES - 1];
        contents.extend_from_slice("é\n".as_bytes());
        let path = temp_file("split-utf8.rs", &contents);

        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
            panic!("expected rust file stats");
        };

        assert_eq!(stats.files, 1);
        assert_eq!(stats.lines, 1);
        assert_eq!(stats.code, 1);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn propagates_reader_errors_instead_of_returning_partial_stats() {
        let mut initial_failure = FailingReader {
            first_line_pending: false,
        };
        assert!(read_prefix(&mut initial_failure).is_err());

        let later_failure = FailingReader {
            first_line_pending: true,
        };
        assert!(count_lines(later_failure, &UNKNOWN).is_err());
    }
}
