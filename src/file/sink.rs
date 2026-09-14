use crate::{
    file::FileStats,
    language::{self, LanguageId},
};
use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub struct Sink {
    files: AtomicU64,
    collect_samples: bool,
    inner: Mutex<SinkInner>,
}

#[derive(Default)]
struct SinkInner {
    all: Stats,
    unknown: Stats,
    per_language: Vec<Stats>,
    unknown_formats: HashMap<String, u64>,
    samples: Vec<(Option<LanguageId>, Stats)>,
}

impl Sink {
    #[cfg(test)]
    pub fn new() -> Arc<Self> {
        Self::new_with_samples(false)
    }

    pub fn new_with_samples(collect_samples: bool) -> Arc<Self> {
        Arc::new(Self {
            files: AtomicU64::new(0),
            collect_samples,
            inner: Mutex::new(SinkInner {
                per_language: vec![Stats::default(); language::count()],
                ..SinkInner::default()
            }),
        })
    }

    pub fn collects_samples(&self) -> bool {
        self.collect_samples
    }

    pub fn record_progress(&self, files: u64) {
        self.files.fetch_add(files, Ordering::Relaxed);
        trace_event!("progress_update", None, serde_json::json!({"files": files}));
    }

    pub fn add_batch(&self, batch: &mut Batch) {
        if batch.all.files == 0 {
            return;
        }

        let _span = trace_span!("sink_merge_batch", None);
        trace_event!(
            "sink_batch",
            None,
            serde_json::json!({"files": batch.all.files, "lines": batch.all.lines})
        );
        let mut sink = self.inner.lock().unwrap();
        sink.all += batch.all;
        sink.unknown += batch.unknown;

        for (sink_stats, batch_stats) in sink.per_language.iter_mut().zip(&mut batch.per_language) {
            if batch_stats.files == 0 {
                continue;
            }

            *sink_stats += *batch_stats;
            *batch_stats = Stats::default();
        }

        for (format, files) in batch.unknown_formats.drain() {
            *sink.unknown_formats.entry(format).or_default() += files;
        }

        sink.samples.append(&mut batch.samples);

        batch.clear();
    }

    pub fn files(&self) -> u64 {
        self.files.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Summary {
        let _span = trace_span!("sink_snapshot", None);
        let sink = self.inner.lock().unwrap();
        let mut languages = sink
            .per_language
            .iter()
            .enumerate()
            .filter_map(|(index, &stats)| (stats.files > 0).then_some((LanguageId(index), stats)))
            .collect::<Vec<_>>();

        languages.sort_by_key(|&(language_id, _)| language_id.0);

        let mut unknown_formats = sink
            .unknown_formats
            .iter()
            .map(|(format, &files)| (format.clone(), files))
            .collect::<Vec<_>>();
        unknown_formats.sort_by(|(left_format, left_files), (right_format, right_files)| {
            right_files
                .cmp(left_files)
                .then_with(|| left_format.cmp(right_format))
        });

        Summary {
            all: sink.all,
            unknown: sink.unknown,
            unknown_formats,
            languages,
            samples: sink.samples.clone(),
        }
    }
}

pub struct Batch {
    collect_samples: bool,
    all: Stats,
    unknown: Stats,
    per_language: Vec<Stats>,
    unknown_formats: HashMap<String, u64>,
    samples: Vec<(Option<LanguageId>, Stats)>,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            collect_samples: false,
            all: Stats::default(),
            unknown: Stats::default(),
            per_language: vec![Stats::default(); language::count()],
            unknown_formats: HashMap::new(),
            samples: Vec::new(),
        }
    }
}

impl Batch {
    pub fn with_samples(collect: bool) -> Self {
        let mut batch = Self::default();
        batch.collect_samples = collect;
        batch
    }

    pub fn add(&mut self, file_stats: FileStats) {
        #[cfg(feature = "trace")]
        let (lines, known) = match &file_stats {
            FileStats::Known { stats, .. } => (stats.lines, true),
            FileStats::Unknown { stats, .. } => (stats.lines, false),
        };
        trace_event!(
            "batch_add_file",
            None,
            serde_json::json!({"lines": lines, "known": known})
        );
        match file_stats {
            FileStats::Known { language_id, stats } => {
                if self.collect_samples {
                    self.samples.push((Some(language_id), stats));
                }
                self.all += stats;
                self.per_language[language_id.0] += stats;
            }
            FileStats::Unknown { format, stats } => {
                if self.collect_samples {
                    self.samples.push((None, stats));
                }
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

    fn clear(&mut self) {
        self.all = Stats::default();
        self.unknown = Stats::default();
        self.unknown_formats.clear();
    }
}

pub struct Summary {
    pub all: Stats,
    pub unknown: Stats,
    pub unknown_formats: Vec<(String, u64)>,
    pub languages: Vec<(LanguageId, Stats)>,
    pub samples: Vec<(Option<LanguageId>, Stats)>,
}

#[derive(Default, Clone, Copy, serde::Serialize)]
pub struct Stats {
    pub files: u64,
    pub lines: u64,
    pub comments: u64,
    pub blanks: u64,
    pub code: u64,
}

impl AddAssign for Stats {
    fn add_assign(&mut self, other: Self) {
        self.files += other.files;
        self.lines += other.lines;
        self.comments += other.comments;
        self.blanks += other.blanks;
        self.code += other.code;
    }
}
