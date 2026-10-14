//! `phasor` — a parallel, factor-driven vectorized backtesting engine.
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

pub use phasor_analysis as analysis;
pub use phasor_core as core;
#[cfg(feature = "data")]
pub use phasor_data as data;
pub use phasor_exec as exec;
pub use phasor_expr as expr;
pub use phasor_generic as generic;
pub use phasor_indicators as indicators;
pub use phasor_model as model;
pub use phasor_records as records;
pub use phasor_signals as signals;
pub use phasor_sim as sim;

pub mod sweep;

/// The common imports.
pub mod prelude {
    pub use phasor_core::columns::{Columns, Label, Level};
    pub use phasor_core::flex::FlexArray;
    pub use phasor_core::{Error, Result};
    pub use phasor_exec::{ExecuteOrderState, Order, OrderResult, ProcessOrderState};
    pub use phasor_model::enums::*;
    pub use phasor_model::ids::{InstrumentId, Symbol, Venue};
    pub use phasor_model::types::{Price, Quantity};

    pub use crate::sweep::SignalSweep;
    pub use phasor_analysis::metrics::{DAILY, sharpe_ratio, total_return};
    pub use phasor_analysis::stats::{Select, Tag, Value};
    pub use phasor_analysis::{Portfolio, Reduce, Scoring, best_column, group_by};
    pub use phasor_core::matrix::Matrix;
    #[cfg(feature = "polars")]
    pub use phasor_expr::PolarsBackend;
    pub use phasor_expr::{
        ComputeBackend, FactorInputs, FactorSet, FactorValues, NativeBackend, alpha158, alpha360,
    };
    pub use phasor_indicators::{MaParams, combinations, ma};
    pub use phasor_records::{OrderRecords, exit_trades};
    pub use phasor_signals::{ExitWindow, FirstSignal, clean_enex, exit_after, partition_ranges};
    pub use phasor_sim::groups::{CallSeq, Groups};
    pub use phasor_sim::{FromSignals, simulate_from_signals, simulate_from_signals_parallel};
}
