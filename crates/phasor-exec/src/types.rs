//! State, request and result types for order execution.

use phasor_model::{Direction, OrderSide, OrderStatus, SizeType, StatusInfo};

/// Account state that an order execution reads and rewrites.
///
/// `debt` and `free_cash` are the two fields that are easy to get wrong.
///
/// - `debt` is the proceeds of open short sales, held at their average entry
///   price. It is what a short position "owes" back.
/// - `free_cash` is cash net of the margin those shorts tie up. Opening or
///   extending a short moves it by `proceeds - 2 * short_value`: once for the
///   position's own value and once for the round-trip reservation. Closing
///   unwinds it symmetrically through `debt / |position|`.
///
/// Carrying only `cash` and `position` and deriving the rest does not work —
/// the reservation is path-dependent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExecuteOrderState {
    /// Cash on hand.
    pub cash: f64,
    /// Signed position size; negative is short.
    pub position: f64,
    /// Proceeds of open shorts, at average entry price.
    pub debt: f64,
    /// Cash net of short margin reservation.
    pub free_cash: f64,
}

/// [`ExecuteOrderState`] plus the bookkeeping the simulation loop maintains.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessOrderState {
    /// Cash on hand.
    pub cash: f64,
    /// Signed position size.
    pub position: f64,
    /// Proceeds of open shorts.
    pub debt: f64,
    /// Cash net of short margin reservation.
    pub free_cash: f64,
    /// Price used to value the position.
    pub val_price: f64,
    /// Total portfolio value.
    pub value: f64,
    /// Index of the next order record slot.
    pub oidx: usize,
    /// Index of the next log record slot.
    pub lidx: usize,
}

impl ProcessOrderState {
    /// A flat state with the given cash, valued at `val_price`.
    #[must_use]
    pub const fn flat(cash: f64, val_price: f64) -> Self {
        Self {
            cash,
            position: 0.0,
            debt: 0.0,
            free_cash: cash,
            val_price,
            value: cash,
            oidx: 0,
            lidx: 0,
        }
    }
}

/// A requested order.
///
/// Defaults mirror the reference engine: an infinite size means "as much as
/// possible" and an infinite price means "use the bar's close". Those
/// infinities are live sentinels, which is why [`phasor_core::math::is_close`]
/// refuses to call two infinities equal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Order {
    /// Requested size, interpreted per `size_type`.
    pub size: f64,
    /// Requested price; `inf` means the close.
    pub price: f64,
    /// How to interpret `size`.
    pub size_type: SizeType,
    /// Which sides this column may take.
    pub direction: Direction,
    /// Proportional fee rate.
    pub fees: f64,
    /// Flat fee per fill.
    pub fixed_fees: f64,
    /// Proportional slippage, applied against the trade direction.
    pub slippage: f64,
    /// Reject fills below this size.
    pub min_size: f64,
    /// Clamp fills to this size.
    pub max_size: f64,
    /// Round sizes down to a multiple of this.
    pub size_granularity: f64,
    /// The most cash one buy may spend, fees included. `inf` is no cap.
    ///
    /// Applied to the cash available before `percent` takes its share, so
    /// `Percent(0.5)` under a cap of 100 spends at most 50. Sells and shorts
    /// receive cash rather than spending it and are not affected. Not in the
    /// reference, so not in the log schema either.
    pub cash_limit: f64,
    /// Probability of a random rejection, in `[0, 1]`.
    ///
    /// When non-zero the caller must supply the random draw; see
    /// [`crate::execute_order`].
    pub reject_prob: f64,
    /// Whether to reserve cash to cover an existing short.
    pub lock_cash: bool,
    /// Whether a partial fill is acceptable.
    pub allow_partial: bool,
    /// Whether a rejection should be an error instead of a status.
    pub raise_reject: bool,
    /// Whether to emit a log record.
    pub log: bool,
}

impl Default for Order {
    fn default() -> Self {
        Self {
            size: f64::INFINITY,
            price: f64::INFINITY,
            size_type: SizeType::Amount,
            direction: Direction::Both,
            fees: 0.0,
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
            log: false,
        }
    }
}

/// The outcome of an order request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrderResult {
    /// Filled size; `NaN` if nothing filled.
    pub size: f64,
    /// Fill price; `NaN` if nothing filled.
    pub price: f64,
    /// Fees charged; `NaN` if nothing filled.
    pub fees: f64,
    /// Which side filled.
    pub side: Option<OrderSide>,
    /// Filled, ignored or rejected.
    pub status: OrderStatus,
    /// Why, when not filled.
    pub status_info: Option<StatusInfo>,
}

impl OrderResult {
    /// An outcome where nothing filled.
    #[must_use]
    pub const fn not_filled(status: OrderStatus, info: StatusInfo) -> Self {
        Self {
            size: f64::NAN,
            price: f64::NAN,
            fees: f64::NAN,
            side: None,
            status,
            status_info: Some(info),
        }
    }

    /// A filled outcome.
    #[must_use]
    pub const fn filled(size: f64, price: f64, fees: f64, side: OrderSide) -> Self {
        Self {
            size,
            price,
            fees,
            side: Some(side),
            status: OrderStatus::Filled,
            status_info: None,
        }
    }

    /// Whether this order actually traded.
    #[must_use]
    pub const fn is_filled(&self) -> bool {
        matches!(self.status, OrderStatus::Filled)
    }
}
