//! Record schemas: the contract between the two tiers.
//!
//! The vectorized tier and the phase-2 event-driven tier compute completely
//! differently and will never share a simulation loop. What they do share is
//! this: **the same records come out**. Unifying the output rather than the
//! computation is the whole architecture — it means the analysis layer is
//! written once and works on either tier's results, and it means a
//! vectorized sweep and an event-driven replay can be compared column by
//! column. See `docs/ARCHITECTURE.md §2.2`.
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

pub mod cash;
pub mod drawdowns;
pub mod logs;
pub mod mapped;
mod trades;

pub use cash::CashRecords;
pub use drawdowns::{DrawdownRecords, DrawdownStatus, find_drawdowns};
pub use logs::{Journal, LogEntry, LogRecords, LogSink};

/// Re-exported so a caller can turn a stored discriminant back into a reason.
pub use barq_model::StatusInfo as StatusInfoLookup;
pub use mapped::{ColMap, MappedArray};
pub use trades::{TradeRecords, entry_trades, exit_trades, positions};

use barq_model::{CashFlowKind, OrderSide};

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

    /// Records cash that moved without a fill: the amount actually applied,
    /// after any cap, not the amount asked for.
    fn push_cash(&mut self, row: usize, col: usize, amount: f64, kind: CashFlowKind);
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
/// `docs/ARCHITECTURE.md §11.2`.
pub trait SinkBuffer: RecordSink + logs::LogSink + Send + Sized {
    /// An empty buffer with room for `capacity` fills.
    fn with_capacity(capacity: usize) -> Self;

    /// Merges buffers that are already in group order.
    ///
    /// Not necessarily a plain concatenation: a sink holding logs has to
    /// re-base their `order_id`s onto the merged fills. That is exactly why
    /// merging is the sink's business rather than the loop's. Takes the
    /// buffers by value so a single one is returned as it is.
    fn merge(parts: Vec<Self>) -> Self;
}

impl SinkBuffer for OrderRecords {
    fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    fn merge(mut parts: Vec<Self>) -> Self {
        if parts.len() == 1 {
            return parts.pop().expect("one part");
        }
        Self::concat(&parts)
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

    fn merge(mut parts: Vec<Self>) -> Self {
        if parts.len() == 1 {
            return parts.pop().expect("one part");
        }
        Self::concat(&parts)
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
    /// Cash that moved without a fill: deposits, earnings.
    ///
    /// Carried here rather than beside the fills so that everything which
    /// already moves a run's records — sinks, merges, sorting, the
    /// portfolio's constructors — moves these too. Empty unless the run asked
    /// for a cash flow. `len`, `ids` and every per-fill accessor count fills
    /// only.
    pub cash: CashRecords,
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
            cash: CashRecords::new(),
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
            cash: CashRecords::new(),
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
        self.cash.extend(&other.cash);
    }

    /// Puts the records into column-major order: by column, then by row.
    ///
    /// A shared-cash group emits its fills in execution order, which is the
    /// order the call sequence chose within each bar; the analysis layer's
    /// per-symbol derivations want them grouped by column instead. Stable,
    /// so fills of one column on one bar keep their execution order.
    pub fn sort_by_column(&mut self) {
        self.cash.sort_by_column();
        if self.col.windows(2).all(|w| w[0] < w[1])
            && self
                .col
                .windows(2)
                .zip(self.idx.windows(2))
                .all(|(c, i)| c[0] < c[1] || i[0] <= i[1])
        {
            return;
        }
        let mut indices: Vec<usize> = (0..self.len()).collect();
        indices.sort_by_key(|&i| (self.col[i], self.idx[i]));
        macro_rules! reorder {
            ($($field:ident),*) => { $(self.$field = indices.iter().map(|&i| self.$field[i]).collect();)* };
        }
        reorder!(col, idx, size, price, fees, side);
    }

    /// Merges buffers that are already in column order.
    ///
    /// A dense parallel run produces millions of fills per thread, and copying
    /// them all on the one thread that joined the workers was a visible share
    /// of its wall clock. Past [`Self::PARALLEL_CONCAT`] fills the parts are
    /// copied concurrently, each into its own disjoint span of the output.
    #[must_use]
    pub fn concat(parts: &[Self]) -> Self {
        let total: usize = parts.iter().map(Self::len).sum();
        if total < Self::PARALLEL_CONCAT || parts.len() < 2 {
            let mut out = Self::with_capacity(total);
            for part in parts {
                out.extend(part);
            }
            return out;
        }
        // Zeroed allocations are lazy, so the pages are first touched by the
        // thread that copies into them.
        let mut out = Self {
            col: vec![0; total],
            idx: vec![0; total],
            size: vec![0.0; total],
            price: vec![0.0; total],
            fees: vec![0.0; total],
            side: vec![0; total],
            cash: CashRecords::new(),
        };
        // Cash flows are a handful per bar at most; one serial pass is enough.
        for part in parts {
            out.cash.extend(&part.cash);
        }
        std::thread::scope(|scope| {
            let (mut col, mut idx) = (out.col.as_mut_slice(), out.idx.as_mut_slice());
            let (mut size, mut price) = (out.size.as_mut_slice(), out.price.as_mut_slice());
            let (mut fees, mut side) = (out.fees.as_mut_slice(), out.side.as_mut_slice());
            for part in parts {
                let n = part.len();
                let (c, rest) = col.split_at_mut(n);
                col = rest;
                let (i, rest) = idx.split_at_mut(n);
                idx = rest;
                let (s, rest) = size.split_at_mut(n);
                size = rest;
                let (p, rest) = price.split_at_mut(n);
                price = rest;
                let (f, rest) = fees.split_at_mut(n);
                fees = rest;
                let (d, rest) = side.split_at_mut(n);
                side = rest;
                scope.spawn(move || {
                    c.copy_from_slice(&part.col);
                    i.copy_from_slice(&part.idx);
                    s.copy_from_slice(&part.size);
                    p.copy_from_slice(&part.price);
                    f.copy_from_slice(&part.fees);
                    d.copy_from_slice(&part.side);
                });
            }
        });
        out
    }

    /// Fill count above which [`Self::concat`] copies the parts in parallel.
    /// Below it a thread costs more than the copy it would do.
    pub const PARALLEL_CONCAT: usize = 1 << 16;
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

    fn push_cash(&mut self, row: usize, col: usize, amount: f64, kind: CashFlowKind) {
        self.cash.push(row, col, amount, kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_concat_copies_in_parallel_and_matches_a_serial_one() {
        let make = |offset: i64, n: usize| {
            let mut r = OrderRecords::with_capacity(n);
            for k in 0..n {
                r.push_order(k, (offset as usize) % 7, k as f64, 1.5, 0.1, OrderSide::Buy);
                r.col[k] += offset;
            }
            r
        };
        let parts = [make(0, 40_000), make(1, 0), make(2, 50_000), make(3, 1)];
        let mut serial = OrderRecords::new();
        for part in &parts {
            serial.extend(part);
        }
        assert!(serial.len() >= OrderRecords::PARALLEL_CONCAT);
        assert_eq!(OrderRecords::concat(&parts), serial);
    }

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
