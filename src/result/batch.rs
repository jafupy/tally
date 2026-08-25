use super::Stats;
use crate::{count::FileStats, language::LanguageId};
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
};

type LanguageMap = HashMap<LanguageId, Stats, BuildHasherDefault<LanguageHasher>>;

#[derive(Default)]
struct LanguageHasher(u64);

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
    all: Stats,
    unknown: Stats,
    per_language: LanguageMap,
    unknown_formats: HashMap<String, u64>,
}

impl Batch {
    pub fn add(&mut self, file: FileStats) {
        match file {
            FileStats::Known { language_id, stats } => {
                self.all += stats;
                *self.per_language.entry(language_id).or_default() += stats;
            }
            FileStats::Unknown { format, stats } => {
                self.all += stats;
                self.unknown += stats;
                if let Some(format) = format {
                    *self.unknown_formats.entry(format).or_default() += 1;
                }
            }
        }
    }

    pub fn files(&self) -> u64 {
        self.all.files
    }

    pub(crate) fn totals(&self) -> (Stats, Stats) {
        (self.all, self.unknown)
    }

    pub(crate) fn languages(&self) -> impl Iterator<Item = (LanguageId, Stats)> + '_ {
        self.per_language
            .iter()
            .map(|(&language, &stats)| (language, stats))
    }

    pub(crate) fn unknown_formats(&self) -> impl Iterator<Item = (&str, u64)> + '_ {
        self.unknown_formats
            .iter()
            .map(|(format, &files)| (format.as_str(), files))
    }

    pub(crate) fn absorb(&mut self, other: &mut Self) {
        self.all += other.all;
        self.unknown += other.unknown;

        for (language, stats) in other.per_language.drain() {
            *self.per_language.entry(language).or_default() += stats;
        }
        for (format, files) in other.unknown_formats.drain() {
            *self.unknown_formats.entry(format).or_default() += files;
        }

        other.all = Stats::default();
        other.unknown = Stats::default();
    }
}
