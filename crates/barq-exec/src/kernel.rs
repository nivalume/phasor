//! The order execution kernel.
//!
//! A faithful port of the reference vectorized engine's semantics. Where the
//! two could differ, this file follows the reference, because bit-identical
//! fills against it are the acceptance criterion (see `docs/ARCHITECTURE.md §14`).
//! Places where that produces un-idiomatic Rust are commented rather than
//! tidied up.

use barq_core::error::{Error, Result};
use barq_core::math::{add, is_close, is_close_or_less, is_less};
use barq_model::{Direction, OrderSide, OrderStatus, SizeType, StatusInfo};

use crate::types::{ExecuteOrderState, Order, OrderResult, ProcessOrderState};

/// Outcome of a buy/sell leg: either the new state and a fill, or a
/// non-filling result that leaves the state untouched.
type Leg = (ExecuteOrderState, OrderResult);

/// Three-way sign with `numpy` semantics: both zeros and `NaN` give `0.0`.
///
/// Used only to turn a percentage into a signed infinity. `NaN` mapping to
/// `0.0` is not a deviation: `0.0 * inf` is `NaN`, which is what the
/// reference's `sign(NaN) * inf` produces too.
#[inline]
fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Floor to a multiple of `granularity`, matching Python's `a // g * g`.
#[inline]
fn floor_to(value: f64, granularity: f64) -> f64 {
    (value / granularity).floor() * granularity
}

/// Buys, or covers an existing short.
///
/// Comparisons against zero here are exact, not tolerance-based, matching the
/// reference; `is_close` at these sites changes which orders fill.
///
/// `size` is positive. `percent`, when not `NaN`, caps the spend at that
/// fraction of the available cash.
fn buy(
    state: ExecuteOrderState,
    size: f64,
    price: f64,
    order: &Order,
    percent: f64,
) -> Result<Leg> {
    let adj_price = price * (1.0 + order.slippage);

    // How much cash this order may spend.
    let mut cash_limit = if order.lock_cash {
        if state.position >= 0.0 {
            // In a long position cash == free_cash, unless another column has
            // locked some of it.
            state.free_cash
        } else {
            // Covering the short costs this much; what is left over is what a
            // new long may use.
            let cover_req_cash =
                state.position.abs() * adj_price * (1.0 + order.fees) + order.fixed_fees;
            let cover_free_cash = add(state.free_cash + 2.0 * state.debt, -cover_req_cash);
            if cover_free_cash > 0.0 {
                // Enough to close the short and open a long.
                state.free_cash + 2.0 * state.debt
            } else if cover_free_cash < 0.0 {
                // Not enough to close the short: solve for the part that can
                // be covered. The 2 * avg_entry_price term is the reservation
                // released by shrinking the short.
                let avg_entry_price = state.debt / state.position.abs();
                let max_short_size = (state.free_cash - order.fixed_fees)
                    / (adj_price * (1.0 + order.fees) - 2.0 * avg_entry_price);
                max_short_size * adj_price * (1.0 + order.fees) + order.fixed_fees
            } else {
                // Exactly enough to close the short.
                state.cash
            }
        }
    } else {
        state.cash
    };
    cash_limit = cash_limit.min(state.cash);
    if !percent.is_nan() {
        cash_limit = cash_limit.min(percent * cash_limit);
    }

    if matches!(order.direction, Direction::LongOnly | Direction::Both) {
        if cash_limit == 0.0 {
            return Ok((
                state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::NoCashLong),
            ));
        }
        if size.is_infinite() && cash_limit.is_infinite() {
            return Err(Error::Invariant(
                "attempt to go long infinitely: both size and available cash are infinite".into(),
            ));
        }
    } else if state.position == 0.0 {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::NoOpenPosition),
        ));
    }

    // In a short-only column a buy can only cover, never go long.
    let mut adj_size = if order.direction == Direction::ShortOnly {
        (-state.position).min(size)
    } else {
        size
    };

    if adj_size == 0.0 {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::SizeZero),
        ));
    }

    if adj_size > order.max_size {
        if !order.allow_partial {
            return Ok((
                state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::MaxSizeExceeded),
            ));
        }
        adj_size = order.max_size;
    }

    if !order.size_granularity.is_nan() {
        adj_size = floor_to(adj_size, order.size_granularity);
    }

    let req_cash = adj_size * adj_price;
    let req_fees = req_cash * order.fees + order.fixed_fees;
    let total_req_cash = req_cash + req_fees;

    let (final_size, fees_paid, final_req_cash) = if is_close_or_less(total_req_cash, cash_limit) {
        (adj_size, req_fees, total_req_cash)
    } else {
        // Not enough cash. Fees are inside the cap, not added on top, so the
        // affordable notional has to be back-solved: at 10% fees and $1 fixed,
        // $100 of cash buys $90 of stock.
        let max_req_cash = add(cash_limit, -order.fixed_fees) / (1.0 + order.fees);
        if max_req_cash <= 0.0 {
            return Ok((
                state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::CantCoverFees),
            ));
        }
        let max_acq_size = max_req_cash / adj_price;

        if order.size_granularity.is_nan() {
            (max_acq_size, cash_limit - max_req_cash, cash_limit)
        } else {
            // Granularity is applied a second time here, after the cash
            // constraint, so lot discipline survives a partial fill.
            let final_size = floor_to(max_acq_size, order.size_granularity);
            let new_req_cash = final_size * adj_price;
            let fees_paid = new_req_cash * order.fees + order.fixed_fees;
            (final_size, fees_paid, new_req_cash + fees_paid)
        }
    };

    // Note this tests `adj_size`, not `final_size` — matching the reference.
    if is_close(adj_size, 0.0) {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::SizeZero),
        ));
    }
    if is_less(final_size, order.min_size) {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::MinSizeNotReached),
        ));
    }
    // An infinite request cannot be "partial", by definition.
    if size.is_finite() && is_less(final_size, size) && !order.allow_partial {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::PartialFill),
        ));
    }

    let new_cash = add(state.cash, -final_req_cash);
    let new_position = add(state.position, final_size);

    let (new_debt, new_free_cash) = if state.position < 0.0 {
        // Covering releases the reservation proportionally to the part closed.
        let short_size = if new_position < 0.0 {
            final_size
        } else {
            state.position.abs()
        };
        let avg_entry_price = state.debt / state.position.abs();
        let debt_diff = short_size * avg_entry_price;
        (
            add(state.debt, -debt_diff),
            add(state.free_cash + 2.0 * debt_diff, -final_req_cash),
        )
    } else {
        (state.debt, add(state.free_cash, -final_req_cash))
    };

    Ok((
        ExecuteOrderState {
            cash: new_cash,
            position: new_position,
            debt: new_debt,
            free_cash: new_free_cash,
        },
        OrderResult::filled(final_size, adj_price, fees_paid, OrderSide::Buy),
    ))
}

/// Sells, or opens/extends a short.
///
/// As in [`buy`], comparisons against zero are exact by design.
///
/// `size` is positive.
fn sell(
    state: ExecuteOrderState,
    size: f64,
    price: f64,
    order: &Order,
    mut percent: f64,
) -> Result<Leg> {
    let adj_price = price * (1.0 - order.slippage);

    let mut size_limit = if order.direction == Direction::LongOnly {
        state.position.min(size)
    } else if order.lock_cash || (size.is_infinite() && !percent.is_nan()) {
        // Work out the largest position that can be sold and shorted.
        let long_size = state.position.max(0.0);
        let long_cash = long_size * adj_price * (1.0 - order.fees);
        let total_free_cash = add(state.free_cash, long_cash);

        let max_size_limit = if total_free_cash <= 0.0 {
            if state.position <= 0.0 {
                // Nothing to sell and no free cash to short with.
                return Ok((
                    state,
                    OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::NoCashShort),
                ));
            }
            // A position to close, but no free cash to short beyond it.
            long_size
        } else {
            let max_short_size =
                add(total_free_cash, -order.fixed_fees) / (adj_price * (1.0 + order.fees));
            let limit = add(long_size, max_short_size);
            if limit <= 0.0 {
                return Ok((
                    state,
                    OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::CantCoverFees),
                ));
            }
            limit
        };

        if order.lock_cash {
            // The percentage is consumed here rather than applied again below.
            if size.is_infinite() && !percent.is_nan() {
                let limit = (percent * max_size_limit).min(max_size_limit);
                percent = f64::NAN;
                limit
            } else if !percent.is_nan() {
                let limit = (percent * size).min(max_size_limit);
                percent = f64::NAN;
                limit
            } else {
                size.min(max_size_limit)
            }
        } else {
            // Reached only when size is infinite and a percentage is set.
            max_size_limit
        }
    } else {
        size
    };

    if !percent.is_nan() {
        size_limit *= percent;
    }

    if size_limit > order.max_size {
        if !order.allow_partial {
            return Ok((
                state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::MaxSizeExceeded),
            ));
        }
        size_limit = order.max_size;
    }

    if matches!(order.direction, Direction::ShortOnly | Direction::Both) {
        if size_limit.is_infinite() {
            return Err(Error::Invariant(
                "attempt to go short infinitely: the sellable size is unbounded".into(),
            ));
        }
    } else if state.position == 0.0 {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::NoOpenPosition),
        ));
    }

    if !order.size_granularity.is_nan() {
        size_limit = floor_to(size_limit, order.size_granularity);
    }

    if is_close(size_limit, 0.0) {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::SizeZero),
        ));
    }
    if is_less(size_limit, order.min_size) {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::MinSizeNotReached),
        ));
    }
    if size.is_finite() && is_less(size_limit, size) && !order.allow_partial {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::PartialFill),
        ));
    }

    let acq_cash = size_limit * adj_price;
    let fees_paid = acq_cash * order.fees + order.fixed_fees;
    let final_acq_cash = add(acq_cash, -fees_paid);
    if final_acq_cash < 0.0 {
        return Ok((
            state,
            OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::CantCoverFees),
        ));
    }

    let new_cash = state.cash + final_acq_cash;
    let new_position = add(state.position, -size_limit);

    let (new_debt, new_free_cash) = if new_position < 0.0 {
        // Only the part that ends up short creates debt and ties up margin.
        let short_size = if state.position < 0.0 {
            size_limit
        } else {
            new_position.abs()
        };
        let short_value = short_size * adj_price;
        // The doubled short value is the round-trip reservation.
        let free_cash_diff = add(final_acq_cash, -2.0 * short_value);
        (
            state.debt + short_value,
            add(state.free_cash, free_cash_diff),
        )
    } else {
        (state.debt, state.free_cash + final_acq_cash)
    };

    Ok((
        ExecuteOrderState {
            cash: new_cash,
            position: new_position,
            debt: new_debt,
            free_cash: new_free_cash,
        },
        OrderResult::filled(size_limit, adj_price, fees_paid, OrderSide::Sell),
    ))
}

/// Validates the inputs the kernel's arithmetic depends on.
///
/// These are invariant violations, not rejections — see the crate docs. Two
/// checks the reference needs are absent here because the type system already
/// makes them unrepresentable: `size_type` and `direction` are enums, so they
/// cannot hold an out-of-range discriminant.
/// The seven checks that read only the bar: state, price and direction.
fn validate_bar(state: &ExecuteOrderState, order: &Order, position: f64) -> Result<()> {
    let bail = |msg: &str| Err(Error::Invariant(msg.to_owned()));

    if state.cash.is_nan() || state.cash < 0.0 {
        return bail("cash cannot be NaN and must be 0 or greater");
    }
    if !position.is_finite() {
        return bail("position must be finite");
    }
    if !state.debt.is_finite() || state.debt < 0.0 {
        return bail("debt must be finite and 0 or greater");
    }
    if state.free_cash.is_nan() {
        return bail("free_cash cannot be NaN");
    }

    if !order.price.is_finite() || order.price <= 0.0 {
        return bail("order.price must be finite and greater than 0");
    }
    if order.direction == Direction::LongOnly && position < 0.0 {
        return bail("position is negative but order.direction is LongOnly");
    }
    if order.direction == Direction::ShortOnly && position > 0.0 {
        return bail("position is positive but order.direction is ShortOnly");
    }
    Ok(())
}

/// The seven checks that read only fields a whole column usually shares.
///
/// Separated from [`validate_bar`] so a caller with column-constant
/// parameters can run them once instead of once per bar — see
/// [`ValidatedOrder`]. Every check in either half is independent of the
/// others, so splitting them changes no outcome; what it must not change is
/// which message a doubly-invalid order reports, and the call site keeps the
/// two halves in their original order for that reason.
///
/// Returns the message rather than an [`Error`] so that a [`ValidatedOrder`]
/// stays `Copy` and one pointer wide.
fn param_violation(order: &Order) -> Option<&'static str> {
    if !order.fees.is_finite() {
        return Some("order.fees must be finite");
    }
    if !order.fixed_fees.is_finite() {
        return Some("order.fixed_fees must be finite");
    }
    if !order.slippage.is_finite() || order.slippage < 0.0 {
        return Some("order.slippage must be finite and 0 or greater");
    }
    if !order.min_size.is_finite() || order.min_size < 0.0 {
        return Some("order.min_size must be finite and 0 or greater");
    }
    if order.max_size.is_nan() || order.max_size <= 0.0 {
        return Some("order.max_size must be greater than 0");
    }
    // NaN is the "no granularity" sentinel and passes both tests.
    if order.size_granularity.is_infinite() || order.size_granularity <= 0.0 {
        return Some("order.size_granularity must be NaN, or finite and greater than 0");
    }
    if !order.reject_prob.is_finite() || !(0.0..=1.0).contains(&order.reject_prob) {
        return Some("order.reject_prob must be between 0 and 1");
    }
    None
}

/// An [`Order`] whose column-invariant parameters have already been checked.
///
/// Seven of the fourteen checks [`execute_order`] runs look only at fees,
/// slippage, size bounds, granularity and rejection probability — fields a
/// whole column normally shares. A bar that produces no fill still pays for
/// all seven, and on a realistic strategy most bars produce no fill. Checking
/// them once per column instead is worth 12–15% there.
///
/// Build one per column, then call [`set_bar`](Self::set_bar) each bar with
/// the four fields the parameter checks do not cover. Rebuild it when a
/// parameter genuinely varies down the column.
///
/// # Why construction cannot fail
///
/// The reference ignores a `NaN` size *before* it validates anything, so a
/// column of `NaN` sizes and an invalid fee rate must produce ignored orders,
/// not an error. Failing eagerly here would change that. The violation is
/// therefore carried and raised at the exact point the per-bar check would
/// have raised it — which costs one branch instead of seven.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValidatedOrder {
    order: Order,
    violation: Option<&'static str>,
}

impl ValidatedOrder {
    /// Checks `order`'s column-invariant parameters.
    #[must_use]
    pub fn new(order: Order) -> Self {
        Self {
            violation: param_violation(&order),
            order,
        }
    }

    /// Overwrites the four fields that vary bar to bar.
    ///
    /// None of them is covered by the parameter checks, so the validation
    /// this type carries stays sound.
    #[inline]
    pub const fn set_bar(
        &mut self,
        size: f64,
        price: f64,
        size_type: SizeType,
        direction: Direction,
    ) {
        self.order.size = size;
        self.order.price = price;
        self.order.size_type = size_type;
        self.order.direction = direction;
    }

    /// The underlying request.
    #[must_use]
    pub const fn order(&self) -> &Order {
        &self.order
    }

    /// Reports a parameter violation now rather than at the next execution.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invariant`] if a parameter check failed.
    pub fn check(&self) -> Result<()> {
        match self.violation {
            Some(msg) => Err(Error::Invariant(msg.to_owned())),
            None => Ok(()),
        }
    }
}

/// Executes one order against one state.
///
/// Pure and deterministic. When `order.reject_prob` is non-zero the caller
/// supplies `reject_draw`, a sample from `U(0, 1)`; the order is rejected when
/// that draw is below the probability. Inverting control this way — rather
/// than reaching for a global RNG as the reference does — is what lets a
/// parameter sweep seed each column reproducibly and still run in parallel.
///
/// # Errors
///
/// Returns [`Error::Invariant`] when an input violates something the
/// arithmetic depends on, or when `reject_prob` is non-zero and no draw was
/// supplied. A *rejected* order is not an error; it comes back as an
/// [`OrderResult`] with [`OrderStatus::Rejected`].
pub fn execute_order(
    state: &ProcessOrderState,
    order: &Order,
    reject_draw: Option<f64>,
) -> Result<Leg> {
    execute_validated(state, &ValidatedOrder::new(*order), reject_draw)
}

/// [`execute_order`], with the parameter checks already done.
///
/// Identical in every observable way — same fills, same rejections, same
/// error at the same point. It exists only so a column loop can hoist seven
/// checks out of itself; see [`ValidatedOrder`].
///
/// # Errors
///
/// As [`execute_order`].
pub fn execute_validated(
    state: &ProcessOrderState,
    validated: &ValidatedOrder,
    reject_draw: Option<f64>,
) -> Result<Leg> {
    let order = &validated.order;
    // Numerical stability: snap everything that should be zero to zero before
    // any comparison sees it. Residue here keeps closed positions nominally
    // open and makes affordability tests flap.
    let snap = |v: f64| if is_close(v, 0.0) { 0.0 } else { v };
    let cash = snap(state.cash);
    let position = snap(state.position);
    let debt = snap(state.debt);
    let free_cash = snap(state.free_cash);
    let val_price = snap(state.val_price);
    let value = snap(state.value);

    let exec_state = ExecuteOrderState {
        cash,
        position,
        debt,
        free_cash,
    };

    // Ignore checks come before validation: a NaN size is ignored even when
    // the state would otherwise be rejected.
    if order.size.is_nan() {
        return Ok((
            exec_state,
            OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::SizeNaN),
        ));
    }
    if order.price.is_nan() {
        return Ok((
            exec_state,
            OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::PriceNaN),
        ));
    }

    // The bar half first, then the parameter half: that is the order the
    // checks ran in when they were one function, and it decides which message
    // an order that violates both reports.
    validate_bar(&exec_state, order, position)?;
    validated.check()?;

    if order.reject_prob > 0.0 && reject_draw.is_none() {
        return Err(Error::Invariant(
            "order.reject_prob is non-zero but no random draw was supplied".into(),
        ));
    }

    // The size cascade. Order matters: each stage rewrites the size type, and
    // only `Amount` reaches the buy/sell dispatch. See docs/ARCHITECTURE.md §7.3.
    let mut order_size = order.size;
    let mut size_type = order.size_type;

    if order.direction == Direction::ShortOnly {
        // In a short-only column a positive request means "short more".
        order_size = -order_size;
    }

    if size_type == SizeType::TargetPercent {
        if value.is_nan() {
            return Ok((
                exec_state,
                OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::ValueNaN),
            ));
        }
        if value <= 0.0 {
            return Ok((
                exec_state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::ValueZeroNeg),
            ));
        }
        order_size *= value;
        size_type = SizeType::TargetValue;
    }

    if matches!(size_type, SizeType::Value | SizeType::TargetValue) {
        if val_price.is_infinite() || val_price <= 0.0 {
            return Err(Error::Invariant(
                "val_price must be finite and greater than 0".into(),
            ));
        }
        if val_price.is_nan() {
            return Ok((
                exec_state,
                OrderResult::not_filled(OrderStatus::Ignored, StatusInfo::ValPriceNaN),
            ));
        }
        order_size /= val_price;
        size_type = if size_type == SizeType::Value {
            SizeType::Amount
        } else {
            SizeType::TargetAmount
        };
    }

    if size_type == SizeType::TargetAmount {
        order_size -= position;
        size_type = SizeType::Amount;
    }

    if size_type == SizeType::Amount
        && matches!(order.direction, Direction::ShortOnly | Direction::Both)
        && order_size.is_infinite()
        && order_size < 0.0
    {
        // Negative infinity has a special meaning: short 100% of resources.
        order_size = -1.0;
        size_type = SizeType::Percent;
    }

    let mut percent = f64::NAN;
    if size_type == SizeType::Percent {
        percent = order_size.abs();
        // A zero request yields `0.0 * inf` == NaN here, exactly as upstream.
        order_size = sign(order_size) * f64::INFINITY;
    }

    // Note zero and NaN both take the sell branch, as in the reference.
    let (new_state, result) = if order_size > 0.0 {
        buy(exec_state, order_size, order.price, order, percent)?
    } else {
        sell(exec_state, -order_size, order.price, order, percent)?
    };

    // Random rejection discards the fill and restores the original state.
    if order.reject_prob > 0.0 {
        let draw = reject_draw.unwrap_or(f64::NAN);
        if draw < order.reject_prob {
            return Ok((
                exec_state,
                OrderResult::not_filled(OrderStatus::Rejected, StatusInfo::RandomEvent),
            ));
        }
    }

    Ok((new_state, result))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "these assert on exact expected balances (50.0, 48.5, 0.0); an \
              approximate assertion here would not catch the arithmetic errors \
              the tests exist to catch"
)]
mod tests {
    use super::*;

    fn flat() -> ProcessOrderState {
        ProcessOrderState::flat(100.0, 10.0)
    }

    #[test]
    fn plain_buy_spends_cash_and_takes_a_position() {
        let order = Order {
            size: 5.0,
            price: 10.0,
            ..Default::default()
        };
        let (state, result) = execute_order(&flat(), &order, None).unwrap();
        assert_eq!(state.cash, 50.0);
        assert_eq!(state.position, 5.0);
        assert!(result.is_filled());
        assert_eq!(result.side, Some(OrderSide::Buy));
    }

    #[test]
    fn fees_come_out_of_the_cash_cap_not_on_top() {
        // 5 @ 10 = 50 notional, 1% = 0.5, plus 1 fixed => 48.5 left.
        let order = Order {
            size: 5.0,
            price: 10.0,
            fees: 0.01,
            fixed_fees: 1.0,
            ..Default::default()
        };
        let (state, result) = execute_order(&flat(), &order, None).unwrap();
        assert_eq!(result.fees, 1.5);
        assert_eq!(state.cash, 48.5);
    }

    #[test]
    fn a_bad_parameter_is_an_error() {
        let order = Order {
            size: 5.0,
            price: 10.0,
            fees: f64::INFINITY,
            ..Default::default()
        };
        assert!(execute_order(&flat(), &order, None).is_err());
    }

    #[test]
    fn a_nan_size_is_ignored_even_when_a_parameter_is_bad() {
        // The whole reason `ValidatedOrder` carries its violation instead of
        // returning it: hoisting the parameter checks out of a column loop
        // must not turn this column's ignored orders into an error.
        let order = Order {
            size: f64::NAN,
            price: 10.0,
            fees: f64::INFINITY,
            ..Default::default()
        };
        let (_, result) = execute_order(&flat(), &order, None).unwrap();
        assert_eq!(result.status, OrderStatus::Ignored);
        assert_eq!(result.status_info, Some(StatusInfo::SizeNaN));
    }

    #[test]
    fn a_bad_state_outranks_a_bad_parameter() {
        // Both halves are violated. The bar half ran first before the split
        // and must still, or the error message changes.
        let mut state = flat();
        state.cash = -1.0;
        let order = Order {
            size: 5.0,
            price: 10.0,
            fees: f64::INFINITY,
            ..Default::default()
        };
        let err = execute_order(&state, &order, None).unwrap_err();
        assert!(format!("{err}").contains("cash"));
    }

    #[test]
    fn set_bar_reuses_the_validation() {
        let mut v = ValidatedOrder::new(Order {
            fees: 0.01,
            fixed_fees: 1.0,
            ..Default::default()
        });
        v.set_bar(5.0, 10.0, SizeType::Amount, Direction::Both);
        let (hoisted, _) = execute_validated(&flat(), &v, None).unwrap();

        let (inline, _) = execute_order(
            &flat(),
            &Order {
                size: 5.0,
                price: 10.0,
                fees: 0.01,
                fixed_fees: 1.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(hoisted, inline);
    }

    #[test]
    fn rejection_is_a_status_not_an_error() {
        let order = Order {
            size: 5.0,
            price: 10.0,
            ..Default::default()
        };
        let broke = ProcessOrderState::flat(0.0, 10.0);
        let (state, result) = execute_order(&broke, &order, None).unwrap();
        assert_eq!(result.status, OrderStatus::Rejected);
        assert_eq!(result.status_info, Some(StatusInfo::NoCashLong));
        // State is untouched by a rejection.
        assert_eq!(state.cash, 0.0);
        assert_eq!(state.position, 0.0);
    }

    #[test]
    fn invariant_violation_is_an_error() {
        let order = Order {
            size: 5.0,
            price: 10.0,
            ..Default::default()
        };
        let bad = ProcessOrderState {
            cash: -1.0,
            ..ProcessOrderState::flat(0.0, 10.0)
        };
        assert!(execute_order(&bad, &order, None).is_err());
    }

    #[test]
    fn nan_size_is_ignored_even_when_the_state_is_invalid() {
        // The ignore checks run before validation.
        let order = Order {
            size: f64::NAN,
            price: 10.0,
            ..Default::default()
        };
        let bad = ProcessOrderState {
            cash: -1.0,
            ..ProcessOrderState::flat(0.0, 10.0)
        };
        let (_, result) = execute_order(&bad, &order, None).unwrap();
        assert_eq!(result.status, OrderStatus::Ignored);
        assert_eq!(result.status_info, Some(StatusInfo::SizeNaN));
    }

    #[test]
    fn reject_prob_requires_a_draw() {
        let order = Order {
            size: 5.0,
            price: 10.0,
            reject_prob: 0.5,
            ..Default::default()
        };
        assert!(execute_order(&flat(), &order, None).is_err());

        // Draw below the probability rejects and restores the state.
        let (state, result) = execute_order(&flat(), &order, Some(0.1)).unwrap();
        assert_eq!(result.status_info, Some(StatusInfo::RandomEvent));
        assert_eq!(state.cash, 100.0);

        // Draw above it fills.
        let (state, result) = execute_order(&flat(), &order, Some(0.9)).unwrap();
        assert!(result.is_filled());
        assert_eq!(state.cash, 50.0);
    }

    #[test]
    fn shorting_records_debt_and_reserves_margin() {
        let order = Order {
            size: -5.0,
            price: 10.0,
            ..Default::default()
        };
        let (state, result) = execute_order(&flat(), &order, None).unwrap();
        assert!(result.is_filled());
        assert_eq!(state.position, -5.0);
        // Proceeds land in cash, debt records them, and free_cash is reduced
        // by the round-trip reservation: 100 + 50 - 2*50 = 50.
        assert_eq!(state.cash, 150.0);
        assert_eq!(state.debt, 50.0);
        assert_eq!(state.free_cash, 50.0);
    }

    #[test]
    fn sign_matches_numpy_for_zero_and_nan() {
        assert_eq!(sign(3.0), 1.0);
        assert_eq!(sign(-3.0), -1.0);
        assert_eq!(sign(0.0), 0.0);
        assert_eq!(sign(-0.0), 0.0);
        assert_eq!(sign(f64::NAN), 0.0);
        // Which matters because this is what it feeds:
        assert!((sign(0.0) * f64::INFINITY).is_nan());
    }
}
