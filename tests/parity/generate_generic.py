#!/usr/bin/env python3
"""Generate differential fixtures for the rolling and elementwise array layer.

These functions are arithmetically trivial and their edge cases are not, so the
fixture series are chosen to be awkward rather than representative:

* a leading gap, which `ffill` cannot fill and `bfill` can;
* a trailing gap, the mirror case;
* interior gaps, which shrink a window rather than poisoning it;
* an all-`NaN` series, where a maximum must stay `NaN` rather than becoming
  `-inf`;
* a constant series, where the expanded variance cancels and a naive form
  returns `NaN` instead of zero, and where the exponential mean drifts unless
  the equality guard is there;
* a series shorter than its own window;
* values around 1e8 next to values around 1e-8, where the accumulating form of
  the mean and the re-summing form part company.

The last one matters most. `rolling_mean` keeps a running sum and subtracts
what falls out of the window rather than re-summing it — `O(1)` per bar, and a
different answer in the last bits. A fixture set of well-scaled numbers would
never see the difference.

Usage::

    python tests/parity/generate_generic.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import math
from pathlib import Path

import numpy as np
from vectorbt.generic import nb as gnb

from generate_execute_order import fmt_float

COLUMNS = [
    "fn",
    "series",
    "series2",
    "window",
    "minp",
    "ddof",
    "adjust",
    "n",
    "fill",
    "wait",
    "outcome",
    "result",
    "note",
]

NAN = math.nan


def series_set() -> list[tuple[str, list[float]]]:
    return [
        ("plain", [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
        ("wobble", [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0]),
        ("leading_gap", [NAN, NAN, 3.0, 4.0, 5.0]),
        ("trailing_gap", [1.0, 2.0, 3.0, NAN, NAN]),
        ("interior_gaps", [1.0, NAN, 3.0, NAN, 5.0, 6.0]),
        ("all_nan", [NAN, NAN, NAN]),
        ("constant", [0.1, 0.1, 0.1, 0.1, 0.1]),
        ("constant_negative", [-2.5, -2.5, -2.5, -2.5]),
        ("single", [7.0]),
        ("two", [1.0, 2.0]),
        ("wide_scale", [1e8, 1e-8, 1e8, 1e-8, 1e8, 1e-8]),
        ("negatives", [-3.0, 5.0, -1.0, 2.0, -8.0]),
        ("zeros", [0.0, 0.0, 1.0, 0.0]),
    ]


def fmt_list(values) -> str:
    return ";".join(fmt_float(float(v)) for v in values)


def fmt_bools(values) -> str:
    return ";".join("true" if bool(v) else "false" for v in values)


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []

    def add(fn, name, values, result, note="", **params):
        r = blank()
        r.update(
            {
                "fn": fn,
                "series": name,
                "outcome": "ok",
                "result": result,
                "note": note,
            }
        )
        for k, v in params.items():
            r[k] = v
        out.append(r)

    for name, values in series_set():
        a = np.asarray(values, dtype=np.float64)

        # --- rolling ----------------------------------------------------
        for window in (1, 2, 3, 5, 10):
            for minp in (None, 1, window):
                if minp is not None and minp > window:
                    continue
                minp_txt = "" if minp is None else str(minp)
                try:
                    res = gnb.rolling_mean_1d_nb(a, window, minp)
                except Exception:  # noqa: BLE001
                    continue
                add(
                    "rolling_mean",
                    name,
                    values,
                    fmt_list(res),
                    window=str(window),
                    minp=minp_txt,
                )
                add(
                    "rolling_min",
                    name,
                    values,
                    fmt_list(gnb.rolling_min_1d_nb(a, window, minp)),
                    window=str(window),
                    minp=minp_txt,
                )
                add(
                    "rolling_max",
                    name,
                    values,
                    fmt_list(gnb.rolling_max_1d_nb(a, window, minp)),
                    window=str(window),
                    minp=minp_txt,
                )
                for ddof in (0, 1):
                    add(
                        "rolling_std",
                        name,
                        values,
                        fmt_list(gnb.rolling_std_1d_nb(a, window, minp, ddof)),
                        window=str(window),
                        minp=minp_txt,
                        ddof=str(ddof),
                    )

        # --- expanding --------------------------------------------------
        # The reference raises when `minp` exceeds the series length, so a
        # short series produces a rejection here rather than a result.
        def add_or_error(fn, call, **params):
            try:
                add(fn, name, values, fmt_list(call()), **params)
            except ValueError:
                r = blank()
                r.update({"fn": fn, "series": name, "outcome": "error",
                          "note": "min_periods above the series length"})
                r.update(params)
                out.append(r)

        for minp in (1, 2):
            add(
                "expanding_min",
                name,
                values,
                fmt_list(gnb.expanding_min_1d_nb(a, minp)),
                minp=str(minp),
            )
            add(
                "expanding_max",
                name,
                values,
                fmt_list(gnb.expanding_max_1d_nb(a, minp)),
                minp=str(minp),
            )
            add_or_error(
                "expanding_mean",
                lambda m=minp: gnb.expanding_mean_1d_nb(a, m),
                minp=str(minp),
            )
            for ddof in (0, 1):
                add_or_error(
                    "expanding_std",
                    lambda m=minp, d=ddof: gnb.expanding_std_1d_nb(a, m, d),
                    minp=str(minp),
                    ddof=str(ddof),
                )

        # --- exponentially weighted -------------------------------------
        for span in (2, 3, 5):
            for adjust in (False, True):
                for minp in (0, 1):
                    add(
                        "ewm_mean",
                        name,
                        values,
                        fmt_list(gnb.ewm_mean_1d_nb(a, span, minp, adjust)),
                        window=str(span),
                        minp=str(minp),
                        adjust="true" if adjust else "false",
                    )
                    add(
                        "ewm_std",
                        name,
                        values,
                        fmt_list(gnb.ewm_std_1d_nb(a, span, minp, adjust)),
                        window=str(span),
                        minp=str(minp),
                        adjust="true" if adjust else "false",
                    )

        # --- elementwise ------------------------------------------------
        for n in (1, 2, 5):
            add(
                "fshift",
                name,
                values,
                fmt_list(gnb.fshift_1d_nb(a, n, np.nan)),
                n=str(n),
                fill="nan",
            )
            add(
                "bshift",
                name,
                values,
                fmt_list(gnb.bshift_1d_nb(a, n, np.nan)),
                n=str(n),
                fill="nan",
            )
            add("diff", name, values, fmt_list(gnb.diff_1d_nb(a, n)), n=str(n))
            add(
                "pct_change",
                name,
                values,
                fmt_list(gnb.pct_change_1d_nb(a, n)),
                n=str(n),
            )
        add("ffill", name, values, fmt_list(gnb.ffill_1d_nb(a)))
        add("bfill", name, values, fmt_list(gnb.bfill_1d_nb(a)))
        add(
            "fillna",
            name,
            values,
            fmt_list(gnb.fillna_1d_nb(a.copy(), -1.0)),
            fill="-1.0",
        )

    # --- crossovers, which need two series ------------------------------
    pairs = [
        ("rises_through", [0.0, 2.0, 3.0, 4.0], [1.0, 1.0, 1.0, 1.0]),
        ("starts_above", [2.0, 3.0, 4.0], [1.0, 1.0, 1.0]),
        ("touches_then_rises", [0.0, 1.0, 2.0], [1.0, 1.0, 1.0]),
        ("touches_then_falls", [2.0, 1.0, 0.0], [1.0, 1.0, 1.0]),
        ("gap_between", [0.0, NAN, 2.0], [1.0, 1.0, 1.0]),
        ("gap_in_other", [0.0, 2.0, 3.0], [1.0, NAN, 1.0]),
        ("crosses_twice", [0.0, 2.0, 0.0, 2.0], [1.0, 1.0, 1.0, 1.0]),
        # Equality resets the counter but not the below-memory. Only a
        # sequence that is already above when it touches can tell the two
        # apart, and only when `wait` makes the counter matter.
        ("above_then_touch_then_above", [0.0, 2.0, 1.0, 2.0, 3.0], [1.0] * 5),
        ("above_then_touch_twice", [0.0, 2.0, 1.0, 1.0, 2.0], [1.0] * 5),
        ("falls_back_before_wait", [0.0, 2.0, 0.5, 4.0], [1.0, 1.0, 1.0, 1.0]),
        ("both_nan", [NAN, NAN], [NAN, NAN]),
        ("equal_throughout", [1.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
    ]
    for name, x, y in pairs:
        ax = np.asarray(x, dtype=np.float64)
        ay = np.asarray(y, dtype=np.float64)
        for wait in (0, 1, 2):
            r = blank()
            r.update(
                {
                    "fn": "crossed_above",
                    "series": fmt_list(x),
                    "series2": fmt_list(y),
                    "wait": str(wait),
                    "outcome": "ok",
                    "result": fmt_bools(gnb.crossed_above_1d_nb(ax, ay, wait)),
                    "note": name,
                }
            )
            out.append(r)

    # --- rejections ------------------------------------------------------
    for fn, window, minp in [("rolling_mean", 2, 3), ("rolling_std", 2, 3)]:
        r = blank()
        r.update(
            {
                "fn": fn,
                "series": "plain",
                "window": str(window),
                "minp": str(minp),
                "ddof": "0" if fn == "rolling_std" else "",
                "outcome": "error",
                "note": "min_periods above the window",
            }
        )
        out.append(r)
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "generic.tsv",
    )
    args = parser.parse_args()

    data = rows()
    # Series are written by name; the harness resolves them from the same
    # table, so a value cannot drift between the two sides.
    header_lines = [
        "# series definitions, shared with the Rust harness",
    ]
    for name, values in series_set():
        header_lines.append(f"#series\t{name}\t{fmt_list(values)}")

    lines = header_lines + ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    by_fn: dict[str, int] = {}
    for r in data:
        by_fn[r["fn"]] = by_fn.get(r["fn"], 0) + 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name in sorted(by_fn):
        print(f"  {name}: {by_fn[name]}")


if __name__ == "__main__":
    main()
