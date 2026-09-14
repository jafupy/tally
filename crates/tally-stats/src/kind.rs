use std::fmt;

#[derive(Debug)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ParseError {}

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
                        return Err(ParseError(format!(
                            "invalid extended statistic '{value}'; use min, max, mean, median, sd, or p0..p100"
                        )));
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
