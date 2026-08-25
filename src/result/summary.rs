use super::{Batch, Stats};
use crate::language::{self, LanguageId};

pub struct Summary {
    pub all: Stats,
    pub unknown: Stats,
    pub unknown_formats: Vec<(String, u64)>,
    pub languages: Vec<(LanguageId, Stats)>,
}

pub fn from_batch(batch: &Batch) -> Summary {
    let (all, unknown) = batch.totals();
    let mut languages = batch.languages().collect::<Vec<_>>();
    radix_sort_languages(&mut languages);

    let mut unknown_formats = batch
        .unknown_formats()
        .map(|(format, files)| (format.to_owned(), files))
        .collect::<Vec<_>>();
    unknown_formats.sort_unstable_by(|(left_format, left_files), (right_format, right_files)| {
        right_files
            .cmp(left_files)
            .then_with(|| left_format.cmp(right_format))
    });

    Summary {
        all,
        unknown,
        unknown_formats,
        languages,
    }
}

fn radix_sort_languages(languages: &mut Vec<(LanguageId, Stats)>) {
    if languages.is_empty() {
        return;
    }

    let keys = languages
        .iter()
        .map(|&(language, stats)| sort_key(language, stats))
        .collect::<Vec<_>>();
    let mut order = (0..languages.len())
        .map(|index| u16::try_from(index).expect("language catalogue exceeds u16 indices"))
        .collect::<Vec<_>>();
    let mut scratch = vec![0u16; order.len()];

    for byte in 0..10 {
        let shift = byte * 8;
        let mut offsets = [0usize; 256];

        for &index in &order {
            offsets[key_byte(keys[usize::from(index)], shift)] += 1;
        }

        let mut next = 0;
        for count in &mut offsets {
            let start = next;
            next += *count;
            *count = start;
        }

        for &index in &order {
            let bucket = key_byte(keys[usize::from(index)], shift);
            scratch[offsets[bucket]] = index;
            offsets[bucket] += 1;
        }
        std::mem::swap(&mut order, &mut scratch);
    }

    let sorted = order
        .into_iter()
        .map(|index| languages[usize::from(index)])
        .collect();
    *languages = sorted;
}

fn sort_key(language: LanguageId, stats: Stats) -> u128 {
    (u128::from(!stats.code) << 16) | u128::from(language::get(language).alphabetical_rank)
}

fn key_byte(key: u128, shift: usize) -> usize {
    ((key >> shift) & 0xff) as usize
}
