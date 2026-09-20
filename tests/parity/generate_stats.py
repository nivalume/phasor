#!/usr/bin/env python3
"""Generate differential fixtures for the statistics table.

This is the only generator that goes through the reference's *user-facing*
API rather than its kernels. That is the point: `Portfolio.stats()` is where
the row titles, the row order, the percent scalings and the open-trade default
live, and none of those are visible from the kernel tier. A port can agree
with every kernel and still print a table nobody can compare against.

What it pins that nothing else does:

* the twenty-eight rows and their exact titles, in order;
* which rows are scaled to percent, and that **Max Drawdown is negated** —
  reported positive while every drawdown record is negative;
* that the trade-quality rows default to **closed trades only**, so an open
  trade's mark to market never enters a win rate;
* that the three trade *counts* are not subject to that filter.

Cases cover a column that traded and closed out, one still holding at the last
bar, one that never traded at all, a losing column, a shorting column, and one
with costs — because most of the degenerate rows (a NaN win rate, an infinite
profit factor, a missing losing average) only appear in one of those.

Usage::

    python tests/parity/generate_stats.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import warnings
from pathlib import Path

import numpy as np
import pandas as pd
import vectorbt as vbt

# A frequency of one day per bar, purely so the annualized rows are computed
# at all; the durations it produces are converted straight back to bar counts.
warnings.filterwarnings("ignore", message=".*requires frequency.*")

COLUMNS = [
    "case",
    "n_rows",
    "n_cols",
    "col",
    "init_cash",
    "fees",
    "fixed_fees",
    "slippage",
    "incl_open",
    "close",
    "size",
    "titles",
    "values",
    "note",
]

TITLES = [
    "Start",
    "End",
    "Period",
    "Start Value",
    "End Value",
    "Total Return [%]",
    "Benchmark Return [%]",
    "Max Gross Exposure [%]",
    "Total Fees Paid",
    "Max Drawdown [%]",
    "Max Drawdown Duration",
    "Total Trades",
    "Total Closed Trades",
    "Total Open Trades",
    "Open Trade PnL",
    "Win Rate [%]",
    "Best Trade [%]",
    "Worst Trade [%]",
    "Avg Winning Trade [%]",
    "Avg Losing Trade [%]",
    "Avg Winning Trade Duration",
    "Avg Losing Trade Duration",
    "Profit Factor",
    "Expectancy",
    "Sharpe Ratio",
    "Calmar Ratio",
    "Omega Ratio",
    "Sortino Ratio",
]


def as_bars(v):
    """A Timedelta back to the bar count it stands for."""
    if isinstance(v, pd.Timedelta):
        return v / pd.Timedelta("1D")
    if v is pd.NaT:
        return np.nan
    return v


def fmt(v) -> str:
    if v is None:
        return "nan"
    try:
        f = float(v)
    except (TypeError, ValueError):
        return "nan"
    if np.isnan(f):
        return "nan"
    if np.isinf(f):
        return "inf" if f > 0 else "-inf"
    return repr(f)


def floats(a) -> str:
    return ";".join(fmt(v) for v in np.asarray(a, dtype=float).flatten(order="F"))


CASES = [
    (
        "closed out",
        [10.0, 11.0, 12.0, 11.0, 13.0],
        [1.0, 0.0, -1.0, 0.0, 0.0],
        "a single round trip that closed",
    ),
    (
        "still holding",
        [10.0, 11.0, 12.0, 11.0, 13.0],
        [1.0, 0.0, -1.0, 0.0, 1.0],
        "one closed trade and one open at the last bar",
    ),
    (
        "never traded",
        [10.0, 11.0, 12.0, 11.0, 13.0],
        [0.0, 0.0, 0.0, 0.0, 0.0],
        "no orders at all: most rows have no answer",
    ),
    (
        "all losses",
        [10.0, 9.0, 8.0, 7.0, 6.0],
        [1.0, 0.0, -1.0, 1.0, -1.0],
        "two losing round trips: no winning average",
    ),
    (
        "all wins",
        [10.0, 11.0, 12.0, 13.0, 14.0],
        [1.0, 0.0, -1.0, 1.0, -1.0],
        "two winning round trips: an infinite profit factor",
    ),
    (
        "mixed",
        [10.0, 12.0, 9.0, 11.0, 8.0, 12.0, 10.0],
        [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 0.0],
        "winners and losers together",
    ),
    (
        "shorting",
        [10.0, 9.0, 8.0, 9.0, 7.0],
        [-1.0, 0.0, 1.0, -1.0, 0.0],
        "a short that profits as the price falls",
    ),
    (
        "flat price",
        [10.0, 10.0, 10.0, 10.0, 10.0],
        [1.0, 0.0, -1.0, 0.0, 0.0],
        "no volatility: the infinite-ratio column",
    ),
    (
        "monotone up",
        [10.0, 11.0, 12.0, 13.0, 14.0],
        [1.0, 0.0, 0.0, 0.0, 0.0],
        "no drawdown at all, so no Calmar",
    ),
]

COST_GRID = [
    (0.0, 0.0, 0.0),
    (0.001, 0.0, 0.0),
    (0.0, 1.0, 0.0),
    (0.001, 0.5, 0.0005),
]


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    for name, close_vals, size_vals, note in CASES:
        for fees, fixed_fees, slippage in COST_GRID:
            close = pd.DataFrame({"a": close_vals})
            size = pd.DataFrame({"a": size_vals})
            pf = vbt.Portfolio.from_orders(
                close,
                size,
                init_cash=1000.0,
                fees=fees,
                fixed_fees=fixed_fees,
                slippage=slippage,
                # A frequency is required, or the four annualized ratio rows
                # are silently dropped from the table rather than computed.
                # It makes the duration rows Timedeltas, which are converted
                # back to bar counts below — this port has no calendar and
                # reports bars.
                freq="1D",
            )
            for incl_open in (False, True):
                s = pf.stats(column="a", settings=dict(incl_open=incl_open))
                assert list(s.index) == TITLES, list(s.index)
                s = s.map(as_bars)
                r = blank()
                r.update(
                    case=name,
                    n_rows=str(len(close_vals)),
                    n_cols="1",
                    col="0",
                    init_cash="1000.0",
                    fees=fmt(fees),
                    fixed_fees=fmt(fixed_fees),
                    slippage=fmt(slippage),
                    incl_open=str(incl_open),
                    close=floats(close_vals),
                    size=floats(size_vals),
                    titles="|".join(TITLES),
                    values=";".join(fmt(v) for v in s.values),
                    note=note,
                )
                out.append(r)
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "stats.tsv",
    )
    args = parser.parse_args()

    data = rows()
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")
    print(f"wrote {len(data)} fixtures to {args.out}")


if __name__ == "__main__":
    main()
