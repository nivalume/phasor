//! Qlib-style factor expressions over the engine's column-major panels.
//!
//! Expressions are compiled once and can then be evaluated against many
//! [`FactorInputs`]. The result is already a [`phasor_core::matrix::Matrix`], so
//! it feeds the existing signal and portfolio layers directly.

mod alpha158;
mod alpha360;
mod backend;
mod cross;
mod eval;
mod graph;
mod order;
mod parse;
#[cfg(feature = "polars")]
mod polars_backend;
mod stage;
mod window;
mod wq101;

pub use alpha158::alpha158;
pub use alpha360::alpha360;
pub use backend::{ComputeBackend, NativeBackend};
pub use eval::{FactorInputs, FactorValues};
pub use parse::Expression;
#[cfg(feature = "polars")]
pub use polars_backend::PolarsBackend;
pub use wq101::{wq101, wq101_all, wq101_formula};

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::OnceLock;

use phasor_core::error::{Error, Result};

use crate::parse::parse;

/// A named, compiled collection of factor expressions.
#[derive(Debug, Clone)]
pub struct FactorSet {
    factors: Vec<(String, Expression)>,
    graph: graph::Graph,
    /// The stages, when a factor reads across symbols.
    plan: Option<stage::Staged>,
    /// The factors split into separately compiled groups, built on first use
    /// by a panel narrower than the thread pool.
    parts: OnceLock<Vec<eval::Part>>,
}

impl FactorSet {
    /// Parses and validates `(name, expression)` pairs.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or repeated name, invalid syntax, an
    /// unsupported operator, or an invalid literal parameter (a fractional or
    /// negative window, a quantile outside `0..=1`, a negative EMA span).
    pub fn compile<N, S, I>(factors: I) -> Result<Self>
    where
        N: Into<String>,
        S: AsRef<str>,
        I: IntoIterator<Item = (N, S)>,
    {
        let mut seen = BTreeSet::new();
        let mut compiled = Vec::new();
        for (name, source) in factors {
            let name = name.into();
            if name.is_empty() {
                return Err(Error::Invariant("factor name cannot be empty".into()));
            }
            if !seen.insert(name.clone()) {
                return Err(Error::Invariant(format!("duplicate factor name {name:?}")));
            }
            let expr = parse(source.as_ref())
                .map_err(|message| Error::Invariant(format!("factor {name:?}: {message}")))?;
            compiled.push((name, expr));
        }
        if compiled.is_empty() {
            return Err(Error::Invariant("a factor set cannot be empty".into()));
        }
        let graph =
            graph::Graph::compile(compiled.iter().map(|(name, expr)| (name.as_str(), expr)))?;
        let plan = stage::Staged::plan(&graph);
        Ok(Self {
            factors: compiled,
            graph,
            plan,
            parts: OnceLock::new(),
        })
    }

    /// Names in output order.
    pub fn names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.factors.iter().map(|(name, _)| name.as_str())
    }

    /// Compiled expressions in output order.
    pub fn expressions(&self) -> impl ExactSizeIterator<Item = (&str, &Expression)> {
        self.factors
            .iter()
            .map(|(name, expression)| (name.as_str(), expression))
    }

    /// Source fields required to evaluate this set.
    #[must_use]
    pub fn required_fields(&self) -> BTreeSet<&str> {
        let mut fields = BTreeSet::new();
        for (_, expr) in &self.factors {
            expr.fields(&mut fields);
        }
        fields
    }

    /// Whether any expression reads a future row.
    #[must_use]
    pub fn uses_future(&self) -> bool {
        self.factors.iter().any(|(_, expr)| expr.uses_future())
    }

    /// Evaluates every factor over every input symbol.
    ///
    /// # Errors
    ///
    /// Returns an error when an input field is missing or has the wrong shape,
    /// or an operator receives invalid arguments.
    pub fn evaluate(&self, inputs: &FactorInputs<'_>) -> Result<FactorValues> {
        self.evaluate_with(inputs, &NativeBackend)
    }

    /// Evaluates with an explicitly selected compute backend.
    ///
    /// # Errors
    ///
    /// Propagates input, operator, or backend execution errors.
    pub fn evaluate_with(
        &self,
        inputs: &FactorInputs<'_>,
        backend: &dyn ComputeBackend,
    ) -> Result<FactorValues> {
        backend.evaluate(self, inputs)
    }

    /// Evaluates `batch` symbols at a time, handing each batch's values to
    /// `sink` with the range of symbols they cover, in order. Without a
    /// cross-sectional operator only one batch is held at a time, so peak
    /// memory is bounded by `batch`, not by the panel; with one, every symbol
    /// is needed for every row, so the whole panel is evaluated first and
    /// then handed over in batches.
    ///
    /// # Errors
    ///
    /// As [`FactorSet::evaluate`], a zero `batch`, and whatever `sink`
    /// returns, which stops the run.
    pub fn evaluate_batches(
        &self,
        inputs: &FactorInputs<'_>,
        batch: usize,
        mut sink: impl FnMut(Range<usize>, FactorValues) -> Result<()>,
    ) -> Result<()> {
        if batch == 0 {
            return Err(Error::Invariant(
                "a batch must hold at least one symbol".into(),
            ));
        }
        let (n_rows, n_cols) = inputs.shape();
        if self.plan.is_some() {
            let all = self.evaluate(inputs)?;
            for start in (0..n_cols).step_by(batch) {
                let cols = start..(start + batch).min(n_cols);
                sink(cols.clone(), all.columns(cols)?)?;
            }
            return Ok(());
        }
        for start in (0..n_cols).step_by(batch) {
            let cols = start..(start + batch).min(n_cols);
            let mut part = FactorInputs::new(n_rows, cols.len())?;
            for (name, values) in inputs.fields() {
                part = part.with_field(name, &values[cols.start * n_rows..cols.end * n_rows])?;
            }
            sink(cols, self.evaluate(&part)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use phasor_model::OhlcvPanel;

    #[test]
    fn literals_and_fields_broadcast_and_shared_nodes_are_reused() {
        let panel = OhlcvPanel {
            close: vec![1.0, 2.0, 4.0, 10.0, 20.0, 40.0],
            n_rows: 3,
            n_cols: 2,
            ..OhlcvPanel::default()
        };
        let inputs = FactorInputs::from_ohlcv(&panel).unwrap();
        let factors = FactorSet::compile([
            ("constant", "1 + 2"),
            ("field", "+$close"),
            ("left", "2 * $close"),
            ("right", "$close - 1"),
            ("choice", "If($close > 3, 1, -1)"),
            ("rolling_literal", "Mean(4, 2)"),
            ("pair_literal", "Cov($close, 5, 2)"),
            ("shared", "Ref($close, 1) / Ref($close, 1)"),
            ("repeat", "2 * $close"),
        ])
        .unwrap();
        let values = factors.evaluate(&inputs).unwrap();
        let column = |name: &str, col: usize| values.get(name).unwrap().column(col).to_vec();

        assert_eq!(column("constant", 1), [3.0; 3]);
        assert_eq!(column("field", 1), [10.0, 20.0, 40.0]);
        assert_eq!(column("left", 0), [2.0, 4.0, 8.0]);
        assert_eq!(column("right", 1), [9.0, 19.0, 39.0]);
        assert_eq!(column("choice", 0), [-1.0, -1.0, 1.0]);
        assert_eq!(column("rolling_literal", 1), [4.0; 3]);
        assert!(column("pair_literal", 0)[0].is_nan());
        assert_eq!(column("pair_literal", 0)[1..], [0.0, 0.0]);
        assert!(column("shared", 0)[0].is_nan());
        assert_eq!(column("shared", 1)[1..], [1.0, 1.0]);
        assert_eq!(column("repeat", 1), column("left", 1));
    }

    #[test]
    fn missing_fields_fail_before_any_work() {
        let panel = OhlcvPanel {
            close: vec![1.0, 2.0],
            n_rows: 2,
            n_cols: 1,
            ..OhlcvPanel::default()
        };
        let inputs = FactorInputs::from_ohlcv(&panel).unwrap();
        let factors = FactorSet::compile([("f", "$vwap / $close")]).unwrap();
        let error = factors.evaluate(&inputs).unwrap_err().to_string();
        assert!(error.contains("$vwap"), "{error}");
    }

    #[test]
    fn each_symbol_evaluates_exactly_as_it_would_alone() {
        let (n_rows, n_cols) = (90, 3);
        let series = |base: f64| {
            (0..n_cols)
                .flat_map(|col| {
                    (0..n_rows).map(move |row| {
                        if (row + col) % 17 == 0 {
                            f64::NAN
                        } else {
                            base + col as f64 * 3.0 + ((row * 7 + col) % 11) as f64
                        }
                    })
                })
                .collect::<Vec<_>>()
        };
        let panel = OhlcvPanel {
            open: series(10.0),
            high: series(12.0),
            low: series(8.0),
            close: series(10.5),
            volume: series(1000.0),
            n_rows,
            n_cols,
        };
        let vwap = series(10.2);
        let factors = alpha158().unwrap();
        let all = factors
            .evaluate(
                &FactorInputs::from_ohlcv(&panel)
                    .unwrap()
                    .with_field("vwap", &vwap)
                    .unwrap(),
            )
            .unwrap();
        for col in 0..n_cols {
            let one = |values: &[f64]| values[col * n_rows..(col + 1) * n_rows].to_vec();
            let single = OhlcvPanel {
                open: one(&panel.open),
                high: one(&panel.high),
                low: one(&panel.low),
                close: one(&panel.close),
                volume: one(&panel.volume),
                n_rows,
                n_cols: 1,
            };
            let single_vwap = one(&vwap);
            let alone = factors
                .evaluate(
                    &FactorInputs::from_ohlcv(&single)
                        .unwrap()
                        .with_field("vwap", &single_vwap)
                        .unwrap(),
                )
                .unwrap();
            for ((name, matrix), (_, expected)) in all.iter().zip(alone.iter()) {
                let same = matrix
                    .column(col)
                    .iter()
                    .zip(expected.column(0))
                    .all(|(a, b)| a.to_bits() == b.to_bits());
                assert!(same, "{name} column {col}");
            }
        }
    }

    #[test]
    fn expressions_feed_column_major_factor_matrices() {
        let panel = OhlcvPanel {
            close: vec![1.0, 2.0, 4.0, 10.0, 20.0, 40.0],
            n_rows: 3,
            n_cols: 2,
            ..OhlcvPanel::default()
        };
        let inputs = FactorInputs::from_ohlcv(&panel).unwrap();
        let factors = FactorSet::compile([
            ("roc", "Ref($close, 1) / $close"),
            ("ma", "Mean($close, 2) / $close"),
        ])
        .unwrap();
        let values = factors.evaluate(&inputs).unwrap();

        assert!(values.get("roc").unwrap().get(0, 0).is_nan());
        assert!((values.get("roc").unwrap().get(2, 0) - 0.5).abs() < 1e-12);
        assert!((values.get("ma").unwrap().get(1, 1) - 0.75).abs() < 1e-12);
    }

    #[test]
    fn alpha158_has_the_reference_shape_and_fields() {
        let factors = alpha158().unwrap();
        assert_eq!(factors.names().len(), 158);
        assert_eq!(
            factors.names().skip(13).take(7).collect::<Vec<_>>(),
            ["ROC5", "ROC10", "ROC20", "ROC30", "ROC60", "MA5", "MA10"]
        );
        assert_eq!(
            factors.required_fields(),
            BTreeSet::from(["close", "high", "low", "open", "volume", "vwap"])
        );
        assert!(!factors.uses_future());
        assert!(
            FactorSet::compile([("label", "Ref($close, -2)")])
                .unwrap()
                .uses_future()
        );
    }

    #[test]
    fn every_alpha158_factor_evaluates_for_every_symbol() {
        let n_rows = 70;
        let n_cols = 2;
        let series = |base: f64, step: f64| {
            (0..n_cols)
                .flat_map(|col| {
                    (0..n_rows).map(move |row| base + col as f64 * 10.0 + row as f64 * step)
                })
                .collect::<Vec<_>>()
        };
        let panel = OhlcvPanel {
            open: series(100.0, 0.10),
            high: series(101.0, 0.12),
            low: series(99.0, 0.08),
            close: series(100.5, 0.11),
            volume: series(1_000.0, 3.0),
            n_rows,
            n_cols,
        };
        let vwap = series(100.4, 0.105);
        let inputs = FactorInputs::from_ohlcv(&panel)
            .unwrap()
            .with_field("vwap", &vwap)
            .unwrap();
        let values = alpha158().unwrap().evaluate(&inputs).unwrap();

        assert_eq!(values.iter().len(), 158);
        assert!(
            values
                .iter()
                .all(|(_, value)| { value.n_rows() == n_rows && value.n_cols() == n_cols })
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "these values are exactly representable results of simple arithmetic"
    )]
    fn common_qlib_conditionals_and_expanding_windows_work() {
        let panel = OhlcvPanel {
            close: vec![1.0, 2.0, 3.0, 4.0],
            n_rows: 4,
            n_cols: 1,
            ..OhlcvPanel::default()
        };
        let values = FactorSet::compile([
            (
                "conditional",
                "If(($close>=2)&($close<4),Power($close,2),-1)",
            ),
            ("expanding", "Mean($close,0)"),
        ])
        .unwrap()
        .evaluate(&FactorInputs::from_ohlcv(&panel).unwrap())
        .unwrap();

        assert_eq!(
            values.get("conditional").unwrap().column(0),
            [-1.0, 4.0, 9.0, -1.0]
        );
        assert_eq!(
            values.get("expanding").unwrap().column(0),
            [1.0, 1.5, 2.0, 2.5]
        );
    }

    #[test]
    fn zero_offset_ref_and_delta_use_the_current_row() {
        let panel = OhlcvPanel {
            close: vec![1.0, 2.0, 3.0],
            n_rows: 3,
            n_cols: 1,
            ..OhlcvPanel::default()
        };
        let values = FactorSet::compile([("same", "Ref($close,0)"), ("zero", "Delta($close,0)")])
            .unwrap()
            .evaluate(&FactorInputs::from_ohlcv(&panel).unwrap())
            .unwrap();

        assert_eq!(values.get("same").unwrap().column(0), [1.0, 2.0, 3.0]);
        assert_eq!(values.get("zero").unwrap().column(0), [0.0, 0.0, 0.0]);
    }
}
