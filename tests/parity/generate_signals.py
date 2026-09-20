#!/usr/bin/env python3
"""Generate differential fixtures for the signal resolution pipeline.

Unlike every other generator here, this one is **exhaustive**. The pipeline's
inputs are a position sign, a handful of booleans and a few small enums, so the
whole space fits in a few thousand rows — and when a space is that small,
sampling it is a choice to test less for no reason. A random fixture set would
almost certainly miss `Adjacent` with a flat position, which drops both
signals, or `CloseReduce`, which differs from `Close` in one field.

Five functions, one row each per input combination:

``signal_conflict``
    An entry and an exit in the same direction.
``dir_conflict``
    A long entry and a short entry together.
``opposite_entry``
    An entry against the position already held.
``stop``
    A fired stop, which overrides the input signals.
``to_size``
    The whole lot, reduced to a size, a size type and a direction.

Usage::

    python tests/parity/generate_signals.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import itertools
import math
from pathlib import Path

import numpy as np
from vectorbt.portfolio.enums import (
    AccumulationMode,
    ConflictMode,
    Direction,
    DirectionConflictMode,
    OppositeEntryMode,
    SizeType,
    StopExitMode,
)
from vectorbt.portfolio.nb import (
    generate_stop_signal_nb,
    resolve_dir_conflict_nb,
    resolve_opposite_entry_nb,
    resolve_signal_conflict_nb,
    signals_to_size_nb,
)

from generate_execute_order import fmt_float

POSITIONS = [-5.0, 0.0, 7.0]
BOOLS = [False, True]

ACCUMULATE_NAMES = {0: "Disabled", 1: "Both", 2: "AddOnly", 3: "RemoveOnly"}
CONFLICT_NAMES = {0: "Ignore", 1: "Entry", 2: "Exit", 3: "Adjacent", 4: "Opposite"}
DIR_CONFLICT_NAMES = {0: "Ignore", 1: "Long", 2: "Short", 3: "Adjacent", 4: "Opposite"}
OPPOSITE_NAMES = {0: "Ignore", 1: "Close", 2: "CloseReduce", 3: "Reverse", 4: "ReverseReduce"}
STOP_EXIT_NAMES = {0: "Close", 1: "CloseReduce", 2: "Reverse", 3: "ReverseReduce"}
DIRECTION_NAMES = {0: "LongOnly", 1: "ShortOnly", 2: "Both"}
SIZE_TYPE_NAMES = {0: "Amount", 1: "Value", 2: "Percent", 3: "TargetAmount", 5: "TargetPercent"}

COLUMNS = [
    "fn",
    "position",
    "long_entry",
    "long_exit",
    "short_entry",
    "short_exit",
    "is_entry",
    "is_exit",
    "direction",
    "mode",
    "accumulate",
    "size",
    "size_type",
    "val_price",
    "outcome",
    "out_long_entry",
    "out_long_exit",
    "out_short_entry",
    "out_short_exit",
    "out_is_entry",
    "out_is_exit",
    "out_accumulate",
    "out_size",
    "out_size_type",
    "out_direction",
]


def b(v: bool) -> str:
    return "true" if v else "false"


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []

    # --- signal_conflict: entry and exit in one direction -------------------
    for pos, is_entry, is_exit, direction, mode in itertools.product(
        POSITIONS, BOOLS, BOOLS, [0, 1, 2], [0, 1, 2, 3, 4]
    ):
        e, x = resolve_signal_conflict_nb(pos, is_entry, is_exit, direction, mode)
        r = blank()
        r.update(
            {
                "fn": "signal_conflict",
                "position": fmt_float(pos),
                "is_entry": b(is_entry),
                "is_exit": b(is_exit),
                "direction": DIRECTION_NAMES[direction],
                "mode": CONFLICT_NAMES[mode],
                "outcome": "ok",
                "out_is_entry": b(e),
                "out_is_exit": b(x),
            }
        )
        out.append(r)

    # --- dir_conflict: a long entry and a short entry together --------------
    for pos, le, se, mode in itertools.product(POSITIONS, BOOLS, BOOLS, [0, 1, 2, 3, 4]):
        a, c = resolve_dir_conflict_nb(pos, le, se, mode)
        r = blank()
        r.update(
            {
                "fn": "dir_conflict",
                "position": fmt_float(pos),
                "long_entry": b(le),
                "short_entry": b(se),
                "mode": DIR_CONFLICT_NAMES[mode],
                "outcome": "ok",
                "out_long_entry": b(a),
                "out_short_entry": b(c),
            }
        )
        out.append(r)

    # --- opposite_entry: an entry against the position held -----------------
    for pos, (le, lx, se, sx), mode, acc in itertools.product(
        POSITIONS,
        itertools.product(BOOLS, repeat=4),
        [0, 1, 2, 3, 4],
        [0, 1, 2, 3],
    ):
        a, bb, c, d, new_acc = resolve_opposite_entry_nb(pos, le, lx, se, sx, mode, acc)
        r = blank()
        r.update(
            {
                "fn": "opposite_entry",
                "position": fmt_float(pos),
                "long_entry": b(le),
                "long_exit": b(lx),
                "short_entry": b(se),
                "short_exit": b(sx),
                "mode": OPPOSITE_NAMES[mode],
                "accumulate": ACCUMULATE_NAMES[acc],
                "outcome": "ok",
                "out_long_entry": b(a),
                "out_long_exit": b(bb),
                "out_short_entry": b(c),
                "out_short_exit": b(d),
                "out_accumulate": ACCUMULATE_NAMES[new_acc],
            }
        )
        out.append(r)

    # --- stop: a fired stop overrides the input signals ---------------------
    for pos, mode, acc in itertools.product(POSITIONS, [0, 1, 2, 3], [0, 1, 2, 3]):
        a, bb, c, d, new_acc = generate_stop_signal_nb(pos, mode, acc)
        r = blank()
        r.update(
            {
                "fn": "stop",
                "position": fmt_float(pos),
                "mode": STOP_EXIT_NAMES[mode],
                "accumulate": ACCUMULATE_NAMES[acc],
                "outcome": "ok",
                "out_long_entry": b(a),
                "out_long_exit": b(bb),
                "out_short_entry": b(c),
                "out_short_exit": b(d),
                "out_accumulate": ACCUMULATE_NAMES[new_acc],
            }
        )
        out.append(r)

    # --- to_size: the whole thing reduced to an order -----------------------
    # `nan` means "as much as the position"; the negative and unsupported
    # cases are here to pin the two rejections.
    sizes = [0.0, 2.0, math.inf, math.nan, -1.0]
    size_types = [SizeType.Amount, SizeType.Value, SizeType.Percent, SizeType.TargetPercent]
    for pos, (le, lx, se, sx), size, st, acc in itertools.product(
        POSITIONS,
        itertools.product(BOOLS, repeat=4),
        sizes,
        size_types,
        [0, 1, 2, 3],
    ):
        r = blank()
        r.update(
            {
                "fn": "to_size",
                "position": fmt_float(pos),
                "long_entry": b(le),
                "long_exit": b(lx),
                "short_entry": b(se),
                "short_exit": b(sx),
                "size": fmt_float(size),
                "size_type": SIZE_TYPE_NAMES[int(st)],
                "accumulate": ACCUMULATE_NAMES[acc],
                "val_price": fmt_float(10.0),
            }
        )
        try:
            order_size, out_type, out_dir = signals_to_size_nb(
                pos, le, lx, se, sx, size, st, acc, 10.0
            )
        except Exception:  # noqa: BLE001 - the rejection is the fixture
            r["outcome"] = "error"
        else:
            r.update(
                {
                    "outcome": "ok",
                    "out_size": fmt_float(order_size),
                    "out_size_type": SIZE_TYPE_NAMES[int(out_type)],
                    "out_direction": DIRECTION_NAMES[int(out_dir)],
                }
            )
        out.append(r)

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "signals.tsv",
    )
    args = parser.parse_args()

    data = rows()
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    by_fn: dict[str, int] = {}
    for r in data:
        by_fn[r["fn"]] = by_fn.get(r["fn"], 0) + 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name, n in by_fn.items():
        print(f"  {name}: {n}")
    errors = sum(1 for r in data if r["outcome"] == "error")
    print(f"  ({errors} rejection cases)")


if __name__ == "__main__":
    main()
