//! Turning Python's strings into the engine's enums.
//!
//! Small, and tested one variant at a time on purpose. A mapping table is the
//! kind of code that is obviously right while being wrong in one row —
//! `"targetvalue"` landing on `TargetAmount` compiles, runs, and produces a
//! backtest that is merely different. Nothing downstream can notice, because
//! both are valid size types.
//!
//! These return `Result<_, String>` rather than `PyResult` so they can be
//! tested without a Python interpreter; the caller wraps the message.

use phasor_model::{AccumulationMode, Direction, SizeType, TradesType};

/// Every accepted spelling of a size type, for the error message.
const SIZE_TYPES: &str = "amount, value, percent, targetamount, targetvalue, targetpercent";

/// How a size is interpreted.
///
/// # Errors
///
/// A message naming the accepted values.
pub fn size_type(name: &str) -> Result<SizeType, String> {
    match name {
        "amount" => Ok(SizeType::Amount),
        "value" => Ok(SizeType::Value),
        "percent" => Ok(SizeType::Percent),
        "targetamount" => Ok(SizeType::TargetAmount),
        "targetvalue" => Ok(SizeType::TargetValue),
        "targetpercent" => Ok(SizeType::TargetPercent),
        other => Err(format!(
            "size_type must be one of {SIZE_TYPES}; got {other:?}"
        )),
    }
}

/// Which side of the market a column may take.
///
/// `"long"` and `"short"` are accepted alongside the reference's own
/// `"longonly"`/`"shortonly"`, because the shorter spelling is what people
/// reach for and rejecting it teaches nothing.
///
/// # Errors
///
/// A message naming the accepted values.
pub fn direction(name: &str) -> Result<Direction, String> {
    match name {
        "longonly" | "long" => Ok(Direction::LongOnly),
        "shortonly" | "short" => Ok(Direction::ShortOnly),
        "both" => Ok(Direction::Both),
        other => Err(format!(
            "direction must be 'longonly', 'shortonly' or 'both'; got {other:?}"
        )),
    }
}

/// Whether a repeated signal changes an open position's size.
///
/// # Errors
///
/// A message naming the accepted values.
pub fn accumulate(name: &str) -> Result<AccumulationMode, String> {
    match name {
        "disabled" => Ok(AccumulationMode::Disabled),
        "both" => Ok(AccumulationMode::Both),
        "addonly" => Ok(AccumulationMode::AddOnly),
        "removeonly" => Ok(AccumulationMode::RemoveOnly),
        other => Err(format!(
            "accumulate must be 'disabled', 'both', 'addonly' or 'removeonly'; got {other:?}"
        )),
    }
}

/// Which trade view the statistics are scored over.
///
/// # Errors
///
/// A message naming the accepted values.
pub fn trades_type(name: &str) -> Result<TradesType, String> {
    match name {
        "exit" => Ok(TradesType::ExitTrades),
        "entry" => Ok(TradesType::EntryTrades),
        "position" | "positions" => Ok(TradesType::Positions),
        other => Err(format!(
            "trades must be 'exit', 'entry' or 'positions'; got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_type_maps_to_its_own_variant() {
        // Spelled out rather than looped, because the thing being tested is
        // precisely the pairing, and a loop over pairs would be the same table
        // asserting itself.
        assert_eq!(size_type("amount").unwrap(), SizeType::Amount);
        assert_eq!(size_type("value").unwrap(), SizeType::Value);
        assert_eq!(size_type("percent").unwrap(), SizeType::Percent);
        assert_eq!(size_type("targetamount").unwrap(), SizeType::TargetAmount);
        assert_eq!(size_type("targetvalue").unwrap(), SizeType::TargetValue);
        assert_eq!(size_type("targetpercent").unwrap(), SizeType::TargetPercent);
    }

    #[test]
    fn every_direction_maps_to_its_own_variant() {
        assert_eq!(direction("longonly").unwrap(), Direction::LongOnly);
        assert_eq!(direction("long").unwrap(), Direction::LongOnly);
        assert_eq!(direction("shortonly").unwrap(), Direction::ShortOnly);
        assert_eq!(direction("short").unwrap(), Direction::ShortOnly);
        assert_eq!(direction("both").unwrap(), Direction::Both);
    }

    #[test]
    fn every_accumulation_mode_maps_to_its_own_variant() {
        assert_eq!(accumulate("disabled").unwrap(), AccumulationMode::Disabled);
        assert_eq!(accumulate("both").unwrap(), AccumulationMode::Both);
        assert_eq!(accumulate("addonly").unwrap(), AccumulationMode::AddOnly);
        assert_eq!(
            accumulate("removeonly").unwrap(),
            AccumulationMode::RemoveOnly
        );
    }

    #[test]
    fn every_trade_view_maps_to_its_own_variant() {
        assert_eq!(trades_type("exit").unwrap(), TradesType::ExitTrades);
        assert_eq!(trades_type("entry").unwrap(), TradesType::EntryTrades);
        assert_eq!(trades_type("position").unwrap(), TradesType::Positions);
        assert_eq!(trades_type("positions").unwrap(), TradesType::Positions);
    }

    #[test]
    fn an_unknown_name_names_the_alternatives() {
        // The value the caller passed has to appear too: "must be one of ..."
        // without it leaves them hunting for which argument was wrong.
        let e = size_type("Amount").unwrap_err();
        assert!(e.contains("targetpercent"), "{e}");
        assert!(e.contains("\"Amount\""), "{e}");
        assert!(direction("").unwrap_err().contains("longonly"));
        assert!(accumulate("add").unwrap_err().contains("addonly"));
        assert!(trades_type("exits").unwrap_err().contains("'exit'"));
    }

    #[test]
    fn matching_is_case_sensitive_and_says_so_by_failing() {
        // Accepting "Amount" as well would double the table for nothing; what
        // matters is that the rejection is loud rather than a silent fallback
        // to the default size type.
        assert!(size_type("AMOUNT").is_err());
        assert!(direction("LongOnly").is_err());
    }
}
