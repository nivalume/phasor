use std::collections::BTreeSet;

/// Backend-neutral expression tree produced by [`crate::FactorSet::compile`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expression {
    Number(u64),
    Field(String),
    Unary(char, Box<Self>),
    Binary(String, Box<Self>, Box<Self>),
    Call(String, Vec<Self>),
}

impl Expression {
    pub(crate) fn number(value: f64) -> Self {
        Self::Number(value.to_bits())
    }

    pub(crate) fn fields<'a>(&'a self, out: &mut BTreeSet<&'a str>) {
        match self {
            Self::Field(name) => {
                out.insert(name);
            }
            Self::Unary(_, child) => child.fields(out),
            Self::Binary(_, left, right) => {
                left.fields(out);
                right.fields(out);
            }
            Self::Call(_, args) => {
                for arg in args {
                    arg.fields(out);
                }
            }
            Self::Number(_) => {}
        }
    }

    pub(crate) fn uses_future(&self) -> bool {
        match self {
            Self::Call(name, args) => {
                (name == "Ref" && args.get(1).is_some_and(|arg| match arg {
                    Self::Number(bits) => f64::from_bits(*bits) < 0.0,
                    Self::Unary('-', inner) => {
                        matches!(inner.as_ref(), Self::Number(bits) if f64::from_bits(*bits) > 0.0)
                    }
                    _ => false,
                })) || args.iter().any(Self::uses_future)
            }
            Self::Unary(_, child) => child.uses_future(),
            Self::Binary(_, left, right) => left.uses_future() || right.uses_future(),
            Self::Number(_) | Self::Field(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Field(String),
    Name(String),
    Op(String),
    LParen,
    RParen,
    Comma,
    End,
}

pub(crate) fn parse(source: &str) -> std::result::Result<Expression, String> {
    if source.len() > 16_384 {
        return Err("expression exceeds 16384 bytes".into());
    }
    let tokens = lex(source)?;
    let mut parser = Parser { tokens, at: 0 };
    let expr = parser.expression(0, 0)?;
    if parser.peek() != &Token::End {
        return Err(format!("unexpected token {:?}", parser.peek()));
    }
    validate(&expr)?;
    Ok(expr)
}

fn lex(source: &str) -> std::result::Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            b',' => {
                out.push(Token::Comma);
                i += 1;
            }
            b'+' | b'-' | b'*' | b'/' | b'>' | b'<' | b'=' | b'!' | b'&' | b'|' => {
                let start = i;
                i += 1;
                if i < bytes.len()
                    && matches!(
                        (bytes[start], bytes[i]),
                        (b'>', b'=') | (b'<', b'=') | (b'=', b'=') | (b'!', b'=') | (b'*', b'*')
                    )
                {
                    i += 1;
                }
                if i == start + 1 && matches!(bytes[start], b'=' | b'!') {
                    return Err(format!(
                        "operator {:?} must be followed by '='",
                        bytes[start] as char
                    ));
                }
                out.push(Token::Op(source[start..i].to_owned()));
            }
            b'$' => {
                let start = i + 1;
                i = start;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if i == start {
                    return Err("expected a field name after '$'".into());
                }
                out.push(Token::Field(source[start..i].to_ascii_lowercase()));
            }
            c if c.is_ascii_digit() || c == b'.' => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_digit()
                        || matches!(bytes[i], b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    if matches!(bytes[i], b'+' | b'-') && !matches!(bytes[i - 1], b'e' | b'E') {
                        break;
                    }
                    i += 1;
                }
                let value = source[start..i]
                    .parse::<f64>()
                    .map_err(|_| format!("invalid number {:?}", &source[start..i]))?;
                out.push(Token::Number(value));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(Token::Name(source[start..i].to_owned()));
            }
            other => {
                return Err(format!(
                    "unexpected character {:?} at byte {i}",
                    other as char
                ));
            }
        }
    }
    out.push(Token::End);
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }
    fn take(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        self.at += 1;
        token
    }

    fn expression(&mut self, min_bp: u8, depth: usize) -> std::result::Result<Expression, String> {
        if depth > 128 {
            return Err("expression nesting exceeds 128".into());
        }
        let mut left = match self.take() {
            Token::Number(value) => Expression::number(value),
            Token::Field(name) => Expression::Field(name),
            Token::Op(op) if op == "+" || op == "-" => Expression::Unary(
                op.as_bytes()[0] as char,
                Box::new(self.expression(11, depth + 1)?),
            ),
            Token::LParen => {
                let inner = self.expression(0, depth + 1)?;
                if self.take() != Token::RParen {
                    return Err("expected ')'".into());
                }
                inner
            }
            Token::Name(name) => {
                if self.take() != Token::LParen {
                    return Err(format!("expected '(' after {name}"));
                }
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    loop {
                        args.push(self.expression(0, depth + 1)?);
                        if self.peek() != &Token::Comma {
                            break;
                        }
                        self.take();
                    }
                }
                if self.take() != Token::RParen {
                    return Err(format!("expected ')' after {name} arguments"));
                }
                Expression::Call(name, args)
            }
            token => return Err(format!("expected expression, found {token:?}")),
        };

        while let Token::Op(op) = self.peek() {
            let op = op.clone();
            let (left_bp, right_bp) = match op.as_str() {
                "|" => (1, 2),
                "&" => (3, 4),
                ">" | "<" | ">=" | "<=" | "==" | "!=" => (5, 6),
                "+" | "-" => (7, 8),
                "*" | "/" => (9, 10),
                "**" => (12, 11),
                _ => break,
            };
            if left_bp < min_bp {
                break;
            }
            self.take();
            let right = self.expression(right_bp, depth + 1)?;
            left = Expression::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
}

fn validate(expr: &Expression) -> std::result::Result<(), String> {
    if let Expression::Call(name, args) = expr {
        let arity = match name.as_str() {
            "Abs" | "Sign" | "Log" | "Not" | "Exp" | "Sqrt" | "IsNan" | "CSRank" | "CSZScore"
            | "CSDemean" => 1..=1,
            "Greater" | "Less" | "Power" | "Add" | "Sub" | "Mul" | "Div" | "Gt" | "Ge" | "Lt"
            | "Le" | "Eq" | "Ne" | "And" | "Or" | "FillNa" | "Mask" | "SignedPower"
            | "IndNeutralize" | "CSResi" => 2..=2,
            "Scale" => 1..=2,
            "If" | "Clip" => 3..=3,
            "Ref" | "Mean" | "Std" | "Sum" | "Max" | "Min" | "Rank" | "IdxMax" | "IdxMin"
            | "Slope" | "Rsquare" | "Resi" | "Var" | "Skew" | "Kurt" | "Med" | "Mad" | "Count"
            | "Delta" | "WMA" | "EMA" | "DecayLinear" | "Prod" => 2..=2,
            "Quantile" | "Corr" | "Cov" => 3..=3,
            _ => return Err(format!("unsupported operator {name:?}")),
        };
        if !arity.contains(&args.len()) {
            return Err(format!("{name} received {} arguments", args.len()));
        }
    }
    match expr {
        Expression::Unary(_, child) => validate(child),
        Expression::Binary(_, left, right) => {
            validate(left)?;
            validate(right)
        }
        Expression::Call(_, args) => {
            for arg in args {
                validate(arg)?;
            }
            Ok(())
        }
        Expression::Number(_) | Expression::Field(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_scientific_numbers_parse() {
        let expr = parse("($close-Ref($close, 1))/(2*$close+1e-12)").unwrap();
        assert!(matches!(expr, Expression::Binary(ref op, _, _) if op == "/"));
    }

    #[test]
    fn rejects_unknown_calls() {
        assert!(parse("Magic($close)").unwrap_err().contains("unsupported"));
    }
}
