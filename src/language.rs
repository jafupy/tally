use regex::{Regex, RegexBuilder};
use std::cmp::Ordering;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LanguageId(pub usize);

#[derive(Debug)]
pub struct LanguageDef {
    pub name: &'static str,
    pub line_comments: &'static [&'static str],
    pub block_comments: &'static [(&'static str, &'static str)],
}

#[derive(Debug)]
pub struct DisambiguationRule {
    pub regex: &'static str,
    pub score: u32,
}

include!(concat!(env!("OUT_DIR"), "/languages.rs"));

const DISAMBIGUATION_MIN_SCORE: u32 = 4;
const DISAMBIGUATION_MIN_MARGIN: u32 = 2;

pub fn get(id: LanguageId) -> &'static LanguageDef {
    &LANGUAGES[id.0]
}

pub fn count() -> usize {
    LANGUAGES.len()
}

pub fn detect_path(path: &Path, contents_prefix: Option<&str>) -> Option<LanguageId> {
    let filename = path.file_name()?.to_str()?;

    if let Some(language_id) = filename_language(filename) {
        return Some(language_id);
    }

    if let Some(language_id) = contents_prefix.and_then(detect_shebang) {
        return Some(language_id);
    }

    let extension = path.extension()?.to_str()?;
    let candidates = extension_languages(extension);

    match candidates {
        [] => None,
        [language_id] => Some(*language_id),
        candidates => {
            disambiguate(candidates, contents_prefix).or_else(|| candidates.first().copied())
        }
    }
}

fn cmp_ignore_ascii_case(left: &str, right: &str) -> Ordering {
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
    if !contents.as_bytes().starts_with(b"#!") {
        return None;
    }

    let line = contents
        .split_once('\n')
        .map_or(contents, |(line, _)| line)
        .strip_prefix("#!")?;

    if line.contains("python") {
        return language_named("Python");
    }

    if line.contains("node") || line.contains("deno") || line.contains("bun") {
        return language_named("JavaScript");
    }

    if line.contains("sh") || line.contains("bash") || line.contains("zsh") || line.contains("ksh")
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_exact_filenames() {
        let language_id = detect_path(Path::new("Dockerfile"), None).unwrap();
        assert_eq!(get(language_id).name, "Dockerfile");

        let language_id = detect_path(Path::new("Makefile"), None).unwrap();
        assert_eq!(get(language_id).name, "Makefile");
    }

    #[test]
    fn detects_suffixes_and_case_insensitive_extensions() {
        let language_id = detect_path(Path::new("config.yaml.in"), Some("key: value\n")).unwrap();
        assert_eq!(get(language_id).name, "YAML");

        let language_id = detect_path(Path::new("MAIN.RS"), Some("fn main() {}\n")).unwrap();
        assert_eq!(get(language_id).name, "Rust");
    }

    #[test]
    fn disambiguates_m_files() {
        let matlab =
            detect_path(Path::new("plot.m"), Some("function y = plot(x)\ny = x;\n")).unwrap();
        assert_eq!(get(matlab).name, "MATLAB");

        let objc = detect_path(
            Path::new("main.m"),
            Some("#import <Foundation/Foundation.h>\n@interface App : NSObject\n@end\n"),
        )
        .unwrap();
        assert_eq!(get(objc).name, "Objective-C");
    }

    #[test]
    fn disambiguates_v_files() {
        let v = detect_path(Path::new("main.v"), Some("module main\nfn main() {\n}\n")).unwrap();
        assert_eq!(get(v).name, "V");

        let verilog = detect_path(
            Path::new("counter.v"),
            Some("module counter(input clk);\nalways @(posedge clk) begin\nend\nendmodule\n"),
        )
        .unwrap();
        assert_eq!(get(verilog).name, "Verilog");
    }
}
