#!/usr/bin/env python3
"""Generate differential fixtures for the eight indicators.

The array layer underneath is already pinned function by function, so what
this verifies is the composition: which window feeds which average, what is
cached and reused across a sweep, and the readings that are easy to
special-case wrongly.

Those degenerate readings are most of the value here:

* an RSI of exactly 100 when nothing fell, which arrives through a division by
  zero rather than a branch;
* a stochastic whose window has a flat range, dividing zero by zero;
* the true range of the first bar, which has no previous close;
* on-balance volume's first bar, which compares against a `NaN` and therefore
  counts positive — a consequence of `NaN` comparing false, not a rule anyone
  wrote down;
* Bollinger bands where the deviation is zero, so all three bands coincide.

The sweeps are deliberately wide enough to exercise the caching: windows that
repeat across combinations, and a window that is "fast" in one MACD row and
"slow" in another.

Usage::

    python tests/parity/generate_indicators.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import itertools
import math
from pathlib import Path

import numpy as np
from vectorbt.indicators import nb as inb

from generate_execute_order import fmt_float

COLUMNS = [
    "indicator",
    "case",
    "params",
    "n_rows",
    "n_cols",
    "high",
    "low",
    "close",
    "volume",
    "outputs",
    "result",
    "note",
]

NAN = math.nan


def flat(a: np.ndarray) -> str:
    """Column-major, matching the engine's layout."""
    return ";".join(fmt_float(float(v)) for v in np.asarray(a, dtype=float).flatten(order="F"))


def as_2d(values: list[float] | list[list[float]]) -> np.ndarray:
    a = np.asarray(values, dtype=np.float64)
    return a.reshape(-1, 1) if a.ndim == 1 else a


class Case:
    """One price series, possibly several columns."""

    def __init__(self, name, close, high=None, low=None, volume=None, note=""):
        self.name = name
        self.close = as_2d(close)
        n_rows, n_cols = self.close.shape
        self.high = as_2d(high) if high is not None else self.close + 1.0
        self.low = as_2d(low) if low is not None else self.close - 1.0
        self.volume = (
            as_2d(volume)
            if volume is not None
            else np.full((n_rows, n_cols), 100.0) + np.arange(n_rows).reshape(-1, 1)
        )
        self.note = note
        self.n_rows = n_rows
        self.n_cols = n_cols


def cases() -> list[Case]:
    rng = np.random.default_rng(20240920)
    out = [
        Case("rising", [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], note="nothing ever falls"),
        Case("falling", [8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], note="nothing ever rises"),
        Case(
            "flat",
            [5.0] * 8,
            high=[[5.0]] * 8,
            low=[[5.0]] * 8,
            note="zero range and zero deviation: every ratio divides by zero",
        ),
        Case(
            "wobble",
            [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0],
            note="the ordinary case",
        ),
        Case(
            "with_gaps",
            [3.0, NAN, 4.0, 1.0, NAN, 9.0, 2.0, 6.0],
            note="missing bars shrink windows rather than poisoning them",
        ),
        Case(
            "gapping_open",
            [10.0, 10.5, 21.0, 20.5, 9.0, 9.5],
            high=[[10.5], [11.0], [21.5], [21.0], [9.5], [10.0]],
            low=[[9.5], [10.0], [20.5], [20.0], [8.5], [9.0]],
            note="jumps the true range must catch and the bar range would miss",
        ),
        Case(
            "two_symbols",
            [[1.0, 10.0], [3.0, 9.0], [2.0, 11.0], [5.0, 8.0], [4.0, 12.0], [7.0, 7.0]],
            note="columns must not mix",
        ),
        Case(
            "short",
            [1.0, 2.0],
            note="shorter than most of the windows asked for",
        ),
    ]
    out.append(
        Case(
            "missing_volume",
            [3.0, 4.0, 2.0, 5.0, 1.0, 6.0],
            volume=[[100.0], [NAN], [300.0], [NAN], [500.0], [600.0]],
            note="a bar with no volume contributes nothing to on-balance "
            "volume rather than poisoning the running total",
        )
    )
    n_rows, n_cols = 24, 3
    close = np.abs(rng.normal(30.0, 4.0, size=(n_rows, n_cols))) + 5.0
    high = close * (1.0 + rng.random((n_rows, n_cols)) * 0.05)
    low = close * (1.0 - rng.random((n_rows, n_cols)) * 0.05)
    volume = rng.integers(100, 10_000, size=(n_rows, n_cols)).astype(float)
    out.append(Case("random", close, high, low, volume, "a seeded random panel"))
    return out


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []

    def add(indicator, case, params, outputs, results, note=""):
        out.append(
            {
                "indicator": indicator,
                "case": case.name,
                "params": params,
                "n_rows": str(case.n_rows),
                "n_cols": str(case.n_cols),
                "high": flat(case.high),
                "low": flat(case.low),
                "close": flat(case.close),
                "volume": flat(case.volume),
                "outputs": ",".join(outputs),
                # Each output is a full column-major matrix; outputs are
                # separated by `|`.
                "result": "|".join(flat(r) for r in results),
                "note": note or case.note,
            }
        )

    ma_grid = [(w, e) for w in (1, 2, 3, 5) for e in (False, True)]
    stoch_grid = [(3, 2, False), (3, 2, True), (4, 3, False), (3, 3, False)]
    macd_grid = [
        (2, 3, 2, False, False),
        (2, 3, 2, True, True),
        # The two flags must differ somewhere, or nothing distinguishes the
        # signal line's smoothing from the MACD line's.
        (2, 3, 2, True, False),
        (2, 3, 2, False, True),
        # 3 is slow in the first row and fast here: the cache must serve both.
        (3, 4, 2, False, False),
    ]
    bb_grid = [(w, e, a) for w in (2, 3) for e in (False,) for a in (1.0, 2.0, 2.5)]

    for case in cases():
        close, high, low, volume = case.close, case.high, case.low, case.volume

        for window, ewm in ma_grid:
            p = f"window={window},ewm={ewm}"
            add("ma", case, p, ["ma"], [inb.ma_nb(close, window, ewm, False)])
            add(
                "mstd",
                case,
                p,
                ["mstd"],
                [inb.mstd_nb(close, window, ewm, False, 0)],
            )
            cache = inb.rsi_cache_nb(close, [window], [ewm], False)
            add(
                "rsi",
                case,
                p,
                ["rsi"],
                [inb.rsi_apply_nb(close, window, ewm, False, cache)],
            )
            tr, atr_cache = inb.atr_cache_nb(high, low, close, [window], [ewm], False)
            tr_out, atr_out = inb.atr_apply_nb(
                high, low, close, window, ewm, False, tr, atr_cache
            )
            add("atr", case, p, ["tr", "atr"], [tr_out, atr_out])

        for window, ewm, alpha in bb_grid:
            ma_cache, mstd_cache = inb.bb_cache_nb(
                close, [window], [ewm], [alpha], False, 0
            )
            mid, up, low_band = inb.bb_apply_nb(
                close, window, ewm, alpha, False, 0, ma_cache, mstd_cache
            )
            add(
                "bbands",
                case,
                f"window={window},ewm={ewm},alpha={alpha}",
                ["middle", "upper", "lower"],
                [mid, up, low_band],
            )

        for k_window, d_window, d_ewm in stoch_grid:
            cache = inb.stoch_cache_nb(
                high, low, close, [k_window], [d_window], [d_ewm], False
            )
            k, d = inb.stoch_apply_nb(
                high, low, close, k_window, d_window, d_ewm, False, cache
            )
            add(
                "stoch",
                case,
                f"k_window={k_window},d_window={d_window},d_ewm={d_ewm}",
                ["percent_k", "percent_d"],
                [k, d],
            )

        for fast, slow, signal, macd_ewm, signal_ewm in macd_grid:
            cache = inb.macd_cache_nb(
                close, [fast], [slow], [signal], [macd_ewm], [signal_ewm], False
            )
            line, sig = inb.macd_apply_nb(
                close, fast, slow, signal, macd_ewm, signal_ewm, False, cache
            )
            add(
                "macd",
                case,
                (
                    f"fast_window={fast},slow_window={slow},signal_window={signal},"
                    f"macd_ewm={macd_ewm},signal_ewm={signal_ewm}"
                ),
                ["macd", "signal"],
                [line, sig],
            )

        add("obv", case, "", ["obv"], [inb.obv_custom_nb(close, volume)])

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "indicators.tsv",
    )
    args = parser.parse_args()

    data = rows()
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    by: dict[str, int] = {}
    for r in data:
        by[r["indicator"]] = by.get(r["indicator"], 0) + 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name in sorted(by):
        print(f"  {name}: {by[name]}")


if __name__ == "__main__":
    main()
