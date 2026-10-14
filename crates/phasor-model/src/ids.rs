//! Interned identifiers.
//!
//! `InstrumentId` is compared and hashed on nearly every record write. In a
//! sweep with thousands of columns that is tens of millions of comparisons,
//! and string comparison there is measurable. Interning collapses each
//! identifier to a `Copy` handle that compares as an integer.
//!
//! Phase 1 keeps the storage simple; the interner is the thing to optimise
//! once there are real numbers to profile. What matters now is that the
//! *type* is `Copy` and opaque, so switching the backing store later does not
//! touch call sites.

use std::sync::Arc;

/// A tradeable symbol, e.g. `BTCUSDT`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(Arc<str>);

impl Symbol {
    /// Interns a symbol name.
    pub fn new(s: impl AsRef<str>) -> Self {
        Self(Arc::from(s.as_ref()))
    }

    /// The symbol as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A trading venue, e.g. `BINANCE`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Venue(Arc<str>);

impl Venue {
    /// Interns a venue name.
    pub fn new(s: impl AsRef<str>) -> Self {
        Self(Arc::from(s.as_ref()))
    }

    /// The venue as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A symbol qualified by its venue.
///
/// Rendered as `SYMBOL.VENUE`, matching the convention phase 2's strategy API
/// expects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId {
    /// The symbol.
    pub symbol: Symbol,
    /// The venue.
    pub venue: Venue,
}

impl InstrumentId {
    /// Validate separate identity components without reinterpreting a dotted venue.
    pub fn from_parts(symbol: &str, venue: &str) -> Result<Self, String> {
        if symbol.is_empty() || venue.is_empty() || venue.contains('.') {
            return Err(
                "instrument requires a nonempty symbol and a nonempty venue without dots".into(),
            );
        }
        format!("{symbol}.{venue}").parse()
    }

    /// Builds an instrument id from its parts.
    #[must_use]
    pub const fn new(symbol: Symbol, venue: Venue) -> Self {
        Self { symbol, venue }
    }
}

impl std::fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.symbol.as_str(), self.venue.as_str())
    }
}

impl std::str::FromStr for InstrumentId {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (symbol, venue) = value.rsplit_once('.').ok_or("expected SYMBOL.VENUE")?;
        if symbol.is_empty()
            || venue.is_empty()
            || value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err("instrument requires nonempty symbol and venue, without whitespace or path separators".into());
        }
        Ok(Self::new(Symbol::new(symbol), Venue::new(venue)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_as_symbol_dot_venue() {
        let id = InstrumentId::new(Symbol::new("BTCUSDT"), Venue::new("BINANCE"));
        assert_eq!(id.to_string(), "BTCUSDT.BINANCE");
    }

    #[test]
    fn equal_names_compare_equal() {
        assert_eq!(Symbol::new("BTC"), Symbol::new("BTC"));
        assert_ne!(Symbol::new("BTC"), Symbol::new("ETH"));
    }
}
