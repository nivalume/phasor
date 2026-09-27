//! Python bindings for barq.
//!
//! A thin shell, deliberately. The engine's own [`Portfolio`] already holds the
//! interesting design — lazy derivation, one cached rebuild chain, per-column
//! metrics — so this layer's whole job is to get NumPy's buffers in and results
//! out without copying and without lying about layout.
//!
//! ```python
//! import numpy as np, barq
//!
//! pf = barq.from_signals(close, entries, exits, init_cash=10_000, fees=0.001)
//! print(pf.stats())            # the 28-row table
//! sharpe = pf.sharpe_ratio()   # one value per column
//! ```
//!
//! # What is exposed, and what is not
//!
//! Both vectorized loops — `from_signals` and `from_orders` — the portfolio they
//! return, its metrics, its statistics table, and its order, trade and log
//! records as NumPy arrays. Factor expressions, Alpha158, and Alpha360 are
//! exposed through the Python shell over this module's compiled
//! [`expr::PyFactorSet`].
//!
//! Not exposed yet: the rolling metrics, drawdown
//! records, and the indicator sweep. Each is a small addition on this
//! scaffolding, and none of them changes it.
//!
//! # Two things worth knowing before using it
//!
//! **Arrays must be column-major.** The engine is; NumPy is not by default.
//! The Python wrapper converts, and the Rust layer refuses anything it cannot
//! read correctly rather than reading it wrongly — see [`array`] for why that
//! is a refusal rather than a silent transpose.
//!
//! **A column that never traded has an infinite Sharpe.** That is the
//! reference's answer and it tops a naive ranking. `traded_only` masks those to
//! `NaN`, which every reduction skips.
//!
//! **`log=True` answers why a bar did not trade**, which no order record can:
//! absence of a fill is consistent with a dozen causes. It is off by default
//! because a log row is 38 fields written for every order *considered*.

#![deny(unsafe_code)]

pub mod array;
#[allow(unsafe_code)]
mod arrow;
pub mod data;
pub mod expr;
pub mod params;
pub mod parse;

use std::sync::OnceLock;

use barq_analysis::stats::{Select, Value};
use barq_analysis::{Cache, Portfolio as CorePortfolio, Scoring};
use barq_core::flex::FlexArray;
use barq_core::matrix::Matrix;
use barq_model::TradesType;
use barq_records::{LogRecords, OrderRecords};
use barq_sim::from_orders::FromOrders;
use barq_sim::from_signals::FromSignals;
use barq_sim::groups::{CallSeq, Groups};
use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyDictMethods};

use array::{
    Borrowed, OneDim, borrow_bool, borrow_f64, borrow_opt_f64, columns_of, flex_bool, flex_or_bool,
    flex_or_f64, to_py_err,
};
use expr::PyFactorSet;
use params::Shared;

/// A completed run, and everything derivable from it.
///
/// # Why the arrays are owned here
///
/// The engine's `Portfolio` borrows its prices, which is right for Rust: a
/// sweep's close matrix is the largest thing in the system and the result of a
/// run has no reason to own a second copy. Across the FFI boundary that does
/// not survive — Python may free its array the moment this returns — so the
/// close prices and starting cash are copied once, here, and the borrow is
/// rebuilt per call from the copy.
///
/// The copy is one array, once. The rebuild chain — the value curve, returns,
/// trades and drawdowns — is derived on demand into [`Self::cache`], which
/// outlives the per-call engine view, so a curve computed for one call is
/// still there for the next. [`Self::clear_cache`] gives the memory back.
#[pyclass(name = "Portfolio", module = "barq")]
#[derive(Debug)]
pub struct PyPortfolio {
    orders: OrderRecords,
    /// The fills in column-major order when the run shared cash and emitted
    /// them in execution order; `None` when `orders` already is. Sorted once
    /// here rather than on every rebuild.
    sorted_orders: Option<OrderRecords>,
    /// Derivations kept across calls; the engine view borrows it.
    cache: Cache,
    /// Buy-and-hold returns, the default benchmark for `stats`, computed once.
    benchmark: OnceLock<Matrix>,
    /// Empty unless the run asked for logs. Not part of the derivation chain:
    /// nothing computed from a portfolio reads a log, which is the point —
    /// logs explain the run rather than contributing to it.
    logs: LogRecords,
    close: Vec<f64>,
    source_cols: usize,
    column_map: Option<Vec<usize>>,
    init_cash: Vec<f64>,
    groups: Groups,
    cash_sharing: bool,
    n_rows: usize,
    n_cols: usize,
    ann_factor: f64,
    risk_free: f64,
    incl_open: bool,
    trades_type: TradesType,
}

impl PyPortfolio {
    /// Rebuilds the engine's portfolio over this object's own buffers.
    ///
    /// Called per method rather than held, because the engine's type borrows
    /// and a `#[pyclass]` cannot hold a borrow of itself. The records and the
    /// cache are lent rather than cloned, so the rebuild itself is free and
    /// whatever one call derives, the next one finds.
    fn core(&self) -> PyResult<CorePortfolio<'_>> {
        let close = market_array(
            &self.close,
            self.n_rows,
            self.source_cols,
            self.column_map.as_deref(),
        )?;
        let cash = FlexArray::per_column(&self.init_cash, self.n_cols).map_err(to_py_err)?;
        CorePortfolio::from_borrowed(
            self.sorted_orders.as_ref().unwrap_or(&self.orders),
            close,
            cash,
            self.n_rows,
            self.n_cols,
            Scoring {
                ann_factor: self.ann_factor,
                risk_free: self.risk_free,
                trades_type: self.trades_type,
                incl_open: self.incl_open,
            },
            self.groups.clone(),
            &self.cache,
        )
        .map_err(to_py_err)
    }

    /// Assembles the result of a run into a portfolio, sorting the records
    /// once if a shared account emitted them in execution order.
    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per thing a run leaves behind"
    )]
    fn assemble(
        orders: OrderRecords,
        logs: LogRecords,
        close: Vec<f64>,
        source_cols: usize,
        column_map: Option<Vec<usize>>,
        init_cash: Vec<f64>,
        groups: Groups,
        cash_sharing: bool,
        n_rows: usize,
        n_cols: usize,
        scoring: Scoring,
    ) -> Self {
        let sorted_orders = groups.is_shared().then(|| {
            let mut sorted = orders.clone();
            sorted.sort_by_column();
            sorted
        });
        Self {
            orders,
            sorted_orders,
            cache: Cache::default(),
            benchmark: OnceLock::new(),
            logs,
            close,
            source_cols,
            column_map,
            init_cash,
            groups,
            cash_sharing,
            n_rows,
            n_cols,
            ann_factor: scoring.ann_factor,
            risk_free: scoring.risk_free,
            incl_open: scoring.incl_open,
            trades_type: scoring.trades_type,
        }
    }
}

#[pymethods]
impl PyPortfolio {
    /// Bytes retained for source prices, independent of parameter count.
    #[getter]
    fn price_nbytes(&self) -> usize {
        self.close.len() * std::mem::size_of::<f64>()
    }

    /// Number of bars.
    #[getter]
    const fn n_rows(&self) -> usize {
        self.n_rows
    }

    /// Number of columns.
    #[getter]
    const fn n_cols(&self) -> usize {
        self.n_cols
    }

    /// Number of independent cash accounts.
    #[getter]
    fn n_groups(&self) -> usize {
        self.groups.len()
    }

    /// Whether cash sharing was requested.
    #[getter]
    const fn cash_sharing(&self) -> bool {
        self.cash_sharing
    }

    /// Sizes of the contiguous account groups.
    #[getter]
    fn group_lens<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
        pyo3::types::PyTuple::new(py, (0..self.groups.len()).map(|g| self.groups.len_of(g)))
    }

    /// Number of fills across every column.
    #[getter]
    fn n_fills(&self) -> usize {
        self.orders.len()
    }

    /// The statistics table for one column, as a dict of title to value.
    ///
    /// Row order is the reference's and is stable, which is what lets two runs
    /// be diffed. A metric with no answer comes back as `float('nan')`; one
    /// whose inputs were not supplied comes back as `None`, which is a
    /// different statement.
    #[pyo3(signature = (col = None, *, group = None))]
    fn stats<'py>(
        &self,
        py: Python<'py>,
        col: Option<usize>,
        group: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let col = self.account_index(col, group)?;
        let table = py.detach(|| -> PyResult<_> {
            let pf = self.core()?;
            let pf = if self.cash_sharing {
                pf
            } else {
                let benchmark = self.benchmark.get_or_init(|| pf.buy_and_hold());
                pf.with_benchmark_ref(benchmark).map_err(to_py_err)?
            };
            pf.stats(col, &Select::all()).map_err(to_py_err)
        })?;
        let out = PyDict::new(py);
        for (title, value) in table {
            match value {
                Value::Count(n) => out.set_item(title, n)?,
                Value::Float(v) => out.set_item(title, v)?,
                // Distinct from NaN on purpose: NaN says the metric was
                // computed and had no answer, None says nobody supplied the
                // inputs for it.
                Value::Missing => out.set_item(title, py.None())?,
            }
        }
        Ok(out)
    }

    /// The common per-account metrics, computed from one rebuild.
    ///
    /// Returned together rather than one at a time because each individual
    /// accessor rebuilds the chain; asking for six of them separately does six
    /// times the work for the same answer.
    fn metrics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let (columns, counts) = py.detach(|| -> PyResult<_> {
            let pf = self.core()?;
            let columns = [
                ("total_return", pf.total_return()),
                ("annualized_return", pf.annualized_return()),
                ("max_drawdown", pf.max_drawdown()),
                ("sharpe_ratio", pf.sharpe_ratio()),
                ("sortino_ratio", pf.sortino_ratio()),
                ("calmar_ratio", pf.calmar_ratio()),
            ];
            // An integer array, matching `trade_count()`. A float count would
            // round-trip fine and then read as a measurement rather than a tally.
            let counts: Vec<i64> = pf
                .trade_counts()
                .map_err(to_py_err)?
                .into_iter()
                .map(|n| n as i64)
                .collect();
            Ok((columns, counts))
        })?;
        let out = PyDict::new(py);
        for (name, values) in columns {
            out.set_item(name, values.into_pyarray(py))?;
        }
        out.set_item("trade_count", counts.into_pyarray(py))?;
        Ok(out)
    }

    /// Sharpe ratio per account.
    ///
    /// A column that never traded has a flat curve and answers `inf`, which
    /// sorts as the best column in a sweep. [`Self::traded_only`] is how a
    /// ranking says what it is ranking.
    fn sharpe_ratio<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<f64>>> {
        Ok(py
            .detach(|| self.core().map(|pf| pf.sharpe_ratio()))?
            .into_pyarray(py))
    }

    /// Total return per account.
    fn total_return<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<f64>>> {
        Ok(py
            .detach(|| self.core().map(|pf| pf.total_return()))?
            .into_pyarray(py))
    }

    /// Worst drawdown per column, as a negative fraction.
    fn max_drawdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<f64>>> {
        Ok(py
            .detach(|| self.core().map(|pf| pf.max_drawdown()))?
            .into_pyarray(py))
    }

    /// Number of trades per account.
    fn trade_count<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<i64>>> {
        let counts: Vec<i64> = py
            .detach(|| self.core()?.trade_counts().map_err(to_py_err))?
            .into_iter()
            .map(|n| n as i64)
            .collect();
        Ok(counts.into_pyarray(py))
    }

    /// A per-column metric with the untraded columns masked to `NaN`.
    ///
    /// The trap this exists for: a column whose signals never fired has zero
    /// return over zero volatility, so its Sharpe is `inf` and it tops a naive
    /// ranking. `NaN` is skipped by every reduction, so masking is how a caller
    /// says "rank the columns that actually did something".
    fn traded_only<'py>(
        &self,
        py: Python<'py>,
        values: numpy::PyReadonlyArray1<'_, f64>,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let slice = values
            .as_slice()
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let masked = py.detach(|| self.core()?.traded_only(slice).map_err(to_py_err))?;
        Ok(masked.into_pyarray(py))
    }

    /// The value curve of an account. Shared runs require group=.
    #[pyo3(signature = (col = None, *, group = None))]
    fn value<'py>(
        &self,
        py: Python<'py>,
        col: Option<usize>,
        group: Option<usize>,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let index = self.account_index(col, group)?;
        Ok(py
            .detach(|| self.core().map(|pf| pf.value().column(index).to_vec()))?
            .into_pyarray(py))
    }

    /// Per-bar account returns.
    #[pyo3(signature = (col = None, *, group = None))]
    fn returns<'py>(
        &self,
        py: Python<'py>,
        col: Option<usize>,
        group: Option<usize>,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let index = self.account_index(col, group)?;
        Ok(py
            .detach(|| self.core().map(|pf| pf.returns().column(index).to_vec()))?
            .into_pyarray(py))
    }

    /// Cash held by an account at the end of each bar.
    #[pyo3(signature = (col = None, *, group = None))]
    fn cash<'py>(
        &self,
        py: Python<'py>,
        col: Option<usize>,
        group: Option<usize>,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let index = self.account_index(col, group)?;
        Ok(py
            .detach(|| self.core().map(|pf| pf.cash().column(index).to_vec()))?
            .into_pyarray(py))
    }

    /// Position quantity of one symbol column.
    fn assets<'py>(&self, py: Python<'py>, col: usize) -> PyResult<Bound<'py, PyArray1<f64>>> {
        self.column_of(py, col, |pf, c| pf.assets().column(c).to_vec())
    }

    /// The fills, as a dict of NumPy arrays.
    ///
    /// Struct-of-arrays, as the record layer stores them, so a caller can build
    /// a DataFrame from it without a transpose.
    fn orders<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("col", self.orders.col.clone().into_pyarray(py))?;
        out.set_item("idx", self.orders.idx.clone().into_pyarray(py))?;
        out.set_item("size", self.orders.size.clone().into_pyarray(py))?;
        out.set_item("price", self.orders.price.clone().into_pyarray(py))?;
        out.set_item("fees", self.orders.fees.clone().into_pyarray(py))?;
        let side: Vec<i64> = self.orders.side.iter().map(|s| i64::from(*s)).collect();
        out.set_item("side", side.into_pyarray(py))?;
        Ok(out)
    }

    /// Number of log rows, which is zero unless the run asked for them.
    #[getter]
    fn n_logs(&self) -> usize {
        self.logs.len()
    }

    /// The logs, as a dict of NumPy arrays — the 38-field schema.
    ///
    /// One row per order **considered**, filled or not, which is the difference
    /// from [`Self::orders`] and the whole reason to ask for logs. Without them
    /// a bar that did not trade leaves no evidence at all, and "no order record"
    /// is consistent with a dozen causes: the size resolved to zero, the cash
    /// ran out, the granularity floored it away, the direction forbade it, the
    /// price was `NaN`.
    ///
    /// `order_id` joins a row to the fill it produced, or is `-1` when nothing
    /// filled. `id` is the row's own index.
    ///
    /// # Errors
    ///
    /// Only if building an array fails.
    fn logs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let l = &self.logs;
        let out = PyDict::new(py);
        let put_i = |name: &str, v: Vec<i64>| -> PyResult<()> {
            out.set_item(name, v.into_pyarray(py))?;
            Ok(())
        };
        put_i("id", l.ids())?;
        put_i("group", l.group.clone())?;
        put_i("col", l.col.clone())?;
        put_i("idx", l.idx.clone())?;
        put_i("req_size_type", l.req_size_type.clone())?;
        put_i("req_direction", l.req_direction.clone())?;
        put_i("res_side", l.res_side.clone())?;
        put_i("res_status", l.res_status.clone())?;
        put_i("res_status_info", l.res_status_info.clone())?;
        put_i("order_id", l.order_id.clone())?;

        let put_f = |name: &str, v: Vec<f64>| -> PyResult<()> {
            out.set_item(name, v.into_pyarray(py))?;
            Ok(())
        };
        put_f("cash", l.cash.clone())?;
        put_f("position", l.position.clone())?;
        put_f("debt", l.debt.clone())?;
        put_f("free_cash", l.free_cash.clone())?;
        put_f("val_price", l.val_price.clone())?;
        put_f("value", l.value.clone())?;
        put_f("req_size", l.req_size.clone())?;
        put_f("req_price", l.req_price.clone())?;
        put_f("req_fees", l.req_fees.clone())?;
        put_f("req_fixed_fees", l.req_fixed_fees.clone())?;
        put_f("req_slippage", l.req_slippage.clone())?;
        put_f("req_min_size", l.req_min_size.clone())?;
        put_f("req_max_size", l.req_max_size.clone())?;
        put_f("req_size_granularity", l.req_size_granularity.clone())?;
        put_f("req_reject_prob", l.req_reject_prob.clone())?;
        put_f("new_cash", l.new_cash.clone())?;
        put_f("new_position", l.new_position.clone())?;
        put_f("new_debt", l.new_debt.clone())?;
        put_f("new_free_cash", l.new_free_cash.clone())?;
        put_f("new_val_price", l.new_val_price.clone())?;
        put_f("new_value", l.new_value.clone())?;
        put_f("res_size", l.res_size.clone())?;
        put_f("res_price", l.res_price.clone())?;
        put_f("res_fees", l.res_fees.clone())?;

        out.set_item("req_lock_cash", l.req_lock_cash.clone())?;
        out.set_item("req_allow_partial", l.req_allow_partial.clone())?;
        out.set_item("req_raise_reject", l.req_raise_reject.clone())?;
        out.set_item("req_log", l.req_log.clone())?;
        Ok(out)
    }

    /// Why each logged order ended the way it did, in words.
    ///
    /// `"filled"` for the ones that did, and the rejection's own reason for the
    /// ones that did not. The fields are enough to work it out; this is the
    /// answer without the lookup table.
    fn log_reasons(&self) -> Vec<&'static str> {
        self.logs.reasons()
    }

    /// How often each outcome occurred in one column, commonest first.
    ///
    /// The first diagnostic to reach for on a column that traded far less than
    /// expected: there is almost always one reason dominating.
    ///
    /// # Errors
    ///
    /// A Python `ValueError` if `col` is outside the run's shape.
    #[pyo3(signature = (col = 0))]
    fn reason_counts<'py>(&self, py: Python<'py>, col: usize) -> PyResult<Bound<'py, PyDict>> {
        if col >= self.n_cols {
            return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                "column {col} is outside 0..{}",
                self.n_cols
            )));
        }
        let tally = self
            .logs
            .reason_counts(col, self.n_cols)
            .map_err(to_py_err)?;
        let out = PyDict::new(py);
        for (reason, count) in tally {
            out.set_item(reason, count)?;
        }
        Ok(out)
    }

    /// The trades, in whichever view this portfolio is scored with.
    fn trades<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let pf = self.core()?;
        // Derived (and cached) without the GIL; the arrays are built with it.
        let t = py.detach(|| pf.trades().map_err(to_py_err))?;
        let out = PyDict::new(py);
        out.set_item("col", t.col.clone().into_pyarray(py))?;
        out.set_item("size", t.size.clone().into_pyarray(py))?;
        out.set_item("entry_idx", t.entry_idx.clone().into_pyarray(py))?;
        out.set_item("entry_price", t.entry_price.clone().into_pyarray(py))?;
        out.set_item("entry_fees", t.entry_fees.clone().into_pyarray(py))?;
        out.set_item("exit_idx", t.exit_idx.clone().into_pyarray(py))?;
        out.set_item("exit_price", t.exit_price.clone().into_pyarray(py))?;
        out.set_item("exit_fees", t.exit_fees.clone().into_pyarray(py))?;
        out.set_item("pnl", t.pnl.clone().into_pyarray(py))?;
        out.set_item("ret", t.ret.clone().into_pyarray(py))?;
        // Derived rather than stored, and not `exit_idx - entry_idx`: a closed
        // trade is measured exclusively and an open one inclusively. See
        // `docs/ARCHITECTURE.md §9.6`.
        out.set_item("duration", t.duration().into_pyarray(py))?;
        let direction: Vec<i64> = t.direction.iter().map(|d| i64::from(*d)).collect();
        out.set_item("direction", direction.into_pyarray(py))?;
        let status: Vec<i64> = t.status.iter().map(|s| i64::from(*s)).collect();
        out.set_item("status", status.into_pyarray(py))?;
        out.set_item("parent_id", t.parent_id.clone().into_pyarray(py))?;
        Ok(out)
    }

    /// Drops every derived series this portfolio has cached.
    ///
    /// The value curve, returns, trades and drawdowns are kept between calls
    /// so that reading six metrics costs one derivation, not six. Each is a
    /// `bars × accounts` matrix; on a large sweep that is memory worth giving
    /// back once the numbers are in hand. The next call derives again.
    fn clear_cache(&mut self) {
        self.cache.clear();
        self.benchmark = OnceLock::new();
    }

    fn __repr__(&self) -> String {
        format!(
            "Portfolio({} bars x {} columns, {} fills)",
            self.n_rows,
            self.n_cols,
            self.orders.len()
        )
    }
}

impl PyPortfolio {
    /// One column of a derived series, checking the column exists first.
    fn account_index(&self, col: Option<usize>, group: Option<usize>) -> PyResult<usize> {
        if col.is_some() && group.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "specify col or group, not both",
            ));
        }
        if self.cash_sharing && col.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "shared-account series require group=, not col",
            ));
        }
        let index = group.or(col).unwrap_or(0);
        if index >= self.groups.len() {
            return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                "account {index} is outside 0..{}",
                self.groups.len()
            )));
        }
        Ok(index)
    }

    fn column_of<'py>(
        &self,
        py: Python<'py>,
        col: usize,
        pick: impl Fn(&CorePortfolio<'_>, usize) -> Vec<f64> + Send + Sync,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        if col >= self.n_cols {
            return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                "column {col} is outside 0..{}",
                self.n_cols
            )));
        }
        Ok(py
            .detach(|| self.core().map(|pf| pick(&pf, col)))?
            .into_pyarray(py))
    }
}

/// Resolves group capital once, expanding only for the simulator's per-column API.
fn account_inputs(
    shared: bool,
    lens: Option<Vec<usize>>,
    cash: &Option<Borrowed<'_>>,
    n_rows: usize,
    n_cols: usize,
    default: FlexArray<'_, f64>,
) -> PyResult<(Groups, Vec<f64>)> {
    let bad = pyo3::exceptions::PyValueError::new_err;
    if !shared && lens.is_some() {
        return Err(bad("group_lens requires cash_sharing=True"));
    }
    let groups = if shared {
        Groups::from_lens(&lens.unwrap_or_else(|| vec![n_cols]), n_cols).map_err(to_py_err)?
    } else {
        Groups::ungrouped(n_cols)
    };
    if shared && matches!(cash, Some(Borrowed::Slice(_))) {
        return Err(bad(
            "shared init_cash must be a scalar or one value per group",
        ));
    }
    let balances = flex_or_f64(
        cash,
        if shared { 1 } else { n_rows },
        groups.len(),
        OneDim::PerColumn,
        default,
    )?;
    let expanded = (0..n_cols)
        .map(|col| balances.get(0, groups.of_column(col)))
        .collect();
    Ok((groups, expanded))
}

/// Runs a signal-driven backtest.
///
/// `close`, `entries` and `exits` are `(n_rows, n_cols)` arrays and set the
/// shape. Everything else takes a scalar, a 1-D array or a full matrix, the
/// same flexibility the engine has internally.
///
/// # Errors
///
/// A Python `ValueError` for a shape or layout that cannot be read, or for an
/// invariant the execution kernel refuses — with the engine's own message,
/// which names the row and column.
#[pyfunction]
#[pyo3(signature = (
    close,
    entries,
    exits,
    *,
    open = None,
    high = None,
    low = None,
    init_cash = None,
    size = None,
    size_type = "amount",
    price = None,
    direction = "longonly",
    fees = None,
    fixed_fees = None,
    slippage = None,
    min_size = None,
    max_size = None,
    size_granularity = None,
    lock_cash = None,
    allow_partial = None,
    raise_reject = None,
    log = None,
    accumulate = "disabled",
    sl_stop = None,
    sl_trail = None,
    tp_stop = None,
    val_price = None,
    ffill_val_price = true,
    update_value = false,
    ann_factor = 365.0,
    risk_free = 0.0,
    trades = "exit",
    incl_open = false,
    threads = 1,
    cash_sharing = false,
    group_lens = None,
    call_seq = "default",
    column_map = None,
))]
#[allow(
    clippy::too_many_arguments,
    reason = "one argument per simulation parameter, which is the API"
)]
fn from_signals(
    py: Python<'_>,
    close: &Bound<'_, PyAny>,
    entries: &Bound<'_, PyAny>,
    exits: &Bound<'_, PyAny>,
    open: Option<&Bound<'_, PyAny>>,
    high: Option<&Bound<'_, PyAny>>,
    low: Option<&Bound<'_, PyAny>>,
    init_cash: Option<&Bound<'_, PyAny>>,
    size: Option<&Bound<'_, PyAny>>,
    size_type: &str,
    price: Option<&Bound<'_, PyAny>>,
    direction: &str,
    fees: Option<&Bound<'_, PyAny>>,
    fixed_fees: Option<&Bound<'_, PyAny>>,
    slippage: Option<&Bound<'_, PyAny>>,
    min_size: Option<&Bound<'_, PyAny>>,
    max_size: Option<&Bound<'_, PyAny>>,
    size_granularity: Option<&Bound<'_, PyAny>>,
    lock_cash: Option<&Bound<'_, PyAny>>,
    allow_partial: Option<&Bound<'_, PyAny>>,
    raise_reject: Option<&Bound<'_, PyAny>>,
    log: Option<&Bound<'_, PyAny>>,
    accumulate: &str,
    sl_stop: Option<&Bound<'_, PyAny>>,
    sl_trail: Option<&Bound<'_, PyAny>>,
    tp_stop: Option<&Bound<'_, PyAny>>,
    val_price: Option<&Bound<'_, PyAny>>,
    ffill_val_price: bool,
    update_value: bool,
    ann_factor: f64,
    risk_free: f64,
    trades: &str,
    incl_open: bool,
    threads: usize,
    cash_sharing: bool,
    group_lens: Option<Vec<usize>>,
    call_seq: &str,
    column_map: Option<Vec<usize>>,
) -> PyResult<PyPortfolio> {
    // `close` first, so every other parameter can be checked against a known
    // shape rather than one inferred from whichever array arrived first.
    let close_b = borrow_f64(close, "close")?;
    let Borrowed::Slice(close_arr) = &close_b else {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "close must be a 2-D float64 array; it is what sets the run's shape",
        ));
    };
    let (close_flat, n_rows, source_cols) = columns_of(close_arr)?;
    let close_view = market_array(close_flat, n_rows, source_cols, column_map.as_deref())?;
    let n_cols = column_map.as_ref().map_or(source_cols, Vec::len);

    // A stop that can never fire is worse than a rejected argument: the run
    // succeeds and reports a strategy that never stopped out. The stop level is
    // tested against the bar's open, low and high, all of which default to NaN,
    // so every comparison is false and the stop is silently inert.
    if (sl_stop.is_some() || tp_stop.is_some()) && (high.is_none() || low.is_none()) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "sl_stop or tp_stop was given without high and low. A stop level is tested \
             against the bar's range, and with no range no stop can ever fire — the run \
             would report a stopless strategy. Pass high and low (open too, for gaps)",
        ));
    }

    let entries_b = borrow_bool(entries, "entries")?;
    let exits_b = borrow_bool(exits, "exits")?;
    let open_b = borrow_opt_f64(open, "open")?;
    let high_b = borrow_opt_f64(high, "high")?;
    let low_b = borrow_opt_f64(low, "low")?;
    let init_cash_b = borrow_opt_f64(init_cash, "init_cash")?;
    let val_price_b = borrow_opt_f64(val_price, "val_price")?;
    let sl_stop_b = borrow_opt_f64(sl_stop, "sl_stop")?;
    let tp_stop_b = borrow_opt_f64(tp_stop, "tp_stop")?;
    let sl_trail_b = array::borrow_opt_bool(sl_trail, "sl_trail")?;
    let p = Shared::borrow(
        size,
        price,
        fees,
        fixed_fees,
        slippage,
        min_size,
        max_size,
        size_granularity,
        lock_cash,
        allow_partial,
        raise_reject,
        log,
    )?;

    let bad = pyo3::exceptions::PyValueError::new_err;
    let dir = parse::direction(direction).map_err(bad)?;
    let st = parse::size_type(size_type).map_err(bad)?;
    if !matches!(
        st,
        barq_model::SizeType::Amount | barq_model::SizeType::Value | barq_model::SizeType::Percent
    ) {
        return Err(bad(
            "Only Amount, Value and Percent are supported by signals; use from_orders for targets"
                .to_owned(),
        ));
    }
    let acc = parse::accumulate(accumulate).map_err(bad)?;
    let trades_type = parse::trades_type(trades).map_err(bad)?;

    // Every default comes from here rather than from this function's signature,
    // so the two cannot drift. See `array::borrow_opt_f64`.
    let d = FromSignals::default();
    let per_col = OneDim::PerColumn;
    let (groups, account_cash) = account_inputs(
        cash_sharing,
        group_lens,
        &init_cash_b,
        n_rows,
        n_cols,
        d.init_cash,
    )?;
    let call_seq = match call_seq {
        "default" => CallSeq::Default,
        "auto" => CallSeq::Auto,
        _ => return Err(bad("call_seq must be 'default' or 'auto'".to_owned())),
    };

    let args = FromSignals {
        groups: Some(groups.clone()),
        call_seq,
        val_price: flex_or_f64(&val_price_b, n_rows, n_cols, per_col, d.val_price)?,
        ffill_val_price,
        update_value,
        n_rows,
        n_cols,
        close: close_view,
        open: market_optional(&open_b, n_rows, source_cols, column_map.as_deref(), d.open)?,
        high: market_optional(&high_b, n_rows, source_cols, column_map.as_deref(), d.high)?,
        low: market_optional(&low_b, n_rows, source_cols, column_map.as_deref(), d.low)?,
        entries: flex_bool(&entries_b, n_rows, n_cols)?,
        exits: flex_bool(&exits_b, n_rows, n_cols)?,
        init_cash: FlexArray::per_column(&account_cash, n_cols).map_err(to_py_err)?,
        direction: FlexArray::Scalar(dir),
        size_type: FlexArray::Scalar(st),
        accumulate: FlexArray::Scalar(acc),
        size: flex_or_f64(&p.size, n_rows, n_cols, per_col, d.size)?,
        price: flex_or_f64(&p.price, n_rows, n_cols, per_col, d.price)?,
        fees: flex_or_f64(&p.fees, n_rows, n_cols, per_col, d.fees)?,
        fixed_fees: flex_or_f64(&p.fixed_fees, n_rows, n_cols, per_col, d.fixed_fees)?,
        slippage: flex_or_f64(&p.slippage, n_rows, n_cols, per_col, d.slippage)?,
        min_size: flex_or_f64(&p.min_size, n_rows, n_cols, per_col, d.min_size)?,
        max_size: flex_or_f64(&p.max_size, n_rows, n_cols, per_col, d.max_size)?,
        size_granularity: flex_or_f64(
            &p.size_granularity,
            n_rows,
            n_cols,
            per_col,
            d.size_granularity,
        )?,
        lock_cash: flex_or_bool(&p.lock_cash, n_rows, n_cols, d.lock_cash)?,
        allow_partial: flex_or_bool(&p.allow_partial, n_rows, n_cols, d.allow_partial)?,
        raise_reject: flex_or_bool(&p.raise_reject, n_rows, n_cols, d.raise_reject)?,
        log: flex_or_bool(&p.log, n_rows, n_cols, d.log)?,
        sl_stop: flex_or_f64(&sl_stop_b, n_rows, n_cols, per_col, d.sl_stop)?,
        sl_trail: flex_or_bool(&sl_trail_b, n_rows, n_cols, d.sl_trail)?,
        tp_stop: flex_or_f64(&tp_stop_b, n_rows, n_cols, per_col, d.tp_stop)?,
        ..FromSignals::default()
    };

    // Two entry points rather than always taking the logged one: a run that is
    // not investigating anything should not allocate a journal. The GIL is
    // released for the run: every input is a borrowed slice the guards above
    // keep alive, and nothing in the loop touches Python.
    let (orders, logs) = if p.wants_logs() {
        let journal = py
            .detach(|| {
                if threads > 1 {
                    barq_sim::simulate_from_signals_logged_parallel(&args, threads)
                } else {
                    barq_sim::simulate_from_signals_logged(&args)
                }
            })
            .map_err(to_py_err)?;
        (journal.orders, journal.logs)
    } else {
        let orders = py
            .detach(|| {
                if threads > 1 {
                    barq_sim::simulate_from_signals_parallel(&args, threads)
                } else {
                    barq_sim::simulate_from_signals(&args)
                }
            })
            .map_err(to_py_err)?;
        (orders, LogRecords::new())
    };

    // The starting cash is materialized per column so the portfolio can be
    // rebuilt without keeping Python's array alive.
    let init_cash_owned: Vec<f64> = (0..n_cols).map(|c| args.init_cash.get(0, c)).collect();

    Ok(PyPortfolio::assemble(
        orders,
        logs,
        close_flat.to_vec(),
        source_cols,
        column_map,
        init_cash_owned,
        groups,
        cash_sharing,
        n_rows,
        n_cols,
        Scoring {
            ann_factor,
            risk_free,
            trades_type,
            incl_open,
        },
    ))
}

/// Runs a backtest from one order request per element.
///
/// The other shape of a vectorized run, and the more general one: instead of
/// signals resolved into sizes, `size` *is* the request, element by element. A
/// rebalance is a matrix of target weights; a schedule is a matrix that is zero
/// except on the bars that trade.
///
/// All six size types work here, unlike the signal path — nothing has to be
/// reconciled with a direction, so a target is unambiguous.
///
/// # Errors
///
/// A Python `ValueError` for a shape or layout that cannot be read, or for an
/// invariant the execution kernel refuses.
#[pyfunction]
#[pyo3(signature = (
    close,
    *,
    size = None,
    size_type = "amount",
    price = None,
    direction = "both",
    init_cash = None,
    fees = None,
    fixed_fees = None,
    slippage = None,
    min_size = None,
    max_size = None,
    size_granularity = None,
    lock_cash = None,
    allow_partial = None,
    raise_reject = None,
    log = None,
    val_price = None,
    ffill_val_price = true,
    update_value = false,
    ann_factor = 365.0,
    risk_free = 0.0,
    trades = "exit",
    incl_open = false,
    threads = 1,
    cash_sharing = false,
    group_lens = None,
    call_seq = "default",
    column_map = None,
))]
#[allow(
    clippy::too_many_arguments,
    reason = "one argument per simulation parameter, which is the API"
)]
fn from_orders(
    py: Python<'_>,
    close: &Bound<'_, PyAny>,
    size: Option<&Bound<'_, PyAny>>,
    size_type: &str,
    price: Option<&Bound<'_, PyAny>>,
    direction: &str,
    init_cash: Option<&Bound<'_, PyAny>>,
    fees: Option<&Bound<'_, PyAny>>,
    fixed_fees: Option<&Bound<'_, PyAny>>,
    slippage: Option<&Bound<'_, PyAny>>,
    min_size: Option<&Bound<'_, PyAny>>,
    max_size: Option<&Bound<'_, PyAny>>,
    size_granularity: Option<&Bound<'_, PyAny>>,
    lock_cash: Option<&Bound<'_, PyAny>>,
    allow_partial: Option<&Bound<'_, PyAny>>,
    raise_reject: Option<&Bound<'_, PyAny>>,
    log: Option<&Bound<'_, PyAny>>,
    val_price: Option<&Bound<'_, PyAny>>,
    ffill_val_price: bool,
    update_value: bool,
    ann_factor: f64,
    risk_free: f64,
    trades: &str,
    incl_open: bool,
    threads: usize,
    cash_sharing: bool,
    group_lens: Option<Vec<usize>>,
    call_seq: &str,
    column_map: Option<Vec<usize>>,
) -> PyResult<PyPortfolio> {
    let close_b = borrow_f64(close, "close")?;
    let Borrowed::Slice(close_arr) = &close_b else {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "close must be a 2-D float64 array; it is what sets the run's shape",
        ));
    };
    let (close_flat, n_rows, source_cols) = columns_of(close_arr)?;
    let close_view = market_array(close_flat, n_rows, source_cols, column_map.as_deref())?;
    let n_cols = column_map.as_ref().map_or(source_cols, Vec::len);

    let init_cash_b = borrow_opt_f64(init_cash, "init_cash")?;
    let val_price_b = borrow_opt_f64(val_price, "val_price")?;
    let p = Shared::borrow(
        size,
        price,
        fees,
        fixed_fees,
        slippage,
        min_size,
        max_size,
        size_granularity,
        lock_cash,
        allow_partial,
        raise_reject,
        log,
    )?;

    let bad = pyo3::exceptions::PyValueError::new_err;
    let dir = parse::direction(direction).map_err(bad)?;
    let st = parse::size_type(size_type).map_err(bad)?;
    let trades_type = parse::trades_type(trades).map_err(bad)?;

    let d = FromOrders::default();
    let per_col = OneDim::PerColumn;
    let (groups, account_cash) = account_inputs(
        cash_sharing,
        group_lens,
        &init_cash_b,
        n_rows,
        n_cols,
        d.init_cash,
    )?;
    let call_seq = match call_seq {
        "default" => CallSeq::Default,
        "auto" => CallSeq::Auto,
        _ => return Err(bad("call_seq must be 'default' or 'auto'".to_owned())),
    };

    let args = FromOrders {
        n_rows,
        n_cols,
        groups: groups.clone(),
        call_seq,
        close: close_view,
        init_cash: FlexArray::per_column(&account_cash, n_cols).map_err(to_py_err)?,
        val_price: flex_or_f64(&val_price_b, n_rows, n_cols, per_col, d.val_price)?,
        direction: FlexArray::Scalar(dir),
        size_type: FlexArray::Scalar(st),
        size: flex_or_f64(&p.size, n_rows, n_cols, per_col, d.size)?,
        price: flex_or_f64(&p.price, n_rows, n_cols, per_col, d.price)?,
        fees: flex_or_f64(&p.fees, n_rows, n_cols, per_col, d.fees)?,
        fixed_fees: flex_or_f64(&p.fixed_fees, n_rows, n_cols, per_col, d.fixed_fees)?,
        slippage: flex_or_f64(&p.slippage, n_rows, n_cols, per_col, d.slippage)?,
        min_size: flex_or_f64(&p.min_size, n_rows, n_cols, per_col, d.min_size)?,
        max_size: flex_or_f64(&p.max_size, n_rows, n_cols, per_col, d.max_size)?,
        size_granularity: flex_or_f64(
            &p.size_granularity,
            n_rows,
            n_cols,
            per_col,
            d.size_granularity,
        )?,
        lock_cash: flex_or_bool(&p.lock_cash, n_rows, n_cols, d.lock_cash)?,
        allow_partial: flex_or_bool(&p.allow_partial, n_rows, n_cols, d.allow_partial)?,
        raise_reject: flex_or_bool(&p.raise_reject, n_rows, n_cols, d.raise_reject)?,
        log: flex_or_bool(&p.log, n_rows, n_cols, d.log)?,
        ffill_val_price,
        update_value,
    };

    let (orders, logs) = if p.wants_logs() {
        let journal = py
            .detach(|| {
                if threads > 1 {
                    barq_sim::simulate_from_orders_logged_parallel(&args, threads)
                } else {
                    barq_sim::simulate_from_orders_logged(&args)
                }
            })
            .map_err(to_py_err)?;
        (journal.orders, journal.logs)
    } else {
        let orders = py
            .detach(|| {
                if threads > 1 {
                    barq_sim::simulate_from_orders_parallel(&args, threads)
                } else {
                    barq_sim::simulate_from_orders(&args)
                }
            })
            .map_err(to_py_err)?;
        (orders, LogRecords::new())
    };

    let init_cash_owned: Vec<f64> = (0..n_cols).map(|c| args.init_cash.get(0, c)).collect();

    Ok(PyPortfolio::assemble(
        orders,
        logs,
        close_flat.to_vec(),
        source_cols,
        column_map,
        init_cash_owned,
        groups,
        cash_sharing,
        n_rows,
        n_cols,
        Scoring {
            ann_factor,
            risk_free,
            trades_type,
            incl_open,
        },
    ))
}

fn market_array<'a>(
    data: &'a [f64],
    rows: usize,
    cols: usize,
    map: Option<&'a [usize]>,
) -> PyResult<FlexArray<'a, f64>> {
    match map {
        Some(map) => FlexArray::mapped(data, rows, cols, map),
        None => FlexArray::full(data, rows, cols),
    }
    .map_err(to_py_err)
}

fn market_optional<'a>(
    value: &'a Option<Borrowed<'a>>,
    rows: usize,
    cols: usize,
    map: Option<&'a [usize]>,
    default: FlexArray<'a, f64>,
) -> PyResult<FlexArray<'a, f64>> {
    match value {
        Some(Borrowed::Slice(a)) => {
            let (data, r, c) = columns_of(a)?;
            if (r, c) != (rows, cols) {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "OHLC arrays must have the source close shape",
                ));
            }
            market_array(data, rows, cols, map)
        }
        Some(Borrowed::Row(_)) if map.is_some() => Err(pyo3::exceptions::PyValueError::new_err(
            "mapped OHLC must be a source-shaped matrix or scalar",
        )),
        _ => flex_or_f64(value, rows, cols, OneDim::PerColumn, default),
    }
}

/// The extension module.
#[pymodule]
fn _barq(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<data::PyInstrumentId>()?;
    m.add_class::<data::PyBarType>()?;
    m.add_class::<data::PyContext>()?;
    m.add_class::<data::PyDataset>()?;
    m.add_function(wrap_pyfunction!(data::catalog_files, m)?)?;
    m.add_function(wrap_pyfunction!(data::load_data, m)?)?;
    m.add_class::<PyPortfolio>()?;
    m.add_class::<PyFactorSet>()?;
    m.add_function(wrap_pyfunction!(from_signals, m)?)?;
    m.add_function(wrap_pyfunction!(from_orders, m)?)?;
    m.add_function(wrap_pyfunction!(expr::alpha158, m)?)?;
    m.add_function(wrap_pyfunction!(expr::alpha360, m)?)?;
    m.add_function(wrap_pyfunction!(expr::crossed_above, m)?)?;
    m.add(
        "__doc__",
        "Rust engine for barq. Use the `barq` package, not this.",
    )?;
    Ok(())
}
