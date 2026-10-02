use super::Stats;
use crate::{file::FileStats, language::LanguageId};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

type LanguageMap = HashMap<LanguageId, Stats, BuildHasherDefault<LanguageHasher>>;

#[derive(Default)]
pub(super) struct LanguageHasher(u64);

impl Hasher for LanguageHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = 0xcbf29ce484222325u64;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        self.0 = hash;
    }
    fn write_usize(&mut self, value: usize) {
        self.0 = value as u64;
    }
}

#[derive(Default)]
pub struct Batch {
    pub(super) collect_samples: bool,
    pub(super) all: Stats,
    pub(super) unknown: Stats,
    pub(super) per_language: LanguageMap,
    pub(super) unknown_formats: crate::debug::UnknownFormats,
    pub(super) samples: Vec<(Option<LanguageId>, Stats)>,
}

impl Batch {
    pub fn with_samples(collect: bool) -> Self {
        Self {
            collect_samples: collect,
            ..Self::default()
        }
    }

    pub fn add(&mut self, file_stats: FileStats) {
        trace_event!(batch_add_file, None, &file_stats);
        match file_stats {
            FileStats::Known { language_id, stats } => {
                if self.collect_samples {
                    self.samples.push((Some(language_id), stats));
                }
                self.all += stats;
                *self.per_language.entry(language_id).or_default() += stats;
            }
            FileStats::Unknown { format, stats } => {
                if self.collect_samples {
                    self.samples.push((None, stats));
                }
                self.all += stats;
                self.unknown += stats;
                self.unknown_formats.record(format);
            }
        }
    }

    pub fn files(&self) -> u64 {
        self.all.files
    }

    pub(super) fn clear(&mut self) {
        self.all = Stats::default();
        self.unknown = Stats::default();
        self.unknown_formats.clear();
    }
}
