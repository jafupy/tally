use super::Stats;
use crate::language::LanguageId;

pub struct Summary {
    pub all: Stats,
    pub unknown: Stats,
    #[cfg(feature = "debug")]
    pub unknown_formats: Vec<(String, u64)>,
    pub languages: Vec<(LanguageId, Stats)>,
    pub samples: Vec<(Option<LanguageId>, Stats)>,
}
