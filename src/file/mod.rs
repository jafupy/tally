mod lines;
mod reader;
mod syntax;

use lines::count_lines;
pub use lines::count_selected_contents;
use reader::{ReusableBufReader, read_prefix};

use crate::language::{self, LanguageDef, LanguageId};
use crate::result::Stats;
use memchr::memchr;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const BUFFER_BYTES: usize = 64 * 1024;
const DETECTION_PREFIX_BYTES: usize = 16 * 1024;
const UNKNOWN: LanguageDef = LanguageDef {
    name: "Unknown",
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

pub fn parse_file(path: &Path, debug: bool) -> io::Result<Option<FileStats>> {
    let mut buffer = vec![0; BUFFER_BYTES];
    parse_file_buffered(path, debug, &mut buffer)
}

pub fn parse_opened_file(path: &Path, file: File, debug: bool) -> io::Result<Option<FileStats>> {
    let mut buffer = vec![0; BUFFER_BYTES];
    parse_file_buffered_inner(path, debug, &mut buffer, Some(file))
}

pub fn parse_file_buffered(
    path: &Path,
    debug: bool,
    buffer: &mut [u8],
) -> io::Result<Option<FileStats>> {
    parse_file_buffered_inner(path, debug, buffer, None)
}

fn parse_file_buffered_inner(
    path: &Path,
    debug: bool,
    buffer: &mut [u8],
    opened_file: Option<File>,
) -> io::Result<Option<FileStats>> {
    let _file_context = trace_context!(path);
    let _file_span = trace_span!("parse_file", Some(path));
    let file = {
        let _open_span = trace_span!("file_open", Some(path));
        match opened_file {
            Some(file) => file,
            None => File::open(path)?,
        }
    };
    trace_event!(file_opened, Some(path));
    let mut reader = ReusableBufReader::new(file, buffer);

    let language_id = {
        let _prefix_span = trace_span!("read_prefix", Some(path));
        let prefix = read_prefix(&mut reader)?;
        trace_event!(prefix_read, Some(path), prefix.len());
        let Some(contents_prefix) = text_prefix(prefix) else {
            trace_event!(file_skip, Some(path), "binary");
            return Ok(None);
        };
        let _detect_span = trace_span!("detect_language", Some(path));
        language::detect_path(path, Some(contents_prefix))
    };

    match language_id {
        Some(language_id) => {
            let language = language::get(language_id);
            trace_event!(language_detected, Some(path), language.name);
            let _count_span = trace_span!("count_lines", Some(path));
            let stats = count_lines(reader, language)?;
            trace_event!(file_counted, Some(path), &stats);
            Ok(Some(FileStats::Known { language_id, stats }))
        }
        None => {
            trace_event!(language_unknown, Some(path));
            let _count_span = trace_span!("count_lines", Some(path));
            let stats = count_lines(reader, &UNKNOWN)?;
            let format = crate::debug::unknown_format(path, debug);
            trace_event!(unknown_file_counted, Some(path), &stats, &format);
            Ok(Some(FileStats::Unknown { format, stats }))
        }
    }
}

pub fn parse_stdin(verbose: bool) -> io::Result<Option<FileStats>> {
    let stdin = io::stdin();
    parse_reader(
        Path::new("-"),
        verbose,
        BufReader::with_capacity(BUFFER_BYTES, stdin.lock()),
    )
}

fn parse_reader(
    path: &Path,
    verbose: bool,
    mut reader: impl BufRead,
) -> io::Result<Option<FileStats>> {
    let language_id = {
        let prefix = read_prefix(&mut reader)?;
        let Some(contents_prefix) = text_prefix(prefix) else {
            return Ok(None);
        };
        language::detect_path(path, Some(contents_prefix))
    };

    match language_id {
        Some(language_id) => {
            let stats = count_lines(reader, language::get(language_id))?;
            Ok(Some(FileStats::Known { language_id, stats }))
        }
        None => {
            let stats = count_lines(reader, &UNKNOWN)?;
            let format = crate::debug::unknown_format(path, verbose);
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

pub fn read_buffer() -> Vec<u8> {
    vec![0; BUFFER_BYTES]
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
    fn ignores_comment_delimiters_inside_multiline_quotes() {
        for extension in ["js", "ts", "svelte", "vue", "astro", "go"] {
            let path = temp_file(
                &format!("multiline-template.{extension}"),
                b"const x = `\nhello /* not a comment\nstill string\n`;\nfoo();\n",
            );
            let Ok(Some(FileStats::Known { language_id, stats })) = parse_file(&path, false) else {
                panic!("expected known file stats for .{extension}");
            };

            assert_eq!(
                language::get(language_id).name,
                match extension {
                    "js" => "JavaScript",
                    "ts" => "TypeScript",
                    "svelte" => "Svelte",
                    "vue" => "Vuejs Component",
                    "astro" => "Astro",
                    "go" => "Go",
                    _ => unreachable!(),
                }
            );
            assert_eq!(stats.lines, 5, ".{extension}");
            assert_eq!(stats.comments, 0, ".{extension}");
            assert_eq!(stats.code, 5, ".{extension}");

            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn ignores_comment_delimiters_inside_multiline_quotes_across_buffers() {
        let contents = format!(
            "const x = `{}\n/* not a comment\nstill string\n`;\nfoo();\n",
            "x".repeat(BUFFER_BYTES)
        );
        let path = temp_file("long-multiline-template.js", contents.as_bytes());
        let Ok(Some(FileStats::Known { stats, .. })) = parse_file(&path, false) else {
            panic!("expected JavaScript file stats");
        };

        assert_eq!(stats.lines, 5);
        assert_eq!(stats.comments, 0);
        assert_eq!(stats.code, 5);

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
