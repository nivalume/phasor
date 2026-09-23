//! Trailing-window order statistics in `O(log n)` per row.
//!
//! `Quantile`, `Med`, `Rank` and `Mad` all need the window sorted. Re-sorting
//! every window costs `O(window · log window)` per row. Instead, each column
//! is sorted once, every row gets a fixed position in that order, and a
//! Fenwick tree over positions holds which rows are currently in the window.
//! Entering and leaving the window is a point update, the k-th smallest value
//! is a descent of the tree, and "how many values are below x" is a prefix
//! query — all `O(log n)`.
//!
//! Counts and positions are integers, so `Quantile`, `Med` and `Rank` are
//! bit-identical to sorting the window. `Mad` also needs partial sums, which
//! are summed as offsets from the column's first value.
//!
//! `IdxMax`/`IdxMin` use a monotonic queue instead: see [`rolling_arg_extreme`].

/// Which order statistic [`ordered`] reports.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Order {
    Quantile(f64),
    Rank,
    Mad,
}

/// Runs `stat` over every trailing window of `values`, `NaN` where the window
/// has no observation.
pub(crate) fn ordered(values: &[f64], window: usize, stat: Order) -> Vec<f64> {
    let window = window.max(1);
    let mut tree = Ordered::new(values);
    let mut out = Vec::with_capacity(values.len());
    for (row, &value) in values.iter().enumerate() {
        if row >= window {
            tree.update(row - window, false);
        }
        tree.update(row, true);
        out.push(if tree.count == 0 {
            f64::NAN
        } else {
            match stat {
                Order::Quantile(q) => tree.quantile(q),
                Order::Rank => tree.rank(value),
                Order::Mad => tree.mad(),
            }
        });
    }
    out
}

struct Ordered {
    /// The column's non-`NaN` values, ascending.
    sorted: Vec<f64>,
    /// Row → position in `sorted`, `None` for `NaN`.
    position: Vec<Option<usize>>,
    /// Fenwick trees (1-based) of window membership and of shifted values.
    counts: Vec<u32>,
    sums: Vec<f64>,
    shift: f64,
    count: usize,
    total: f64,
    /// Highest power of two not above `sorted.len()`, for the descent.
    top: usize,
}

impl Ordered {
    fn new(values: &[f64]) -> Self {
        let mut rows: Vec<usize> = (0..values.len())
            .filter(|&row| !values[row].is_nan())
            .collect();
        rows.sort_by(|&a, &b| values[a].total_cmp(&values[b]).then(a.cmp(&b)));
        let mut position = vec![None; values.len()];
        for (rank, &row) in rows.iter().enumerate() {
            position[row] = Some(rank);
        }
        let sorted: Vec<f64> = rows.iter().map(|&row| values[row]).collect();
        let len = sorted.len();
        Self {
            shift: sorted.first().copied().unwrap_or(0.0),
            sorted,
            position,
            counts: vec![0; len + 1],
            sums: vec![0.0; len + 1],
            count: 0,
            total: 0.0,
            top: if len == 0 { 0 } else { 1 << len.ilog2() },
        }
    }

    fn update(&mut self, row: usize, insert: bool) {
        let Some(rank) = self.position[row] else {
            return;
        };
        let d = self.sorted[rank] - self.shift;
        let (step, d) = if insert { (1_i64, d) } else { (-1, -d) };
        self.count = self.count.wrapping_add_signed(step as isize);
        self.total += d;
        let mut i = rank + 1;
        while i < self.counts.len() {
            self.counts[i] = self.counts[i].wrapping_add_signed(step as i32);
            self.sums[i] += d;
            i += i & i.wrapping_neg();
        }
    }

    /// The `k`-th smallest value in the window, 0-based.
    fn kth(&self, k: usize) -> f64 {
        let (mut at, mut remaining) = (0, k as u32 + 1);
        let mut step = self.top;
        while step > 0 {
            let next = at + step;
            if next < self.counts.len() && self.counts[next] < remaining {
                at = next;
                remaining -= self.counts[next];
            }
            step >>= 1;
        }
        self.sorted[at]
    }

    /// Count and shifted sum of the window's values at positions `< end`.
    fn prefix(&self, end: usize) -> (usize, f64) {
        let (mut count, mut sum, mut i) = (0, 0.0, end);
        while i > 0 {
            count += self.counts[i] as usize;
            sum += self.sums[i];
            i &= i - 1;
        }
        (count, sum)
    }

    /// Linear interpolation between order statistics, as NumPy's default.
    fn quantile(&self, q: f64) -> f64 {
        let index = (self.count - 1) as f64 * q;
        let lower = index.floor() as usize;
        let fraction = index - lower as f64;
        let low = self.kth(lower);
        low + fraction * (self.kth(index.ceil() as usize) - low)
    }

    /// Percentile of `last` in the window, ties sharing their average rank.
    fn rank(&self, last: f64) -> f64 {
        if last.is_nan() {
            return f64::NAN;
        }
        let (below, _) = self.prefix(self.sorted.partition_point(|&v| v < last));
        let (not_above, _) = self.prefix(self.sorted.partition_point(|&v| v <= last));
        let equal = (not_above - below) as f64;
        (below as f64 + (equal + 1.0) / 2.0) / self.count as f64
    }

    /// Mean absolute deviation around the mean.
    #[allow(
        clippy::float_cmp,
        reason = "a window of identical values has exactly zero deviation"
    )]
    fn mad(&self) -> f64 {
        // The partial sums carry rounding residue scaled by the distance from
        // the shift; a constant window must still report an exact zero.
        if self.kth(0) == self.kth(self.count - 1) {
            return 0.0;
        }
        let n = self.count as f64;
        let mean = self.total / n;
        let (below, below_sum) =
            self.prefix(self.sorted.partition_point(|&v| v - self.shift < mean));
        let below = below as f64;
        let above_sum = self.total - below_sum;
        (mean * below - below_sum + above_sum - mean * (n - below)).max(0.0) / n
    }
}

/// Qlib's rolling `IdxMax`/`IdxMin`: the 1-based window position of the first
/// extreme value, where a `NaN` counts as more extreme than any number (so the
/// first `NaN` in the window wins), and `NaN` for an all-`NaN` window.
pub(crate) fn rolling_arg_extreme(values: &[f64], window: usize, minimum: bool) -> Vec<f64> {
    use std::collections::VecDeque;

    let window = window.max(1);
    let mut nans = VecDeque::new();
    // Rows whose value no later row beats; the front is the earliest extreme.
    let mut candidates: VecDeque<usize> = VecDeque::new();
    let mut valid = 0_usize;
    let mut out = Vec::with_capacity(values.len());
    for (row, &value) in values.iter().enumerate() {
        if row >= window {
            let leaving = row - window;
            if values[leaving].is_nan() {
                nans.pop_front();
            } else {
                valid -= 1;
                if candidates.front() == Some(&leaving) {
                    candidates.pop_front();
                }
            }
        }
        if value.is_nan() {
            nans.push_back(row);
        } else {
            valid += 1;
            while candidates.back().is_some_and(|&back| {
                if minimum {
                    values[back] > value
                } else {
                    values[back] < value
                }
            }) {
                candidates.pop_back();
            }
            candidates.push_back(row);
        }
        let start = (row + 1).saturating_sub(window);
        out.push(if valid == 0 {
            f64::NAN
        } else {
            let best = nans
                .front()
                .or(candidates.front())
                .copied()
                .unwrap_or(start);
            (best - start + 1) as f64
        });
    }
    out
}
