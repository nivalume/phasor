//! `barq` — a parallel, factor-driven vectorized backtesting engine.
//!
//! This crate is the façade: it re-exports the workspace's crates and
//! provides a [`prelude`]. Read `docs/ARCHITECTURE.md` first; the short
//! version is:
//!
//! - Factor expressions produce a `(time x symbol)` panel.
//! - Signals derived from it fan out across a column axis where parameters
//!   and symbols are the same dimension.
//! - Account groups are independent; symbols within a cash-sharing group
//!   execute sequentially, and parameter groups run in parallel.
//! - Both the vectorized tier and the phase-2 event-driven tier emit the
//!   same records, so one analysis layer serves both and their results can
//!   be compared directly.
//!
//! # A whole sweep, end to end
//!
//! `examples/sweep.py` is the shortest honest answer to "what does using this
//! look like": prices in, a parameter grid, one parallel run, and an answer
//! attributed back to the parameter that produced it. It is also an
//! integration test, so it cannot rot.
//!
//! # Status
//!
//! Phase 1. The execution kernel, both simulation loops, the record and trade
//! layers, the analysis layer, the array layer and the indicators are
//! implemented and pinned against the reference. The factor DSL computes the
//! full Alpha158 and Alpha360 expression sets and feeds the existing signal simulator. See
//! `docs/ARCHITECTURE.md §17` for what is planned and known gaps.

pub use barq_analysis as analysis;
pub use barq_core as core;
#[cfg(feature = "data")]
pub use barq_data as data;
pub use barq_exec as exec;
pub use barq_expr as expr;
pub use barq_generic as generic;
pub use barq_indicators as indicators;
pub use barq_model as model;
pub use barq_records as records;
pub use barq_signals as signals;
pub use barq_sim as sim;

pub mod sweep;

/// The common imports.
pub mod prelude {
    pub use barq_core::columns::{Columns, Label, Level};
    pub use barq_core::flex::FlexArray;
    pub use barq_core::{Error, Result};
    pub use barq_exec::{ExecuteOrderState, Order, OrderResult, ProcessOrderState};
    pub use barq_model::enums::*;
    pub use barq_model::ids::{InstrumentId, Symbol, Venue};
    pub use barq_model::types::{Price, Quantity};

    pub use crate::sweep::SignalSweep;
    pub use barq_analysis::metrics::{DAILY, sharpe_ratio, total_return};
    pub use barq_analysis::stats::{Select, Tag, Value};
    pub use barq_analysis::{Portfolio, Reduce, Scoring, best_column, group_by};
    pub use barq_core::matrix::Matrix;
    #[cfg(feature = "polars")]
    pub use barq_expr::PolarsBackend;
    pub use barq_expr::{
        ComputeBackend, FactorInputs, FactorSet, FactorValues, NativeBackend, alpha158, alpha360,
    };
    pub use barq_indicators::{MaParams, combinations, ma};
    pub use barq_records::{OrderRecords, exit_trades};
    pub use barq_signals::{ExitWindow, FirstSignal, clean_enex, exit_after, partition_ranges};
    pub use barq_sim::groups::{CallSeq, Groups};
    pub use barq_sim::{FromSignals, simulate_from_signals, simulate_from_signals_parallel};
}
