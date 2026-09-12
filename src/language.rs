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
