use std::collections::HashMap;
use std::sync::Arc;

use barq_core::error::{Error, Result};
use barq_core::matrix::Matrix;
use barq_generic::rolling::{rolling_apply_1d, rolling_max_1d, rolling_mean_1d, rolling_min_1d};
use barq_model::OhlcvPanel;
use rayon::prelude::*;

use crate::FactorSet;
use crate::graph::{self, BinaryOp, Graph, NodeId, Op, Rolling};
use crate::order::{Order, ordered, rolling_arg_extreme};
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
    let graph = &set.graph;
    let (n_rows, n_cols) = (inputs.n_rows, inputs.n_cols);
    let fields =
        graph
            .fields
            .iter()
            .map(|name| {
                inputs.fields.get(name).copied().ok_or_else(|| {
                    Error::Invariant(format!("factor input is missing field ${name}"))
                })
            })
            .collect::<Result<Vec<&[f64]>>>()?;

    // One allocation per output rather than cloning a zeroed buffer, so the
    // pages are zeroed lazily by the columns that write them, in parallel.
    let mut outputs: Vec<Vec<f64>> = (0..graph.outputs.len())
        .map(|_| vec![0.0; n_rows * n_cols])
        .collect();
    if n_rows > 0 {
        // Each column's slice of every output, so columns can be filled in
        // parallel without sharing a mutable matrix.
        let mut columns: Vec<Vec<&mut [f64]>> = (0..n_cols)
            .map(|_| Vec::with_capacity(outputs.len()))
            .collect();
        for output in &mut outputs {
            for (column, cells) in columns.iter_mut().zip(output.chunks_mut(n_rows)) {
                column.push(cells);
            }
        }
        columns.into_par_iter().enumerate().for_each_init(
            || vec![Vec::new(); graph.slots],
            |scratch, (col, mut out)| {
                let fields: Vec<&[f64]> = fields
                    .iter()
                    .map(|values| &values[col * n_rows..(col + 1) * n_rows])
                    .collect();
                run_column(graph, &fields, n_rows, scratch, &mut out);
            },
        );
    }
    let names = set.names().map(str::to_owned).collect();
    let matrices = outputs
        .into_iter()
        .map(|cells| Matrix::from_column_major(cells, n_rows, n_cols))
        .collect::<Result<Vec<_>>>()?;
    FactorValues::from_matrices(names, matrices)
}

/// An operand of one symbol's step: a literal, or a series.
#[derive(Clone, Copy)]
enum Arg<'s> {
    Scalar(f64),
    Series(&'s [f64]),
}

impl<'s> Arg<'s> {
    fn at(self, row: usize) -> f64 {
        match self {
            Self::Scalar(value) => value,
            Self::Series(values) => values[row],
        }
    }

    /// The operand as a series, filling `buffer` for a literal.
    fn series<'b>(self, buffer: &'b mut Vec<f64>, n_rows: usize) -> &'b [f64]
    where
        's: 'b,
    {
        match self {
            Self::Series(values) => values,
            Self::Scalar(value) => {
                buffer.clear();
                buffer.resize(n_rows, value);
                buffer
            }
        }
    }
}

/// Evaluates the whole graph for one symbol, writing each factor's column.
fn run_column(
    graph: &Graph,
    fields: &[&[f64]],
    n_rows: usize,
    scratch: &mut [Vec<f64>],
    out: &mut [&mut [f64]],
) {
    for node in &graph.nodes {
        let Some(slot) = node.slot else {
            // Fields and literals are read in place; write them only if a
            // factor is the bare field or literal.
            for &position in &node.outputs {
                match node.op {
                    Op::Field(index) => out[position].copy_from_slice(fields[index]),
                    Op::Const(bits) => out[position].fill(f64::from_bits(bits)),
                    _ => unreachable!("only fields and literals have no slot"),
                }
            }
            continue;
        };
        // Take the target out of the scratch so its inputs can be borrowed;
        // the schedule never gives a node the slot of one of its inputs.
        let mut target = std::mem::take(&mut scratch[slot]);
        {
            let arg = |id: NodeId| {
                let input = &graph.nodes[id];
                match (input.op, input.slot) {
                    (Op::Const(bits), _) => Arg::Scalar(f64::from_bits(bits)),
                    (Op::Field(index), _) => Arg::Series(fields[index]),
                    (_, Some(slot)) => Arg::Series(&scratch[slot]),
                    (_, None) => unreachable!("computed nodes have slots"),
                }
            };
            let mut args = [Arg::Scalar(0.0); 3];
            for (slot, &id) in args.iter_mut().zip(&node.inputs) {
                *slot = arg(id);
            }
            step(node.op, &args[..node.inputs.len()], n_rows, &mut target);
        }
        for &position in &node.outputs {
            out[position].copy_from_slice(&target);
        }
        scratch[slot] = target;
    }
}

/// Computes one node's series for one symbol into `target`.
fn step(op: Op, args: &[Arg<'_>], n_rows: usize, target: &mut Vec<f64>) {
    let mut buffer = Vec::new();
    match op {
        Op::Unary(op) => {
            fill(target, n_rows, |row| graph::unary(op, args[0].at(row)));
        }
        Op::Binary(op) => binary_series(op, args[0], args[1], n_rows, target),
        Op::If => fill(target, n_rows, |row| {
            graph::if_else(args[0].at(row), args[1].at(row), args[2].at(row))
        }),
        Op::Ref(offset) => {
            let column = args[0].series(&mut buffer, n_rows);
            fill(target, n_rows, |row| {
                let source = row as i64 - offset;
                if offset == 0 {
                    column[row]
                } else if source < 0 || source >= n_rows as i64 {
                    f64::NAN
                } else {
                    column[source as usize]
                }
            });
        }
        Op::Delta(offset) => {
            let column = args[0].series(&mut buffer, n_rows);
            fill(target, n_rows, |row| {
                if offset == 0 {
                    if column[row].is_nan() { f64::NAN } else { 0.0 }
                } else if row < offset {
                    f64::NAN
                } else {
                    column[row] - column[row - offset]
                }
            });
        }
        Op::Ema(alpha) => {
            let column = args[0].series(&mut buffer, n_rows);
            *target = match alpha {
                None => ema_expanding(column),
                Some(bits) => ema_adjusted(column, f64::from_bits(bits)),
            };
        }
        Op::Rolling(kind, window) => {
            let column = args[0].series(&mut buffer, n_rows);
            *target = rolling(kind, column, window.resolve(n_rows));
        }
        Op::Pair { corr, window } => {
            let mut other = Vec::new();
            let left = args[0].series(&mut buffer, n_rows);
            let right = args[1].series(&mut other, n_rows);
            let pairs: Vec<(f64, f64)> = left.iter().copied().zip(right.iter().copied()).collect();
            let window = window.resolve(n_rows);
            *target = if corr {
                slide(&pairs, window, &mut Pair::corr())
            } else {
                slide(&pairs, window, &mut Pair::cov())
            };
        }
        Op::Field(_) | Op::Const(_) => unreachable!("fields and literals are not computed"),
    }
}

fn fill(target: &mut Vec<f64>, n_rows: usize, f: impl Fn(usize) -> f64) {
    target.clear();
    target.extend((0..n_rows).map(f));
}

/// An elementwise binary operator with the operator and operand kinds
/// resolved outside the loop, so each case compiles to a straight loop.
fn binary_series(
    op: BinaryOp,
    left: Arg<'_>,
    right: Arg<'_>,
    n_rows: usize,
    target: &mut Vec<f64>,
) {
    fn apply(
        left: Arg<'_>,
        right: Arg<'_>,
        n_rows: usize,
        target: &mut Vec<f64>,
        f: impl Fn(f64, f64) -> f64,
    ) {
        target.clear();
        match (left, right) {
            (Arg::Series(a), Arg::Series(b)) => {
                target.extend(a.iter().zip(b).map(|(&a, &b)| f(a, b)))
            }
            (Arg::Series(a), Arg::Scalar(b)) => target.extend(a.iter().map(|&a| f(a, b))),
            (Arg::Scalar(a), Arg::Series(b)) => target.extend(b.iter().map(|&b| f(a, b))),
            (Arg::Scalar(a), Arg::Scalar(b)) => target.resize(n_rows, f(a, b)),
        }
    }
    macro_rules! dispatch {
        ($($variant:ident),*) => {
            match op {
                $(BinaryOp::$variant => apply(left, right, n_rows, target, |a, b| {
                    graph::binary(BinaryOp::$variant, a, b)
                }),)*
            }
        };
    }
    dispatch!(
        Add, Sub, Mul, Div, Pow, Gt, Lt, Ge, Le, Eq, Ne, And, Or, Greater, Less
    );
}

fn rolling(kind: Rolling, input: &[f64], window: usize) -> Vec<f64> {
    let window = window.max(1);
    let checked = "window and min_periods are valid";
    match kind {
        Rolling::Mean => rolling_mean_1d(input, window, Some(1)).expect(checked),
        Rolling::Std => slide(input, window, &mut Deviation::std()),
        Rolling::Var => slide(input, window, &mut Deviation::var()),
        Rolling::Max => rolling_max_1d(input, window, Some(1)).expect(checked),
        Rolling::Min => rolling_min_1d(input, window, Some(1)).expect(checked),
        Rolling::Sum => slide(input, window, &mut Sum::default()),
        Rolling::Quantile(q) => ordered(input, window, Order::Quantile(f64::from_bits(q))),
        Rolling::Med => ordered(input, window, Order::Quantile(0.5)),
        Rolling::Mad => ordered(input, window, Order::Mad),
        Rolling::Count => rolling_apply_1d(input, window, Some(1), |_, values| {
            values.iter().filter(|value| !value.is_nan()).count() as f64
        })
        .expect(checked),
        Rolling::Skew => slide(input, window, &mut Shape::skew()),
        Rolling::Kurt => slide(input, window, &mut Shape::kurt()),
        Rolling::Wma => slide(input, window, &mut Wma::default()),
        Rolling::Rank => ordered(input, window, Order::Rank),
        Rolling::IdxMax => rolling_arg_extreme(input, window, false),
        Rolling::IdxMin => rolling_arg_extreme(input, window, true),
        Rolling::Slope => slide(input, window, &mut Regression::new(Fit::Slope)),
        Rolling::Rsquare => slide(input, window, &mut Regression::new(Fit::Rsquare)),
        Rolling::Resi => slide(input, window, &mut Regression::new(Fit::Resi)),
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

    fn rolling_single(name: &str, input: &[f64], window: usize, q: f64) -> Result<Vec<f64>> {
        let kind = match name {
            "Sum" => Rolling::Sum,
            "Std" => Rolling::Std,
            "Var" => Rolling::Var,
            "WMA" => Rolling::Wma,
            "Slope" => Rolling::Slope,
            "Rsquare" => Rolling::Rsquare,
            "Resi" => Rolling::Resi,
            "Med" => Rolling::Med,
            "Mad" => Rolling::Mad,
            "Skew" => Rolling::Skew,
            "Kurt" => Rolling::Kurt,
            "Rank" => Rolling::Rank,
            "IdxMax" => Rolling::IdxMax,
            "IdxMin" => Rolling::IdxMin,
            "Quantile" => Rolling::Quantile(q.to_bits()),
            "Max" => Rolling::Max,
            "Min" => Rolling::Min,
            other => panic!("no rolling operator {other}"),
        };
        Ok(rolling(kind, input, window))
    }

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
