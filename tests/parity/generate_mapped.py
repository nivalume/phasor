#!/usr/bin/env python3
"""Generate differential fixtures for the mapped-array layer.

The input space here is genuinely small — a column assignment for each of a
handful of records, plus a value vector — so it is enumerated rather than
sampled. Every assignment of up to four records to up to three columns is
covered, which is what makes the awkward cases certain to appear instead of
likely:

* a column with no records at all, which must still get an entry;
* records arriving out of column order, which is the gathering path;
* a column whose records are interleaved with another's, so a stable grouping
  is the only thing keeping them chronological;
* ties in a top-N selection, where the two ends resolve in opposite
  directions;
* NaN records, which are selected only after every comparable one.

One deliberate divergence is **not** covered here, because the reference
cannot express it: `reduce_mapped_to_idx_nb` with an argmax-style reduction
raises on an all-NaN column, where this port returns the fill value. The
index fixtures therefore use finite values only, and the divergence is pinned
by `a_column_of_only_nan_has_no_extreme` in the Rust unit tests.

Usage::

    python tests/parity/generate_mapped.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import itertools
from pathlib import Path

import numpy as np
from vectorbt.generic.nb import (
    argmax_reduce_nb,
    argmin_reduce_nb,
    max_reduce_nb,
    mean_reduce_nb,
    min_reduce_nb,
    std_reduce_nb,
    sum_reduce_nb,
)
from vectorbt.records.nb import (
    bottom_n_inout_map_nb,
    col_map_nb,
    mapped_to_mask_nb,
    mapped_value_counts_nb,
    reduce_mapped_nb,
    reduce_mapped_to_idx_nb,
    stack_expand_mapped_nb,
    top_n_inout_map_nb,
)

COLUMNS = [
    "fn",
    "n_cols",
    "col_arr",
    "values",
    "codes",
    "n_uniques",
    "idx_arr",
    "n",
    "ddof",
    "fill",
    "result",
    "note",
]


# The reductions `MappedArray.sum()`, `.mean()` and friends actually dispatch
# to. They are all NaN-skipping, which is *not* what the plain numpy
# equivalents do, and an earlier version of this generator reached for
# `np.sum`/`np.min` instead and so happily confirmed a port that propagated
# NaN. Taking the reference's own kernels is the whole point.
REDUCTIONS = {
    "sum": (sum_reduce_nb, 0.0, ()),
    "mean": (mean_reduce_nb, np.nan, ()),
    "min": (min_reduce_nb, np.nan, ()),
    "max": (max_reduce_nb, np.nan, ()),
    "std0": (std_reduce_nb, np.nan, (0,)),
    "std1": (std_reduce_nb, np.nan, (1,)),
}


def ints(a) -> str:
    return ";".join(str(int(v)) for v in np.asarray(a).flatten(order="F"))


def floats(a) -> str:
    out = []
    for v in np.asarray(a, dtype=float).flatten(order="F"):
        if np.isnan(v):
            out.append("nan")
        elif np.isinf(v):
            out.append("inf" if v > 0 else "-inf")
        else:
            out.append(repr(float(v)))
    return ";".join(out)


def bits(a) -> str:
    return "".join("X" if v else "." for v in np.asarray(a).flatten(order="F"))


def value_sets(n: int) -> list[tuple[str, list[float]]]:
    """A few value vectors per column assignment, chosen for what they break."""
    if n == 0:
        return [("empty", [])]
    out = [("distinct", [float(i + 1) for i in range(n)])]
    out.append(("descending", [float(n - i) for i in range(n)]))
    out.append(("all tied", [7.0] * n))
    if n >= 2:
        # A tie that spans two columns, so a per-column selection must not
        # compare across the boundary.
        tied = [1.0] * n
        tied[n // 2] = 2.0
        out.append(("one standout", tied))
        nan_mix = [float(i + 1) for i in range(n)]
        nan_mix[0] = np.nan
        out.append(("a leading NaN", nan_mix))
    out.append(("all NaN", [np.nan] * n))
    if n >= 3:
        neg = [-1.0, 0.0, -0.0] + [float(i) for i in range(3, n)]
        out.append(("signed zero and negatives", neg))
    if n >= 2:
        # An argmax substitutes -inf for a missing value, so a real -inf and a
        # NaN tie and the *earlier* one wins. Skipping NaN instead would
        # always pick the real one. Nothing but this case tells them apart.
        inf = [np.nan, -np.inf] + [-np.inf] * (n - 2)
        out.append(("a NaN tying with a real -inf", inf))
        out.append(("both infinities", [np.inf, -np.inf] + [0.0] * (n - 2)))
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

    assignments: list[tuple[int, tuple[int, ...]]] = []
    for n_cols in (1, 2, 3):
        for n in range(0, 5):
            for combo in itertools.product(range(n_cols), repeat=n):
                assignments.append((n_cols, combo))

    for n_cols, combo in assignments:
        col_arr = np.array(combo, dtype=np.int64)
        n = len(combo)
        col_text = ints(col_arr) if n else ""
        col_idxs, col_lens = col_map_nb(col_arr, n_cols)

        add(
            "col_map",
            n_cols=n_cols,
            col_arr=col_text,
            result=f"{ints(col_idxs)}|{ints(col_lens)}",
        )

        for note, vals in value_sets(n):
            values = np.array(vals, dtype=np.float64)
            vtext = floats(values)

            for name, (func, fill, extra) in REDUCTIONS.items():
                res = reduce_mapped_nb(values, (col_idxs, col_lens), fill, func, *extra)
                add(
                    f"reduce_{name}",
                    n_cols=n_cols,
                    col_arr=col_text,
                    values=vtext,
                    fill=floats([fill]),
                    result=floats(res),
                    note=note,
                )

            # `MappedArray.count()` is not a reduction at all: it reads the
            # segment lengths straight off the map, so it counts records
            # rather than measurable ones.
            add(
                "reduce_count",
                n_cols=n_cols,
                col_arr=col_text,
                values=vtext,
                fill="0.0",
                result=floats(col_lens),
                note=note,
            )

            for fill in (-1.0, 0.0):
                add(
                    "stack_expand",
                    n_cols=n_cols,
                    col_arr=col_text,
                    values=vtext,
                    fill=floats([fill]),
                    result=floats(
                        stack_expand_mapped_nb(values, (col_idxs, col_lens), fill)
                    ),
                    note=note,
                )

            for k in (0, 1, 2, 3, 5):
                for which, func in (
                    ("top_n", top_n_inout_map_nb),
                    ("bottom_n", bottom_n_inout_map_nb),
                ):
                    mask = mapped_to_mask_nb(values, (col_idxs, col_lens), func, k)
                    add(
                        which,
                        n_cols=n_cols,
                        col_arr=col_text,
                        values=vtext,
                        n=k,
                        result=bits(mask),
                        note=note,
                    )

            # Indices. NaN is allowed here — the substitution it triggers is
            # exactly what the infinity value set exists to probe — but a
            # column that is *entirely* NaN makes the reference raise, so
            # those assignments are skipped and the port's answer for them is
            # pinned by a Rust unit test instead.
            all_nan_col = any(
                lens and bool(np.isnan(values[col_idxs[start : start + lens]]).all())
                for start, lens in zip(
                    np.cumsum(col_lens) - col_lens, col_lens, strict=True
                )
            )
            if n and not all_nan_col:
                idx_arr = np.array([10 * (i + 1) for i in range(n)], dtype=np.int64)
                for which, func in (
                    ("idxmax", argmax_reduce_nb),
                    ("idxmin", argmin_reduce_nb),
                ):
                    res = reduce_mapped_to_idx_nb(
                        values, (col_idxs, col_lens), idx_arr, -1.0, func
                    )
                    add(
                        which,
                        n_cols=n_cols,
                        col_arr=col_text,
                        values=vtext,
                        idx_arr=ints(idx_arr),
                        fill="-1",
                        result=ints(res),
                        note=note,
                    )

        # Value counts, over a code per record.
        for n_uniques in (1, 2, 3):
            if n == 0:
                codes = np.empty(0, dtype=np.int64)
            else:
                codes = np.array([i % n_uniques for i in range(n)], dtype=np.int64)
            add(
                "value_counts",
                n_cols=n_cols,
                col_arr=col_text,
                codes=ints(codes),
                n_uniques=n_uniques,
                result=ints(mapped_value_counts_nb(codes, n_uniques, (col_idxs, col_lens))),
            )

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "mapped.tsv",
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
