//! Indicators, and the parameter sweep that turns them into a column axis.
//!
//! This is the crate that makes a parameter sweep one call. An indicator run
//! over three windows against four symbols does not return three results of
//! four columns — it returns **one result of twelve**, which the simulator
//! then treats as twelve independent columns and threads across cores without
//! knowing that eight of them share a symbol.
//!
//! ```text
//! close (time × symbol)  ──▶  ma(close, &[w10, w20, w30])
//!                             ──▶ ma (time × (3 × symbol)), plus labels
//!                             ──▶ signals ──▶ simulate ──▶ metrics
//!                                                          ──▶ group_by("window")
//! ```
//!
//! The labels are what make the far end of that pipeline mean anything: a
//! twelve-column Sharpe vector is a pile of numbers until
//! [`barq_core::columns::Columns`] can say which window and which symbol
//! produced each one.
//!
//! # What is here
//!
//! [`sweep`] builds the column axis. [`indicators`] holds the eight the
//! reference ships: moving average and standard deviation, Bollinger bands,
//! RSI, the stochastic oscillator, MACD, average true range and on-balance
//! volume. Each caches its expensive intermediates by the parameters that
//! determine them, so a wide grid does not recompute a shared moving average
//! once per row of it.
//!
//! # Verification
//!
//! Every indicator is pinned against the reference by
//! `tests/parity_indicators.rs`, including the degenerate readings that are
//! easy to special-case wrongly — an RSI of exactly 100 when nothing fell, a
//! stochastic with a flat range, a true range on the first bar.

pub mod indicators;
pub mod sweep;

pub use indicators::{
    BbParams, Indicator, MaParams, MacdParams, StochParams, atr, bbands, ma, macd, mstd, obv, rsi,
    stoch, true_range,
};
pub use sweep::{ParamLevel, column_of, combinations, sweep_columns};
