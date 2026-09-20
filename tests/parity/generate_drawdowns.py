#!/usr/bin/env python3
"""Generate differential fixtures for the drawdown record layer.

A single max-drawdown number hides how many separate episodes there were, how
long each lasted, and whether any of them recovered. This layer produces a
record per episode, and the cases where a port goes wrong are all about where
one episode ends and the next begins:

* a flat stretch at the top, which must extend the peak rather than open an
  episode;
* regaining *exactly* the old peak, which counts as recovery;
* a V-shaped recovery straight into another decline, which is two records and
  not one;
* a series still under water at the last bar, which is an open episode with a
  duration that is only a lower bound;
* a gap, which advances nothing — and a gap on the *last* bar, which therefore
  cannot close an episode at all.

Series are enumerated rather than hand-picked wherever they can be: every
monotone-step path of up to six bars over a small value alphabet, which is
what makes the boundary cases certain rather than likely.

Usage::

    python tests/parity/generate_drawdowns.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import itertools
from pathlib import Path

import numpy as np
from vectorbt.generic.nb import (
    dd_decline_duration_nb,
    dd_drawdown_nb,
    dd_recovery_duration_nb,
    dd_recovery_duration_ratio_nb,
    dd_recovery_return_nb,
    get_drawdowns_nb,
)

COLUMNS = ["fn", "n_rows", "n_cols", "ts", "result", "note"]

NAN = np.nan


def fmt(v: float) -> str:
    v = float(v)
    if np.isnan(v):
        return "nan"
    if np.isinf(v):
        return "inf" if v > 0 else "-inf"
    return repr(v)


def floats(a) -> str:
    return ";".join(fmt(v) for v in np.asarray(a, dtype=float).flatten(order="F"))


def records_text(recs) -> str:
    """Records as `col,peak,start,valley,end,peak_val,valley_val,end_val,status`."""
    if len(recs) == 0:
        return "-"
    return "|".join(
        ",".join(
            [
                str(int(r["col"])),
                str(int(r["peak_idx"])),
                str(int(r["start_idx"])),
                str(int(r["valley_idx"])),
                str(int(r["end_idx"])),
                fmt(r["peak_val"]),
                fmt(r["valley_val"]),
                fmt(r["end_val"]),
                str(int(r["status"])),
            ]
        )
        for r in recs
    )


def derived_text(recs) -> str:
    """The five derived quantities, one group per record."""
    if len(recs) == 0:
        return "-"
    dd = dd_drawdown_nb(recs["peak_val"], recs["valley_val"])
    decline = dd_decline_duration_nb(recs["start_idx"], recs["valley_idx"])
    recovery = dd_recovery_duration_nb(recs["valley_idx"], recs["end_idx"])
    ratio = dd_recovery_duration_ratio_nb(
        recs["start_idx"], recs["valley_idx"], recs["end_idx"]
    )
    ret = dd_recovery_return_nb(recs["valley_val"], recs["end_val"])
    return "|".join(
        ",".join(fmt(v) for v in (dd[i], decline[i], recovery[i], ratio[i], ret[i]))
        for i in range(len(recs))
    )


def step_paths(max_len: int, alphabet: tuple[float, ...]) -> list[list[float]]:
    """Every path of up to `max_len` bars over a small value alphabet."""
    out: list[list[float]] = []
    for n in range(1, max_len + 1):
        for combo in itertools.product(alphabet, repeat=n):
            out.append(list(combo))
    return out


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []

    def add(fn, **fields):
        r = blank()
        r["fn"] = fn
        r.update({k: ("" if v is None else str(v)) for k, v in fields.items()})
        out.append(r)

    def emit(vals, n_rows, n_cols, note):
        ts = np.array(vals, dtype=np.float64).reshape((n_rows, n_cols), order="F")
        recs = get_drawdowns_nb(ts)
        add(
            "drawdowns",
            n_rows=n_rows,
            n_cols=n_cols,
            ts=floats(ts),
            result=records_text(recs),
            note=note,
        )
        add(
            "derived",
            n_rows=n_rows,
            n_cols=n_cols,
            ts=floats(ts),
            result=derived_text(recs),
            note=note,
        )

    # --- every short path over a three-value alphabet ----------------------
    # Three distinct levels is the smallest alphabet that can express a peak,
    # a valley and a partial recovery, and equal neighbours cover the flat
    # stretches that must not open an episode.
    for path in step_paths(5, (1.0, 2.0, 3.0)):
        emit(path, len(path), 1, "exhaustive step path")

    # --- paths that include a gap ------------------------------------------
    # Enumerated separately over a two-value alphabet plus NaN, so a gap
    # appears in every position including the last.
    for path in step_paths(4, (1.0, 2.0, NAN)):
        emit(path, len(path), 1, "exhaustive path with gaps")

    # --- hand-picked shapes worth naming ------------------------------------
    named = [
        ([10.0, 8.0, 6.0, 9.0, 11.0], "decline then full recovery"),
        ([10.0, 8.0, 6.0, 7.0], "still under water at the last bar"),
        ([10.0, 8.0, 10.0, 7.0, 6.0], "recovery straight into another decline"),
        ([10.0, 6.0, 7.0, 8.0, 11.0], "the valley is not the last bar of the decline"),
        ([10.0, 9.0, 8.0, 6.0, 9.0, 11.0], "two-phase durations"),
        ([5.0, 5.0, 5.0], "flat throughout"),
        ([1.0, 2.0, 3.0], "monotone up"),
        ([3.0, 2.0, 1.0], "monotone down"),
        ([10.0, 0.0, 5.0], "a valley at zero"),
        ([0.0, -1.0, 1.0], "crossing zero"),
        ([-1.0, -2.0, -1.0], "entirely negative"),
        ([10.0, np.inf, 5.0], "an infinite peak"),
        ([10.0, 8.0, 10.0, 8.0, 10.0, 8.0], "a sawtooth"),
    ]
    for vals, note in named:
        emit(vals, len(vals), 1, note)

    # --- several columns, to pin the column-major layout --------------------
    multi = [
        ([10.0, 8.0, 11.0, 5.0, 4.0, 3.0], 3, 2, "two columns, one of each status"),
        ([1.0, 2.0, 3.0, 3.0, 2.0, 1.0], 3, 2, "one rising column, one falling"),
        ([NAN, NAN, NAN, 10.0, 5.0, 10.0], 3, 2, "an empty column beside a full one"),
    ]
    for vals, n_rows, n_cols, note in multi:
        emit(vals, n_rows, n_cols, note)

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "drawdowns.tsv",
    )
    args = parser.parse_args()

    data = rows()
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    by: dict[str, int] = {}
    for r in data:
        by[r["fn"]] = by.get(r["fn"], 0) + 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name in sorted(by):
        print(f"  {name}: {by[name]}")


if __name__ == "__main__":
    main()
