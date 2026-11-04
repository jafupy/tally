use super::syntax::{StreamingLine, Syntax, contains_non_whitespace, count_line};
use super::{BUFFER_BYTES, DETECTION_PREFIX_BYTES, UNKNOWN, text_prefix};
use crate::language::{self, LanguageDef, LanguageId, QuoteDef};
use crate::result::Stats;
use memchr::memchr_iter;
use std::collections::HashSet;
use std::io::{self, BufRead};
use std::path::Path;

pub fn count_selected_contents(
    path: &Path,
    contents: &[u8],
    selected: &HashSet<usize>,
) -> Option<(Option<LanguageId>, Stats)> {
    let prefix = &contents[..contents.len().min(DETECTION_PREFIX_BYTES)];
    let contents_prefix = text_prefix(prefix)?;
    let language_id = language::detect_path(path, Some(contents_prefix));
    let language = language_id.map(language::get).unwrap_or(&UNKNOWN);
    let syntax = Syntax::new(language);
    let mut block_comment = None;
    let mut multiline_quote = None;
    let mut stats = Stats::default();

    for (index, line) in contents.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let mut line_stats = Stats::default();
        count_line(
            line,
            &syntax,
            &mut block_comment,
            &mut multiline_quote,
            &mut line_stats,
            syntax.find_block_start(line).is_some(),
        );
        if selected.contains(&(index + 1)) {
            stats += line_stats;
        }
    }
    Some((language_id, stats))
}

pub(super) fn count_lines(mut reader: impl BufRead, language: &LanguageDef) -> io::Result<Stats> {
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
                let _line = trace_value!(stats);

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
                trace_event!(line_classified, None, _line, &stats);

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

    let _line = trace_value!(stats);
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
    trace_event!(line_classified, None, _line, &stats);

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
                trace_event!(plain_line_classified, None, stats.lines, line_has_code);
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
        trace_event!(plain_line_classified, None, stats.lines, line_has_code);
        if line_has_code {
            stats.code += 1;
        } else {
            stats.blanks += 1;
        }
    }

    Ok(stats)
}
