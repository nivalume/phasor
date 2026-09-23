//! Qlib-style factor expressions over the engine's column-major panels.
//!
//! Expressions are compiled once and can then be evaluated against many
//! [`FactorInputs`]. The result is already a [`barq_core::matrix::Matrix`], so
//! it feeds the existing signal and portfolio layers directly.

mod alpha158;
mod alpha360;
mod backend;
mod eval;
mod order;
mod parse;
#[cfg(feature = "polars")]
mod polars_backend;
mod window;

pub use alpha158::alpha158;
pub use alpha360::alpha360;
pub use backend::{ComputeBackend, NativeBackend};
pub use eval::{FactorInputs, FactorValues};
pub use parse::Expression;
#[cfg(feature = "polars")]
pub use polars_backend::PolarsBackend;

use std::collections::BTreeSet;

use barq_core::error::{Error, Result};

use crate::parse::parse;

/// A named, compiled collection of factor expressions.
#[derive(Debug, Clone)]
pub struct FactorSet {
    factors: Vec<(String, Expression)>,
}

impl FactorSet {
    /// Parses and validates `(name, expression)` pairs.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or repeated name, invalid syntax, or an
    /// unsupported operator.
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
        Ok(Self { factors: compiled })
    }

    /// Names in output order.
    #[must_use]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use barq_model::OhlcvPanel;

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
