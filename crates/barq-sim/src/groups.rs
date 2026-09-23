//! Column grouping, and the call order within a group.
//!
//! Phase 1 was built on one assumption: **columns are independent**. That is
//! what makes the parallel driver a plain split, and it is true whenever the
//! columns are separate parameter sets or separate symbols, which is what a
//! sweep is.
//!
//! Cash sharing is where it stops being true. A group of columns drawing on
//! one cash balance is no longer N independent simulations — it is one
//! simulation over N instruments, and within a bar the columns must be
//! executed in some order because each one's fill changes what the next can
//! afford.
//!
//! # What this costs, precisely
//!
//! The parallel axis moves from *column* to *group*. Nothing else changes:
//! groups remain independent of each other, each thread still owns its
//! buffers, and records come out in group then execution order. So
//!
//! - a sweep of independent columns is one group each, and scales as before;
//! - a sweep of *portfolios* — say 200 parameter sets over a shared 5-symbol
//!   book — is 200 groups of 5, and still scales across 200 units;
//! - a single grouped portfolio is one group, and does not parallelize at
//!   all.
//!
//! The last case is the honest limit and it is inherent, not an artefact: a
//! shared cash balance is a sequential dependency across the whole group.
//! `docs/ARCHITECTURE.md §11.2` carries the measured figures.
//!
//! # Call order
//!
//! Within a bar, [`CallSeq::Auto`] sorts the group's columns by approximate
//! order value so that sells execute before buys. That matters because a buy
//! that arrives before the sell funding it is rejected for want of cash, and
//! the rejection is silent — the column simply does not trade. It is the
//! single most common surprise in a shared-cash backtest.

use barq_core::error::{Error, Result};
use barq_model::{Direction, SizeType};

/// How columns are partitioned into cash-sharing groups.
///
/// Stored as lengths rather than boundaries because that is how the reference
/// carries them, and because it makes "one group per column" the obvious
/// default rather than a special case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Groups {
    lens: Vec<usize>,
    starts: Vec<usize>,
    /// Which group each column belongs to.
    of_col: Vec<usize>,
}

impl Groups {
    /// Every column its own group: no sharing, and the phase-1 behaviour.
    #[must_use]
    pub fn ungrouped(n_cols: usize) -> Self {
        Self {
            lens: vec![1; n_cols],
            starts: (0..n_cols).collect(),
            of_col: (0..n_cols).collect(),
        }
    }

    /// Groups of the given lengths, laid out left to right.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invariant`] if a length is zero or the lengths do not
    /// sum to `n_cols`. A zero-length group has no cash balance to share and
    /// would silently shift every later group's columns.
    pub fn from_lens(lens: &[usize], n_cols: usize) -> Result<Self> {
        let mut starts = Vec::with_capacity(lens.len());
        let mut of_col = vec![0_usize; n_cols];
        let mut acc = 0_usize;
        for (g, &len) in lens.iter().enumerate() {
            if len == 0 {
                return Err(Error::Invariant(format!("group {g} has no columns")));
            }
            if len > n_cols - acc {
                return Err(Error::Invariant(format!(
                    "group lengths run past column {n_cols}"
                )));
            }
            starts.push(acc);
            of_col[acc..acc + len].fill(g);
            acc += len;
        }
        if acc != n_cols {
            return Err(Error::Invariant(format!(
                "group lengths cover {acc} columns, not {n_cols}"
            )));
        }
        Ok(Self {
            lens: lens.to_vec(),
            starts,
            of_col,
        })
    }

    /// Number of groups.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lens.len()
    }

    /// Whether there are no groups, which means no columns.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lens.is_empty()
    }

    /// Columns in group `g`, as a half-open range.
    #[must_use]
    pub fn range(&self, g: usize) -> std::ops::Range<usize> {
        self.starts[g]..self.starts[g] + self.lens[g]
    }

    /// How many columns group `g` holds.
    #[must_use]
    pub fn len_of(&self, g: usize) -> usize {
        self.lens[g]
    }

    /// Which group a column belongs to.
    #[must_use]
    pub fn of_column(&self, col: usize) -> usize {
        self.of_col[col]
    }

    /// Whether any group holds more than one column.
    ///
    /// This is the switch the whole feature turns on: when false, the
    /// simulation is the phase-1 per-column one and every group-level
    /// computation is skipped.
    #[must_use]
    pub fn is_shared(&self) -> bool {
        self.lens.iter().any(|&l| l > 1)
    }

    /// The largest group, which bounds the per-bar scratch space.
    #[must_use]
    pub fn max_len(&self) -> usize {
        self.lens.iter().copied().max().unwrap_or(0)
    }
}

/// The order a group's columns are executed in within a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallSeq<'a> {
    /// Left to right, as the columns are laid out.
    #[default]
    Default,
    /// Sorted each bar by approximate order value, ascending.
    ///
    /// Sells come first and release cash before the buys that need it. Without
    /// this, a rebalance that sells one instrument to fund another depends on
    /// which happens to sit further left — and the funding buy is rejected for
    /// want of cash if it loses. The rejection is silent.
    ///
    /// "Approximate" is exact about what it is: the value is estimated from
    /// the size and the *last* valuation price, before any of the bar's fills.
    /// It decides the order, not the fills.
    Auto,
    /// An explicit per-bar order, as within-group indices.
    ///
    /// Laid out column-major like everything else: entry `col * n_rows + row`
    /// holds the *relative* index, in `0..group_len`, of the column to execute
    /// in that position.
    Explicit(&'a [usize]),
}

impl CallSeq<'_> {
    /// Fills `out` with the execution order for one bar of one group.
    ///
    /// `values` holds the approximate order value of each of the group's
    /// columns, in layout order, and is consumed as scratch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invariant`] if an explicit sequence is the wrong
    /// length or names a column outside the group.
    pub fn resolve(
        &self,
        group: std::ops::Range<usize>,
        row: usize,
        n_rows: usize,
        values: &mut [f64],
        out: &mut Vec<usize>,
    ) -> Result<()> {
        let group_len = group.len();
        out.clear();
        out.extend(0..group_len);
        match self {
            Self::Default => Ok(()),
            Self::Auto => {
                insert_argsort(&mut values[..group_len], out);
                Ok(())
            }
            Self::Explicit(seq) => {
                if seq.len() != n_rows * (group.end.max(1)) && seq.len() < n_rows * group.end {
                    return Err(Error::Invariant(format!(
                        "a call sequence of {} entries for {n_rows} rows",
                        seq.len()
                    )));
                }
                for (k, slot) in out.iter_mut().enumerate() {
                    let idx = seq[(group.start + k) * n_rows + row];
                    if idx >= group_len {
                        return Err(Error::Invariant(format!(
                            "call index {idx} is outside a group of {group_len}"
                        )));
                    }
                    *slot = idx;
                }
                Ok(())
            }
        }
    }
}

/// What an order is roughly worth, for deciding the call order.
///
/// Positive means it will spend cash, negative that it will release some.
/// Only the sign and the relative magnitude matter — this never becomes a fill
/// size, and it is computed from the *last* valuation price, before any of the
/// bar's fills. An exact figure would need the order executed, which is the
/// thing being ordered.
///
/// `NaN` where the estimate cannot be made, which sorts last.
#[must_use]
pub fn approx_order_value(
    size: f64,
    size_type: SizeType,
    direction: Direction,
    cash: f64,
    position: f64,
    free_cash: f64,
    val_price: f64,
    value: f64,
) -> f64 {
    // A short-only column states its size positively and means the other way.
    let size = if direction == Direction::ShortOnly {
        -size
    } else {
        size
    };
    let asset_value = position * val_price;
    match size_type {
        SizeType::Amount => size * val_price,
        SizeType::Value => size,
        SizeType::Percent => {
            if size >= 0.0 {
                size * cash
            } else if direction == Direction::LongOnly {
                size * asset_value
            } else {
                // Closing a short needs the position bought back *and* its
                // reserve released, which is why the long side is doubled.
                size * (2.0 * asset_value.max(0.0) + free_cash.max(0.0))
            }
        }
        SizeType::TargetAmount => size * val_price - asset_value,
        SizeType::TargetValue => size - asset_value,
        SizeType::TargetPercent => size * value - asset_value,
    }
}

/// Insertion sort of `values`, permuting `order` alongside.
///
/// Ascending, stable, and **`NaN` sorts to the end** — an unmeasurable order
/// value means "no idea what this is worth", and the safe place for it is
/// after everything that does release cash. Written as the reference writes
/// it, including that a `NaN` already in place is always shifted right rather
/// than compared.
///
/// Insertion sort rather than a better one because a group is a handful of
/// instruments, not a dataset, and this runs once per bar per group.
fn insert_argsort(values: &mut [f64], order: &mut [usize]) {
    for j in 1..values.len() {
        let v_j = values[j];
        let o_j = order[j];
        let mut i = j as isize - 1;
        while i >= 0 && (values[i as usize] > v_j || values[i as usize].is_nan()) {
            values[i as usize + 1] = values[i as usize];
            order[i as usize + 1] = order[i as usize];
            i -= 1;
        }
        values[(i + 1) as usize] = v_j;
        order[(i + 1) as usize] = o_j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ungrouped_is_one_group_per_column() {
        let g = Groups::ungrouped(3);
        assert_eq!(g.len(), 3);
        assert!(!g.is_shared());
        assert_eq!(g.range(1), 1..2);
        assert_eq!(g.of_column(2), 2);
    }

    #[test]
    fn groups_lay_out_left_to_right() {
        let g = Groups::from_lens(&[2, 3], 5).unwrap();
        assert_eq!(g.len(), 2);
        assert!(g.is_shared());
        assert_eq!(g.range(0), 0..2);
        assert_eq!(g.range(1), 2..5);
        assert_eq!(g.of_column(0), 0);
        assert_eq!(g.of_column(4), 1);
        assert_eq!(g.max_len(), 3);
    }

    #[test]
    fn groups_of_one_are_not_sharing() {
        // The switch the whole feature turns on. All-ones must read as
        // unshared or every ungrouped run would pay for machinery it does not
        // use.
        assert!(!Groups::from_lens(&[1, 1, 1], 3).unwrap().is_shared());
        assert!(Groups::from_lens(&[1, 2], 3).unwrap().is_shared());
    }

    #[test]
    fn a_zero_length_group_is_rejected() {
        // It has no cash balance to share, and it would silently shift every
        // later group's columns.
        assert!(Groups::from_lens(&[2, 0, 1], 3).is_err());
    }

    #[test]
    fn lengths_must_cover_every_column() {
        assert!(Groups::from_lens(&[2], 3).is_err());
        assert!(Groups::from_lens(&[2, 2], 3).is_err());
    }

    #[test]
    fn the_default_order_is_left_to_right() {
        let mut out = Vec::new();
        let mut values = [3.0, 1.0, 2.0];
        CallSeq::Default
            .resolve(2..5, 0, 4, &mut values, &mut out)
            .unwrap();
        assert_eq!(out, [0, 1, 2]);
    }

    #[test]
    fn auto_puts_sells_before_buys() {
        // The whole point: a negative order value releases cash and must run
        // before the positive one that needs it.
        let mut out = Vec::new();
        let mut values = [100.0, -50.0, 20.0];
        CallSeq::Auto
            .resolve(0..3, 0, 1, &mut values, &mut out)
            .unwrap();
        assert_eq!(out, [1, 2, 0]);
    }

    #[test]
    fn auto_is_stable_between_equal_values() {
        let mut out = Vec::new();
        let mut values = [5.0, 5.0, 5.0];
        CallSeq::Auto
            .resolve(0..3, 0, 1, &mut values, &mut out)
            .unwrap();
        assert_eq!(out, [0, 1, 2], "equal values keep their layout order");
    }

    #[test]
    fn an_unmeasurable_order_value_goes_last() {
        // "No idea what this is worth" belongs after everything that does
        // release cash, not before it.
        let mut out = Vec::new();
        let mut values = [f64::NAN, 10.0, -1.0];
        CallSeq::Auto
            .resolve(0..3, 0, 1, &mut values, &mut out)
            .unwrap();
        assert_eq!(out, [2, 1, 0]);
    }

    #[test]
    fn an_explicit_sequence_is_read_per_bar_and_per_group() {
        // Column-major, holding within-group indices. Two bars, one group of
        // two columns: bar 0 runs them in order, bar 1 runs them reversed.
        let n_rows = 2;
        let seq = [0, 1, 1, 0];
        let mut out = Vec::new();
        let mut values = [0.0, 0.0];
        let cs = CallSeq::Explicit(&seq);
        cs.resolve(0..2, 0, n_rows, &mut values, &mut out).unwrap();
        assert_eq!(out, [0, 1]);
        cs.resolve(0..2, 1, n_rows, &mut values, &mut out).unwrap();
        assert_eq!(out, [1, 0]);
    }

    #[test]
    fn an_out_of_range_call_index_is_rejected() {
        let seq = [0, 9];
        let mut out = Vec::new();
        let mut values = [0.0, 0.0];
        assert!(
            CallSeq::Explicit(&seq)
                .resolve(0..2, 0, 1, &mut values, &mut out)
                .is_err()
        );
    }
}
