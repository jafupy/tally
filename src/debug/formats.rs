#[cfg(feature = "debug")]
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct UnknownFormats {
    #[cfg(feature = "debug")]
    counts: HashMap<String, u64>,
}

impl UnknownFormats {
    pub(crate) fn record(&mut self, format: Option<String>) {
        #[cfg(feature = "debug")]
        if let Some(format) = format {
            *self.counts.entry(format).or_default() += 1;
        }
        #[cfg(not(feature = "debug"))]
        let _ = format;
    }

    pub(crate) fn merge(&mut self, other: &mut Self) {
        #[cfg(feature = "debug")]
        for (format, count) in other.counts.drain() {
            *self.counts.entry(format).or_default() += count;
        }
        #[cfg(not(feature = "debug"))]
        let _ = other;
    }

    pub(crate) fn clear(&mut self) {
        #[cfg(feature = "debug")]
        self.counts.clear();
    }

    #[cfg(feature = "debug")]
    pub(crate) fn snapshot(&self) -> Vec<(String, u64)> {
        let mut rows = self
            .counts
            .iter()
            .map(|(format, &count)| (format.clone(), count))
            .collect::<Vec<_>>();
        rows.sort_by(|(a_format, a_count), (b_format, b_count)| {
            b_count.cmp(a_count).then_with(|| a_format.cmp(b_format))
        });
        rows
    }

    #[cfg(not(feature = "debug"))]
    pub(crate) fn snapshot(&self) {}
}
