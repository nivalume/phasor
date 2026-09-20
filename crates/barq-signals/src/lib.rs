//! Generating, cleaning and ranking entry and exit signals.
//!
//! The engine *consumes* entries and exits. This crate is where they come
//! from — which until now was the caller's problem, and is the difference
//! between an engine that runs a backtest and one you can write a strategy
//! with.
//!
//! Four things, and they are four different jobs:
//!
//! - [`clean`] turns a raw condition array into an alternating sequence. A
//!   thresholded factor fires on every bar the condition holds, not on the
//!   bar it starts holding; feeding that to a simulation is a column of
//!   redundant orders.
//! - [`generate`] builds exits from entries, or both from scratch, through a
//!   choice function. That callback is the abstraction the reference's own
//!   Rust backend cannot expose — a Numba callback does not cross into
//!   native code, so its dispatcher refuses every callback-taking function.
//!   In Rust it monomorphizes away and costs nothing.
//! - [`rank`] says where a signal sits within its run, which is what "enter
//!   on the third consecutive signal" needs.
//! - [`ranges`] reads a signal array as spans rather than bars, so "how long
//!   does this rule stay on" has an answer in the same record shape the
//!   order and trade layers use.
//!
//! # Randomized signals are deliberately not here
//!
//! The reference generates random entries and exits for null-model
//! comparison, and it would be easy to add. It is left out because it could
//! not be verified the way everything else here is: the answer depends on a
//! random stream, and NumPy's stream is not reproducible from Rust. A
//! differential fixture would compare two different random draws and pass for
//! the wrong reason.
//!
//! What a null model actually needs is a *matched* one — the same number of
//! signals, in the same runs, reshuffled — and that is a design question
//! rather than a port. See `docs/08-verification.md`.

pub mod clean;
pub mod generate;
pub mod ranges;
pub mod rank;

pub use clean::{FirstSignal, clean_enex, clean_enex_1d};
pub use generate::{ExitWindow, exit_after, exit_on, generate_alternating, generate_exits};
pub use ranges::{
    RangeRecords, RangeStatus, between_partition_ranges, between_ranges, partition_ranges,
};
pub use rank::{
    Gaps, LeadingRun, normalized_average_index, nth_index, partition_rank, signal_rank,
};
