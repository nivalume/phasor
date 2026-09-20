//! Bar / OHLCV data.
//!
//! Phase 1 is bar-only by design (see `docs/01-architecture.md`). The
//! vectorized tier reads OHLCV as four parallel column-major `f64` arrays
//! rather than as a `Bar` struct per row — a struct-of-arrays layout, because
//! the stop engine touches `open`, `high` and `low` in separate passes and an
//! array-of-structs layout would pull the whole row into cache for each.
//!
//! The [`Bar`] struct exists for the phase-2 event-driven tier, where a
//! strategy's `on_bar` needs one bar at a time.

use crate::ids::InstrumentId;
use crate::types::{Price, Quantity};

/// Nanoseconds since the Unix epoch.
pub type UnixNanos = u64;

/// A single OHLCV bar.
///
/// Carries two timestamps: `ts_event` is when the bar closed at the venue,
/// `ts_init` is when it became available to us. Replay orders by `ts_init`,
/// because that is the only one that cannot leak the future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    /// Which instrument this bar belongs to.
    pub instrument_id: InstrumentId,
    /// Opening price.
    pub open: Price,
    /// Highest price.
    pub high: Price,
    /// Lowest price.
    pub low: Price,
    /// Closing price.
    pub close: Price,
    /// Traded volume.
    pub volume: Quantity,
    /// When the bar closed at the venue.
    pub ts_event: UnixNanos,
    /// When the bar became available to the engine.
    pub ts_init: UnixNanos,
}

/// Column-major OHLCV panel: the shape the vectorized tier actually consumes.
///
/// Each field is `n_rows * n_cols` elements, column-major, so one column's
/// full history is contiguous. See `docs/03-column-layout.md`.
#[derive(Debug, Clone, Default)]
pub struct OhlcvPanel {
    /// Opening prices.
    pub open: Vec<f64>,
    /// Highest prices.
    pub high: Vec<f64>,
    /// Lowest prices.
    pub low: Vec<f64>,
    /// Closing prices.
    pub close: Vec<f64>,
    /// Traded volumes.
    pub volume: Vec<f64>,
    /// Number of bars.
    pub n_rows: usize,
    /// Number of columns.
    pub n_cols: usize,
}
