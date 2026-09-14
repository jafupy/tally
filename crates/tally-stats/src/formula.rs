use crate::distribution::percentile;

#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    pub name: String,
    expression: Expr,
}

impl Formula {
    pub(crate) fn parse(input: &str) -> Result<Self, String> {
        let (name, expression) = input.split_once('=').ok_or("formula needs NAME=EXPR")?;
        let valid_name = name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        });
        if name.is_empty() || !valid_name {
            return Err("formula name must contain only letters, digits, or underscores and start with a letter or underscore".into());
        }
        if expression.len() > 1024 {
            return Err("formula expression is too long".into());
        }
        let mut parser = Parser {
            input: expression.as_bytes(),
            position: 0,
        };
        let expression = parser.expression()?;
        parser.skip_space();
        if parser.position != parser.input.len() {
            return Err(format!(
                "unexpected text at formula position {}",
                parser.position + 1
            ));
        }
        Ok(Self {
            name: name.into(),
            expression,
        })
    }

    pub(crate) fn evaluate(&self, sorted: &[u64]) -> Option<f64> {
        if sorted.is_empty() {
            return None;
        }
        self.expression
            .evaluate(sorted)
            .filter(|value| value.is_finite())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Number(f64),
    Negative(Box<Expr>),
    Binary(char, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Function {
    Count,
    Sum,
    SumSq,
    Percentile,
    Sqrt,
}

impl Expr {
    fn evaluate(&self, sorted: &[u64]) -> Option<f64> {
        let value = match self {
            Self::Number(value) => *value,
            Self::Negative(expression) => -expression.evaluate(sorted)?,
            Self::Binary(operator, left, right) => {
                let left = left.evaluate(sorted)?;
                let right = right.evaluate(sorted)?;
                match operator {
                    '+' => left + right,
                    '-' => left - right,
                    '*' => left * right,
                    '/' if right != 0.0 => left / right,
                    '^' => left.powf(right),
                    _ => return None,
                }
            }
            Self::Call(function, arguments) => match function {
                Function::Count => sorted.len() as f64,
                Function::Sum => sorted.iter().map(|&value| value as f64).sum(),
                Function::SumSq => {
                    let center = match arguments.first() {
                        Some(argument) => argument.evaluate(sorted)?,
                        None => 0.0,
                    };
                    sorted
                        .iter()
                        .map(|&value| (value as f64 - center).powi(2))
                        .sum()
                }
                Function::Percentile => {
                    let p = arguments[0].evaluate(sorted)?;
                    if !(0.0..=100.0).contains(&p) {
                        return None;
                    }
                    percentile(sorted, p)
                }
                Function::Sqrt => arguments[0].evaluate(sorted)?.sqrt(),
            },
        };
        value.is_finite().then_some(value)
    }
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_space();
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let mut result = self.product()?;
        loop {
            let operator = if self.eat(b'+') {
                '+'
            } else if self.eat(b'-') {
                '-'
            } else {
                break;
            };
            result = Expr::Binary(operator, Box::new(result), Box::new(self.product()?));
        }
        Ok(result)
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut result = self.unary()?;
        loop {
            let operator = if self.eat(b'*') {
                '*'
            } else if self.eat(b'/') {
                '/'
            } else {
                break;
            };
            result = Expr::Binary(operator, Box::new(result), Box::new(self.unary()?));
        }
        Ok(result)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat(b'-') {
            return Ok(Expr::Negative(Box::new(self.unary()?)));
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Result<Expr, String> {
        let left = self.primary()?;
        if self.eat(b'^') {
            Ok(Expr::Binary('^', Box::new(left), Box::new(self.unary()?)))
        } else {
            Ok(left)
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        self.skip_space();
        if self.eat(b'(') {
            let expression = self.expression()?;
            if !self.eat(b')') {
                return Err(format!(
                    "expected ')' at formula position {}",
                    self.position + 1
                ));
            }
            return Ok(expression);
        }
        let start = self.position;
        if self
            .input
            .get(start)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            while self
                .input
                .get(self.position)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
            {
                self.position += 1;
            }
            let text = std::str::from_utf8(&self.input[start..self.position]).unwrap();
            return text
                .parse::<f64>()
                .map(Expr::Number)
                .map_err(|_| format!("invalid number at formula position {}", start + 1));
        }
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            self.position += 1;
        }
        if start == self.position {
            return Err(format!("expected value at formula position {}", start + 1));
        }
        let name = std::str::from_utf8(&self.input[start..self.position]).unwrap();
        let function = match name {
            "count" => Function::Count,
            "sum" => Function::Sum,
            "sumsq" => Function::SumSq,
            "p" | "percentile" => Function::Percentile,
            "sqrt" => Function::Sqrt,
            _ => return Err(format!("unknown formula function '{name}'")),
        };
        if !self.eat(b'(') {
            return Err(format!("expected '(' after {name}"));
        }
        let mut arguments = Vec::new();
        if !self.eat(b')') {
            loop {
                arguments.push(self.expression()?);
                if self.eat(b')') {
                    break;
                }
                if !self.eat(b',') {
                    return Err(format!(
                        "expected ',' or ')' at formula position {}",
                        self.position + 1
                    ));
                }
            }
        }
        let valid = match function {
            Function::Count | Function::Sum => arguments.is_empty(),
            Function::SumSq => arguments.len() <= 1,
            Function::Percentile | Function::Sqrt => arguments.len() == 1,
        };
        if !valid {
            return Err(format!("wrong number of arguments for {name}"));
        }
        Ok(Expr::Call(function, arguments))
    }
}
