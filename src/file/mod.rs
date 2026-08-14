mod sink;

use crate::language::{self, LanguageDef, LanguageId, QuoteDef};
use memchr::{memchr, memchr_iter, memchr2, memchr3, memmem};
pub use sink::{Batch, Sink, Stats, Summary};
use std::fs::File;
use std::io::{self, BufRead, Read};
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

pub fn parse_file(path: &Path, verbose: bool) -> io::Result<Option<FileStats>> {
    let mut buffer = vec![0; BUFFER_BYTES];
    parse_file_buffered(path, verbose, &mut buffer)
}

pub fn parse_file_buffered(
    path: &Path,
    verbose: bool,
    buffer: &mut [u8],
) -> io::Result<Option<FileStats>> {
    let mut reader = ReusableBufReader::new(File::open(path)?, buffer);

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
    vec![0; BUFFER_BYTES]
}

struct ReusableBufReader<'a> {
    file: File,
    buffer: &'a mut [u8],
    position: usize,
    filled: usize,
}

impl<'a> ReusableBufReader<'a> {
    fn new(file: File, buffer: &'a mut [u8]) -> Self {
        debug_assert!(buffer.len() >= BUFFER_BYTES);
        Self {
            file,
            buffer: &mut buffer[..BUFFER_BYTES],
            position: 0,
            filled: 0,
        }
    }
}

impl Read for ReusableBufReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let amount = available.len().min(output.len());
        output[..amount].copy_from_slice(&available[..amount]);
        self.consume(amount);
        Ok(amount)
    }
}

impl BufRead for ReusableBufReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.filled {
            self.filled = self.file.read(self.buffer)?;
            self.position = 0;
        }
        Ok(&self.buffer[self.position..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.filled);
    }
}

fn read_prefix(reader: &mut impl BufRead) -> io::Result<&[u8]> {
    let buffer = reader.fill_buf()?;
    Ok(&buffer[..buffer.len().min(DETECTION_PREFIX_BYTES)])
}

fn count_lines(mut reader: impl BufRead, language: &LanguageDef) -> io::Result<Stats> {
    if language.line_comments.is_empty() && language.block_comments.is_empty() {
        return count_plain_lines(reader);
    }

    let syntax = Syntax::new(language);
    let mut stats = Stats {
        files: 1,
        ..Stats::default()
    };
    let mut block_comment: Option<&str> = None;
    let mut multiline_quote: Option<&QuoteDef> = None;
    let mut partial_line = Vec::new();
    let mut long_line: Option<StreamingLine<'_>> = None;

    loop {
        let consumed = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break;
            }

            let mut start = 0;
            let mut next_block = syntax.find_block_start(buffer);

            for newline in memchr_iter(b'\n', buffer) {
                let end = newline + 1;
                let line = &buffer[start..end];

                if let Some(mut state) = long_line.take() {
                    state.push(line, &mut block_comment);
                    state.finish(&mut block_comment, &mut multiline_quote, &mut stats);
                } else if partial_line.is_empty() {
                    count_line(
                        line,
                        &syntax,
                        &mut block_comment,
                        &mut multiline_quote,
                        &mut stats,
                        next_block.is_some_and(|at| at < end),
                    );
                } else if partial_line.len() + line.len() <= BUFFER_BYTES {
                    partial_line.extend_from_slice(line);
                    count_line(
                        &partial_line,
                        &syntax,
                        &mut block_comment,
                        &mut multiline_quote,
                        &mut stats,
                        true,
                    );
                    partial_line.clear();
                } else {
                    let mut state = StreamingLine::new(&syntax, multiline_quote);
                    state.push(&partial_line, &mut block_comment);
                    state.push(line, &mut block_comment);
                    state.finish(&mut block_comment, &mut multiline_quote, &mut stats);
                    partial_line.clear();
                }

                start = end;
                if next_block.is_some_and(|at| at < end) {
                    next_block = syntax
                        .find_block_start(&buffer[start..])
                        .map(|relative_at| start + relative_at);
                }
            }

            if start < buffer.len() {
                let fragment = &buffer[start..];
                if let Some(state) = &mut long_line {
                    state.push(fragment, &mut block_comment);
                } else if partial_line.len() + fragment.len() <= BUFFER_BYTES {
                    partial_line.extend_from_slice(fragment);
                } else {
                    let mut state = StreamingLine::new(&syntax, multiline_quote);
                    state.push(&partial_line, &mut block_comment);
                    state.push(fragment, &mut block_comment);
                    partial_line.clear();
                    long_line = Some(state);
                }
            }

            buffer.len()
        };

        reader.consume(consumed);
    }

    if let Some(state) = long_line {
        state.finish(&mut block_comment, &mut multiline_quote, &mut stats);
    } else if !partial_line.is_empty() {
        count_line(
            &partial_line,
            &syntax,
            &mut block_comment,
            &mut multiline_quote,
            &mut stats,
            true,
        );
    }

    Ok(stats)
}

fn count_plain_lines(mut reader: impl BufRead) -> io::Result<Stats> {
    let mut stats = Stats {
        files: 1,
        ..Stats::default()
    };
    let mut line_has_code = false;
    let mut line_pending = false;

    loop {
        let consumed = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break;
            }

            let mut start = 0;
            for end in memchr_iter(b'\n', buffer) {
                line_has_code |= contains_non_whitespace(&buffer[start..end]);
                stats.lines += 1;
                if line_has_code {
                    stats.code += 1;
                } else {
                    stats.blanks += 1;
                }
                line_has_code = false;
                line_pending = false;
                start = end + 1;
            }

            if start < buffer.len() {
                line_has_code |= contains_non_whitespace(&buffer[start..]);
                line_pending = true;
            }

            buffer.len()
        };
        reader.consume(consumed);
    }

    if line_pending {
        stats.lines += 1;
        if line_has_code {
            stats.code += 1;
        } else {
            stats.blanks += 1;
        }
    }

    Ok(stats)
}

struct Syntax<'a> {
    language: &'a LanguageDef,
    single_block_finder: Option<memmem::Finder<'a>>,
}

impl<'a> Syntax<'a> {
    fn new(language: &'a LanguageDef) -> Self {
        let single_block_finder = match language.block_comments {
            [(start, _)] => Some(memmem::Finder::new(start.as_bytes())),
            _ => None,
        };
        Self {
            language,
            single_block_finder,
        }
    }

    #[inline(always)]
    fn find_block_start(&self, bytes: &[u8]) -> Option<usize> {
        match self.language.block_comments {
            [] => None,
            [_] => self.single_block_finder.as_ref()?.find(bytes),
            _ => {
                let mut offset = 0;
                while offset < bytes.len() {
                    let at =
                        offset + find_candidate(&bytes[offset..], self.language.block_candidates)?;
                    if self
                        .language
                        .block_comments
                        .iter()
                        .any(|(start, _)| bytes[at..].starts_with(start.as_bytes()))
                    {
                        return Some(at);
                    }
                    offset = at + 1;
                }
                None
            }
        }
    }

    fn find_comment_start(&self, bytes: &[u8]) -> Option<(usize, CommentStart<'a>)> {
        let mut offset = 0;
        while offset < bytes.len() {
            let at = offset + find_candidate(&bytes[offset..], self.language.comment_candidates)?;
            let remainder = &bytes[at..];

            if self
                .language
                .line_comments
                .iter()
                .any(|comment| remainder.starts_with(comment.as_bytes()))
            {
                return Some((at, CommentStart::Line));
            }

            if let Some(&(start, end)) = self
                .language
                .block_comments
                .iter()
                .find(|(start, _)| remainder.starts_with(start.as_bytes()))
            {
                return Some((
                    at,
                    CommentStart::Block {
                        start_len: start.len(),
                        end,
                    },
                ));
            }

            offset = at + 1;
        }
        None
    }

    fn find_quote_start(&self, bytes: &[u8]) -> Option<(usize, &'a QuoteDef)> {
        let mut offset = 0;
        while offset < bytes.len() {
            let at = offset + find_candidate(&bytes[offset..], self.language.quote_candidates)?;
            let remainder = &bytes[at..];
            if let Some(quote) = self
                .language
                .quotes
                .iter()
                .find(|quote| remainder.starts_with(quote.start.as_bytes()))
            {
                return Some((at, quote));
            }
            offset = at + 1;
        }
        None
    }

    #[inline(always)]
    fn starts_line_comment(&self, bytes: &[u8]) -> bool {
        self.language
            .line_comments
            .iter()
            .any(|comment| bytes.starts_with(comment.as_bytes()))
    }

    #[inline(always)]
    fn starts_block_comment(&self, bytes: &[u8]) -> bool {
        self.language
            .block_comments
            .iter()
            .any(|(start, _)| bytes.starts_with(start.as_bytes()))
    }
}

enum CommentStart<'a> {
    Line,
    Block { start_len: usize, end: &'a str },
}

#[derive(Default)]
struct LineState<'a> {
    quote: Option<&'a QuoteDef>,
    saw_code: bool,
    saw_comment: bool,
    in_line_comment: bool,
}

#[inline(always)]
fn count_line<'a>(
    line: &[u8],
    syntax: &Syntax<'a>,
    block_comment: &mut Option<&'a str>,
    multiline_quote: &mut Option<&'a QuoteDef>,
    stats: &mut Stats,
    may_open_block: bool,
) {
    stats.lines += 1;
    let trimmed = trim_start_ascii(line);

    if trimmed.is_empty() && multiline_quote.is_none() {
        stats.blanks += 1;
        return;
    }

    if block_comment.is_none() && multiline_quote.is_none() {
        if syntax.starts_line_comment(trimmed) {
            stats.comments += 1;
            return;
        }

        if !may_open_block && !syntax.language.quotes.iter().any(|quote| quote.multiline) {
            stats.code += 1;
            return;
        }

        if !syntax.starts_block_comment(trimmed) {
            stats.code += 1;
            let mut state = LineState {
                saw_code: true,
                ..LineState::default()
            };
            scan_bytes(trimmed, trimmed.len(), syntax, block_comment, &mut state);
            *multiline_quote = state.quote.filter(|quote| quote.multiline);
            return;
        }
    }

    let mut state = LineState {
        quote: *multiline_quote,
        ..LineState::default()
    };
    scan_bytes(trimmed, trimmed.len(), syntax, block_comment, &mut state);
    *multiline_quote = state.quote.filter(|quote| quote.multiline);
    apply_line_state(state, stats);
}

fn scan_bytes<'a>(
    bytes: &[u8],
    process_until: usize,
    syntax: &Syntax<'a>,
    block_comment: &mut Option<&'a str>,
    state: &mut LineState<'a>,
) -> usize {
    if state.in_line_comment {
        return bytes.len();
    }

    let mut position = 0;
    while position < process_until {
        let remainder = &bytes[position..];

        if let Some(end) = *block_comment {
            state.saw_comment = true;
            match find_token(remainder, end.as_bytes()) {
                Some(end_at) if position + end_at < process_until => {
                    position += end_at + end.len();
                    *block_comment = None;
                    continue;
                }
                _ => return process_until,
            }
        }

        if let Some(quote) = state.quote {
            state.saw_code = true;
            match find_quote_end(remainder, quote) {
                Some((end_at, after_end)) if position + end_at < process_until => {
                    position += after_end;
                    state.quote = None;
                    continue;
                }
                _ => return process_until,
            }
        }

        let comment = syntax.find_comment_start(remainder);
        let comment_at = comment
            .as_ref()
            .map_or(process_until - position, |(at, _)| *at);
        let scan_until = comment_at.min(process_until - position);

        if let Some((quote_at, quote)) = syntax.find_quote_start(&remainder[..scan_until]) {
            if contains_non_whitespace(&remainder[..quote_at]) {
                state.saw_code = true;
            }
            state.saw_code = true;
            position += quote_at + quote.start.len();
            state.quote = Some(quote);
            continue;
        }

        if position + comment_at >= process_until {
            if contains_non_whitespace(&remainder[..process_until - position]) {
                state.saw_code = true;
            }
            return process_until;
        }

        let Some((_, comment)) = comment else {
            if contains_non_whitespace(&remainder[..process_until - position]) {
                state.saw_code = true;
            }
            return process_until;
        };

        if contains_non_whitespace(&remainder[..comment_at]) {
            state.saw_code = true;
        }
        state.saw_comment = true;

        match comment {
            CommentStart::Line => {
                state.in_line_comment = true;
                return bytes.len();
            }
            CommentStart::Block { start_len, end } => {
                position += comment_at + start_len;
                *block_comment = Some(end);
            }
        }
    }

    position
}

fn find_quote_end(bytes: &[u8], quote: &QuoteDef) -> Option<(usize, usize)> {
    let end = quote.end.as_bytes();
    let mut offset = 0;

    if end.len() == 1 {
        if let Some(escape) = quote.escape {
            while offset < bytes.len() {
                let relative = memchr2(end[0], escape, &bytes[offset..])?;
                let at = offset + relative;
                if bytes[at] == escape {
                    offset = (at + 2).min(bytes.len());
                } else {
                    return Some((at, at + 1));
                }
            }
            return None;
        }

        let at = memchr(end[0], bytes)?;
        return Some((at, at + 1));
    }

    loop {
        let end_at = memmem::find(&bytes[offset..], end).map(|at| offset + at);
        let escape_at = quote
            .escape
            .and_then(|escape| memchr(escape, &bytes[offset..]))
            .map(|at| offset + at);

        match (end_at, escape_at) {
            (Some(end_at), Some(escape_at)) if escape_at < end_at => {
                offset = (escape_at + 2).min(bytes.len());
            }
            (Some(end_at), _) => return Some((end_at, end_at + end.len())),
            (None, Some(escape_at)) => offset = (escape_at + 2).min(bytes.len()),
            (None, None) => return None,
        }
    }
}

fn find_token(bytes: &[u8], token: &[u8]) -> Option<usize> {
    match token {
        [] => Some(0),
        [byte] => memchr(*byte, bytes),
        [first, second] => {
            let mut offset = 0;
            while offset + 1 < bytes.len() {
                let at = offset + memchr(*first, &bytes[offset..])?;
                if bytes.get(at + 1) == Some(second) {
                    return Some(at);
                }
                offset = at + 1;
            }
            None
        }
        _ => memmem::find(bytes, token),
    }
}

fn find_candidate(bytes: &[u8], candidates: &[u8]) -> Option<usize> {
    match candidates {
        [] => None,
        [one] => memchr(*one, bytes),
        [one, two] => memchr2(*one, *two, bytes),
        [one, two, three] => memchr3(*one, *two, *three, bytes),
        [one, two, three, rest @ ..] => {
            let mut nearest = memchr3(*one, *two, *three, bytes);
            for &candidate in rest {
                if let Some(at) = memchr(candidate, bytes)
                    && nearest.is_none_or(|nearest_at| at < nearest_at)
                {
                    nearest = Some(at);
                }
            }
            nearest
        }
    }
}

struct StreamingLine<'a> {
    syntax: &'a Syntax<'a>,
    pending: Vec<u8>,
    state: LineState<'a>,
}

impl<'a> StreamingLine<'a> {
    fn new(syntax: &'a Syntax<'a>, multiline_quote: Option<&'a QuoteDef>) -> Self {
        Self {
            syntax,
            pending: Vec::with_capacity(BUFFER_BYTES),
            state: LineState {
                quote: multiline_quote,
                ..LineState::default()
            },
        }
    }

    fn push(&mut self, bytes: &[u8], block_comment: &mut Option<&'a str>) {
        if self.state.in_line_comment || bytes.is_empty() {
            return;
        }

        self.pending.extend_from_slice(bytes);
        let keep = self.syntax.language.max_delimiter_len.saturating_sub(1);
        let process_until = self.pending.len().saturating_sub(keep);
        let consumed = scan_bytes(
            &self.pending,
            process_until,
            self.syntax,
            block_comment,
            &mut self.state,
        );
        self.discard(consumed);
    }

    fn finish(
        mut self,
        block_comment: &mut Option<&'a str>,
        multiline_quote: &mut Option<&'a QuoteDef>,
        stats: &mut Stats,
    ) {
        if !self.state.in_line_comment {
            let len = self.pending.len();
            scan_bytes(
                &self.pending,
                len,
                self.syntax,
                block_comment,
                &mut self.state,
            );
        }
        stats.lines += 1;
        *multiline_quote = self.state.quote.filter(|quote| quote.multiline);
        apply_line_state(self.state, stats);
    }

    fn discard(&mut self, consumed: usize) {
        if consumed == 0 {
            return;
        }
        let remaining = self.pending.len() - consumed;
        self.pending.copy_within(consumed.., 0);
        self.pending.truncate(remaining);
    }
}

fn apply_line_state(state: LineState<'_>, stats: &mut Stats) {
    if state.saw_code {
        stats.code += 1;
    } else if state.saw_comment {
        stats.comments += 1;
    } else {
        stats.blanks += 1;
    }
}

#[inline(always)]
fn trim_start_ascii(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    &bytes[start..]
}

fn contains_non_whitespace(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| !byte.is_ascii_whitespace())
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
