//! Domain model: value types, identifiers, market data, and the enums that
//! drive the execution state machine.
//!
//! # Two lineages, deliberately
//!
//! The *execution* enums ([`enums`]) follow the reference vectorized engine,
//! because reproducing its fill semantics bit-for-bit is the acceptance test
//! for this project (see `docs/08-verification.md`).
//!
//! The *value and identifier* types ([`types`], [`ids`]) follow
//! NautilusTrader's shape — fixed-point raw integers with per-instance
//! precision, interned identifiers — because phase 2 puts a Nautilus-style
//! strategy API on top of this same model and a mismatch there would mean
//! converting on every call. See `docs/07-nautilus-compat.md`.
//!
//! Every line here is an independent implementation written against public
//! semantics. No code is taken from either project; see `docs/09-licensing.md`.

pub mod bar;
pub mod enums;
pub mod ids;
pub mod types;

pub use bar::{Bar, BarType, OhlcvPanel};
pub use enums::{
    AccumulationMode, ConflictMode, Direction, DirectionConflictMode, OppositeEntryMode, OrderSide,
    OrderStatus, SizeType, StatusInfo, StopEntryPrice, StopExitMode, StopExitPrice, StopUpdateMode,
    TradeDirection, TradeStatus, TradesType,
};
