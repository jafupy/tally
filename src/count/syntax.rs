use super::lines::BUFFER_BYTES;
use crate::{
    language::{LanguageDef, QuoteDef},
    result::Stats,
};
use memchr::{memchr, memchr2, memchr3, memmem};

pub(super) struct Syntax<'a> {
    language: &'a LanguageDef,
    single_block_finder: Option<memmem::Finder<'a>>,
}

impl<'a> Syntax<'a> {
    pub(super) fn new(language: &'a LanguageDef) -> Self {
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
    pub(super) fn find_block_start(&self, bytes: &[u8]) -> Option<usize> {
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
pub(super) fn count_line<'a>(
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

pub(super) struct StreamingLine<'a> {
    syntax: &'a Syntax<'a>,
    pending: Vec<u8>,
    state: LineState<'a>,
}

impl<'a> StreamingLine<'a> {
    pub(super) fn new(syntax: &'a Syntax<'a>, multiline_quote: Option<&'a QuoteDef>) -> Self {
        Self {
            syntax,
            pending: Vec::with_capacity(BUFFER_BYTES),
            state: LineState {
                quote: multiline_quote,
                ..LineState::default()
            },
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8], block_comment: &mut Option<&'a str>) {
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

    pub(super) fn finish(
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

pub(super) fn contains_non_whitespace(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| !byte.is_ascii_whitespace())
}
