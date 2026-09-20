//! Record schemas: the contract between the two tiers.
//!
//! The vectorized tier and the phase-2 event-driven tier compute completely
//! differently and will never share a simulation loop. What they do share is
//! this: **the same records come out**. Unifying the output rather than the
//! computation is the whole architecture — it means the analysis layer is
//! written once and works on either tier's results, and it means a
//! vectorized sweep and an event-driven replay can be compared column by
//! column. See `docs/01-architecture.md`.
//!
//! # Layout
//!
//! Records are struct-of-arrays, not array-of-structs, so they hand off to
//! Arrow without a transpose and so that a query touching one field does not
//! pull the other six through cache.
//!
//! # Ordering and identity
//!
//! Records are emitted in column-major order — all of column 0's fills, then
//! all of column 1's — and `id` is the record's position in the merged array.
//! That is what makes a parallel simulation reproducible: each column fills
//! its own buffer with no shared counter, the buffers are concatenated in
//! column order, and ids are assigned afterwards. A shared atomic counter
//! would make ids depend on thread interleaving.
//!
//! # Status
//!
//! Phase 1. Order records, all three trade views, drawdown episodes, the
//! mapped-array reduction layer and log records are implemented.

pub mod drawdowns;
pub mod logs;
pub mod mapped;
mod trades;

pub use drawdowns::{DrawdownRecords, DrawdownStatus, find_drawdowns};
pub use logs::{Journal, LogEntry, LogRecords, LogSink};

/// Re-exported so a caller can turn a stored discriminant back into a reason.
pub use barq_model::StatusInfo as StatusInfoLookup;
pub use mapped::{ColMap, MappedArray};
pub use trades::{TradeRecords, entry_trades, exit_trades, positions};

use barq_model::OrderSide;

/// Where filled orders are written.
///
/// A trait rather than a concrete array so the phase-2 event-driven tier can
/// reuse the record layer without inheriting the vectorized tier's buffer
/// model. Retrofitting this later would mean touching every write site.
pub trait RecordSink {
    /// Records one fill.
    fn push_order(
        &mut self,
        row: usize,
        col: usize,
        size: f64,
        price: f64,
        fees: f64,
        side: OrderSide,
    );
}

/// A sink a parallel simulation can allocate per thread and merge afterwards.
///
/// The two things a loop needs from its output buffer besides writing to it:
/// make one, and put several back together in order. Both concrete sinks
/// already had them; naming them as a trait is what lets one simulation loop
/// serve both the plain and the logged variant instead of two copies of the
/// same threading code differing only in a type.
///
/// `Send` is part of the bound because the parallel path hands each thread its
/// own buffer. Nothing is shared, which is the whole design — see
/// `docs/02-parallelism.md`.
pub trait SinkBuffer: RecordSink + logs::LogSink + Send + Sized {
    /// An empty buffer with room for `capacity` fills.
    fn with_capacity(capacity: usize) -> Self;

    /// Merges buffers that are already in group order.
    ///
    /// Not necessarily a plain concatenation: a sink holding logs has to
    /// re-base their `order_id`s onto the merged fills. That is exactly why
    /// merging is the sink's business rather than the loop's.
    fn merge(parts: &[Self]) -> Self;
}

impl SinkBuffer for OrderRecords {
    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn merge(parts: &[Self]) -> Self {
        Self::concat(parts)
    }
}

impl SinkBuffer for logs::Journal {
    fn with_capacity(capacity: usize) -> Self {
        // The fills are bounded by the shape, so that half is preallocated.
        // How many logs there will be is not knowable here — it depends on a
        // per-order flag — and a run with logging on everywhere would want the
        // same bound again at five times the width. Since logging is opt-in and
        // its useful shapes are one column or one row, growing that half is
        // cheaper than reserving for the case nobody asks for.
        Self {
            orders: OrderRecords::with_capacity(capacity),
            logs: logs::LogRecords::new(),
        }
    }

    fn merge(parts: &[Self]) -> Self {
        Self::concat(parts)
    }
}

/// Filled orders, struct-of-arrays.
///
/// `id` is not stored per push: it is the record's index, assigned when
/// buffers are merged. See the module docs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrderRecords {
    /// Column the fill belongs to.
    pub col: Vec<i64>,
    /// Row (bar index) the fill happened on.
    pub idx: Vec<i64>,
    /// Filled size, always positive; direction is in `side`.
    pub size: Vec<f64>,
    /// Fill price, after slippage.
    pub price: Vec<f64>,
    /// Fees charged.
    pub fees: Vec<f64>,
    /// Buy or sell, as the enum's discriminant.
    pub side: Vec<i8>,
}

impl OrderRecords {
    /// An empty record set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            col: Vec::new(),
            idx: Vec::new(),
            size: Vec::new(),
            price: Vec::new(),
            fees: Vec::new(),
            side: Vec::new(),
        }
    }

    /// An empty record set with room for `capacity` fills.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            col: Vec::with_capacity(capacity),
            idx: Vec::with_capacity(capacity),
            size: Vec::with_capacity(capacity),
            price: Vec::with_capacity(capacity),
            fees: Vec::with_capacity(capacity),
            side: Vec::with_capacity(capacity),
        }
    }

    /// Number of fills.
    #[must_use]
    pub fn len(&self) -> usize {
        self.col.len()
    }

    /// Whether nothing filled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.col.is_empty()
    }

    /// The `id` of each record: its index, per the ordering contract.
    #[must_use]
    pub fn ids(&self) -> Vec<i64> {
        (0..self.len() as i64).collect()
    }

    /// Appends another record set, preserving order.
    ///
    /// Used to merge per-column buffers back into column-major order after a
    /// parallel run.
    pub fn extend(&mut self, other: &Self) {
        self.col.extend_from_slice(&other.col);
        self.idx.extend_from_slice(&other.idx);
        self.size.extend_from_slice(&other.size);
        self.price.extend_from_slice(&other.price);
        self.fees.extend_from_slice(&other.fees);
        self.side.extend_from_slice(&other.side);
    }

    /// Merges buffers that are already in column order.
    #[must_use]
    pub fn concat(parts: &[Self]) -> Self {
        let total = parts.iter().map(Self::len).sum();
        let mut out = Self::with_capacity(total);
        for part in parts {
            out.extend(part);
        }
        out
    }
}

impl RecordSink for OrderRecords {
    fn push_order(
        &mut self,
        row: usize,
        col: usize,
        size: f64,
        price: f64,
        fees: f64,
        side: OrderSide,
    ) {
        self.col.push(col as i64);
        self.idx.push(row as i64);
        self.size.push(size);
        self.price.push(price);
        self.fees.push(fees);
        self.side.push(side as i8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_appends_to_every_column() {
        let mut r = OrderRecords::new();
        assert!(r.is_empty());
        r.push_order(3, 1, 5.0, 10.0, 0.5, OrderSide::Buy);
        r.push_order(7, 1, 2.0, 11.0, 0.2, OrderSide::Sell);
        assert_eq!(r.len(), 2);
        assert_eq!(r.idx, [3, 7]);
        assert_eq!(r.col, [1, 1]);
        assert_eq!(r.side, [OrderSide::Buy as i8, OrderSide::Sell as i8]);
    }

    #[test]
    fn ids_are_positions_in_the_merged_array() {
        let mut a = OrderRecords::new();
        a.push_order(0, 0, 1.0, 1.0, 0.0, OrderSide::Buy);
        let mut b = OrderRecords::new();
        b.push_order(0, 1, 2.0, 2.0, 0.0, OrderSide::Buy);
        b.push_order(1, 1, 3.0, 3.0, 0.0, OrderSide::Sell);

        let merged = OrderRecords::concat(&[a, b]);
        assert_eq!(merged.ids(), [0, 1, 2]);
        // Concatenating in column order preserves column-major ordering.
        assert_eq!(merged.col, [0, 1, 1]);
    }
}
