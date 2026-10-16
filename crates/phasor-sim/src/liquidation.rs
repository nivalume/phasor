//! Maintenance margin and liquidation.
//!
//! An exchange closes a margin position once the account can no longer carry
//! it: when its equity falls below the maintenance margin of its positions.
//! The loops test every account once per bar, after the bar's prices are
//! resolved and its margin columns settled, before any order is valued or
//! placed, so a position is tested from the bar after it opened — as a stop
//! is.
//!
//! # The test
//!
//! Each column holding a position is given a path through the bar: from its
//! *start*, the bar's open, to its *check* price, the `mark_price` where there
//! is one and otherwise the bar's adverse extreme — the low for a long, the
//! high for a short. Along the paths, moved together by a fraction `α` from 0
//! to 1, the account's equity and its maintenance requirement are both linear
//! in `α`:
//!
//! ```text
//! E(α) = cash + Σ (position × p(α) − owed)
//! M(α) = Σ maintenance_margin × |position| × p(α)       (notional)
//!      + Σ maintenance_margin × |position|              (per unit)
//! ```
//!
//! If `E(1) ≥ M(1)` the account survives the bar. Otherwise it is liquidated
//! where it first breached: at `α = E(0) − M(0)` over `(E(0) − M(0)) − (E(1) −
//! M(1))`, or at the open (`α = 0`) when it was already breached there, the
//! gap a stop fills at too. With one position this is the textbook
//! liquidation price, and the path through the bar is what makes the fill
//! price exact rather than the bar's extreme.
//!
//! Without a mark price every column is tested at its own extreme, which
//! treats a hedge as if both legs could move against the account at once: a
//! spot long and a perpetual short reach their lows and highs at different
//! times. A hedged account wants a mark price — the close, if nothing else —
//! which puts every column at one instant.
//!
//! In `from_signals`, a stop that fires on the bar was hit on the way to the
//! extreme: its column is checked at the less adverse of its check price and
//! the stop's level, so a stop inside the liquidation price closes the
//! position first, and one beyond it does not save it.
//!
//! # Cross and isolated
//!
//! A **cross** margin column shares the account's equity: the test above runs
//! over the account, with every cross column that carries a maintenance margin
//! closed together, in layout order, at its own point of the path. Cash
//! columns count in the equity and are never closed: they are collateral.
//!
//! An **isolated** column carries only its own margin. It is not settled into
//! the account's cash each bar, so its profit or loss is its own, and it is
//! tested alone: `E(α) = locked_cash + position × p(α) − owed`. Its margin is
//! all it can lose. The account's test leaves it out, margin and position
//! both.
//!
//! # What a liquidation leaves behind
//!
//! The fill is an order record like any other, with `fees` at
//! `liquidation_fee` of its notional, flagged as a liquidation in the records
//! and the logs. A liquidation that leaves an isolated column's margin
//! negative, or a cross account with no cross or cash position left and less
//! cash than its isolated positions' margin, is a loss the account cannot
//! pay: the exchange's insurance fund takes it. That write-off is a
//! [`CashFlowKind::Liquidation`] record, positive, booked to the isolated
//! column or to the account's first column.

use phasor_core::error::{Error, Result};
use phasor_core::flex::FlexArray;
use phasor_core::math::{add, is_less};
use phasor_exec::{
    Direction, ExecuteOrderState, Order, ProcessOrderState, SizeType, ValidatedOrder,
    execute_validated,
};
use phasor_model::{CashFlowKind, MarginBasis};
use phasor_records::{Balances, LogEntry, LogSink, RecordSink};

use crate::cash::is_off;

/// The inputs of the test, borrowed from a loop's arguments.
#[derive(Clone, Copy)]
pub(crate) struct Inputs<'s, 'a> {
    pub(crate) maintenance_margin: &'s FlexArray<'a, f64>,
    pub(crate) liquidation_fee: &'s FlexArray<'a, f64>,
    pub(crate) mark_price: &'s FlexArray<'a, f64>,
    pub(crate) open: &'s FlexArray<'a, f64>,
    pub(crate) high: &'s FlexArray<'a, f64>,
    pub(crate) low: &'s FlexArray<'a, f64>,
    pub(crate) close: &'s FlexArray<'a, f64>,
}

impl Inputs<'_, '_> {
    /// Whether no column can ever carry a maintenance margin.
    pub(crate) fn off(&self) -> bool {
        is_off(self.maintenance_margin)
    }
}

/// A column as the test reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Column {
    /// Signed position.
    pub(crate) position: f64,
    /// What is subtracted from `position × price` to leave the column's
    /// share of the equity: a cash column's long debt, a margin column's
    /// signed cost basis.
    pub(crate) owed: f64,
    /// The initial margin the position posted.
    pub(crate) locked_cash: f64,
    /// Whether it is a margin column.
    pub(crate) margin: bool,
    /// Whether it is an isolated margin column.
    pub(crate) isolated: bool,
    /// What its maintenance margin is measured in.
    pub(crate) basis: MarginBasis,
    /// Its last valuation price, the price of last resort.
    pub(crate) val_price: f64,
    /// The level a stop that fired on the bar was hit at; `NaN` otherwise.
    pub(crate) stop: f64,
}

/// A column's path through the bar and what it owes the maintenance margin.
#[derive(Debug, Clone, Copy)]
struct Path {
    start: f64,
    check: f64,
    /// The maintenance margin per unit of notional, or per unit of size;
    /// zero for a column that carries none.
    rate: f64,
    unit: bool,
}

impl Path {
    fn price(&self, alpha: f64) -> f64 {
        self.start + alpha * (self.check - self.start)
    }

    fn requirement(&self, position: f64, price: f64) -> f64 {
        if self.rate == 0.0 {
            0.0
        } else if self.unit {
            self.rate * position.abs()
        } else {
            self.rate * position.abs() * price
        }
    }
}

/// Reads one column's maintenance margin and rejects what no account can
/// meet. `NaN` and zero carry none.
fn rate(inputs: &Inputs<'_, '_>, row: usize, col: usize, basis: MarginBasis) -> Result<f64> {
    let rate = inputs.maintenance_margin.get(row, col);
    if rate.is_nan() {
        return Ok(0.0);
    }
    let bad = !rate.is_finite() || rate < 0.0 || (basis == MarginBasis::Notional && rate >= 1.0);
    if bad {
        return Err(Error::Invariant(format!(
            "maintenance_margin must be NaN, or finite, 0 or greater and below 1 of the \
             notional; got {rate} at row {row}, column {col}"
        )));
    }
    Ok(rate)
}

/// The path of a column holding a position through bar `row`.
fn path(inputs: &Inputs<'_, '_>, row: usize, col: usize, c: &Column, rate: f64) -> Path {
    let close = inputs.close.get(row, col);
    let mut open = inputs.open.get(row, col);
    if open.is_nan() {
        open = close;
    }
    let long = c.position > 0.0;
    let mut check = inputs.mark_price.get(row, col);
    if check.is_nan() {
        check = if long {
            let low = inputs.low.get(row, col);
            if low.is_nan() { open.min(close) } else { low }
        } else {
            let high = inputs.high.get(row, col);
            if high.is_nan() { open.max(close) } else { high }
        };
    }
    if check.is_nan() {
        check = c.val_price;
    }
    if !c.stop.is_nan() {
        // Hit on the way: the less adverse of the two.
        check = if long {
            check.max(c.stop)
        } else {
            check.min(c.stop)
        };
    }
    let start = if open.is_nan() { check } else { open };
    Path {
        start,
        check,
        rate,
        unit: c.basis == MarginBasis::Unit,
    }
}

/// Where along the paths an account first breaches, or `None` if it does
/// not.
///
/// `base` is what the account holds besides the positions in `held`: its
/// cash, or an isolated column's margin.
fn breach(base: f64, held: &[Held]) -> Option<f64> {
    let surplus = |alpha: f64| {
        let (mut equity, mut required) = (base, 0.0);
        for h in held {
            let price = h.path.price(alpha);
            equity += h.position * price - h.owed;
            required += h.path.requirement(h.position, price);
        }
        (equity, required)
    };
    let (e1, m1) = surplus(1.0);
    if !is_less(e1, m1) {
        return None;
    }
    let (e0, m0) = surplus(0.0);
    let (g0, g1) = (e0 - m0, e1 - m1);
    Some(if g0 <= 0.0 { 0.0 } else { g0 / (g0 - g1) })
}

/// A column holding a position, on its path through the bar.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Held {
    k: usize,
    position: f64,
    owed: f64,
    path: Path,
}

/// Tests the account of `cols` at bar `row`, writing each column to close and
/// the price it closes at into `out`, in layout order.
///
/// `column(k)` reads the `k`-th column after the bar's settlement. A cross
/// account with a held column that has no price at all is not tested: its
/// equity is unknown.
///
/// # Errors
///
/// Returns [`Error::Invariant`] for a maintenance margin no account can meet.
pub(crate) fn test(
    inputs: &Inputs<'_, '_>,
    row: usize,
    cols: std::ops::Range<usize>,
    column: impl Fn(usize) -> Column,
    cash: f64,
    held: &mut Vec<Held>,
    out: &mut Vec<(usize, f64)>,
) -> Result<()> {
    out.clear();
    held.clear();
    let mut base = cash;
    let mut cross_due = false;
    let mut priced = true;
    for (k, col) in cols.enumerate() {
        let c = column(k);
        if c.position == 0.0 {
            continue;
        }
        let rate = if c.margin {
            rate(inputs, row, col, c.basis)?
        } else {
            0.0
        };
        let h = Held {
            k,
            position: c.position,
            owed: c.owed,
            path: path(inputs, row, col, &c, rate),
        };
        if c.isolated {
            // Its margin sits in the account's cash but is not the account's
            // to use; its position is its own.
            base -= c.locked_cash;
            if rate > 0.0 && !h.path.check.is_nan() {
                if let Some(alpha) = breach(c.locked_cash, &[h]) {
                    out.push((k, h.path.price(alpha)));
                }
            }
            continue;
        }
        priced &= !h.path.check.is_nan();
        cross_due |= rate > 0.0;
        held.push(h);
    }
    if cross_due && priced {
        if let Some(alpha) = breach(base, held) {
            out.extend(
                held.iter()
                    .filter(|h| h.path.rate > 0.0)
                    .map(|h| (h.k, h.path.price(alpha))),
            );
            out.sort_by_key(|&(k, _)| k);
        }
    }
    Ok(())
}

/// Where a liquidation happens and what it reads.
pub(crate) struct At<'o> {
    pub(crate) group: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
    /// The column's own request for the bar, whose margin parameters and log
    /// flag the liquidation keeps.
    pub(crate) template: &'o Order,
    pub(crate) isolated: bool,
}

/// Closes a column at `price`, charging `liquidation_fee` of the notional,
/// and records the fill as a liquidation.
///
/// `state` is the column and its account before; the returned state is after.
/// The fill and its balances go to `sink`, and a log row if the column logs.
///
/// # Errors
///
/// Returns [`Error::Invariant`] for a fee that is not finite and 0 or greater,
/// and anything the execution kernel refuses.
pub(crate) fn fill<S: RecordSink + LogSink>(
    inputs: &Inputs<'_, '_>,
    at: &At<'_>,
    price: f64,
    state: &ProcessOrderState,
    sink: &mut S,
    fills: &mut i64,
) -> Result<ExecuteOrderState> {
    let fee = inputs.liquidation_fee.get(at.row, at.col);
    if !fee.is_finite() || fee < 0.0 {
        return Err(Error::Invariant(format!(
            "liquidation_fee must be finite and 0 or greater; got {fee} at row {}, column {}",
            at.row, at.col
        )));
    }
    let request = Order {
        size: 0.0,
        price,
        size_type: SizeType::TargetAmount,
        direction: Direction::Both,
        fees: fee,
        fixed_fees: 0.0,
        slippage: 0.0,
        min_size: 0.0,
        max_size: f64::INFINITY,
        size_granularity: f64::NAN,
        cash_limit: f64::INFINITY,
        reject_prob: 0.0,
        lock_cash: false,
        allow_partial: true,
        raise_reject: false,
        ..*at.template
    };
    let validated = ValidatedOrder::new(request).in_levered_account();
    let (after, result) = execute_validated(state, &validated, None)?;
    let mut order_id = -1_i64;
    if result.is_filled() {
        let side = result.side.expect("a filled order always carries a side");
        sink.push_order(at.row, at.col, result.size, result.price, result.fees, side);
        sink.push_balances(Balances {
            debt: after.debt,
            locked_cash: after.locked_cash,
            margin: true,
            isolated: at.isolated,
            liquidation: true,
        });
        order_id = *fills;
        *fills += 1;
    }
    if request.log {
        sink.push_log(&LogEntry {
            group: at.group,
            col: at.col,
            idx: at.row,
            cash: state.cash,
            position: state.position,
            debt: state.debt,
            locked_cash: state.locked_cash,
            free_cash: state.free_cash,
            val_price: state.val_price,
            value: state.value,
            request,
            new_cash: after.cash,
            new_position: after.position,
            new_debt: after.debt,
            new_locked_cash: after.locked_cash,
            new_free_cash: after.free_cash,
            new_val_price: state.val_price,
            // The position's profit at the valuation price leaves the value;
            // what the fill realized and paid arrives in cash.
            new_value: state.value
                - crate::margin::unsettled(state.position, state.debt, state.val_price)
                + (after.cash - state.cash),
            result,
            order_id,
            liquidation: true,
        });
    }
    Ok(after)
}

/// Writes off a loss the account cannot pay: books `−shortfall` back into
/// `cash` and `free_cash` as a [`CashFlowKind::Liquidation`] record on `col`.
/// Nothing when there is no shortfall.
pub(crate) fn write_off<S: RecordSink>(
    shortfall: f64,
    row: usize,
    col: usize,
    cash: &mut f64,
    free_cash: &mut f64,
    sink: &mut S,
) {
    if shortfall < 0.0 {
        let amount = -shortfall;
        *cash = add(*cash, amount);
        *free_cash = add(*free_cash, amount);
        sink.push_cash(row, col, amount, CashFlowKind::Liquidation);
    }
}

/// Writes off what a cross liquidation left the account owing, once it holds
/// no cross or cash position: its cash below the margin its isolated
/// positions still hold, which is theirs and not the account's to lose.
///
/// `columns` is each column's `(position, locked_cash, isolated)` after the
/// liquidation. Nothing while any other position is open: the account still
/// has something to pay with.
pub(crate) fn write_off_account<S: RecordSink>(
    balance: f64,
    columns: impl Iterator<Item = (f64, f64, bool)>,
    row: usize,
    col: usize,
    cash: &mut f64,
    free_cash: &mut f64,
    sink: &mut S,
) {
    let mut reserved = 0.0;
    for (position, locked_cash, isolated) in columns {
        if isolated {
            reserved += locked_cash;
        } else if position != 0.0 {
            return;
        }
    }
    write_off(balance - reserved, row, col, cash, free_cash, sink);
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the prices and balances here are exact in binary"
)]
mod tests {
    use super::*;
    use phasor_exec::AccountType;
    use phasor_records::{Journal, OrderRecords};

    const NAN: FlexArray<'static, f64> = FlexArray::Scalar(f64::NAN);

    /// One bar of one column: open, high, low, close and a mark.
    struct Bar {
        open: FlexArray<'static, f64>,
        high: FlexArray<'static, f64>,
        low: FlexArray<'static, f64>,
        close: FlexArray<'static, f64>,
        mark: FlexArray<'static, f64>,
        mm: FlexArray<'static, f64>,
        fee: FlexArray<'static, f64>,
    }

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            open: FlexArray::Scalar(open),
            high: FlexArray::Scalar(high),
            low: FlexArray::Scalar(low),
            close: FlexArray::Scalar(close),
            mark: NAN,
            mm: FlexArray::Scalar(0.25),
            fee: FlexArray::Scalar(0.0),
        }
    }

    impl Bar {
        fn inputs(&self) -> Inputs<'_, 'static> {
            Inputs {
                maintenance_margin: &self.mm,
                liquidation_fee: &self.fee,
                mark_price: &self.mark,
                open: &self.open,
                high: &self.high,
                low: &self.low,
                close: &self.close,
            }
        }
    }

    /// A cross margin column holding `position` against a signed basis.
    fn margin(position: f64, owed: f64) -> Column {
        Column {
            position,
            owed,
            locked_cash: 0.0,
            margin: true,
            isolated: false,
            basis: MarginBasis::Notional,
            val_price: f64::NAN,
            stop: f64::NAN,
        }
    }

    fn run(bar: &Bar, cash: f64, cols: &[Column]) -> Vec<(usize, f64)> {
        let (mut held, mut out) = (Vec::new(), Vec::new());
        test(
            &bar.inputs(),
            0,
            0..cols.len(),
            |k| cols[k],
            cash,
            &mut held,
            &mut out,
        )
        .unwrap();
        out
    }

    // Long 4 against a basis of 400 with 100 of cash: E(p) = 4p − 300 and,
    // at a quarter of the notional, M(p) = p. The account breaches below 100.

    #[test]
    fn a_long_is_liquidated_where_its_path_crosses_the_maintenance_margin() {
        // From the open at 128 toward the low at 64: E − M runs from 84 to
        // −108 and is zero 84/192 of the way, at 100.
        let long = margin(4.0, 400.0);
        assert_eq!(
            run(&bar(128.0, 130.0, 64.0, 70.0), 100.0, &[long]),
            [(0, 100.0)]
        );
        // A bar whose low stays above 100 does not breach.
        assert!(run(&bar(128.0, 130.0, 101.0, 110.0), 100.0, &[long]).is_empty());
    }

    #[test]
    fn a_bar_that_opens_through_the_level_liquidates_at_the_open() {
        let long = margin(4.0, 400.0);
        assert_eq!(
            run(&bar(96.0, 97.0, 64.0, 70.0), 100.0, &[long]),
            [(0, 96.0)]
        );
    }

    #[test]
    fn a_short_is_tested_at_the_high() {
        // Short 4 against 400 with 700 of cash: E(p) = 1100 − 4p, M(p) = p,
        // breached above 220. From 200 to the high at 300 it crosses at 220.
        let short = margin(-4.0, -400.0);
        assert_eq!(
            run(&bar(200.0, 300.0, 150.0, 210.0), 700.0, &[short]),
            [(0, 220.0)]
        );
    }

    #[test]
    fn a_unit_maintenance_margin_is_cash_per_unit() {
        // 25 a unit on 4 units is 100 whatever the price: E(p) = 4p − 300
        // falls to it at 100.
        let long = Column {
            basis: MarginBasis::Unit,
            ..margin(4.0, 400.0)
        };
        let b = Bar {
            mm: FlexArray::Scalar(25.0),
            ..bar(128.0, 130.0, 64.0, 70.0)
        };
        assert_eq!(run(&b, 100.0, &[long]), [(0, 100.0)]);
    }

    #[test]
    fn a_mark_price_replaces_the_extreme() {
        let long = margin(4.0, 400.0);
        let b = Bar {
            mark: FlexArray::Scalar(110.0),
            ..bar(128.0, 130.0, 64.0, 70.0)
        };
        assert!(run(&b, 100.0, &[long]).is_empty());
    }

    #[test]
    fn a_stop_inside_the_level_saves_the_position_and_one_beyond_it_does_not() {
        let b = bar(128.0, 130.0, 64.0, 70.0);
        let stopped = |stop| Column {
            stop,
            ..margin(4.0, 400.0)
        };
        assert!(run(&b, 100.0, &[stopped(120.0)]).is_empty());
        // Hit at 96, past the level: the path from 128 to 96 crosses 100.
        assert_eq!(run(&b, 100.0, &[stopped(96.0)]), [(0, 100.0)]);
    }

    #[test]
    fn an_isolated_column_carries_only_its_own_margin() {
        // The same long, isolated with 100 of margin, in an account holding
        // 10,000: the account's cash does not save it.
        let iso = Column {
            isolated: true,
            locked_cash: 100.0,
            ..margin(4.0, 400.0)
        };
        assert_eq!(
            run(&bar(128.0, 130.0, 64.0, 70.0), 10_000.0, &[iso]),
            [(0, 100.0)]
        );
        // And its margin is not the account's: a cross long beside it, which
        // 300 of the account's cash would carry, fails with 100 of it gone.
        let b = Bar {
            open: FlexArray::Scalar(128.0),
            high: FlexArray::Scalar(130.0),
            low: FlexArray::Scalar(101.0),
            close: FlexArray::Scalar(110.0),
            ..bar(0.0, 0.0, 0.0, 0.0)
        };
        let cross = margin(4.0, 400.0);
        assert!(run(&b, 100.0, &[cross]).is_empty());
        let out = run(
            &b,
            100.0,
            &[
                Column {
                    locked_cash: 100.0,
                    ..iso
                },
                cross,
            ],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 1);
    }

    #[test]
    fn a_hedge_survives_at_one_mark_and_not_at_both_extremes() {
        // A spot long bought for cash and a perpetual short of 4 on a basis
        // of 512, with 128 of cash: at one mark the legs cancel, at their
        // own extremes they do not.
        let spot = Column {
            margin: false,
            ..margin(4.0, 0.0)
        };
        let perp = margin(-4.0, -512.0);
        let two = |v: [f64; 2]| -> FlexArray<'static, f64> {
            FlexArray::PerColumn(Box::leak(Box::new(v)))
        };
        let extremes = Bar {
            open: two([128.0, 128.0]),
            high: two([192.0, 192.0]),
            low: two([64.0, 64.0]),
            close: two([128.0, 128.0]),
            ..bar(0.0, 0.0, 0.0, 0.0)
        };
        assert_eq!(run(&extremes, 128.0, &[spot, perp]).len(), 1);
        let marked = Bar {
            mark: two([192.0, 192.0]),
            ..extremes
        };
        assert!(run(&marked, 128.0, &[spot, perp]).is_empty());
    }

    #[test]
    fn a_maintenance_margin_no_account_can_meet_is_an_error() {
        let long = margin(4.0, 400.0);
        for bad in [1.0, -0.1, f64::INFINITY] {
            let b = Bar {
                mm: FlexArray::Scalar(bad),
                ..bar(128.0, 130.0, 64.0, 70.0)
            };
            let (mut held, mut out) = (Vec::new(), Vec::new());
            let err = test(&b.inputs(), 0, 0..1, |_| long, 100.0, &mut held, &mut out);
            assert!(err.is_err(), "{bad}");
        }
        let none = Bar {
            mm: NAN,
            ..bar(128.0, 130.0, 64.0, 70.0)
        };
        assert!(run(&none, 100.0, &[long]).is_empty());
    }

    #[test]
    fn a_liquidation_fills_at_its_price_pays_its_fee_and_is_flagged() {
        let b = Bar {
            fee: FlexArray::Scalar(0.125),
            ..bar(128.0, 130.0, 64.0, 70.0)
        };
        let template = Order {
            account_type: AccountType::Margin,
            initial_margin: 0.25,
            log: true,
            ..Order::default()
        };
        let at = At {
            group: 0,
            row: 3,
            col: 0,
            template: &template,
            isolated: false,
        };
        // Long 4 settled at 128 with 100 posted.
        let before = ProcessOrderState {
            cash: 100.0,
            position: 4.0,
            debt: 512.0,
            locked_cash: 100.0,
            free_cash: 0.0,
            val_price: 128.0,
            value: 100.0,
            oidx: 0,
            lidx: 0,
        };
        let mut journal = Journal::default();
        let mut fills = 0;
        let after = fill(&b.inputs(), &at, 100.0, &before, &mut journal, &mut fills).unwrap();
        // Sold 4 at 100: −112 realized, 50 in fees, the 100 margin released.
        assert_eq!(after.position, 0.0);
        assert_eq!(after.cash, 100.0 - 112.0 - 50.0);
        assert_eq!(after.free_cash, after.cash);
        assert_eq!(journal.orders.price, [100.0]);
        assert_eq!(journal.orders.fees, [50.0]);
        assert_eq!(journal.orders.liquidation, [true]);
        assert_eq!(journal.orders.margin, [true]);
        assert_eq!(journal.logs.liquidation, [true]);
        assert_eq!(journal.logs.order_id, [0]);
        assert_eq!(fills, 1);

        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (after.cash, after.free_cash);
        write_off(cash, 3, 0, &mut cash, &mut free, &mut sink);
        assert_eq!((cash, free), (0.0, 0.0));
        assert_eq!(sink.cash.amount, [62.0]);
        assert_eq!(sink.cash.kind, [CashFlowKind::Liquidation as i8]);
        write_off(5.0, 3, 0, &mut cash, &mut free, &mut sink);
        assert_eq!(sink.cash.len(), 1, "nothing to write off");

        let bad = Bar {
            fee: FlexArray::Scalar(-0.1),
            ..bar(128.0, 130.0, 64.0, 70.0)
        };
        assert!(fill(&bad.inputs(), &at, 100.0, &before, &mut journal, &mut fills).is_err());
    }
}
