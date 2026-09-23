//! Vectorized simulation loops, and where parallelism happens.
//!
//! # Account groups are independent
//!
//! Order and signal loops share capital inside each group. Groups own their
//! state and execute in parallel; symbols within a group execute sequentially.
//! With no sharing, each column is its own account.
//!
//! This is what the reference event-driven engine cannot do: it holds its
//! state in `Rc<RefCell<..>>` behind a process-global message bus, so its
//! sweep is a sequential `for` loop and users are told to run multiple
//! processes. Keeping every engine instance `Send`, with no global state, is
//! a hard constraint on this codebase — see `docs/ARCHITECTURE.md §11.5`.
//!
//! [`from_orders::simulate_from_orders_parallel`] returns records equal to the
//! serial version for the same input, whatever the thread count. That is not
//! incidental: each column fills its own buffer and record ids are assigned
//! after merging in column order, so nothing depends on how threads
//! interleave.
//!
//! # Why this loop is not SIMD
//!
//! Along the time axis there is a hard loop-carried dependency:
//! `state[i] = f(state[i-1], bar[i])`. Across columns, SIMD is possible in
//! principle and a bad trade in practice — the execution kernel is a deep
//! branch tree with fourteen data-dependent early exits, and lanes would
//! diverge on nearly every bar. SIMD belongs in the factor and analysis
//! layers, where the operations have no such dependency. The two techniques
//! stack; they are not alternatives.
//!
//! # Status
//!
//! Phase 1, stage 3, done: `from_orders`, `from_signals` with its seven-step
//! signal pipeline and stop engine, cash sharing with the automatic call
//! order, and a logged variant of each loop.
//!
//! Cash sharing arrived after the rest, and moved the unit of parallel work
//! from the column to the group. With the default grouping the two are the
//! same thing, so nothing changed for independent symbols; with one shared
//! portfolio there is a single unit of work, which is inherent — a shared cash
//! balance is a sequential dependency across the whole group.

pub mod from_orders;
pub mod from_signals;
pub mod groups;
pub mod signals;

/// The smallest fill the reference accepts by default.
///
/// Not zero, which is what one would write. The reference's own default is
/// `1e-8`, and the difference is observable: a `5e-9` order is **rejected**
/// there and would fill here, so a caller who omits `min_size` would get a
/// different engine.
///
/// It is a dust guard rather than a tolerance — the float tolerance layer is
/// separate and finer. What it buys is that a target-size cascade landing a
/// hair away from its target does not emit a fill nobody can act on, and that
/// the rejection says `SizeTooSmall` rather than leaving a record of a
/// nanogram trade.
///
/// Found by exposing log records: `req_min_size` is one of the 37 fields, so
/// comparing them field by field against the reference failed on the first
/// run. No fixture had caught it in either loop, because a randomly generated
/// size is never dust.
pub const DEFAULT_MIN_SIZE: f64 = 1e-8;

pub use from_orders::{
    FromOrders, simulate_from_orders, simulate_from_orders_logged,
    simulate_from_orders_logged_parallel, simulate_from_orders_parallel,
};
pub use from_signals::{
    FromSignals, simulate_from_signals, simulate_from_signals_logged,
    simulate_from_signals_logged_parallel, simulate_from_signals_parallel,
};
