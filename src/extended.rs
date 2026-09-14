use crate::file::Summary;
use std::io::{self, ErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Min,
    Max,
    Median,
    Mean,
    Sd,
    Percentile(u8),
}

impl Kind {
    pub fn name(self) -> String {
        match self {
            Self::Min => "min".into(),
            Self::Max => "max".into(),
            Self::Median => "median".into(),
            Self::Mean => "mean".into(),
            Self::Sd => "sd".into(),
            Self::Percentile(n) => format!("p{n}"),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Min => "Min".into(),
            Self::Max => "Max".into(),
            Self::Median => "Median".into(),
            Self::Mean => "Mean".into(),
            Self::Sd => "SD".into(),
            Self::Percentile(n) => format!("P{n}"),
        }
    }
}

pub fn parse(args: &[String]) -> io::Result<Vec<Kind>> {
    let mut result = Vec::new();
    for arg in args {
        let kinds = match arg.as_str() {
            "default" => vec![Kind::Min, Kind::Max, Kind::Median, Kind::Sd],
            "min" => vec![Kind::Min],
            "max" => vec![Kind::Max],
            "median" => vec![Kind::Median],
            "mean" => vec![Kind::Mean],
            "sd" => vec![Kind::Sd],
            value => {
                let percentile = value
                    .strip_prefix('p')
                    .and_then(|digits| {
                        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                            .then_some(digits)
                    })
                    .and_then(|digits| digits.parse::<u8>().ok())
                    .filter(|&n| n <= 100);
                match percentile {
                    Some(n) => vec![Kind::Percentile(n)],
                    None => {
                        return Err(io::Error::new(
                            ErrorKind::InvalidInput,
                            format!(
                                "invalid extended statistic '{value}'; use min, max, mean, median, sd, or p0..p100"
                            ),
                        ));
                    }
                }
            }
        };
        for kind in kinds {
            if !result.contains(&kind) {
                result.push(kind);
            }
        }
    }
    Ok(result)
}

#[derive(Clone, Copy, serde::Serialize)]
pub struct Values {
    pub blanks: Option<f64>,
    pub comments: Option<f64>,
    pub code: Option<f64>,
}

pub fn calculate_all(
    summary: &Summary,
    language: Option<&str>,
    kinds: &[Kind],
) -> Vec<(Kind, Values)> {
    if kinds.is_empty() {
        return Vec::new();
    }
    let mut columns = [Vec::new(), Vec::new(), Vec::new()];
    for &(id, stats) in &summary.samples {
        let sample_language = id
            .map(|id| crate::language::get(id).name)
            .unwrap_or("Unknown");
        if language.is_none() || language == Some(sample_language) {
            for (column, value) in
                columns
                    .iter_mut()
                    .zip([stats.blanks, stats.comments, stats.code])
            {
                column.push(value);
            }
        }
    }
    for column in &mut columns {
        column.sort_unstable();
    }
    kinds
        .iter()
        .map(|&kind| {
            let [blanks, comments, code] = columns.each_ref().map(|values| measure(values, kind));
            (
                kind,
                Values {
                    blanks,
                    comments,
                    code,
                },
            )
        })
        .collect()
}

fn measure(sorted: &[u64], kind: Kind) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    Some(match kind {
        Kind::Min => sorted[0] as f64,
        Kind::Max => sorted[n - 1] as f64,
        Kind::Median => percentile(sorted, 50),
        Kind::Mean => sorted.iter().map(|&v| v as f64).sum::<f64>() / n as f64,
        Kind::Percentile(p) => percentile(sorted, p),
        Kind::Sd => {
            let mean = sorted.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
            (sorted
                .iter()
                .map(|&v| (v as f64 - mean).powi(2))
                .sum::<f64>()
                / n as f64)
                .sqrt()
        }
    })
}

fn percentile(sorted: &[u64], p: u8) -> f64 {
    let rank = (sorted.len() - 1) as f64 * f64::from(p) / 100.0;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let fraction = rank - lower as f64;
    sorted[lower] as f64 * (1.0 - fraction) + sorted[upper] as f64 * fraction
}
