//! What holding a position costs or earns at each bar's close.
//!
//! Four charges, booked per column after the bar's orders and its
//! `cash_earnings`, on the position the orders left:
//!
//! | Input | Charged on | Amount | Recorded as |
//! |---|---|---|---|
//! | `funding_rate` | any position | `−position × price × rate`: longs pay a positive rate, shorts receive it | [`CashFlowKind::Funding`] |
//! | `long_financing_rate`, `short_financing_rate` | any position | `−|position| × price × rate`, the rate for the position's side | [`CashFlowKind::Financing`] |
//! | `debt_rate` | a cash column's long | `−debt × rate`: what the long borrowed | [`CashFlowKind::Interest`] |
//! | `borrow_rate` | a cash column's short | `−|position| × price × rate` | [`CashFlowKind::BorrowFee`] |
//!
//! `price` is `mark_price`, or the close where it is `NaN`, or the column's
//! last valuation price where both are; with none of them the charge is
//! skipped, as settling is. A positive rate is a cost to the holder.
//!
//! Every rate is **per bar**: what one bar's holding costs, for the time
//! until the next bar. A bar with no settlement is `NaN` or zero. Turning an
//! annual rate into one per bar needs the bars' timestamps, which the engine
//! does not have; the Python layer does it (`phasor.rates.per_bar`).
//!
//! A perpetual's funding is the case where the short's rate is minus the
//! long's, so it is one input; financing has two, because a broker's rates
//! for the two sides differ and both can be costs. Interest and the borrow fee
//! are the two halves of a cash account's leverage: a long that borrowed cash
//! pays on the cash, a short that borrowed shares pays on their value. A margin
//! column borrows neither, so neither applies to it.
//!
//! **A charge is never capped.** It is owed whatever the balance, unlike a
//! `cash_earnings` charge, which stops at the free cash. Any of these inputs
//! therefore runs the account as one that may borrow — the loops' `LEV` copy
//! — so cash may go below zero, after which the account can close but not
//! open. That also keeps their code out of a run that uses none of them.
//!
//! Like every flow that is not a fill, a charge is recorded with the amount
//! applied, so the analysis layer rebuilds the balance from records alone.

use phasor_core::error::Result;
use phasor_core::flex::FlexArray;
use phasor_core::math::add;
use phasor_model::CashFlowKind;
use phasor_records::RecordSink;

use crate::cash::{is_off, read};

/// The inputs of the charges, borrowed from a loop's arguments.
#[derive(Clone, Copy)]
pub(crate) struct Rates<'s, 'a> {
    pub(crate) funding: &'s FlexArray<'a, f64>,
    pub(crate) long_financing: &'s FlexArray<'a, f64>,
    pub(crate) short_financing: &'s FlexArray<'a, f64>,
    pub(crate) debt: &'s FlexArray<'a, f64>,
    pub(crate) borrow: &'s FlexArray<'a, f64>,
    pub(crate) mark_price: &'s FlexArray<'a, f64>,
    pub(crate) close: &'s FlexArray<'a, f64>,
}

impl Rates<'_, '_> {
    /// Whether no rate can ever charge anything, so a run can skip them all.
    pub(crate) fn all_off(&self) -> bool {
        is_off(self.funding)
            && is_off(self.long_financing)
            && is_off(self.short_financing)
            && is_off(self.debt)
            && is_off(self.borrow)
    }
}

/// What a column holds after the bar's orders, as the charges read it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Held {
    pub(crate) position: f64,
    pub(crate) debt: f64,
    pub(crate) margin: bool,
    pub(crate) val_price: f64,
}

/// Applies `amount`, uncapped, and records it.
fn book<S: RecordSink>(
    amount: f64,
    row: usize,
    col: usize,
    kind: CashFlowKind,
    cash: &mut f64,
    free_cash: &mut f64,
    sink: &mut S,
) {
    // `NaN` only when no price was known; zero when the rate was.
    if amount == 0.0 || amount.is_nan() {
        return;
    }
    *cash = add(*cash, amount);
    *free_cash = add(*free_cash, amount);
    sink.push_cash(row, col, amount, kind);
}

/// Books every column's charges for bar `row`, in layout order, and within a
/// column in the order of the module table.
///
/// `held(k)` is the `k`-th column's holding after the bar's orders.
///
/// # Errors
///
/// Returns [`phasor_core::Error::Invariant`] for an infinite rate.
#[inline(never)]
pub(crate) fn charge<S: RecordSink>(
    rates: &Rates<'_, '_>,
    row: usize,
    cols: std::ops::Range<usize>,
    held: impl Fn(usize) -> Held,
    cash: &mut f64,
    free_cash: &mut f64,
    sink: &mut S,
) -> Result<()> {
    for (k, col) in cols.enumerate() {
        let h = held(k);
        if h.position == 0.0 {
            continue;
        }
        let mut price = rates.mark_price.get(row, col);
        if price.is_nan() {
            price = rates.close.get(row, col);
        }
        if price.is_nan() {
            price = h.val_price;
        }
        let long = h.position > 0.0;

        let funding = read(rates.funding, "funding_rate", row, col)?;
        if funding != 0.0 {
            let amount = -h.position * price * funding;
            book(
                amount,
                row,
                col,
                CashFlowKind::Funding,
                cash,
                free_cash,
                sink,
            );
        }
        let financing = if long {
            read(rates.long_financing, "long_financing_rate", row, col)?
        } else {
            read(rates.short_financing, "short_financing_rate", row, col)?
        };
        if financing != 0.0 {
            let amount = -h.position.abs() * price * financing;
            book(
                amount,
                row,
                col,
                CashFlowKind::Financing,
                cash,
                free_cash,
                sink,
            );
        }
        if h.margin {
            continue;
        }
        if long {
            let rate = read(rates.debt, "debt_rate", row, col)?;
            if rate != 0.0 && h.debt > 0.0 {
                book(
                    -h.debt * rate,
                    row,
                    col,
                    CashFlowKind::Interest,
                    cash,
                    free_cash,
                    sink,
                );
            }
        } else {
            let rate = read(rates.borrow, "borrow_rate", row, col)?;
            if rate != 0.0 {
                let amount = -h.position.abs() * price * rate;
                book(
                    amount,
                    row,
                    col,
                    CashFlowKind::BorrowFee,
                    cash,
                    free_cash,
                    sink,
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "every amount here is an exact product of small binary fractions"
)]
mod tests {
    use super::*;
    use phasor_records::OrderRecords;

    const ZERO: FlexArray<'static, f64> = FlexArray::Scalar(0.0);
    const NAN: FlexArray<'static, f64> = FlexArray::Scalar(f64::NAN);

    fn held(position: f64, debt: f64, margin: bool) -> Held {
        Held {
            position,
            debt,
            margin,
            val_price: 8.0,
        }
    }

    #[test]
    fn funding_is_paid_by_longs_and_received_by_shorts() {
        let rate = FlexArray::Scalar(0.25);
        let close = FlexArray::Scalar(10.0);
        let rates = Rates {
            funding: &rate,
            long_financing: &ZERO,
            short_financing: &ZERO,
            debt: &ZERO,
            borrow: &ZERO,
            mark_price: &NAN,
            close: &close,
        };
        assert!(!rates.all_off());
        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (100.0, 50.0);
        let pos = [2.0, -4.0];
        charge(
            &rates,
            3,
            0..2,
            |k| held(pos[k], 0.0, true),
            &mut cash,
            &mut free,
            &mut sink,
        )
        .unwrap();
        // The long pays 2 × 10 × 0.25, the short receives 4 × 10 × 0.25.
        assert_eq!(sink.cash.amount, [-5.0, 10.0]);
        assert_eq!(sink.cash.kind, [3, 3]);
        assert_eq!(sink.cash.idx, [3, 3]);
        assert_eq!((cash, free), (105.0, 55.0));
    }

    #[test]
    fn financing_takes_the_rate_of_the_positions_side_at_the_mark() {
        let long = FlexArray::Scalar(0.5);
        let short = FlexArray::Scalar(-0.25);
        let mark = FlexArray::Scalar(4.0);
        let close = FlexArray::Scalar(10.0);
        let rates = Rates {
            funding: &ZERO,
            long_financing: &long,
            short_financing: &short,
            debt: &ZERO,
            borrow: &ZERO,
            mark_price: &mark,
            close: &close,
        };
        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (0.0, 0.0);
        let pos = [2.0, -2.0];
        charge(
            &rates,
            0,
            0..2,
            |k| held(pos[k], 0.0, false),
            &mut cash,
            &mut free,
            &mut sink,
        )
        .unwrap();
        // 2 × 4 × 0.5 paid; a negative short rate is received.
        assert_eq!(sink.cash.amount, [-4.0, 2.0]);
        assert_eq!(sink.cash.kind, [4, 4]);
        // Uncapped: the account had nothing and owes the difference.
        assert_eq!(cash, -2.0);
    }

    #[test]
    fn interest_is_on_a_cash_longs_debt_and_the_borrow_fee_on_a_cash_shorts_value() {
        let debt = FlexArray::Scalar(0.125);
        let borrow = FlexArray::Scalar(0.5);
        let rates = Rates {
            funding: &ZERO,
            long_financing: &ZERO,
            short_financing: &ZERO,
            debt: &debt,
            borrow: &borrow,
            mark_price: &NAN,
            close: &NAN,
        };
        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (100.0, 100.0);
        // A cash long owing 40, a cash short, and a margin column on each
        // side, which borrows neither.
        let cols = [
            held(3.0, 40.0, false),
            held(-1.0, 8.0, false),
            held(3.0, 40.0, true),
            held(-1.0, 8.0, true),
        ];
        charge(
            &rates,
            0,
            0..4,
            |k| cols[k],
            &mut cash,
            &mut free,
            &mut sink,
        )
        .unwrap();
        // No mark and no close: the short is valued at its last valuation
        // price, 1 × 8 × 0.5.
        assert_eq!(sink.cash.amount, [-5.0, -4.0]);
        assert_eq!(sink.cash.kind, [5, 6]);
        assert_eq!(sink.cash.col, [0, 1]);
        assert_eq!((cash, free), (91.0, 91.0));
    }

    #[test]
    fn a_flat_column_an_unpriced_one_and_a_nan_rate_charge_nothing() {
        let funding = FlexArray::Scalar(f64::NAN);
        let long = FlexArray::Scalar(0.5);
        let rates = Rates {
            funding: &funding,
            long_financing: &long,
            short_financing: &ZERO,
            debt: &ZERO,
            borrow: &ZERO,
            mark_price: &NAN,
            close: &NAN,
        };
        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (1.0, 1.0);
        let cols = [
            held(0.0, 0.0, false),
            Held {
                position: 1.0,
                debt: 0.0,
                margin: true,
                val_price: f64::NAN,
            },
        ];
        charge(
            &rates,
            0,
            0..2,
            |k| cols[k],
            &mut cash,
            &mut free,
            &mut sink,
        )
        .unwrap();
        assert!(sink.cash.is_empty());
        assert_eq!((cash, free), (1.0, 1.0));
    }

    #[test]
    fn an_infinite_rate_is_an_error_and_zero_rates_are_off() {
        let inf = FlexArray::Scalar(f64::INFINITY);
        let close = FlexArray::Scalar(1.0);
        let rates = Rates {
            funding: &inf,
            long_financing: &ZERO,
            short_financing: &ZERO,
            debt: &ZERO,
            borrow: &ZERO,
            mark_price: &NAN,
            close: &close,
        };
        let mut sink = OrderRecords::new();
        let (mut cash, mut free) = (1.0, 1.0);
        let err = charge(
            &rates,
            0,
            0..1,
            |_| held(1.0, 0.0, true),
            &mut cash,
            &mut free,
            &mut sink,
        );
        assert!(err.is_err());
        let off = Rates {
            funding: &NAN,
            ..rates
        };
        assert!(off.all_off());
    }
}
