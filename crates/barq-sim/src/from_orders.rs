//! `simulate_from_orders`: one order per element.
//!
//! The simplest of the vectorized loops. Every element of the target shape
//! becomes an order request; what comes out is a set of fills.
//!
//! # Groups, and where the sequential dependency lives
//!
//! By default every column is its own group and carries its own cash, which
//! is what makes a sweep embarrassingly parallel. With [`Groups`] a set of
//! columns can instead share one balance — a portfolio over several
//! instruments rather than several independent backtests — and then the
//! columns inside a group are executed in sequence within each bar, because
//! each fill changes what the next can afford.
//!
//! The parallel axis therefore moves from column to group. Groups stay
//! independent of one another, so nothing else about the decomposition
//! changes; a single grouped portfolio simply has one unit of work. See
//! [`crate::groups`].

use barq_core::error::{Error, Result};
use barq_core::flex::FlexArray;
use barq_exec::{
    Direction, Order, OrderStatus, ProcessOrderState, SizeType, ValidatedOrder, execute_validated,
};
use barq_records::{Journal, LogEntry, LogSink, OrderRecords, RecordSink, SinkBuffer};

use crate::groups::{CallSeq, Groups, approx_order_value};

/// Inputs to [`simulate_from_orders`].
///
/// Every per-order parameter is a [`FlexArray`], so a scalar, a value per
/// column, a value per bar and a full matrix are all accepted without
/// materializing anything. Defaults match the reference engine.
#[derive(Debug, Clone)]
pub struct FromOrders<'a> {
    /// Number of bars.
    pub n_rows: usize,
    /// Number of columns: parameter combinations times symbols.
    pub n_cols: usize,
    /// Starting cash, per column.
    pub init_cash: FlexArray<'a, f64>,
    /// Closing prices. Used to resolve infinite order and valuation prices.
    pub close: FlexArray<'a, f64>,

    /// Requested size.
    pub size: FlexArray<'a, f64>,
    /// Requested price. `+inf` means this bar's close, `-inf` the previous one.
    pub price: FlexArray<'a, f64>,
    /// How to interpret `size`.
    pub size_type: FlexArray<'a, SizeType>,
    /// Which sides a column may take.
    pub direction: FlexArray<'a, Direction>,
    /// Proportional fee rate.
    pub fees: FlexArray<'a, f64>,
    /// Flat fee per fill.
    pub fixed_fees: FlexArray<'a, f64>,
    /// Proportional slippage.
    pub slippage: FlexArray<'a, f64>,
    /// Reject fills below this size.
    pub min_size: FlexArray<'a, f64>,
    /// Clamp fills to this size.
    pub max_size: FlexArray<'a, f64>,
    /// Round sizes down to a multiple of this.
    pub size_granularity: FlexArray<'a, f64>,
    /// Whether to reserve cash to cover an existing short.
    pub lock_cash: FlexArray<'a, bool>,
    /// Whether a partial fill is acceptable.
    pub allow_partial: FlexArray<'a, bool>,
    /// Whether a rejection should abort the simulation.
    pub raise_reject: FlexArray<'a, bool>,
    /// Valuation price. `+inf` means the order price, `-inf` the previous close.
    pub val_price: FlexArray<'a, f64>,

    /// Whether each order records a full before-and-after log.
    ///
    /// Off everywhere by default. A log is 38 fields against an order
    /// record's 7 and is written for every order *considered* rather than
    /// every one filled, so a sweep with this on everywhere produces
    /// gigabytes nobody reads. The useful shapes are one column, or one row
    /// across a sweep. See [`barq_records::logs`].
    pub log: FlexArray<'a, bool>,

    /// Carry the last non-`NaN` valuation price forward.
    pub ffill_val_price: bool,
    /// Mark to market after each fill rather than only at the bar's start.
    pub update_value: bool,

    /// How columns are partitioned into cash-sharing groups.
    ///
    /// [`Groups::ungrouped`] — one column per group — is the phase-1
    /// behaviour and the default. Anything else makes the columns in a group
    /// share a cash balance and execute in sequence within each bar.
    pub groups: Groups,
    /// The order a shared group's columns execute in within a bar.
    ///
    /// Ignored when nothing is shared, since a group of one has only one
    /// order.
    pub call_seq: CallSeq<'a>,
}

impl FromOrders<'_> {
    /// Whether any parameter [`ValidatedOrder`] covers changes down a column.
    ///
    /// When none does, the column loop checks them once instead of once per
    /// bar. Deliberately excludes `size`, `price`, `size_type` and
    /// `direction`, which the parameter checks do not cover and which
    /// [`ValidatedOrder::set_bar`] rewrites every bar anyway.
    fn params_vary_by_row(&self) -> bool {
        self.fees.varies_by_row()
            || self.fixed_fees.varies_by_row()
            || self.slippage.varies_by_row()
            || self.min_size.varies_by_row()
            || self.max_size.varies_by_row()
            || self.size_granularity.varies_by_row()
            || self.lock_cash.varies_by_row()
            || self.allow_partial.varies_by_row()
            || self.raise_reject.varies_by_row()
    }
}

/// Builds the order request for row `i` of column `col`.
///
/// `size`, `price`, `size_type` and `direction` are left unset: the loop
/// supplies them per bar through [`ValidatedOrder::set_bar`], which is what
/// lets the rest be resolved and checked once when it is column-constant.
fn build_order(args: &FromOrders<'_>, i: usize, col: usize) -> ValidatedOrder {
    ValidatedOrder::new(Order {
        size: f64::NAN,
        price: f64::NAN,
        size_type: SizeType::Amount,
        direction: Direction::Both,
        fees: args.fees.get(i, col),
        fixed_fees: args.fixed_fees.get(i, col),
        slippage: args.slippage.get(i, col),
        min_size: args.min_size.get(i, col),
        max_size: args.max_size.get(i, col),
        size_granularity: args.size_granularity.get(i, col),
        reject_prob: 0.0,
        lock_cash: args.lock_cash.get(i, col),
        allow_partial: args.allow_partial.get(i, col),
        raise_reject: args.raise_reject.get(i, col),
        log: args.log.get(i, col),
    })
}

impl FromOrders<'_> {
    /// Defaults at a given shape, with every column its own group.
    ///
    /// The shape has to be given here because the grouping depends on it, and
    /// a default grouping that did not match `n_cols` would be rejected on the
    /// first call rather than silently.
    #[must_use]
    pub fn new(n_rows: usize, n_cols: usize) -> Self {
        Self {
            n_rows,
            n_cols,
            groups: Groups::ungrouped(n_cols),
            ..Self::default()
        }
    }
}

impl Default for FromOrders<'_> {
    fn default() -> Self {
        Self {
            n_rows: 0,
            n_cols: 0,
            init_cash: FlexArray::Scalar(100.0),
            close: FlexArray::Scalar(f64::NAN),
            size: FlexArray::Scalar(f64::INFINITY),
            price: FlexArray::Scalar(f64::INFINITY),
            size_type: FlexArray::Scalar(SizeType::Amount),
            direction: FlexArray::Scalar(Direction::Both),
            fees: FlexArray::Scalar(0.0),
            fixed_fees: FlexArray::Scalar(0.0),
            slippage: FlexArray::Scalar(0.0),
            min_size: FlexArray::Scalar(crate::DEFAULT_MIN_SIZE),
            max_size: FlexArray::Scalar(f64::INFINITY),
            size_granularity: FlexArray::Scalar(f64::NAN),
            lock_cash: FlexArray::Scalar(false),
            allow_partial: FlexArray::Scalar(true),
            raise_reject: FlexArray::Scalar(false),
            log: FlexArray::Scalar(false),
            val_price: FlexArray::Scalar(f64::INFINITY),
            ffill_val_price: true,
            update_value: false,
            groups: Groups::ungrouped(0),
            call_seq: CallSeq::Default,
        }
    }
}

/// The re-marked valuation price and portfolio value after a fill.
///
/// `update_value` re-marks the valuation price to the price actually filled
/// at, and carries the portfolio value forward by the cash flow plus the
/// change in asset value — rather than re-deriving it, which would give a
/// different number in the last bits.
///
/// **This was dead code until cash sharing arrived**, and the comment it
/// replaces said so: with one column per group the value is re-derived at the
/// top of every bar, so anything written here was overwritten before it could
/// be read. In a shared group the value spans several columns and is threaded
/// through the whole bar, so it is now live and a fixture can see it.
pub(crate) fn updated_value(
    cash_before: f64,
    cash_now: f64,
    position_before: f64,
    position_now: f64,
    val_price_before: f64,
    price: f64,
    value_before: f64,
) -> (f64, f64) {
    let cash_flow = cash_now - cash_before;
    let asset_before = if position_before == 0.0 {
        0.0
    } else {
        position_before * val_price_before
    };
    let asset_now = if position_now == 0.0 {
        0.0
    } else {
        position_now * price
    };
    (price, value_before + cash_flow + (asset_now - asset_before))
}

/// Per-column state that survives from one bar to the next.
///
/// A group carries one of these per column plus a single shared cash balance —
/// which is the whole of what cash sharing means, expressed as a type.
#[derive(Debug, Clone, Copy)]
struct ColumnState {
    position: f64,
    debt: f64,
    last_val_price: f64,
}

impl ColumnState {
    const fn new() -> Self {
        Self {
            position: 0.0,
            debt: 0.0,
            last_val_price: f64::NAN,
        }
    }
}

/// Simulates one group, writing its fills to `sink`.
///
/// Self-contained by construction: it touches no state belonging to any other
/// group. That is what makes the parallel driver safe. With the default
/// grouping — one column each — this is the per-column simulation phase 1
/// had, and a group of one skips every shared-cash computation below.
///
/// # Why the bar is two passes
///
/// Prices and valuation prices for the *whole* group are resolved before any
/// order executes, because the group's value — which a `TargetPercent` order
/// is a fraction of — depends on all of them, and because the execution order
/// may not be the layout order. Resolving inside the execution loop would
/// value the group differently depending on which column happened to go
/// first.
///
/// Records come out in layout order within the group even when execution does
/// not follow it, because `sink` is written in the execution order but each
/// record carries its own column and the caller sorts by column. See the note
/// on ordering in [`barq_records`].
/// `fills` counts the fills already written to `sink`, and is advanced here.
/// It belongs to the caller rather than to this function because a log's
/// `order_id` is an index into the buffer it was written to, and the serial
/// path writes every group into one buffer. Resetting it per group would point
/// group 1's logs at group 0's fills — an in-range index naming the wrong
/// trade, which nothing downstream could flag.
fn simulate_group<S: RecordSink + LogSink>(
    args: &FromOrders<'_>,
    group: usize,
    sink: &mut S,
    fills: &mut i64,
) -> Result<()> {
    let cols = args.groups.range(group);
    let group_len = cols.len();
    let shared = group_len > 1;

    // One cash balance for the group. Read from the group's first column, so
    // an ungrouped run still reads per-column starting cash.
    let mut cash = args.init_cash.get(0, cols.start);
    let mut free_cash = cash;
    let mut state = vec![ColumnState::new(); group_len];

    // Scratch reused across bars rather than reallocated per bar.
    let mut price = vec![f64::NAN; group_len];
    let mut order_value = vec![f64::NAN; group_len];
    let mut seq: Vec<usize> = (0..group_len).collect();

    // Build each request once per column, not once per bar. That hoists two
    // things: the nine flexible-array reads it takes to fill in, and the
    // seven of the kernel's fourteen checks that read only those fields.
    // Both are constant down a column unless a caller says otherwise, which
    // `params_vary_by_row` is what asks.
    //
    // Measured, interleaved against a build without it: +8% construction and
    // +20% validation on sparse workloads, +11% and +10% on dense ones — in
    // total +30% and +22%. Validation dominates on sparse because a bar that
    // does not fill used to pay all fourteen checks and little else.
    let params_vary = args.params_vary_by_row();
    let mut orders: Vec<ValidatedOrder> = cols.clone().map(|c| build_order(args, 0, c)).collect();

    for i in 0..args.n_rows {
        if params_vary {
            for (k, col) in cols.clone().enumerate() {
                orders[k] = build_order(args, i, col);
            }
        }

        // --- first pass: resolve prices for the whole group ---------------
        for (k, col) in cols.clone().enumerate() {
            // Resolve the order price. Infinities are sentinels: +inf means
            // this bar's close, -inf the previous bar's close (and there is
            // no previous one on the first bar).
            let mut p = args.price.get(i, col);
            if p.is_infinite() {
                p = if p > 0.0 {
                    args.close.get(i, col)
                } else if i > 0 {
                    args.close.get(i - 1, col)
                } else {
                    f64::NAN
                };
            }
            price[k] = p;

            // Resolve the valuation price, whose +inf means "the order price".
            let mut val_price = args.val_price.get(i, col);
            if val_price.is_infinite() {
                val_price = if val_price > 0.0 {
                    p
                } else if i > 0 {
                    args.close.get(i - 1, col)
                } else {
                    f64::NAN
                };
            }
            if !val_price.is_nan() || !args.ffill_val_price {
                state[k].last_val_price = val_price;
            }
        }

        // --- the group's value, and the order to execute in ---------------
        //
        // Computed once per bar for a shared group, because every column's
        // `TargetPercent` is a fraction of the same number. For a group of
        // one it is recomputed per column below, which is the same thing.
        let mut group_value = cash;
        if shared {
            for st in &state {
                if st.position != 0.0 {
                    group_value += st.position * st.last_val_price;
                }
            }
            if args.call_seq == CallSeq::Auto {
                for (k, col) in cols.clone().enumerate() {
                    order_value[k] = approx_order_value(
                        args.size.get(i, col),
                        args.size_type.get(i, col),
                        args.direction.get(i, col),
                        cash,
                        state[k].position,
                        free_cash,
                        state[k].last_val_price,
                        group_value,
                    );
                }
            }
            args.call_seq
                .resolve(cols.clone(), i, args.n_rows, &mut order_value, &mut seq)?;
        }

        // --- second pass: execute -----------------------------------------
        for (slot, &picked) in seq.iter().enumerate().take(group_len) {
            let k = if shared { picked } else { slot };
            let col = cols.start + k;
            let st = state[k];

            // An unshared column values only itself, which is what phase 1
            // did and what a group of one still means.
            if !shared {
                group_value = if st.position == 0.0 {
                    cash
                } else {
                    cash + st.position * st.last_val_price
                };
            }
            let value = group_value;

            let order = &mut orders[k];
            order.set_bar(
                args.size.get(i, col),
                price[k],
                args.size_type.get(i, col),
                args.direction.get(i, col),
            );

            let process = ProcessOrderState {
                cash,
                position: st.position,
                debt: st.debt,
                free_cash,
                val_price: st.last_val_price,
                value,
                oidx: 0,
                lidx: 0,
            };

            let (exec_state, result) = execute_validated(&process, order, None)?;

            if result.status == OrderStatus::Rejected && order.order().raise_reject {
                return Err(Error::Invariant(format!(
                    "order rejected at row {i}, column {col}: {}",
                    result
                        .status_info
                        .map_or("unknown reason", barq_model::StatusInfo::describe)
                )));
            }

            // `update_value` re-marks the valuation price to the price
            // actually filled at, and moves the value with it. Within a bar
            // that matters only for a shared group, where the value is
            // threaded across columns; across bars the re-marked price
            // survives when the next bar's valuation is unusable and
            // forward-fill is on, and then changes the size of a target-sized
            // order.
            let (new_val_price, new_value) = if result.is_filled() && args.update_value {
                updated_value(
                    cash,
                    exec_state.cash,
                    st.position,
                    exec_state.position,
                    st.last_val_price,
                    result.price,
                    value,
                )
            } else {
                (st.last_val_price, value)
            };

            let mut order_id = -1_i64;
            if result.is_filled() {
                let side = result.side.expect("a filled order always carries a side");
                sink.push_order(i, col, result.size, result.price, result.fees, side);
                // The record's own index, so a log joins back to the fill it
                // produced. Counted rather than read off the sink, because a
                // sink need not be an array.
                order_id = *fills;
                *fills += 1;
            }

            if order.order().log {
                sink.push_log(&LogEntry {
                    group,
                    col,
                    idx: i,
                    cash,
                    position: st.position,
                    debt: st.debt,
                    free_cash,
                    val_price: st.last_val_price,
                    value,
                    request: *order.order(),
                    new_cash: exec_state.cash,
                    new_position: exec_state.position,
                    new_debt: exec_state.debt,
                    new_free_cash: exec_state.free_cash,
                    new_val_price,
                    new_value,
                    result,
                    order_id,
                });
            }

            cash = exec_state.cash;
            free_cash = exec_state.free_cash;
            state[k].position = exec_state.position;
            state[k].debt = exec_state.debt;
            // The reference guards this write with the same NaN/ffill test
            // used during resolution. Here that guard provably cannot change
            // anything: `new_val_price` is either the current
            // `last_val_price`, or a filled order's price, which is finite.
            // The only skipped case would assign NaN over NaN.
            //
            // Written unconditionally rather than carrying a branch no fixture
            // can reach. Restore the guard when a loop appears where
            // `new_val_price` can be NaN independently of `last_val_price`.
            state[k].last_val_price = new_val_price;
            // The group's value is threaded, not re-derived. **Every order in
            // a bar is sized against the value the bar opened with**, and a
            // fill moves that value only when `update_value` says to.
            // Re-summing the group after each fill is the obvious alternative
            // and it is wrong: it makes a rebalance's later legs see a book
            // the earlier ones have already changed, which is not what the
            // reference models and not what "decide the whole bar at once"
            // means.
            group_value = new_value;
        }
    }

    Ok(())
}

/// Rejects a grouping that does not cover the shape.
///
/// A mismatch here would silently simulate the wrong columns, so it is checked
/// once at entry rather than trusted.
fn check_groups(args: &FromOrders<'_>) -> Result<()> {
    let covered: usize = (0..args.groups.len()).map(|g| args.groups.len_of(g)).sum();
    if covered == args.n_cols {
        Ok(())
    } else {
        Err(Error::Invariant(format!(
            "the grouping covers {covered} columns, but the shape has {}",
            args.n_cols
        )))
    }
}

/// Simulates every group on one thread into any sink.
///
/// Generic so that the plain and the logged variant are one loop rather than
/// two copies differing in a type. The sink decides whether logs are kept:
/// [`OrderRecords`] discards them through `LogSink`'s defaulted no-op, so a run
/// that is not collecting logs pays nothing for the call site existing.
fn simulate_into<S: SinkBuffer>(args: &FromOrders<'_>) -> Result<S> {
    // Preallocated to the worst case — one fill per element, the bound the
    // reference's `max_orders` also defaults to — matching the parallel path
    // so the two have the same memory profile. Measured at +19% on a
    // fill-heavy sweep and +6% on a single long column; growing by doubling
    // spends that on realloc and memcpy. Costs memory on a sparse run, where
    // it measures as noise either way.
    check_groups(args)?;
    let mut sink = S::with_capacity(args.n_rows * args.n_cols);
    // One counter across every group, because they share one buffer. See
    // [`simulate_group`].
    let mut fills = 0_i64;
    for group in 0..args.groups.len() {
        simulate_group(args, group, &mut sink, &mut fills)?;
    }
    Ok(sink)
}

/// Simulates every column on one thread.
///
/// # Errors
///
/// Propagates any invariant violation from the execution kernel, and any
/// rejection on a column whose `raise_reject` is set.
pub fn simulate_from_orders(args: &FromOrders<'_>) -> Result<OrderRecords> {
    simulate_into(args)
}

/// Simulates every column, keeping the logs of the orders that asked for one.
///
/// Identical fills to [`simulate_from_orders`] — the logs are additional, never
/// alternative, which is what lets an investigation re-run a workload with
/// logging on and compare. A test asserts it.
///
/// Nothing is logged unless `args.log` says so somewhere, so calling this with
/// the default flags returns a journal with an empty log half. That is the
/// intended way to log one column of a sweep: leave the entry point in place
/// and switch the flag.
///
/// # Errors
///
/// As [`simulate_from_orders`].
pub fn simulate_from_orders_logged(args: &FromOrders<'_>) -> Result<Journal> {
    simulate_into(args)
}

/// Simulates every group across `n_threads`, producing byte-identical output.
///
/// Groups are split into contiguous chunks, each thread fills its own
/// buffers, and the buffers are concatenated in column order afterwards. No
/// shared counter, no locking, and therefore no dependence on how the threads
/// interleave — `simulate_from_orders` and this function return equal records
/// for the same input, which is asserted in the tests.
///
/// Uses `std::thread::scope` rather than a work-stealing pool. That is partly
/// because the crates.io mirror is unavailable in this environment, and partly
/// because it is a genuine test of the architecture: scoped threads require
/// the whole argument set to be `Sync` and every column's state to be owned
/// locally, which is exactly the constraint `docs/ARCHITECTURE.md §11.2` imposes.
/// Swapping in a real pool later is a change to this function alone.
///
/// # Errors
///
/// As [`simulate_from_orders`]. If several threads error, the one from the
/// lowest column chunk is returned, so the failure does not depend on timing.
pub fn simulate_from_orders_parallel(
    args: &FromOrders<'_>,
    n_threads: usize,
) -> Result<OrderRecords> {
    simulate_into_parallel(args, n_threads)
}

/// Simulates every group across `n_threads`, keeping logs.
///
/// Output equals [`simulate_from_orders_logged`]'s for the same input, logs
/// included — which needs one thing the unlogged path does not: each thread
/// numbers its fills from zero, so the merge re-bases every `order_id` onto the
/// combined records. [`Journal::concat`] is where that happens.
///
/// # Errors
///
/// As [`simulate_from_orders_parallel`].
pub fn simulate_from_orders_logged_parallel(
    args: &FromOrders<'_>,
    n_threads: usize,
) -> Result<Journal> {
    simulate_into_parallel(args, n_threads)
}

/// Simulates every group across `n_threads` into any sink.
///
/// # Errors
///
/// As [`simulate_from_orders`].
fn simulate_into_parallel<S: SinkBuffer>(args: &FromOrders<'_>, n_threads: usize) -> Result<S> {
    // The parallel axis is the group, not the column. With the default
    // grouping the two are the same thing; with one shared portfolio there is
    // a single unit of work and this degrades to the serial path, which is
    // inherent rather than an artefact — a shared cash balance is a
    // sequential dependency across the whole group.
    check_groups(args)?;
    let n_groups = args.groups.len();
    let n_threads = n_threads.max(1).min(n_groups.max(1));
    if n_threads == 1 {
        return simulate_into(args);
    }

    // Contiguous chunks keep each thread's columns adjacent in memory and
    // make the merge an ordered append.
    let chunk = n_groups.div_ceil(n_threads);
    let ranges: Vec<(usize, usize)> = (0..n_groups)
        .step_by(chunk)
        .map(|start| (start, (start + chunk).min(n_groups)))
        .collect();

    let results: Vec<Result<S>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges
            .iter()
            .map(|&(start, end)| {
                scope.spawn(move || {
                    // Preallocate for the worst case — one fill per element,
                    // which is what the reference's `max_orders` also defaults
                    // to. Growing these buffers by doubling puts every thread
                    // in the allocator on a fill-heavy sweep, and measurably
                    // costs scaling: 1.46x -> 1.70x on four cores just from
                    // this. Memory is the trade, and it is bounded by the
                    // shape the caller already chose.
                    let n_cols: usize = (start..end).map(|g| args.groups.len_of(g)).sum();
                    let mut sink = S::with_capacity(args.n_rows * n_cols);
                    // Per buffer, so ids are thread-independent; the merge
                    // re-bases them. A shared counter here would make the
                    // numbering depend on interleaving.
                    let mut fills = 0_i64;
                    for group in start..end {
                        simulate_group(args, group, &mut sink, &mut fills)?;
                    }
                    Ok(sink)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::Invariant("simulation thread panicked".into())))
            })
            .collect()
    });

    // Report the lowest-numbered failure so errors are deterministic.
    let mut parts = Vec::with_capacity(results.len());
    for result in results {
        parts.push(result?);
    }
    Ok(S::merge(&parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five bars, one column, prices 1..5.
    fn close() -> Vec<f64> {
        vec![1.0, 2.0, 3.0, 4.0, 5.0]
    }

    #[test]
    fn buy_and_hold_spends_everything_on_the_first_bar() {
        // The reference's own doc example: with default size (+inf) and price
        // (+inf, meaning the close), all cash goes in at the first close.
        let close = close();
        let args = FromOrders {
            n_rows: 5,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(100.0),
            close: FlexArray::full(&close, 5, 1).unwrap(),
            ..Default::default()
        };
        let records = simulate_from_orders(&args).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records.idx, [0]);
        assert_eq!(records.size, [100.0]);
        assert_eq!(records.price, [1.0]);
    }

    #[test]
    fn negative_infinite_price_uses_the_previous_close() {
        // -inf means the previous bar's close, so the first bar has no price
        // and cannot fill.
        let close = close();
        let args = FromOrders {
            n_rows: 5,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(100.0),
            close: FlexArray::full(&close, 5, 1).unwrap(),
            price: FlexArray::Scalar(f64::NEG_INFINITY),
            size: FlexArray::Scalar(1.0),
            ..Default::default()
        };
        let records = simulate_from_orders(&args).unwrap();
        // Bar 0 has no previous close; bars 1..4 fill at closes 1..4.
        assert_eq!(records.idx, [1, 2, 3, 4]);
        assert_eq!(records.price, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn columns_are_independent() {
        // Two columns with different starting cash must not interfere.
        let close: Vec<f64> = [close(), close()].concat();
        let init = [100.0, 10.0];
        let args = FromOrders {
            n_rows: 5,
            n_cols: 2,
            groups: Groups::ungrouped(2),
            init_cash: FlexArray::per_column(&init, 2).unwrap(),
            close: FlexArray::full(&close, 5, 2).unwrap(),
            ..Default::default()
        };
        let records = simulate_from_orders(&args).unwrap();
        assert_eq!(records.col, [0, 1]);
        assert_eq!(records.size, [100.0, 10.0]);
    }

    #[test]
    fn parallel_matches_serial_exactly() {
        // The reproducibility claim: same records, same order, same ids,
        // regardless of thread count.
        let n_rows = 64;
        let n_cols = 17; // deliberately not a multiple of any thread count
        let mut close = Vec::with_capacity(n_rows * n_cols);
        for col in 0..n_cols {
            for i in 0..n_rows {
                close.push(10.0 + (i as f64) * 0.5 + (col as f64));
            }
        }
        let sizes: Vec<f64> = (0..n_rows * n_cols)
            .map(|k| if k % 3 == 0 { 1.0 } else { -0.5 })
            .collect();

        let args = FromOrders {
            n_rows,
            n_cols,
            groups: Groups::ungrouped(n_cols),
            init_cash: FlexArray::Scalar(1000.0),
            close: FlexArray::full(&close, n_rows, n_cols).unwrap(),
            size: FlexArray::full(&sizes, n_rows, n_cols).unwrap(),
            fees: FlexArray::Scalar(0.001),
            ..Default::default()
        };

        let serial = simulate_from_orders(&args).unwrap();
        assert!(!serial.is_empty(), "test data produced no fills");

        for threads in [2, 3, 4, 8, 16] {
            let parallel = simulate_from_orders_parallel(&args, threads).unwrap();
            assert_eq!(
                serial, parallel,
                "thread count {threads} changed the records"
            );
            assert_eq!(serial.ids(), parallel.ids());
        }
    }

    #[test]
    fn a_log_says_why_a_bar_did_not_trade() {
        // The question logs exist for. Without them the only evidence is that
        // no order record exists, which is consistent with a dozen causes.
        let close = close();
        let sizes = [1.0, 1000.0, 1.0, 0.0, 1.0];
        let args = FromOrders {
            n_rows: 5,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(10.0),
            close: FlexArray::full(&close, 5, 1).unwrap(),
            size: FlexArray::full(&sizes, 5, 1).unwrap(),
            log: FlexArray::Scalar(true),
            ..Default::default()
        };

        // Through the public entry point, so the test covers what a caller
        // reaches rather than the loop underneath it.
        let journal = simulate_from_orders_logged(&args).unwrap();

        // One log per bar, whether or not anything filled — that is what
        // separates "considered and refused" from "never considered".
        assert_eq!(journal.logs.len(), 5);
        let reasons = journal.logs.reasons();
        assert_eq!(reasons[0], "filled");
        assert_eq!(
            reasons[3],
            barq_model::StatusInfo::SizeZero.describe(),
            "a zero size is a refusal with a reason, not a silent absence"
        );
        // A thousand units on ten of cash fills *partially* rather than being
        // refused, because `allow_partial` defaults on — which is itself
        // worth seeing in a log, since the order record alone shows a fill and
        // says nothing about the size that was asked for.
        assert_eq!(reasons[1], "filled");
        assert!(
            journal.logs.res_size[1] < journal.logs.req_size[1],
            "the partial fill is invisible without the requested size beside it"
        );
        // And with partial fills off, the same bar is a refusal with a reason.
        let strict = FromOrders {
            allow_partial: FlexArray::Scalar(false),
            ..FromOrders {
                n_rows: 5,
                n_cols: 1,
                groups: Groups::ungrouped(1),
                init_cash: FlexArray::Scalar(10.0),
                close: FlexArray::full(&close, 5, 1).unwrap(),
                size: FlexArray::full(&sizes, 5, 1).unwrap(),
                log: FlexArray::Scalar(true),
                ..Default::default()
            }
        };
        let refused = simulate_from_orders_logged(&strict).unwrap();
        assert_ne!(refused.logs.reasons()[1], "filled");

        // Every filled log joins back to its order record, and every unfilled
        // one says it has none.
        for i in 0..journal.logs.len() {
            let filled = journal.logs.res_status[i] == barq_model::OrderStatus::Filled as i64;
            assert_eq!(
                journal.logs.order_id[i] >= 0,
                filled,
                "log {i} disagrees with its own status about having an order"
            );
        }
        let ids: Vec<i64> = journal
            .logs
            .order_id
            .iter()
            .copied()
            .filter(|v| *v >= 0)
            .collect();
        assert_eq!(ids, journal.orders.ids(), "the join is not the identity");
    }

    #[test]
    fn logging_does_not_change_what_filled() {
        // Logging is additional, not alternative. An investigation that
        // changed the run would be investigating something else.
        let close = close();
        let sizes = [1.0, 1000.0, 1.0, 0.0, 1.0];
        let base = FromOrders {
            n_rows: 5,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(10.0),
            close: FlexArray::full(&close, 5, 1).unwrap(),
            size: FlexArray::full(&sizes, 5, 1).unwrap(),
            ..Default::default()
        };
        let quiet = simulate_from_orders(&base).unwrap();

        let loud = FromOrders {
            log: FlexArray::Scalar(true),
            ..base
        };
        let journal = simulate_from_orders_logged(&loud).unwrap();
        assert_eq!(journal.orders, quiet);
    }

    /// Three columns, one order each on a different bar, all logged.
    ///
    /// Every column fills exactly once, so the join from logs to orders is the
    /// identity and any re-basing mistake shows up as a wrong index rather than
    /// as a length or range error.
    fn one_fill_per_column() -> Vec<f64> {
        vec![
            1.0, 0.0, 0.0, // column 0 buys on bar 0
            0.0, 1.0, 0.0, // column 1 on bar 1
            0.0, 0.0, 1.0, // column 2 on bar 2
        ]
    }

    #[test]
    fn order_id_points_at_the_right_fill_across_groups() {
        // The bug this pins: the fill counter used to reset per group while all
        // the groups wrote into one buffer, so column 1's log claimed order 0 —
        // column 0's fill, on a different bar, with a plausible size. In range,
        // wrong trade, and nothing downstream could notice.
        let close = vec![10.0, 20.0, 30.0, 10.0, 20.0, 30.0, 10.0, 20.0, 30.0];
        let sizes = one_fill_per_column();
        let args = FromOrders {
            n_rows: 3,
            n_cols: 3,
            groups: Groups::ungrouped(3),
            init_cash: FlexArray::Scalar(1000.0),
            close: FlexArray::full(&close, 3, 3).unwrap(),
            size: FlexArray::full(&sizes, 3, 3).unwrap(),
            log: FlexArray::Scalar(true),
            ..Default::default()
        };
        let journal = simulate_from_orders_logged(&args).unwrap();

        assert_eq!(journal.orders.len(), 3, "one fill per column");
        assert_eq!(journal.logs.len(), 9, "one log per element, filled or not");

        // Each log that filled must name the record holding *its own* column
        // and bar. Checking the pointed-at record rather than the number is
        // what makes this a test of the join instead of a copy of the code.
        for i in 0..journal.logs.len() {
            let id = journal.logs.order_id[i];
            if id < 0 {
                continue;
            }
            let at = id as usize;
            assert_eq!(
                journal.orders.col[at], journal.logs.col[i],
                "log {i} joins to a fill in another column"
            );
            assert_eq!(
                journal.orders.idx[at], journal.logs.idx[i],
                "log {i} joins to a fill on another bar"
            );
        }
    }

    #[test]
    fn a_logged_parallel_run_equals_the_serial_one() {
        // Including the logs, which is the part that needs the merge to re-base
        // ids: each thread numbers its fills from zero.
        let close: Vec<f64> = (0..3 * 8).map(|i| 10.0 + (i % 3) as f64 * 5.0).collect();
        let sizes: Vec<f64> = (0..3 * 8)
            .map(|i| if i % 4 == 0 { 1.0 } else { 0.0 })
            .collect();
        let args = FromOrders {
            n_rows: 3,
            n_cols: 8,
            groups: Groups::ungrouped(8),
            init_cash: FlexArray::Scalar(1000.0),
            close: FlexArray::full(&close, 3, 8).unwrap(),
            size: FlexArray::full(&sizes, 3, 8).unwrap(),
            log: FlexArray::Scalar(true),
            ..Default::default()
        };
        let serial = simulate_from_orders_logged(&args).unwrap();
        assert!(
            serial.orders.len() > 1,
            "nothing filled, so nothing to merge"
        );
        for threads in [2, 4, 8] {
            let parallel = simulate_from_orders_logged_parallel(&args, threads).unwrap();
            assert_eq!(serial.orders, parallel.orders, "{threads} threads");
            // Compared through `Debug` rather than with `==`, because a
            // rejected order's log is mostly `NaN` and `NaN != NaN` — so
            // `Journal`'s derived `PartialEq` is *always false* for any run
            // that rejected anything, including this one. The rendering is
            // exact for the purpose: two `f64`s with the same bits print the
            // same, and differing ones print differently.
            assert_eq!(
                format!("{:?}", serial.logs),
                format!("{:?}", parallel.logs),
                "{threads} threads changed the logs"
            );
        }
    }

    #[test]
    fn the_default_minimum_size_is_the_references_dust_guard() {
        // Not zero, which is what one would write. A `5e-9` order is rejected
        // by the reference and would fill with a zero minimum, so the default
        // is observable and pinned here.
        let close = vec![10.0, 10.0];
        let sizes = [5e-9, 0.0];
        let args = FromOrders {
            n_rows: 2,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(100.0),
            close: FlexArray::full(&close, 2, 1).unwrap(),
            size: FlexArray::full(&sizes, 2, 1).unwrap(),
            log: FlexArray::Scalar(true),
            ..Default::default()
        };
        let journal = simulate_from_orders_logged(&args).unwrap();
        assert!(journal.orders.is_empty(), "dust should not fill");
        assert_eq!(
            journal.logs.reasons()[0],
            barq_model::StatusInfo::MinSizeNotReached.describe()
        );
        #[allow(
            clippy::float_cmp,
            reason = "the log must report the exact value the kernel was given"
        )]
        {
            assert_eq!(journal.logs.req_min_size[0], crate::DEFAULT_MIN_SIZE);
        }

        // And with the minimum turned off it does fill, which is what makes the
        // default a decision rather than a formality.
        let permissive = FromOrders {
            min_size: FlexArray::Scalar(0.0),
            ..args
        };
        assert_eq!(simulate_from_orders(&permissive).unwrap().len(), 1);
    }

    #[test]
    fn raise_reject_turns_a_rejection_into_an_error() {
        let close = close();
        let args = FromOrders {
            n_rows: 5,
            n_cols: 1,
            groups: Groups::ungrouped(1),
            init_cash: FlexArray::Scalar(0.0),
            close: FlexArray::full(&close, 5, 1).unwrap(),
            size: FlexArray::Scalar(1.0),
            raise_reject: FlexArray::Scalar(true),
            ..Default::default()
        };
        assert!(simulate_from_orders(&args).is_err());
    }
}
