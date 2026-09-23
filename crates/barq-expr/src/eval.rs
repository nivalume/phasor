use std::collections::HashMap;
use std::sync::Arc;

use barq_core::error::{Error, Result};
use barq_core::matrix::Matrix;
use barq_generic::rolling::{rolling_apply_1d, rolling_max_1d, rolling_mean_1d, rolling_min_1d};
use barq_model::OhlcvPanel;
use rayon::prelude::*;

use crate::FactorSet;
use crate::order::{Order, ordered, rolling_arg_extreme};
use crate::parse::Expression;
use crate::window::{Deviation, Fit, Pair, Regression, Shape, Sum, Wma, slide};

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

    /// Takes the matrices in factor-set order. A matrix is moved out rather
    /// than copied unless two factors share the same expression.
    pub fn into_matrices(self) -> impl ExactSizeIterator<Item = (String, Matrix)> {
        self.names
            .into_iter()
            .zip(self.values)
            .map(|(name, value)| {
                (
                    name,
                    Arc::try_unwrap(value).unwrap_or_else(|shared| (*shared).clone()),
                )
            })
    }
}

pub(crate) fn evaluate(set: &FactorSet, inputs: &FactorInputs<'_>) -> Result<FactorValues> {
    let mut evaluator = Evaluator::new(set, inputs);
    let mut names = Vec::with_capacity(set.factors.len());
    let mut values = Vec::with_capacity(set.factors.len());
    for (name, expr) in &set.factors {
        names.push(name.clone());
        values.push(evaluator.eval(expr)?.into_matrix(inputs));
    }
    debug_assert!(
        evaluator.cache.is_empty(),
        "every cached value is released by its last use"
    );
    Ok(FactorValues { names, values })
}

/// An evaluated node. Constants and input fields are not materialised as
/// matrices: a literal is broadcast where it is used, and a field borrows the
/// caller's buffer.
#[derive(Debug, Clone)]
enum Value<'a> {
    Scalar(f64),
    Field(&'a [f64]),
    Matrix(Arc<Matrix>),
}

impl Value<'_> {
    /// Column-major cells, or `None` for a scalar.
    fn cells(&self) -> Option<&[f64]> {
        match self {
            Self::Scalar(_) => None,
            Self::Field(values) => Some(values),
            Self::Matrix(matrix) => Some(matrix.as_slice()),
        }
    }

    fn into_matrix(self, inputs: &FactorInputs<'_>) -> Arc<Matrix> {
        match self {
            Self::Scalar(value) => Arc::new(Matrix::filled(inputs.n_rows, inputs.n_cols, value)),
            Self::Field(values) => Arc::new(
                Matrix::from_column_major(values.to_vec(), inputs.n_rows, inputs.n_cols)
                    .expect("field lengths are checked when they are added"),
            ),
            Self::Matrix(matrix) => matrix,
        }
    }
}

/// Evaluates a factor set once per distinct subexpression.
///
/// A subexpression shared by several factors (Alpha158 reads `Ref($close, 1)`
/// dozens of times) is computed once and cached, but only until its last
/// consumer has read it: `uses` counts, per distinct node, how many times
/// evaluation will ask for it, and the cache entry is dropped when that count
/// reaches zero. Without that, every intermediate of every factor stays alive
/// until the end, and peak memory grows with the size of the whole set.
struct Evaluator<'e, 'a> {
    inputs: &'e FactorInputs<'a>,
    uses: HashMap<&'e Expression, usize>,
    cache: HashMap<&'e Expression, Value<'a>>,
}

impl<'e, 'a> Evaluator<'e, 'a> {
    fn new(set: &'e FactorSet, inputs: &'e FactorInputs<'a>) -> Self {
        fn count<'e>(expr: &'e Expression, uses: &mut HashMap<&'e Expression, usize>) {
            let seen = uses.entry(expr).or_insert(0);
            *seen += 1;
            // Only the first request computes the node and so requests its
            // operands; later ones are served from the cache.
            if *seen == 1 {
                for operand in operands(expr) {
                    count(operand, uses);
                }
            }
        }
        let mut uses = HashMap::new();
        for (_, expr) in &set.factors {
            count(expr, &mut uses);
        }
        Self {
            inputs,
            uses,
            cache: HashMap::new(),
        }
    }

    fn eval(&mut self, expr: &'e Expression) -> Result<Value<'a>> {
        let value = match self.cache.get(expr) {
            Some(value) => value.clone(),
            None => {
                let value = self.compute(expr)?;
                self.cache.insert(expr, value.clone());
                value
            }
        };
        let remaining = self.uses.get_mut(expr).map(|uses| {
            *uses = uses.saturating_sub(1);
            *uses
        });
        debug_assert!(remaining.is_some(), "uses are counted for every operand");
        if remaining.unwrap_or(0) == 0 {
            self.cache.remove(expr);
        }
        Ok(value)
    }

    #[allow(
        clippy::float_cmp,
        reason = "the expression language exposes exact Eq/Ne and boolean numeric operators"
    )]
    fn compute(&mut self, expr: &'e Expression) -> Result<Value<'a>> {
        Ok(match expr {
            Expression::Number(bits) => Value::Scalar(f64::from_bits(*bits)),
            Expression::Field(name) => {
                Value::Field(self.inputs.fields.get(name).ok_or_else(|| {
                    Error::Invariant(format!("factor input is missing field ${name}"))
                })?)
            }
            Expression::Unary('+', child) => self.eval(child)?,
            Expression::Unary('-', child) => {
                let child = self.eval(child)?;
                self.map1(&child, |value| -value)
            }
            Expression::Unary(op, _) => {
                return Err(Error::Invariant(format!("unsupported unary {op}")));
            }
            Expression::Binary(op, left, right) => {
                let left = self.eval(left)?;
                let right = self.eval(right)?;
                self.map2(&left, &right, |a, b| match op.as_str() {
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
                })
            }
            Expression::Call(name, args) => self.call(name, args)?,
        })
    }

    #[allow(
        clippy::float_cmp,
        reason = "Qlib's Eq/Ne and numeric boolean operators use exact NumPy comparisons"
    )]
    fn call(&mut self, name: &str, args: &'e [Expression]) -> Result<Value<'a>> {
        // Every `self.eval` here must be listed by `operands`, or the node's
        // use count is wrong and it is either recomputed or never released.
        Ok(match name {
            "Abs" => {
                let source = self.eval(&args[0])?;
                self.map1(&source, f64::abs)
            }
            "Sign" => {
                let source = self.eval(&args[0])?;
                self.map1(
                    &source,
                    |value| {
                        if value == 0.0 { 0.0 } else { value.signum() }
                    },
                )
            }
            "Log" => {
                let source = self.eval(&args[0])?;
                self.map1(&source, f64::ln)
            }
            "Not" => {
                let source = self.eval(&args[0])?;
                self.map1(&source, |value| f64::from(value == 0.0))
            }
            "Power" | "Add" | "Sub" | "Mul" | "Div" | "Gt" | "Ge" | "Lt" | "Le" | "Eq" | "Ne"
            | "And" | "Or" => {
                let left = self.eval(&args[0])?;
                let right = self.eval(&args[1])?;
                self.map2(&left, &right, |a, b| match name {
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
                let condition = self.eval(&args[0])?;
                let left = self.eval(&args[1])?;
                let right = self.eval(&args[2])?;
                self.map3(&condition, &left, &right, |condition, yes, no| {
                    if condition != 0.0 { yes } else { no }
                })
            }
            "Greater" | "Less" => {
                let left = self.eval(&args[0])?;
                let right = self.eval(&args[1])?;
                self.map2(&left, &right, |a, b| {
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
                let source = self.eval(&args[0])?;
                let offset = integer(&args[1], "Ref offset")?;
                self.map_columns(&source, |column| {
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
                })?
            }
            "Mean" | "Std" | "Sum" | "Max" | "Min" | "Quantile" | "Rank" | "IdxMax" | "IdxMin"
            | "Slope" | "Rsquare" | "Resi" | "Var" | "Skew" | "Kurt" | "Med" | "Mad" | "Count"
            | "WMA" => {
                let source = self.eval(&args[0])?;
                let window = self.window(&args[1])?;
                let q = if name == "Quantile" {
                    number(&args[2], "quantile")?
                } else {
                    0.0
                };
                self.map_columns(&source, |column| rolling_single(name, column, window, q))?
            }
            "Delta" => {
                let source = self.eval(&args[0])?;
                let offset = integer(&args[1], "Delta offset")?;
                if offset < 0 {
                    return Err(Error::Invariant("Delta offset cannot be negative".into()));
                }
                self.map_columns(&source, |column| {
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
                })?
            }
            "EMA" => {
                let source = self.eval(&args[0])?;
                let parameter = number(&args[1], "EMA parameter")?;
                if parameter < 0.0 {
                    return Err(Error::Invariant("EMA parameter cannot be negative".into()));
                }
                if parameter == 0.0 {
                    self.map_columns(&source, |column| Ok(ema_expanding(column)))?
                } else {
                    let alpha = if parameter < 1.0 {
                        parameter
                    } else {
                        2.0 / (parameter + 1.0)
                    };
                    self.map_columns(&source, |column| Ok(ema_adjusted(column, alpha)))?
                }
            }
            "Corr" | "Cov" => {
                let left = self.eval(&args[0])?;
                let right = self.eval(&args[1])?;
                let window = self.window(&args[2])?;
                let corr = name == "Corr";
                self.map_column_pairs(&left, &right, |left, right| {
                    let pairs: Vec<(f64, f64)> =
                        left.iter().copied().zip(right.iter().copied()).collect();
                    if corr {
                        slide(&pairs, window, &mut Pair::corr())
                    } else {
                        slide(&pairs, window, &mut Pair::cov())
                    }
                })?
            }
            _ => return Err(Error::Invariant(format!("unsupported operator {name}"))),
        })
    }

    /// A window argument, where `0` means the whole series.
    fn window(&self, expr: &Expression) -> Result<usize> {
        let window = window(expr)?;
        Ok(if window == 0 {
            self.inputs.n_rows.max(1)
        } else {
            window
        })
    }

    fn map1(&self, input: &Value<'_>, f: impl Fn(f64) -> f64 + Sync + Send) -> Value<'a> {
        let Some(cells) = input.cells() else {
            return Value::Scalar(f(scalar(input)));
        };
        self.matrix(
            cells
                .par_iter()
                .with_min_len(PARALLEL_CELLS)
                .map(|&value| f(value))
                .collect(),
        )
    }

    fn map2(
        &self,
        left: &Value<'_>,
        right: &Value<'_>,
        f: impl Fn(f64, f64) -> f64 + Sync + Send,
    ) -> Value<'a> {
        let cells = match (left.cells(), right.cells()) {
            (None, None) => return Value::Scalar(f(scalar(left), scalar(right))),
            (Some(a), Some(b)) => a
                .par_iter()
                .zip(b)
                .with_min_len(PARALLEL_CELLS)
                .map(|(&a, &b)| f(a, b))
                .collect(),
            (Some(a), None) => {
                let b = scalar(right);
                a.par_iter()
                    .with_min_len(PARALLEL_CELLS)
                    .map(|&a| f(a, b))
                    .collect()
            }
            (None, Some(b)) => {
                let a = scalar(left);
                b.par_iter()
                    .with_min_len(PARALLEL_CELLS)
                    .map(|&b| f(a, b))
                    .collect()
            }
        };
        self.matrix(cells)
    }

    fn map3(
        &self,
        first: &Value<'_>,
        second: &Value<'_>,
        third: &Value<'_>,
        f: impl Fn(f64, f64, f64) -> f64 + Sync + Send,
    ) -> Value<'a> {
        if [first, second, third]
            .iter()
            .all(|value| value.cells().is_none())
        {
            return Value::Scalar(f(scalar(first), scalar(second), scalar(third)));
        }
        let at = |value: &Value<'_>, index: usize| match value.cells() {
            Some(cells) => cells[index],
            None => scalar(value),
        };
        self.matrix(
            (0..self.inputs.n_rows * self.inputs.n_cols)
                .into_par_iter()
                .with_min_len(PARALLEL_CELLS)
                .map(|index| f(at(first, index), at(second, index), at(third, index)))
                .collect(),
        )
    }

    /// Applies a per-symbol series transform to every column in parallel.
    fn map_columns(
        &self,
        input: &Value<'_>,
        f: impl Fn(&[f64]) -> Result<Vec<f64>> + Sync,
    ) -> Result<Value<'a>> {
        let n_rows = self.inputs.n_rows;
        let filled;
        let cells = match input.cells() {
            Some(cells) => cells,
            None => {
                filled = vec![scalar(input); n_rows * self.inputs.n_cols];
                &filled
            }
        };
        self.columns_into(|col, out| {
            out.copy_from_slice(&f(&cells[col * n_rows..(col + 1) * n_rows])?);
            Ok(())
        })
    }

    /// Applies a two-series transform to every pair of columns in parallel.
    fn map_column_pairs(
        &self,
        left: &Value<'_>,
        right: &Value<'_>,
        f: impl Fn(&[f64], &[f64]) -> Vec<f64> + Sync,
    ) -> Result<Value<'a>> {
        let n_rows = self.inputs.n_rows;
        let len = n_rows * self.inputs.n_cols;
        let (left_filled, right_filled);
        let left = match left.cells() {
            Some(cells) => cells,
            None => {
                left_filled = vec![scalar(left); len];
                &left_filled
            }
        };
        let right = match right.cells() {
            Some(cells) => cells,
            None => {
                right_filled = vec![scalar(right); len];
                &right_filled
            }
        };
        self.columns_into(|col, out| {
            let rows = col * n_rows..(col + 1) * n_rows;
            out.copy_from_slice(&f(&left[rows.clone()], &right[rows]));
            Ok(())
        })
    }

    /// Builds a matrix by filling each output column independently, in parallel.
    fn columns_into(
        &self,
        fill: impl Fn(usize, &mut [f64]) -> Result<()> + Sync,
    ) -> Result<Value<'a>> {
        let n_rows = self.inputs.n_rows;
        let mut data = vec![0.0; n_rows * self.inputs.n_cols];
        if n_rows > 0 {
            data.par_chunks_mut(n_rows)
                .enumerate()
                .try_for_each(|(col, out)| fill(col, out))?;
        }
        Ok(self.matrix(data))
    }

    fn matrix(&self, cells: Vec<f64>) -> Value<'a> {
        Value::Matrix(Arc::new(
            Matrix::from_column_major(cells, self.inputs.n_rows, self.inputs.n_cols)
                .expect("every operand has the input shape"),
        ))
    }
}

fn scalar(value: &Value<'_>) -> f64 {
    match value {
        Value::Scalar(value) => *value,
        _ => unreachable!("only called for scalar operands"),
    }
}

/// The operands [`Evaluator::call`] evaluates. Window lengths, offsets and
/// other literal parameters are read from the tree, not evaluated.
fn operands(expr: &Expression) -> Vec<&Expression> {
    match expr {
        Expression::Number(_) | Expression::Field(_) => Vec::new(),
        Expression::Unary(_, child) => vec![child],
        Expression::Binary(_, left, right) => vec![left, right],
        Expression::Call(name, args) => match name.as_str() {
            "Ref" | "Delta" | "EMA" | "Mean" | "Std" | "Sum" | "Max" | "Min" | "Quantile"
            | "Rank" | "IdxMax" | "IdxMin" | "Slope" | "Rsquare" | "Resi" | "Var" | "Skew"
            | "Kurt" | "Med" | "Mad" | "Count" | "WMA" => args.iter().take(1).collect(),
            "Corr" | "Cov" => args.iter().take(2).collect(),
            _ => args.iter().collect(),
        },
    }
}

/// Elementwise work below this many cells stays on the calling thread.
const PARALLEL_CELLS: usize = 1 << 14;

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
        "Std" => Ok(slide(input, window, &mut Deviation::std())),
        "Var" => Ok(slide(input, window, &mut Deviation::var())),
        "Max" => rolling_max_1d(input, window, Some(1)),
        "Min" => rolling_min_1d(input, window, Some(1)),
        "Sum" => Ok(slide(input, window, &mut Sum::default())),
        "Quantile" => {
            if !(0.0..=1.0).contains(&q) {
                return Err(Error::Invariant("quantile must be in 0..=1".into()));
            }
            Ok(ordered(input, window, Order::Quantile(q)))
        }
        "Med" => Ok(ordered(input, window, Order::Quantile(0.5))),
        "Mad" => Ok(ordered(input, window, Order::Mad)),
        "Count" => rolling_apply_1d(input, window, Some(1), |_, values| {
            values.iter().filter(|value| !value.is_nan()).count() as f64
        }),
        "Skew" => Ok(slide(input, window, &mut Shape::skew())),
        "Kurt" => Ok(slide(input, window, &mut Shape::kurt())),
        "WMA" => Ok(slide(input, window, &mut Wma::default())),
        "Rank" => Ok(ordered(input, window, Order::Rank)),
        "IdxMax" => Ok(rolling_arg_extreme(input, window, false)),
        "IdxMin" => Ok(rolling_arg_extreme(input, window, true)),
        "Slope" => Ok(slide(input, window, &mut Regression::new(Fit::Slope))),
        "Rsquare" => Ok(slide(input, window, &mut Regression::new(Fit::Rsquare))),
        "Resi" => Ok(slide(input, window, &mut Regression::new(Fit::Resi))),
        _ => Err(Error::Invariant(format!(
            "unsupported rolling operator {name}"
        ))),
    }
}

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(any(test, feature = "polars"))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qlib_rank_is_the_last_values_percentile() {
        assert!((rank_last(&[3.0, 1.0, 2.0]) - 2.0 / 3.0).abs() < 1e-12);
        assert!((rank_last(&[1.0, 2.0, 2.0]) - 2.5 / 3.0).abs() < 1e-12);
    }

    /// Series that stress the running sums: gaps, all-`NaN` stretches,
    /// constant runs, a large level with small moves, and plain noise.
    fn stress_series() -> Vec<Vec<f64>> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1_u64 << 53) as f64
        };
        let mut out = Vec::new();
        for case in 0..6 {
            let series = (0..300)
                .map(|row| {
                    let u = next();
                    match case {
                        0 => u,
                        1 if u < 0.2 => f64::NAN,
                        1 => 100.0 + u,
                        2 if (40..80).contains(&row) => f64::NAN,
                        2 if (120..170).contains(&row) => 7.25,
                        2 => 1e6 * u,
                        3 if u < 0.6 => f64::NAN,
                        3 => (u * 4.0).floor(),
                        4 => 1e4 + row as f64 * 0.5 + u * 1e-3,
                        _ => (row / 25) as f64,
                    }
                })
                .collect();
            out.push(series);
        }
        out
    }

    /// [`regression`] with two-pass centered sums instead of raw ones. The
    /// raw-sum form carries the reference's own cancellation error (a
    /// two-point fit of prices near 100 gives r² = 0.999999998), so the
    /// incremental kernels are checked against the exact form here and against
    /// Qlib itself in `tests/parity_qlib.rs`.
    fn centered_regression(values: &[f64]) -> (f64, f64, f64) {
        let points: Vec<(f64, f64)> = values
            .iter()
            .enumerate()
            .filter_map(|(i, &y)| (!y.is_nan()).then_some(((i + 1) as f64, y)))
            .collect();
        if points.len() < 2 {
            return (f64::NAN, f64::NAN, f64::NAN);
        }
        let n = points.len() as f64;
        let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
        let my = points.iter().map(|p| p.1).sum::<f64>() / n;
        let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
        let syy: f64 = points.iter().map(|p| (p.1 - my).powi(2)).sum();
        let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let slope = sxy / sxx;
        let sample_std = (syy / (n - 1.0)).sqrt();
        let rsquare = if sxx <= 0.0 || syy <= 0.0 || sample_std <= 2e-5 {
            f64::NAN
        } else {
            sxy * sxy / (sxx * syy)
        };
        let residual = values.last().copied().unwrap_or(f64::NAN)
            - (slope * values.len() as f64 + my - slope * mx);
        (slope, rsquare, residual)
    }

    type Reducer = fn(&[f64]) -> f64;

    fn assert_close(label: &str, fast: &[f64], naive: &[f64]) {
        for (row, (&a, &b)) in fast.iter().zip(naive).enumerate() {
            let ok = (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * b.abs().max(1.0);
            assert!(ok, "{label}[{row}]: incremental={a:?}, rescan={b:?}");
        }
    }

    #[test]
    fn incremental_windows_match_a_full_rescan() {
        let reference: [(&str, Reducer); 14] = [
            ("Sum", |v| v.iter().filter(|x| !x.is_nan()).sum()),
            ("Std", |v| {
                rolling_std_qlib(v, v.len(), 1)
                    .unwrap()
                    .last()
                    .copied()
                    .unwrap()
            }),
            ("Var", |v| {
                rolling_std_qlib(v, v.len(), 1)
                    .unwrap()
                    .last()
                    .map(|x| x * x)
                    .unwrap()
            }),
            ("WMA", wma),
            ("Slope", |v| centered_regression(v).0),
            ("Rsquare", |v| centered_regression(v).1),
            ("Resi", |v| centered_regression(v).2),
            ("Med", |v| quantile(v, 0.5)),
            ("Mad", mad),
            ("Skew", skew),
            ("Kurt", kurt),
            ("Rank", rank_last),
            ("IdxMax", |v| arg_extreme(v, false)),
            ("IdxMin", |v| arg_extreme(v, true)),
        ];
        for (case, series) in stress_series().iter().enumerate() {
            for window in [1, 2, 3, 5, 20, 61, 299, 300, 1000] {
                for (name, reduce) in reference {
                    let fast = rolling_single(name, series, window, 0.0).unwrap();
                    let naive =
                        rolling_apply_1d(series, window, Some(1), |_, v| reduce(v)).unwrap();
                    assert_close(
                        &format!("{name} case {case} window {window}"),
                        &fast,
                        &naive,
                    );
                }
                for q in [0.0, 0.2, 0.5, 0.8, 1.0] {
                    let fast = rolling_single("Quantile", series, window, q).unwrap();
                    let naive =
                        rolling_apply_1d(series, window, Some(1), |_, v| quantile(v, q)).unwrap();
                    assert_close(
                        &format!("Quantile {q} case {case} window {window}"),
                        &fast,
                        &naive,
                    );
                }
                for (name, fast, reducer) in [
                    (
                        "Max",
                        rolling_single("Max", series, window, 0.0).unwrap(),
                        (|v: &[f64]| {
                            v.iter()
                                .copied()
                                .filter(|x| !x.is_nan())
                                .fold(f64::NAN, f64::max)
                        }) as Reducer,
                    ),
                    (
                        "Min",
                        rolling_single("Min", series, window, 0.0).unwrap(),
                        |v: &[f64]| {
                            v.iter()
                                .copied()
                                .filter(|x| !x.is_nan())
                                .fold(f64::NAN, f64::min)
                        },
                    ),
                ] {
                    let naive =
                        rolling_apply_1d(series, window, Some(1), |_, v| reducer(v)).unwrap();
                    assert_close(
                        &format!("{name} case {case} window {window}"),
                        &fast,
                        &naive,
                    );
                }
            }
        }
    }

    #[test]
    fn incremental_pairs_match_a_full_rescan() {
        let series = stress_series();
        for (i, left) in series.iter().enumerate() {
            for (j, right) in series.iter().enumerate() {
                let pairs: Vec<(f64, f64)> =
                    left.iter().copied().zip(right.iter().copied()).collect();
                let left = Matrix::from_column_major(left.clone(), left.len(), 1).unwrap();
                let right = Matrix::from_column_major(right.clone(), right.len(), 1).unwrap();
                for window in [1, 2, 5, 20, 61, 300] {
                    let label = format!("case {i}x{j} window {window}");
                    let fast = slide(&pairs, window, &mut Pair::corr());
                    let naive = naive_rolling_pair(&left, &right, window, corr).unwrap();
                    assert_close(&format!("Corr {label}"), &fast, naive.as_slice());
                    let fast = slide(&pairs, window, &mut Pair::cov());
                    let naive = naive_rolling_pair(&left, &right, window, cov).unwrap();
                    assert_close(&format!("Cov {label}"), &fast, naive.as_slice());
                }
            }
        }
    }

    #[test]
    fn regression_finds_a_line_and_its_last_residual() {
        let (slope, r2, residual) = regression(&[2.0, 4.0, 6.0]);
        assert!((slope - 2.0).abs() < 1e-12);
        assert!((r2 - 1.0).abs() < 1e-12);
        assert!(residual.abs() < 1e-12);
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

    fn naive_rolling_pair(
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
}
