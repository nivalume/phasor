//! Execution state-machine enums.
//!
//! Discriminants are pinned because they are written into record arrays and
//! read back by the Python shell and the parity harness. Changing one is a
//! breaking change to the on-disk format, not just to the API.

/// How a requested size should be interpreted.
///
/// These are resolved by a strictly ordered cascade in `barq-exec`, each stage
/// rewriting the size type until only [`SizeType::Amount`] is left. The order
/// matters; see `docs/ARCHITECTURE.md §7.3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum SizeType {
    /// A number of units.
    Amount = 0,
    /// A cash value; divided by the valuation price to get units.
    Value = 1,
    /// A fraction of available cash, in `[0, 1]`.
    Percent = 2,
    /// Target a number of units; the delta against the current position.
    TargetAmount = 3,
    /// Target a cash value.
    TargetValue = 4,
    /// Target a fraction of portfolio value.
    TargetPercent = 5,
}

/// Which side of the market a column may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum Direction {
    /// Long positions only; shorting is rejected.
    LongOnly = 0,
    /// Short positions only.
    ShortOnly = 1,
    /// Both, including reversals through zero.
    Both = 2,
}

/// Which side an order executed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum OrderSide {
    /// Bought.
    Buy = 0,
    /// Sold.
    Sell = 1,
}

/// Outcome of an order request.
///
/// Note that [`OrderStatus::Rejected`] is an ordinary outcome carried in the
/// return value, **not** an `Err`. Errors are reserved for invariant
/// violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum OrderStatus {
    /// Executed, in full or in part.
    Filled = 0,
    /// Nothing to do (no signal, zero size).
    Ignored = 1,
    /// Could not execute; see the accompanying [`StatusInfo`].
    Rejected = 2,
}

/// Why an order was ignored or rejected.
///
/// Fourteen distinct reasons, kept separate because "why did this backtest
/// not trade" is the most common question a user brings to a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum StatusInfo {
    /// Size was `NaN`.
    SizeNaN = 0,
    /// Price was `NaN`.
    PriceNaN = 1,
    /// Valuation price was `NaN`.
    ValPriceNaN = 2,
    /// Portfolio value was `NaN`.
    ValueNaN = 3,
    /// Portfolio value was zero or negative.
    ValueZeroNeg = 4,
    /// Size resolved to zero.
    SizeZero = 5,
    /// Not enough cash to open or extend a short.
    NoCashShort = 6,
    /// Not enough cash to open or extend a long.
    NoCashLong = 7,
    /// Tried to close a position that is not open.
    NoOpenPosition = 8,
    /// Size exceeded the configured maximum.
    MaxSizeExceeded = 9,
    /// Randomly rejected via `reject_prob`.
    RandomEvent = 10,
    /// Proceeds would not cover fees.
    CantCoverFees = 11,
    /// Size fell below the configured minimum.
    MinSizeNotReached = 12,
    /// Partial fill was required but not allowed.
    PartialFill = 13,
}

impl StatusInfo {
    /// Recovers a reason from the discriminant a record stored.
    ///
    /// Records keep the discriminant rather than the enum, because the layout
    /// has to survive a round trip through Arrow or a file. This is the way
    /// back, and it returns `None` for anything that is not a reason — which
    /// includes the `-1` a log writes for an order that filled and therefore
    /// had nothing to explain.
    #[must_use]
    pub const fn from_discriminant(value: i64) -> Option<Self> {
        Some(match value {
            0 => Self::SizeNaN,
            1 => Self::PriceNaN,
            2 => Self::ValPriceNaN,
            3 => Self::ValueNaN,
            4 => Self::ValueZeroNeg,
            5 => Self::SizeZero,
            6 => Self::NoCashShort,
            7 => Self::NoCashLong,
            8 => Self::NoOpenPosition,
            9 => Self::MaxSizeExceeded,
            10 => Self::RandomEvent,
            11 => Self::CantCoverFees,
            12 => Self::MinSizeNotReached,
            13 => Self::PartialFill,
            _ => return None,
        })
    }

    /// A human-readable reason, for reports and error messages.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::SizeNaN => "size is NaN",
            Self::PriceNaN => "price is NaN",
            Self::ValPriceNaN => "valuation price is NaN",
            Self::ValueNaN => "portfolio value is NaN",
            Self::ValueZeroNeg => "portfolio value is zero or negative",
            Self::SizeZero => "size is zero",
            Self::NoCashShort => "not enough cash to short",
            Self::NoCashLong => "not enough cash to go long",
            Self::NoOpenPosition => "no open position to close",
            Self::MaxSizeExceeded => "size exceeds the maximum",
            Self::RandomEvent => "randomly rejected",
            Self::CantCoverFees => "proceeds do not cover fees",
            Self::MinSizeNotReached => "size is below the minimum",
            Self::PartialFill => "partial fill required but not allowed",
        }
    }
}

/// Whether a same-direction signal adds to a position or replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum AccumulationMode {
    /// Signals never change an existing position's size.
    Disabled = 0,
    /// Signals both add to and remove from a position.
    Both = 1,
    /// Signals only add.
    AddOnly = 2,
    /// Signals only remove.
    RemoveOnly = 3,
}

/// How to resolve an entry and an exit firing on the same bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum ConflictMode {
    /// Do nothing.
    Ignore = 0,
    /// The entry wins.
    Entry = 1,
    /// The exit wins.
    Exit = 2,
    /// Whichever is adjacent to the current position wins.
    Adjacent = 3,
    /// Whichever is opposite to the current position wins.
    Opposite = 4,
}

/// How to resolve a long and a short signal firing on the same bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum DirectionConflictMode {
    /// Do nothing.
    Ignore = 0,
    /// Long wins.
    Long = 1,
    /// Short wins.
    Short = 2,
    /// Whichever matches the current position wins.
    Adjacent = 3,
    /// Whichever opposes the current position wins.
    Opposite = 4,
}

/// What an entry signal opposite to the open position should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum OppositeEntryMode {
    /// Do nothing.
    Ignore = 0,
    /// Close the position.
    Close = 1,
    /// Close, then apply accumulation rules.
    CloseReduce = 2,
    /// Close and open the opposite position.
    Reverse = 3,
    /// Reverse, then apply accumulation rules.
    ReverseReduce = 4,
}

/// Which price a stop is anchored to when the position opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum StopEntryPrice {
    /// The valuation price.
    ValPrice = 0,
    /// The requested order price.
    Price = 1,
    /// The actual fill price.
    FillPrice = 2,
    /// The bar's close.
    Close = 3,
}

/// Which price a triggered stop fills at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum StopExitPrice {
    /// The stop price itself, with no slippage.
    StopLimit = 0,
    /// The stop price, plus slippage.
    StopMarket = 1,
    /// The configured order price.
    Price = 2,
    /// The bar's close.
    Close = 3,
}

/// What a triggered stop does to the position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum StopExitMode {
    /// Close it.
    Close = 0,
    /// Close, then apply accumulation rules.
    CloseReduce = 1,
    /// Reverse it.
    Reverse = 2,
    /// Reverse, then apply accumulation rules.
    ReverseReduce = 3,
}

/// Whether a new stop value replaces one that is already live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum StopUpdateMode {
    /// Keep the live stop.
    Keep = 0,
    /// Replace it, unless the new value is `NaN`.
    Override = 1,
    /// Replace it, even with `NaN` (which clears the stop).
    OverrideNaN = 2,
}

/// Which side a trade was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum TradeDirection {
    /// Long.
    Long = 0,
    /// Short.
    Short = 1,
}

/// Whether a trade is still open at the end of the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum TradeStatus {
    /// Still open.
    Open = 0,
    /// Closed.
    Closed = 1,
}

/// Which view of the trade records to present.
///
/// All three share one record schema and differ only in how fills are
/// attributed. Getting this distinction wrong is the most common porting
/// mistake; see `docs/ARCHITECTURE.md §9.5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum TradesType {
    /// One record per entry fill; exits allocated back FIFO.
    EntryTrades = 0,
    /// One record per exit fill; entries averaged into it.
    ExitTrades = 1,
    /// Trades sharing a parent aggregated into one position.
    Positions = 2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_are_pinned() {
        // These are written into record arrays; changing one is a breaking
        // change to the on-disk format.
        assert_eq!(SizeType::TargetPercent as i8, 5);
        assert_eq!(Direction::Both as i8, 2);
        assert_eq!(OrderStatus::Rejected as i8, 2);
        assert_eq!(StatusInfo::PartialFill as i8, 13);
        assert_eq!(TradesType::Positions as i8, 2);
    }

    #[test]
    fn every_status_info_has_a_description() {
        let all = [
            StatusInfo::SizeNaN,
            StatusInfo::PriceNaN,
            StatusInfo::ValPriceNaN,
            StatusInfo::ValueNaN,
            StatusInfo::ValueZeroNeg,
            StatusInfo::SizeZero,
            StatusInfo::NoCashShort,
            StatusInfo::NoCashLong,
            StatusInfo::NoOpenPosition,
            StatusInfo::MaxSizeExceeded,
            StatusInfo::RandomEvent,
            StatusInfo::CantCoverFees,
            StatusInfo::MinSizeNotReached,
            StatusInfo::PartialFill,
        ];
        assert_eq!(all.len(), 14);
        for s in all {
            assert!(!s.describe().is_empty());
        }
    }
}
