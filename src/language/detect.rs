use super::{
    DISAMBIGUATION_RULES, LanguageId, extension_languages, filename_language, language_named,
};
use regex::{Regex, RegexBuilder};
use std::{cmp::Ordering, path::Path, sync::OnceLock};

const DISAMBIGUATION_MIN_SCORE: u32 = 4;
const DISAMBIGUATION_MIN_MARGIN: u32 = 2;

pub fn detect_path(path: &Path, contents_prefix: Option<&str>) -> Option<LanguageId> {
    let filename = path.file_name()?.to_str()?;

    if let Some(language_id) = filename_language(filename) {
        return Some(language_id);
    }

    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        let candidates = extension_languages(extension);
        if !candidates.is_empty() {
            return match candidates {
                [language_id] => Some(*language_id),
                candidates => disambiguate(candidates, contents_prefix)
                    .or_else(|| candidates.first().copied()),
            };
        }
    }

    contents_prefix.and_then(detect_shebang)
}

pub(super) fn cmp_ignore_ascii_case(left: &str, right: &str) -> Ordering {
    for (left_byte, right_byte) in left.bytes().zip(right.bytes()) {
        match left_byte
            .to_ascii_lowercase()
            .cmp(&right_byte.to_ascii_lowercase())
        {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }

    left.len().cmp(&right.len())
}

fn detect_shebang(contents: &str) -> Option<LanguageId> {
    let line = contents
        .split_once('\n')
        .map_or(contents, |(line, _)| line)
        .strip_prefix("#!")?
        .trim_start();
    let mut words = line.split_ascii_whitespace();
    let executable = words.next()?;

    if !executable.starts_with('/') {
        return None;
    }

    let mut interpreter = Path::new(executable).file_name()?.to_str()?;
    if interpreter == "env" {
        interpreter = words.find(|word| !word.starts_with('-') && !word.contains('='))?;
        interpreter = Path::new(interpreter).file_name()?.to_str()?;
    }

    if interpreter.starts_with("python") || interpreter.starts_with("pypy") {
        return language_named("Python");
    }

    if matches!(interpreter, "node" | "nodejs" | "deno" | "bun") {
        return language_named("JavaScript");
    }

    if matches!(interpreter, "sh" | "bash" | "zsh" | "ksh" | "dash") {
        return language_named("Shell");
    }

    None
}

fn disambiguate(candidates: &[LanguageId], contents_prefix: Option<&str>) -> Option<LanguageId> {
    let contents_prefix = contents_prefix?;
    let regexes = compiled_disambiguation_regexes();

    let mut best = None;
    let mut second_score = 0;

    for &candidate in candidates {
        let score = regexes[candidate.0]
            .iter()
            .zip(DISAMBIGUATION_RULES[candidate.0])
            .filter_map(|(regex, rule)| regex.is_match(contents_prefix).then_some(rule.score))
            .sum::<u32>();

        match best {
            None => best = Some((candidate, score)),
            Some((_, best_score)) if score > best_score => {
                second_score = best_score;
                best = Some((candidate, score));
            }
            _ => second_score = second_score.max(score),
        }
    }

    let (language_id, best_score) = best?;
    (best_score >= DISAMBIGUATION_MIN_SCORE
        && best_score >= second_score + DISAMBIGUATION_MIN_MARGIN)
        .then_some(language_id)
}

fn compiled_disambiguation_regexes() -> &'static [Vec<Regex>] {
    static REGEXES: OnceLock<Vec<Vec<Regex>>> = OnceLock::new();

    REGEXES.get_or_init(|| {
        DISAMBIGUATION_RULES
            .iter()
            .map(|rules| {
                rules
                    .iter()
                    .map(|rule| {
                        RegexBuilder::new(rule.regex)
                            .multi_line(true)
                            .build()
                            .expect("generated disambiguation regex failed")
                    })
                    .collect()
            })
            .collect()
    })
}
