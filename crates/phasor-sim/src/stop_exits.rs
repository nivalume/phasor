//! Stop exits without a simulation: where the stops of each entry would
//! fire, read off the bars.
//!
//! [`generate_ohlc_stop_exits`] arms the stops of [`crate::stops`] at each
//! entry's close and walks the bars after it with the same test and the same
//! trailing the simulation uses, so the two agree. With
//! `skip_until_exit = true` and `until_next = false` the exits are the stop
//! exits `from_signals` fills for the same entries, stops and bars when it
//! ignores an entry while a position is held (`accumulate` off).
//!
//! - `wait` is how many bars after the entry the first test is, at least 1:
//!   the entry is at its bar's close.
//! - `until_next` ends an entry's search at the next entry, which starts its
//!   own; a stop that fires on that bar fires first, inside the bar.
//! - `skip_until_exit` ignores the entries before an entry's exit, as a
//!   simulation holding the position would. Without it each entry searches
//!   on its own, and a bar two of them exit on keeps the first's exit.

use phasor_core::error::{Error, Result};
use phasor_core::flex::FlexArray;
use phasor_model::StopLadderMode;

use crate::stops::{self, Kind};

/// Inputs to [`generate_ohlc_stop_exits`]. Stops are fractions of the
/// entry's close, `NaN` disabling one, read on the entry's row; the time
/// stops count rows. See [`crate::stops`].
#[derive(Debug, Clone)]
pub struct OhlcStopExits<'a> {
    /// Number of bars.
    pub n_rows: usize,
    /// Number of columns.
    pub n_cols: usize,
    /// Where positions are entered, at the bar's close.
    pub entries: FlexArray<'a, bool>,
    /// Open; `NaN` is the close.
    pub open: FlexArray<'a, f64>,
    /// High; `NaN` is the higher of the open and the close.
    pub high: FlexArray<'a, f64>,
    /// Low; `NaN` is the lower of the open and the close.
    pub low: FlexArray<'a, f64>,
    /// Close, which the stops are anchored at.
    pub close: FlexArray<'a, f64>,
    /// Stop-loss distance.
    pub sl_stop: FlexArray<'a, f64>,
    /// Whether the stop-loss trails.
    pub sl_trail: FlexArray<'a, bool>,
    /// Trailing stop distance.
    pub tsl_stop: FlexArray<'a, f64>,
    /// The trailing stop's activation threshold.
    pub tsl_th: FlexArray<'a, f64>,
    /// Take-profit distance.
    pub tp_stop: FlexArray<'a, f64>,
    /// Exit this many bars after the entry.
    pub td_stop: FlexArray<'a, f64>,
    /// Exit on this row.
    pub dt_stop: FlexArray<'a, f64>,
    /// Whether the positions are short, which mirrors every stop.
    pub short: bool,
    /// Bars after the entry before the first test; at least 1.
    pub wait: usize,
    /// Whether an entry's search ends at the next entry.
    pub until_next: bool,
    /// Whether entries before an exit are ignored.
    pub skip_until_exit: bool,
}

impl Default for OhlcStopExits<'_> {
    fn default() -> Self {
        Self {
            n_rows: 0,
            n_cols: 0,
            entries: FlexArray::Scalar(false),
            open: FlexArray::Scalar(f64::NAN),
            high: FlexArray::Scalar(f64::NAN),
            low: FlexArray::Scalar(f64::NAN),
            close: FlexArray::Scalar(f64::NAN),
            sl_stop: FlexArray::Scalar(f64::NAN),
            sl_trail: FlexArray::Scalar(false),
            tsl_stop: FlexArray::Scalar(f64::NAN),
            tsl_th: FlexArray::Scalar(f64::NAN),
            tp_stop: FlexArray::Scalar(f64::NAN),
            td_stop: FlexArray::Scalar(f64::NAN),
            dt_stop: FlexArray::Scalar(f64::NAN),
            short: false,
            wait: 1,
            until_next: true,
            skip_until_exit: false,
        }
    }
}

/// Where the stops fired, column-major like the inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct StopExits {
    /// Whether a stop fired on the bar.
    pub exits: Vec<bool>,
    /// The price it fired at, the level or the open past it; `NaN` elsewhere.
    pub price: Vec<f64>,
    /// Which stop: 0 stop-loss, 1 trailing, 2 take-profit, 3 time; `-1`
    /// elsewhere.
    pub kind: Vec<i8>,
}

const fn kind_code(kind: Kind) -> i8 {
    match kind {
        Kind::StopLoss => 0,
        Kind::Trailing => 1,
        Kind::TakeProfit => 2,
        Kind::Time => 3,
    }
}

/// The exits the stops of `args`' entries would take. See the module docs.
///
/// # Errors
///
/// Returns [`Error::Invariant`] for `wait` 0, a negative stop distance or a
/// time stop that cannot be read as one.
pub fn generate_ohlc_stop_exits(args: &OhlcStopExits<'_>) -> Result<StopExits> {
    if args.wait == 0 {
        return Err(Error::Invariant(
            "wait must be 1 or more: an entry is at its bar's close".into(),
        ));
    }
    let (n_rows, n_cols) = (args.n_rows, args.n_cols);
    let mut out = StopExits {
        exits: vec![false; n_rows * n_cols],
        price: vec![f64::NAN; n_rows * n_cols],
        kind: vec![-1; n_rows * n_cols],
    };
    let inputs = stops::Inputs {
        sl_stop: &args.sl_stop,
        sl_trail: &args.sl_trail,
        tp_stop: &args.tp_stop,
        tsl_stop: &args.tsl_stop,
        tsl_th: &args.tsl_th,
        td_stop: &args.td_stop,
        dt_stop: &args.dt_stop,
        ladder: StopLadderMode::Disabled,
    };
    let position = if args.short { -1.0 } else { 1.0 };
    for col in 0..n_cols {
        let mut row = 0;
        while row < n_rows {
            if !args.entries.get(row, col) {
                row += 1;
                continue;
            }
            let (exit, stopped) = search(args, &inputs, row, col, position)?;
            if let Some((at, hit)) = exit {
                let k = col * n_rows + at;
                if !out.exits[k] {
                    out.exits[k] = true;
                    out.price[k] = hit.price;
                    out.kind[k] = kind_code(hit.kind);
                }
            }
            row = if !args.skip_until_exit {
                row + 1
            } else if let Some((at, _)) = exit {
                at + 1
            } else {
                stopped
            };
        }
    }
    Ok(out)
}

/// One entry's search: the bar its stops fire on, if any, and the row the
/// search stopped at — the next entry under `until_next`, or the last bar.
fn search(
    args: &OhlcStopExits<'_>,
    inputs: &stops::Inputs<'_, '_>,
    entry: usize,
    col: usize,
    position: f64,
) -> Result<(Option<(usize, stops::Hit)>, usize)> {
    let anchor = args.close.get(entry, col);
    let mut state = stops::arm(inputs, entry, col, anchor, position)?;
    for row in entry + 1..args.n_rows {
        let close = args.close.get(row, col);
        let mut open = args.open.get(row, col);
        if open.is_nan() {
            open = close;
        }
        let mut high = args.high.get(row, col);
        if high.is_nan() {
            high = open.max(close);
        }
        let mut low = args.low.get(row, col);
        if low.is_nan() {
            low = open.min(close);
        }
        if row >= entry + args.wait {
            if let Some(hit) = stops::test(&state, position, row, open, high, low)? {
                return Ok((Some((row, hit)), row));
            }
        }
        if args.until_next && args.entries.get(row, col) {
            return Ok((None, row));
        }
        stops::trail(&mut state, position, high, low);
    }
    Ok((None, args.n_rows))
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the prices are exact in binary")]
mod tests {
    use super::*;

    const OPEN: [f64; 5] = [100., 99., 97., 93., 95.];
    const HIGH: [f64; 5] = [101., 100., 98., 96., 103.];
    const LOW: [f64; 5] = [99., 96., 94., 92., 95.];
    const CLOSE: [f64; 5] = [100., 97., 96., 95., 102.];

    fn bars<'a>(entries: &'a [bool]) -> OhlcStopExits<'a> {
        OhlcStopExits {
            n_rows: 5,
            n_cols: 1,
            entries: FlexArray::per_row(entries, 5).unwrap(),
            open: FlexArray::per_row(&OPEN, 5).unwrap(),
            high: FlexArray::per_row(&HIGH, 5).unwrap(),
            low: FlexArray::per_row(&LOW, 5).unwrap(),
            close: FlexArray::per_row(&CLOSE, 5).unwrap(),
            ..OhlcStopExits::default()
        }
    }

    #[test]
    fn an_entry_exits_where_its_stop_first_fires() {
        let entries = [true, false, false, false, false];
        let out = generate_ohlc_stop_exits(&OhlcStopExits {
            sl_stop: FlexArray::scalar(0.05),
            ..bars(&entries)
        })
        .unwrap();
        // 95 is first reached on bar 2.
        assert_eq!(out.exits, [false, false, true, false, false]);
        assert_eq!(out.price[2], 100. * (1. - 0.05));
        assert_eq!(out.kind, [-1, -1, 0, -1, -1]);
    }

    #[test]
    fn the_next_entry_ends_a_search_and_skipping_waits_for_the_exit() {
        // Entries on bars 0 and 1; a 6.25% stop: 93.75 from 100 (reached by
        // bar 3's open), 90.9375 from 97 (never).
        let entries = [true, true, false, false, false];
        let base = OhlcStopExits {
            sl_stop: FlexArray::scalar(0.0625),
            ..bars(&entries)
        };
        let run = |until_next, skip| {
            generate_ohlc_stop_exits(&OhlcStopExits {
                until_next,
                skip_until_exit: skip,
                ..base.clone()
            })
            .unwrap()
            .exits
        };
        // The second entry ends the first's search before bar 3.
        assert_eq!(run(true, false), [false; 5]);
        assert_eq!(run(false, false), [false, false, false, true, false]);
        assert_eq!(run(false, true), [false, false, false, true, false]);
        // Waiting two bars changes nothing here; zero is refused.
        let wait = OhlcStopExits {
            wait: 0,
            ..base.clone()
        };
        assert!(generate_ohlc_stop_exits(&wait).is_err());
    }

    #[test]
    fn a_short_mirrors_the_stops_and_a_time_stop_fires_at_the_open() {
        let entries = [true, false, false, false, false];
        let out = generate_ohlc_stop_exits(&OhlcStopExits {
            tp_stop: FlexArray::scalar(0.0625),
            short: true,
            ..bars(&entries)
        })
        .unwrap();
        // A short's take-profit at 93.75: bar 3 opens at 93.
        assert_eq!((out.exits[3], out.price[3], out.kind[3]), (true, 93., 2));
        let out = generate_ohlc_stop_exits(&OhlcStopExits {
            td_stop: FlexArray::scalar(2.),
            ..bars(&entries)
        })
        .unwrap();
        assert_eq!((out.exits[2], out.price[2], out.kind[2]), (true, 97., 3));
    }
}
