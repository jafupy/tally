use super::{Batch, Stats, Summary};
use crate::language::{self, LanguageId};
#[cfg(feature = "debug")]
use std::collections::HashMap;
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
    #[cfg(feature = "debug")]
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

        for (language_id, stats) in batch.per_language.drain() {
            sink.per_language[language_id.0] += stats;
        }

        #[cfg(feature = "debug")]
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

        #[cfg(feature = "debug")]
        let mut unknown_formats = sink
            .unknown_formats
            .iter()
            .map(|(format, &files)| (format.clone(), files))
            .collect::<Vec<_>>();
        #[cfg(feature = "debug")]
        unknown_formats.sort_by(|(left_format, left_files), (right_format, right_files)| {
            right_files
                .cmp(left_files)
                .then_with(|| left_format.cmp(right_format))
        });

        Summary {
            all: sink.all,
            unknown: sink.unknown,
            #[cfg(feature = "debug")]
            unknown_formats,
            languages,
            samples: sink.samples.clone(),
        }
    }
}
