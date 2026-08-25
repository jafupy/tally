mod detect;

use detect::cmp_ignore_ascii_case;
pub use detect::detect_path;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LanguageId(pub(crate) usize);

#[derive(Debug)]
pub struct QuoteDef {
    pub start: &'static str,
    pub end: &'static str,
    pub escape: Option<u8>,
    pub multiline: bool,
}

#[derive(Debug)]
pub struct LanguageDef {
    pub name: &'static str,
    pub alphabetical_rank: u16,
    pub line_comments: &'static [&'static str],
    pub block_comments: &'static [(&'static str, &'static str)],
    pub quotes: &'static [QuoteDef],
    pub comment_candidates: &'static [u8],
    pub block_candidates: &'static [u8],
    pub quote_candidates: &'static [u8],
    pub max_delimiter_len: usize,
}

#[derive(Debug)]
pub struct DisambiguationRule {
    pub regex: &'static str,
    pub score: u32,
}

include!(concat!(env!("OUT_DIR"), "/languages.rs"));

pub fn get(id: LanguageId) -> &'static LanguageDef {
    &LANGUAGES[id.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn includes_the_cloc_language_catalog() {
        assert_eq!(LANGUAGES.len(), 421);
    }

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
