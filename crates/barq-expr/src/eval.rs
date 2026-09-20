use std::collections::HashMap;
use std::sync::Arc;

use barq_core::error::{Error, Result};
use barq_core::matrix::Matrix;
use barq_generic::rolling::{rolling_apply_1d, rolling_max_1d, rolling_mean_1d, rolling_min_1d};
use barq_model::OhlcvPanel;

use crate::FactorSet;
use crate::parse::Expression;

/// Named column-major fields supplied to a factor set.
#[derive(Debug, Clone)]
pub struct FactorInputs<'a> {
    n_rows: usize,
    n_cols: usize,
    fields: HashMap<String, &'a [f64]>,
}

impl<'a> FactorInputs<'a> {
    /// Creates an empty input set with a fixed `(rows, symbols)` shape.
    ///
    /// # Errors
    ///
    /// Returns an error when either dimension is zero.
    pub fn new(n_rows: usize, n_cols: usize) -> Result<Self> {
        if n_rows == 0 || n_cols == 0 {
            return Err(Error::Invariant(format!(
                "factor input shape must be non-zero, got {n_rows}x{n_cols}"
            )));
        }
        Ok(Self {
            n_rows,
            n_cols,
            fields: HashMap::new(),
        })
    }

    /// Borrows the populated fields of an OHLCV panel.
    ///
    /// # Errors
    ///
    /// Returns an error when a populated field does not match the panel shape.
    pub fn from_ohlcv(panel: &'a OhlcvPanel) -> Result<Self> {
        let mut inputs = Self {
            n_rows: panel.n_rows,
            n_cols: panel.n_cols,
            fields: HashMap::new(),
        };
        for (name, values) in [
            ("open", panel.open.as_slice()),
            ("high", panel.high.as_slice()),
            ("low", panel.low.as_slice()),
            ("close", panel.close.as_slice()),
            ("volume", panel.volume.as_slice()),
        ] {
            if !values.is_empty() {
                inputs.add_field(name, values)?;
            }
        }
        Ok(inputs)
    }

    /// Adds a custom field such as `vwap`.
    ///
    /// # Errors
    ///
    /// Returns an error when its length differs from `n_rows * n_cols`.
    pub fn with_field(mut self, name: &str, values: &'a [f64]) -> Result<Self> {
        self.add_field(name, values)?;
        Ok(self)
    }

    fn add_field(&mut self, name: &str, values: &'a [f64]) -> Result<()> {
        let expected = self.n_rows * self.n_cols;
        if values.len() != expected {
            return Err(Error::FlexLen {
                kind: "factor field",
                actual: values.len(),
                expected,
            });
        }
        let name = name.to_ascii_lowercase();
        if name.is_empty() {
            return Err(Error::Invariant("factor field name cannot be empty".into()));
        }
        if self.fields.insert(name.clone(), values).is_some() {
            return Err(Error::Invariant(format!("duplicate factor field {name:?}")));
        }
        Ok(())
    }

    /// `(rows, symbols)` of every supplied field.
    #[must_use]
    pub fn shape(&self) -> (usize, usize) {
        (self.n_rows, self.n_cols)
    }

    /// Iterates over the named column-major input fields.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &'a [f64])> + '_ {
        self.fields
            .iter()
            .map(|(name, values)| (name.as_str(), *values))
    }
}

/// Named factor matrices returned by [`FactorSet::evaluate`].
#[derive(Debug, Clone)]
pub struct FactorValues {
    names: Vec<String>,
    values: Vec<Arc<Matrix>>,
}

impl FactorValues {
    /// Builds backend output from names and matrices in the same order.
    ///
    /// # Errors
    ///
    /// Returns an error when the name and value counts differ.
    pub fn from_matrices(names: Vec<String>, values: Vec<Matrix>) -> Result<Self> {
        if names.len() != values.len() {
            return Err(Error::Invariant(format!(
                "factor backend returned {} names but {} matrices",
                names.len(),
                values.len()
            )));
        }
        Ok(Self {
            names,
            values: values.into_iter().map(Arc::new).collect(),
        })
    }

    /// Looks up one factor by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Matrix> {
        self.names
            .iter()
            .position(|candidate| candidate == name)
            .map(|index| self.values[index].as_ref())
    }

    /// Iterates in factor-set order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Matrix)> {
        self.names
            .iter()
            .zip(&self.values)
            .map(|(name, value)| (name.as_str(), value.as_ref()))
    }
}

pub(crate) fn evaluate(set: &FactorSet, inputs: &FactorInputs<'_>) -> Result<FactorValues> {
    let mut cache = HashMap::new();
    let mut names = Vec::with_capacity(set.factors.len());
    let mut values = Vec::with_capacity(set.factors.len());
    for (name, expr) in &set.factors {
        names.push(name.clone());
        values.push(eval(expr, inputs, &mut cache)?);
    }
    Ok(FactorValues { names, values })
}

#[allow(
    clippy::float_cmp,
    reason = "the expression language exposes exact Eq/Ne and boolean numeric operators"
)]
fn eval(
    expr: &Expression,
    inputs: &FactorInputs<'_>,
    cache: &mut HashMap<Expression, Arc<Matrix>>,
) -> Result<Arc<Matrix>> {
    if let Some(value) = cache.get(expr) {
        return Ok(Arc::clone(value));
    }
    let value = match expr {
        Expression::Number(bits) => {
            Matrix::filled(inputs.n_rows, inputs.n_cols, f64::from_bits(*bits))
        }
        Expression::Field(name) => {
            let values = inputs.fields.get(name).ok_or_else(|| {
                Error::Invariant(format!("factor input is missing field ${name}"))
            })?;
            Matrix::from_column_major(values.to_vec(), inputs.n_rows, inputs.n_cols)?
        }
        Expression::Unary('+', child) => (*eval(child, inputs, cache)?).clone(),
        Expression::Unary('-', child) => {
            map1(eval(child, inputs, cache)?.as_ref(), |value| -value)?
        }
        Expression::Unary(op, _) => {
            return Err(Error::Invariant(format!("unsupported unary {op}")));
        }
        Expression::Binary(op, left, right) => {
            let left = eval(left, inputs, cache)?;
            let right = eval(right, inputs, cache)?;
            map2(&left, &right, |a, b| match op.as_str() {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                "**" => a.powf(b),
                ">" => f64::from(a > b),
                "<" => f64::from(a < b),
                ">=" => f64::from(a >= b),
                "<=" => f64::from(a <= b),
                "==" => f64::from(a == b),
                "!=" => f64::from(a != b),
                "&" => f64::from(a != 0.0 && b != 0.0),
                "|" => f64::from(a != 0.0 || b != 0.0),
                _ => f64::NAN,
            })?
        }
        Expression::Call(name, args) => call(name, args, inputs, cache)?,
    };
    let value = Arc::new(value);
    cache.insert(expr.clone(), Arc::clone(&value));
    Ok(value)
}

#[allow(
    clippy::float_cmp,
    reason = "Qlib's Eq/Ne and numeric boolean operators use exact NumPy comparisons"
)]
fn call(
    name: &str,
    args: &[Expression],
    inputs: &FactorInputs<'_>,
    cache: &mut HashMap<Expression, Arc<Matrix>>,
) -> Result<Matrix> {
    match name {
        "Abs" => map1(eval(&args[0], inputs, cache)?.as_ref(), f64::abs),
        "Sign" => map1(eval(&args[0], inputs, cache)?.as_ref(), |value| {
            if value == 0.0 { 0.0 } else { value.signum() }
        }),
        "Log" => map1(eval(&args[0], inputs, cache)?.as_ref(), f64::ln),
        "Not" => map1(eval(&args[0], inputs, cache)?.as_ref(), |value| {
            f64::from(value == 0.0)
        }),
        "Power" | "Add" | "Sub" | "Mul" | "Div" | "Gt" | "Ge" | "Lt" | "Le" | "Eq" | "Ne"
        | "And" | "Or" => {
            let left = eval(&args[0], inputs, cache)?;
            let right = eval(&args[1], inputs, cache)?;
            map2(&left, &right, |a, b| match name {
                "Power" => a.powf(b),
                "Add" => a + b,
                "Sub" => a - b,
                "Mul" => a * b,
                "Div" => a / b,
                "Gt" => f64::from(a > b),
                "Ge" => f64::from(a >= b),
                "Lt" => f64::from(a < b),
                "Le" => f64::from(a <= b),
                "Eq" => f64::from(a == b),
                "Ne" => f64::from(a != b),
                "And" => f64::from(a != 0.0 && b != 0.0),
                "Or" => f64::from(a != 0.0 || b != 0.0),
                _ => unreachable!(),
            })
        }
        "If" => {
            let condition = eval(&args[0], inputs, cache)?;
            let left = eval(&args[1], inputs, cache)?;
            let right = eval(&args[2], inputs, cache)?;
            map3(&condition, &left, &right, |condition, yes, no| {
                if condition != 0.0 { yes } else { no }
            })
        }
        "Greater" | "Less" => {
            let left = eval(&args[0], inputs, cache)?;
            let right = eval(&args[1], inputs, cache)?;
            map2(&left, &right, |a, b| {
                if a.is_nan() || b.is_nan() {
                    f64::NAN
                } else if name == "Greater" {
                    a.max(b)
                } else {
                    a.min(b)
                }
            })
        }
        "Ref" => {
            let source = eval(&args[0], inputs, cache)?;
            let offset = integer(&args[1], "Ref offset")?;
            map_columns(&source, |column| {
                Ok((0..column.len())
                    .map(|row| {
                        if offset == 0 {
                            column[row]
                        } else {
                            let source_row = row as i64 - offset;
                            if source_row < 0 || source_row >= column.len() as i64 {
                                f64::NAN
                            } else {
                                column[source_row as usize]
                            }
                        }
                    })
                    .collect())
            })
        }
        "Mean" | "Std" | "Sum" | "Max" | "Min" | "Quantile" | "Rank" | "IdxMax" | "IdxMin"
        | "Slope" | "Rsquare" | "Resi" | "Var" | "Skew" | "Kurt" | "Med" | "Mad" | "Count"
        | "WMA" => {
            let source = eval(&args[0], inputs, cache)?;
            let window = window(&args[1])?;
            let q = if name == "Quantile" {
                number(&args[2], "quantile")?
            } else {
                0.0
            };
            let window = if window == 0 {
                source.n_rows().max(1)
            } else {
                window
            };
            map_columns(&source, |column| rolling_single(name, column, window, q))
        }
        "Delta" => {
            let source = eval(&args[0], inputs, cache)?;
            let offset = integer(&args[1], "Delta offset")?;
            if offset < 0 {
                return Err(Error::Invariant("Delta offset cannot be negative".into()));
            }
            map_columns(&source, |column| {
                Ok((0..column.len())
                    .map(|row| {
                        if offset == 0 {
                            if column[row].is_nan() { f64::NAN } else { 0.0 }
                        } else if row < offset as usize {
                            f64::NAN
                        } else {
                            column[row] - column[row - offset as usize]
                        }
                    })
                    .collect())
            })
        }
        "EMA" => {
            let source = eval(&args[0], inputs, cache)?;
            let parameter = number(&args[1], "EMA parameter")?;
            if parameter < 0.0 {
                return Err(Error::Invariant("EMA parameter cannot be negative".into()));
            }
            if parameter == 0.0 {
                return map_columns(&source, |column| Ok(ema_expanding(column)));
            }
            let alpha = if parameter < 1.0 {
                parameter
            } else {
                2.0 / (parameter + 1.0)
            };
            map_columns(&source, |column| Ok(ema_adjusted(column, alpha)))
        }
        "Corr" | "Cov" => {
            let left = eval(&args[0], inputs, cache)?;
            let right = eval(&args[1], inputs, cache)?;
            let window = window(&args[2])?;
            let window = if window == 0 {
                left.n_rows().max(1)
            } else {
                window
            };
            rolling_pair(
                &left,
                &right,
                window,
                if name == "Corr" { corr } else { cov },
            )
        }
        _ => Err(Error::Invariant(format!("unsupported operator {name}"))),
    }
}

fn map1(input: &Matrix, f: impl Fn(f64) -> f64) -> Result<Matrix> {
    Matrix::from_column_major(
        input.as_slice().iter().copied().map(f).collect(),
        input.n_rows(),
        input.n_cols(),
    )
}

fn map2(left: &Matrix, right: &Matrix, f: impl Fn(f64, f64) -> f64) -> Result<Matrix> {
    if (left.n_rows(), left.n_cols()) != (right.n_rows(), right.n_cols()) {
        return Err(Error::Broadcast {
            left: (left.n_rows(), left.n_cols()),
            right: (right.n_rows(), right.n_cols()),
        });
    }
    Matrix::from_column_major(
        left.as_slice()
            .iter()
            .zip(right.as_slice())
            .map(|(&a, &b)| f(a, b))
            .collect(),
        left.n_rows(),
        left.n_cols(),
    )
}

fn map3(
    first: &Matrix,
    second: &Matrix,
    third: &Matrix,
    f: impl Fn(f64, f64, f64) -> f64,
) -> Result<Matrix> {
    if (first.n_rows(), first.n_cols()) != (second.n_rows(), second.n_cols())
        || (first.n_rows(), first.n_cols()) != (third.n_rows(), third.n_cols())
    {
        return Err(Error::Invariant(
            "three factor operands have different shapes".into(),
        ));
    }
    Matrix::from_column_major(
        first
            .as_slice()
            .iter()
            .zip(second.as_slice())
            .zip(third.as_slice())
            .map(|((&a, &b), &c)| f(a, b, c))
            .collect(),
        first.n_rows(),
        first.n_cols(),
    )
}

fn map_columns(input: &Matrix, mut f: impl FnMut(&[f64]) -> Result<Vec<f64>>) -> Result<Matrix> {
    let mut out = Matrix::zeros(input.n_rows(), input.n_cols());
    for col in 0..input.n_cols() {
        let values = f(input.column(col))?;
        out.column_mut(col).copy_from_slice(&values);
    }
    Ok(out)
}

fn number(expr: &Expression, label: &str) -> Result<f64> {
    match expr {
        Expression::Number(bits) => Ok(f64::from_bits(*bits)),
        Expression::Unary('-', inner) => Ok(-number(inner, label)?),
        _ => Err(Error::Invariant(format!(
            "{label} must be a numeric literal"
        ))),
    }
}

fn integer(expr: &Expression, label: &str) -> Result<i64> {
    let value = number(expr, label)?;
    if value.fract() != 0.0 || !value.is_finite() {
        return Err(Error::Invariant(format!("{label} must be an integer")));
    }
    Ok(value as i64)
}

fn window(expr: &Expression) -> Result<usize> {
    let value = integer(expr, "window")?;
    if value < 0 {
        return Err(Error::Invariant("window cannot be negative".into()));
    }
    Ok(value as usize)
}

fn rolling_single(name: &str, input: &[f64], window: usize, q: f64) -> Result<Vec<f64>> {
    match name {
        "Mean" => rolling_mean_1d(input, window, Some(1)),
        "Std" => rolling_std_qlib(input, window, 1),
        "Var" => rolling_std_qlib(input, window, 1)
            .map(|values| values.into_iter().map(|value| value * value).collect()),
        "Max" => rolling_max_1d(input, window, Some(1)),
        "Min" => rolling_min_1d(input, window, Some(1)),
        "Sum" => rolling_apply_1d(input, window, Some(1), |_, values| {
            values.iter().filter(|v| !v.is_nan()).sum()
        }),
        "Quantile" => {
            if !(0.0..=1.0).contains(&q) {
                return Err(Error::Invariant("quantile must be in 0..=1".into()));
            }
            rolling_apply_1d(input, window, Some(1), |_, values| quantile(values, q))
        }
        "Med" => rolling_apply_1d(input, window, Some(1), |_, values| quantile(values, 0.5)),
        "Mad" => rolling_apply_1d(input, window, Some(1), |_, values| mad(values)),
        "Count" => rolling_apply_1d(input, window, Some(1), |_, values| {
            values.iter().filter(|value| !value.is_nan()).count() as f64
        }),
        "Skew" => rolling_apply_1d(input, window, Some(1), |_, values| skew(values)),
        "Kurt" => rolling_apply_1d(input, window, Some(1), |_, values| kurt(values)),
        "WMA" => rolling_apply_1d(input, window, Some(1), |_, values| wma(values)),
        "Rank" => rolling_apply_1d(input, window, Some(1), |_, values| rank_last(values)),
        "IdxMax" => rolling_apply_1d(input, window, Some(1), |_, values| {
            arg_extreme(values, false)
        }),
        "IdxMin" => rolling_apply_1d(input, window, Some(1), |_, values| {
            arg_extreme(values, true)
        }),
        "Slope" => rolling_apply_1d(input, window, Some(1), |_, values| regression(values).0),
        "Rsquare" => rolling_apply_1d(input, window, Some(1), |_, values| regression(values).1),
        "Resi" => rolling_apply_1d(input, window, Some(1), |_, values| regression(values).2),
        _ => Err(Error::Invariant(format!(
            "unsupported rolling operator {name}"
        ))),
    }
}

fn rolling_std_qlib(input: &[f64], window: usize, ddof: usize) -> Result<Vec<f64>> {
    rolling_apply_1d(input, window, Some(1), |_, values| {
        let count = values.iter().filter(|value| !value.is_nan()).count();
        if count <= ddof {
            return f64::NAN;
        }
        let mean = values.iter().filter(|value| !value.is_nan()).sum::<f64>() / count as f64;
        let m2 = values
            .iter()
            .filter(|value| !value.is_nan())
            .map(|value| (value - mean).powi(2))
            .sum::<f64>();
        (m2 / (count - ddof) as f64).sqrt()
    })
}

pub(crate) fn quantile(values: &[f64], q: f64) -> f64 {
    let mut values: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return f64::NAN;
    }
    let index = (values.len() - 1) as f64 * q;
    let lower = index.floor() as usize;
    let fraction = index - lower as f64;
    values[lower] + fraction * (values[index.ceil() as usize] - values[lower])
}

pub(crate) fn mad(values: &[f64]) -> f64 {
    let values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect();
    if values.is_empty() {
        return f64::NAN;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.iter().map(|value| (value - mean).abs()).sum::<f64>() / values.len() as f64
}

pub(crate) fn skew(values: &[f64]) -> f64 {
    let values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect();
    let n = values.len() as f64;
    if values.len() < 3 {
        return f64::NAN;
    }
    let mean = values.iter().sum::<f64>() / n;
    let m2 = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>();
    let m3 = values
        .iter()
        .map(|value| (value - mean).powi(3))
        .sum::<f64>();
    n * (n - 1.0).sqrt() / (n - 2.0) * m3 / m2.powf(1.5)
}

pub(crate) fn kurt(values: &[f64]) -> f64 {
    let values: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect();
    let n = values.len() as f64;
    if values.len() < 4 {
        return f64::NAN;
    }
    let mean = values.iter().sum::<f64>() / n;
    let m2 = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>();
    let m4 = values
        .iter()
        .map(|value| (value - mean).powi(4))
        .sum::<f64>();
    let biased_fisher = n * m4 / (m2 * m2) - 3.0;
    (n - 1.0) / ((n - 2.0) * (n - 3.0)) * ((n + 1.0) * biased_fisher + 6.0)
}

pub(crate) fn wma(values: &[f64]) -> f64 {
    let weight_sum = (values.len() * (values.len() + 1) / 2) as f64;
    let mut sum = 0.0;
    let mut count = 0;
    for (index, &value) in values.iter().enumerate() {
        if !value.is_nan() {
            sum += (index + 1) as f64 / weight_sum * value;
            count += 1;
        }
    }
    if count == 0 {
        f64::NAN
    } else {
        sum / count as f64
    }
}

#[allow(
    clippy::float_cmp,
    reason = "zero denominator exactly means no EMA observation has appeared"
)]
fn ema_adjusted(values: &[f64], alpha: f64) -> Vec<f64> {
    let decay = 1.0 - alpha;
    let (mut numerator, mut denominator) = (0.0, 0.0);
    values
        .iter()
        .map(|&value| {
            numerator *= decay;
            denominator *= decay;
            if !value.is_nan() {
                numerator += value;
                denominator += 1.0;
            }
            if denominator == 0.0 {
                f64::NAN
            } else {
                numerator / denominator
            }
        })
        .collect()
}

pub(crate) fn ema_expanding(values: &[f64]) -> Vec<f64> {
    (1..=values.len())
        .map(|length| {
            if values[..length].iter().all(|value| value.is_nan()) {
                return f64::NAN;
            }
            let decay = 1.0 - 2.0 / (1.0 + length as f64);
            let denominator: f64 = (0..length)
                .map(|index| decay.powi((length - index - 1) as i32))
                .sum();
            values[..length]
                .iter()
                .enumerate()
                .filter(|(_, value)| !value.is_nan())
                .map(|(index, value)| decay.powi((length - index - 1) as i32) / denominator * value)
                .sum()
        })
        .collect()
}

#[allow(
    clippy::float_cmp,
    reason = "Qlib's average-rank tie groups are defined by exact equality"
)]
pub(crate) fn rank_last(values: &[f64]) -> f64 {
    let Some(&last) = values.last() else {
        return f64::NAN;
    };
    if last.is_nan() {
        return f64::NAN;
    }
    let clean: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    let below = clean.iter().filter(|&&value| value < last).count() as f64;
    let equal = clean.iter().filter(|&&value| value == last).count() as f64;
    (below + (equal + 1.0) / 2.0) / clean.len() as f64
}

pub(crate) fn arg_extreme(values: &[f64], minimum: bool) -> f64 {
    let mut best = 0;
    for index in 1..values.len() {
        if (!values[best].is_nan() && values[index].is_nan())
            || (!values[best].is_nan()
                && !values[index].is_nan()
                && if minimum {
                    values[index] < values[best]
                } else {
                    values[index] > values[best]
                })
        {
            best = index;
        }
    }
    (best + 1) as f64
}

/// `(slope, r², residual-at-last-row)` against row positions.
pub(crate) fn regression(values: &[f64]) -> (f64, f64, f64) {
    let points: Vec<(f64, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(index, &value)| (!value.is_nan()).then_some(((index + 1) as f64, value)))
        .collect();
    if points.len() < 2 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let n = points.len() as f64;
    let sx: f64 = points.iter().map(|p| p.0).sum();
    let sy: f64 = points.iter().map(|p| p.1).sum();
    let sxx: f64 = points.iter().map(|p| p.0 * p.0).sum();
    let syy: f64 = points.iter().map(|p| p.1 * p.1).sum();
    let sxy: f64 = points.iter().map(|p| p.0 * p.1).sum();
    let xvar = n * sxx - sx * sx;
    let yvar = n * syy - sy * sy;
    let covariance = n * sxy - sx * sy;
    let slope = covariance / xvar;
    let intercept = sy / n - slope * sx / n;
    let sample_std = (yvar / (n * (n - 1.0))).sqrt();
    let rsquare = if xvar <= 0.0 || yvar <= 0.0 || sample_std <= 2e-5 {
        f64::NAN
    } else {
        covariance * covariance / (xvar * yvar)
    };
    let residual =
        values.last().copied().unwrap_or(f64::NAN) - (slope * values.len() as f64 + intercept);
    (slope, rsquare, residual)
}

fn rolling_pair(
    left: &Matrix,
    right: &Matrix,
    window: usize,
    reducer: impl Fn(&[f64], &[f64]) -> f64,
) -> Result<Matrix> {
    if (left.n_rows(), left.n_cols()) != (right.n_rows(), right.n_cols()) {
        return Err(Error::Broadcast {
            left: (left.n_rows(), left.n_cols()),
            right: (right.n_rows(), right.n_cols()),
        });
    }
    let mut out = Matrix::filled(left.n_rows(), left.n_cols(), f64::NAN);
    for col in 0..left.n_cols() {
        for row in 0..left.n_rows() {
            let start = (row + 1).saturating_sub(window);
            out.column_mut(col)[row] = reducer(
                &left.column(col)[start..=row],
                &right.column(col)[start..=row],
            );
        }
    }
    Ok(out)
}

pub(crate) fn corr(left: &[f64], right: &[f64]) -> f64 {
    let standalone_std = |values: &[f64]| {
        let (count, sum) = values
            .iter()
            .filter(|value| !value.is_nan())
            .fold((0, 0.0), |(count, sum), value| (count + 1, sum + value));
        if count < 2 {
            return f64::NAN;
        }
        let mean = sum / count as f64;
        (values
            .iter()
            .filter(|value| !value.is_nan())
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (count - 1) as f64)
            .sqrt()
    };
    if standalone_std(left) <= 2e-5 || standalone_std(right) <= 2e-5 {
        return f64::NAN;
    }
    let (count, sum_a, sum_b) = left
        .iter()
        .zip(right)
        .filter(|(a, b)| !a.is_nan() && !b.is_nan())
        .fold((0, 0.0, 0.0), |(count, sum_a, sum_b), (&a, &b)| {
            (count + 1, sum_a + a, sum_b + b)
        });
    if count < 2 {
        return f64::NAN;
    }
    let ma = sum_a / count as f64;
    let mb = sum_b / count as f64;
    let (va, vb, covariance) = left
        .iter()
        .zip(right)
        .filter(|(a, b)| !a.is_nan() && !b.is_nan())
        .fold((0.0, 0.0, 0.0), |(va, vb, covariance), (&a, &b)| {
            let da = a - ma;
            let db = b - mb;
            (va + da * da, vb + db * db, covariance + da * db)
        });
    if va == 0.0 || vb == 0.0 {
        return f64::NAN;
    }
    covariance / (va * vb).sqrt()
}

fn cov(left: &[f64], right: &[f64]) -> f64 {
    let pairs: Vec<(f64, f64)> = left
        .iter()
        .zip(right)
        .filter_map(|(&a, &b)| (!a.is_nan() && !b.is_nan()).then_some((a, b)))
        .collect();
    if pairs.len() < 2 {
        return f64::NAN;
    }
    let n = pairs.len() as f64;
    let ma = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mb = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    pairs.iter().map(|p| (p.0 - ma) * (p.1 - mb)).sum::<f64>() / (n - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qlib_rank_is_the_last_values_percentile() {
        assert!((rank_last(&[3.0, 1.0, 2.0]) - 2.0 / 3.0).abs() < 1e-12);
        assert!((rank_last(&[1.0, 2.0, 2.0]) - 2.5 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn regression_finds_a_line_and_its_last_residual() {
        let (slope, r2, residual) = regression(&[2.0, 4.0, 6.0]);
        assert!((slope - 2.0).abs() < 1e-12);
        assert!((r2 - 1.0).abs() < 1e-12);
        assert!(residual.abs() < 1e-12);
    }
}
