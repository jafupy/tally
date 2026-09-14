use crate::Kind;

#[derive(Clone, Copy, serde::Serialize)]
pub struct Values {
    pub blanks: Option<f64>,
    pub comments: Option<f64>,
    pub code: Option<f64>,
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub blanks: u64,
    pub comments: u64,
    pub code: u64,
}

pub fn calculate_all(
    samples: impl IntoIterator<Item = Sample>,
    kinds: &[Kind],
) -> Vec<(Kind, Values)> {
    if kinds.is_empty() {
        return Vec::new();
    }
    let mut columns = [Vec::new(), Vec::new(), Vec::new()];
    for sample in samples {
        for (column, value) in columns
            .iter_mut()
            .zip([sample.blanks, sample.comments, sample.code])
        {
            column.push(value);
        }
    }
    for column in &mut columns {
        column.sort_unstable();
    }
    kinds
        .iter()
        .map(|kind| {
            let [blanks, comments, code] = columns.each_ref().map(|values| measure(values, kind));
            (
                kind.clone(),
                Values {
                    blanks,
                    comments,
                    code,
                },
            )
        })
        .collect()
}

fn measure(sorted: &[u64], kind: &Kind) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    Some(match kind {
        Kind::Min => sorted[0] as f64,
        Kind::Max => sorted[n - 1] as f64,
        Kind::Median => percentile(sorted, 50.0),
        Kind::Mean => sorted.iter().map(|&v| v as f64).sum::<f64>() / n as f64,
        Kind::Percentile(p) => percentile(sorted, f64::from(*p)),
        Kind::Iqr => percentile(sorted, 75.0) - percentile(sorted, 25.0),
        Kind::Variance => population_variance(sorted),
        Kind::Sd => population_variance(sorted).sqrt(),
        Kind::Formula(formula) => return formula.evaluate(sorted),
    })
}

fn population_variance(values: &[u64]) -> f64 {
    let mean = values.iter().map(|&v| v as f64).sum::<f64>() / values.len() as f64;
    values
        .iter()
        .map(|&v| (v as f64 - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64
}

pub(crate) fn percentile(sorted: &[u64], p: f64) -> f64 {
    let rank = (sorted.len() - 1) as f64 * p / 100.0;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let fraction = rank - lower as f64;
    sorted[lower] as f64 * (1.0 - fraction) + sorted[upper] as f64 * fraction
}
