#!/usr/bin/env python3
"""Generate differential fixtures for signal generation, cleaning and ranking.

The inputs are boolean arrays, so the awkward cases are about *shape* rather
than magnitude, and they are cheap to enumerate exhaustively at small sizes.
Every array of up to six bars is covered for the single-column functions,
which is 126 arrays and leaves nothing to chance about runs, leading runs,
trailing runs or empty columns.

The cases that a hand-picked set would miss:

* a column that starts mid-run, where a rank has no way to know how far in it
  already is;
* a run that reaches the last bar, so its range is open rather than closed;
* a reset landing *inside* a run, which restarts the count from that bar
  rather than waiting for the run to end — the opposite of what the code
  reads like;
* a bar carrying both an entry and an exit, which produces neither.

Usage::

    python tests/parity/generate_signals_gen.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import itertools
from pathlib import Path

import numpy as np
from vectorbt.signals.nb import (
    between_partition_ranges_nb,
    between_ranges_nb,
    clean_enex_nb,
    generate_ex_nb,
    nth_index_nb,
    norm_avg_index_nb,
    part_pos_rank_nb,
    partition_ranges_nb,
    rank_nb,
    sig_pos_rank_nb,
)

COLUMNS = [
    "fn",
    "n_rows",
    "n_cols",
    "a",
    "b",
    "reset",
    "entry_first",
    "wait",
    "until_next",
    "skip_until_exit",
    "allow_gaps",
    "after_false",
    "n",
    "result",
    "result2",
    "note",
]


def bits(s: str) -> np.ndarray:
    """`X.X` -> a column vector."""
    return np.array([[c == "X"] for c in s], dtype=np.bool_)


def render(a: np.ndarray) -> str:
    return "".join("X" if v else "." for v in np.asarray(a).flatten(order="F"))


def ints(a) -> str:
    return ";".join(str(int(v)) for v in np.asarray(a).flatten(order="F"))


def floats(a) -> str:
    out = []
    for v in np.asarray(a, dtype=float).flatten(order="F"):
        out.append("nan" if np.isnan(v) else repr(float(v)))
    return ";".join(out)


def ranges_text(records) -> str:
    """Ranges as `col,start,end,status` groups."""
    if len(records) == 0:
        return "-"
    return "|".join(
        f"{int(r['col'])},{int(r['start_idx'])},{int(r['end_idx'])},{int(r['status'])}"
        for r in records
    )


def all_patterns(max_len: int) -> list[str]:
    out = []
    for n in range(1, max_len + 1):
        for combo in itertools.product(".X", repeat=n):
            out.append("".join(combo))
    return out


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    patterns = all_patterns(6)

    def add(fn, **fields):
        r = blank()
        r["fn"] = fn
        r.update({k: str(v) for k, v in fields.items()})
        out.append(r)

    # --- ranking, ranges and index helpers, over every short array ---------
    for p in patterns:
        a = bits(p)
        n = len(p)
        for allow_gaps in (True, False):
            for after_false in (True, False):
                tmp = np.full(1, -1, dtype=np.int64)
                r = rank_nb(a, None, after_false, sig_pos_rank_nb, tmp, allow_gaps)
                add(
                    "signal_rank",
                    n_rows=n,
                    n_cols=1,
                    a=p,
                    allow_gaps=str(allow_gaps),
                    after_false=str(after_false),
                    result=ints(r),
                )
        for after_false in (True, False):
            tmp = np.full(1, 0, dtype=np.int64)
            r = rank_nb(a, None, after_false, part_pos_rank_nb, tmp)
            add(
                "partition_rank",
                n_rows=n,
                n_cols=1,
                a=p,
                after_false=str(after_false),
                result=ints(r),
            )

        add("partition_ranges", n_rows=n, n_cols=1, a=p, result=ranges_text(partition_ranges_nb(a)))
        add("between_ranges", n_rows=n, n_cols=1, a=p, result=ranges_text(between_ranges_nb(a)))
        add(
            "between_partition_ranges",
            n_rows=n,
            n_cols=1,
            a=p,
            result=ranges_text(between_partition_ranges_nb(a)),
        )
        add("norm_avg_index", n_rows=n, n_cols=1, a=p, result=floats(norm_avg_index_nb(a)))
        for nth in (0, 1, 2, -1, -2):
            add("nth_index", n_rows=n, n_cols=1, a=p, n=nth, result=ints(nth_index_nb(a, nth)))

    # --- ranking with a reset, which needs two arrays ----------------------
    reset_cases = [
        ("XXXX", ".X..", "a reset inside a run renumbers from that bar"),
        ("XX.XX.XX", "......X.", "a reset between runs"),
        ("XXXX", "X...", "a reset on the first bar"),
        ("XXXX", "...X", "a reset on the last bar"),
        ("X.X.X.", ".X.X.X", "a reset on every quiet bar"),
        ("......", "XXXXXX", "resets with no signals at all"),
        ("XXXXXX", "......", "no resets at all"),
        (".XX.XX", "X.....", "a stale reset must not fire in a later run"),
    ]
    for pattern, reset, note in reset_cases:
        a, rst = bits(pattern), bits(reset)
        n = len(pattern)
        for allow_gaps in (True, False):
            tmp = np.full(1, -1, dtype=np.int64)
            add(
                "signal_rank",
                n_rows=n,
                n_cols=1,
                a=pattern,
                reset=reset,
                allow_gaps=str(allow_gaps),
                after_false="False",
                result=ints(rank_nb(a, rst, False, sig_pos_rank_nb, tmp, allow_gaps)),
                note=note,
            )
        tmp = np.full(1, 0, dtype=np.int64)
        add(
            "partition_rank",
            n_rows=n,
            n_cols=1,
            a=pattern,
            reset=reset,
            after_false="False",
            result=ints(rank_nb(a, rst, False, part_pos_rank_nb, tmp)),
            note=note,
        )

    # --- cleaning, over every pair of short arrays -------------------------
    for pa in all_patterns(5):
        for pb in all_patterns(5):
            if len(pa) != len(pb):
                continue
            e, x = bits(pa), bits(pb)
            for entry_first in (True, False):
                ce, cx = clean_enex_nb(e, x, entry_first)
                add(
                    "clean_enex",
                    n_rows=len(pa),
                    n_cols=1,
                    a=pa,
                    b=pb,
                    entry_first=str(entry_first),
                    result=render(ce),
                    result2=render(cx),
                )

    # --- exit generation ---------------------------------------------------
    # The reference's `generate_ex_nb` takes a choice function; the fixed-wait
    # exit is the one this port exposes, so the choice here just takes the
    # first bar of the window.
    from numba import njit

    @njit
    def first_choice(from_i, to_i, col):
        return np.array([from_i])

    for p in all_patterns(6):
        e = bits(p)
        n = len(p)
        for wait in (1, 2, 3):
            for until_next, skip in ((True, False), (False, False), (False, True)):
                x = generate_ex_nb(e, wait, until_next, skip, True, first_choice)
                add(
                    "exit_after",
                    n_rows=n,
                    n_cols=1,
                    a=p,
                    wait=wait,
                    until_next=str(until_next),
                    skip_until_exit=str(skip),
                    result=render(x),
                )

    # --- a few multi-column cases, to pin the column-major layout ----------
    multi = [
        (["X.X.", "..X."], ["....", "X..X"]),
        (["XXXX", "...."], ["....", "XXXX"]),
        ([".X.X", "X.X."], ["X.X.", ".X.X"]),
    ]
    for ea, xa in multi:
        e = np.array([[c == "X" for c in col] for col in ea], dtype=np.bool_).T
        x = np.array([[c == "X" for c in col] for col in xa], dtype=np.bool_).T
        ce, cx = clean_enex_nb(e, x, True)
        add(
            "clean_enex",
            n_rows=e.shape[0],
            n_cols=e.shape[1],
            a=render(e),
            b=render(x),
            entry_first="True",
            result=render(ce),
            result2=render(cx),
            note="two columns, laid out column-major",
        )
        add(
            "partition_ranges",
            n_rows=e.shape[0],
            n_cols=e.shape[1],
            a=render(e),
            result=ranges_text(partition_ranges_nb(e)),
            note="two columns",
        )
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "signals_gen.tsv",
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
