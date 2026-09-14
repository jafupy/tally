use crate::formula::Formula;
use std::fmt;

#[derive(Debug)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Min,
    Max,
    Median,
    Mean,
    Sd,
    Iqr,
    Variance,
    Percentile(u8),
    Formula(Formula),
}

impl Kind {
    pub fn name(&self) -> String {
        match self {
            Self::Min => "min".into(),
            Self::Max => "max".into(),
            Self::Median => "median".into(),
            Self::Mean => "mean".into(),
            Self::Sd => "sd".into(),
            Self::Iqr => "iqr".into(),
            Self::Variance => "variance".into(),
            Self::Percentile(n) => format!("p{n}"),
            Self::Formula(formula) => formula.name.clone(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Min => "Min".into(),
            Self::Max => "Max".into(),
            Self::Median => "Median".into(),
            Self::Mean => "Mean".into(),
            Self::Sd => "SD".into(),
            Self::Iqr => "IQR".into(),
            Self::Variance => "Variance".into(),
            Self::Percentile(n) => format!("P{n}"),
            Self::Formula(formula) => formula.name.clone(),
        }
    }
}

pub fn parse(args: &[String]) -> Result<Vec<Kind>, ParseError> {
    let mut result = Vec::new();
    for arg in args {
        let kinds = match arg.as_str() {
            "default" => vec![Kind::Min, Kind::Max, Kind::Median, Kind::Sd],
            "min" => vec![Kind::Min],
            "max" => vec![Kind::Max],
            "median" => vec![Kind::Median],
            "mean" => vec![Kind::Mean],
            "sd" => vec![Kind::Sd],
            "iqr" => vec![Kind::Iqr],
            "variance" => vec![Kind::Variance],
            value => {
                if value.contains('=') {
                    vec![Kind::Formula(Formula::parse(value).map_err(ParseError)?)]
                } else {
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
                            return Err(ParseError(format!(
                                "invalid extended statistic '{value}'; use min, max, mean, median, sd, iqr, variance, p0..p100, or NAME=EXPR"
                            )));
                        }
                    }
                }
            }
        };
        for kind in kinds {
            if let Some(existing) = result
                .iter()
                .find(|existing: &&Kind| existing.name() == kind.name())
            {
                if existing != &kind {
                    return Err(ParseError(format!(
                        "duplicate extended statistic name '{}'",
                        kind.name()
                    )));
                }
            } else {
                result.push(kind);
            }
        }
    }
    Ok(result)
}
