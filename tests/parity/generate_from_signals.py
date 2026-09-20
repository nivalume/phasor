#!/usr/bin/env python3
"""Generate differential fixtures for the ``from_signals`` simulation loop.

The signal pipeline is already pinned function by function against the
reference, and the execution kernel before it. What this file tests is
everything between them and around them: the stop engine, the order the bar's
stages run in, and the bookkeeping that spans bars.

The sequencing is the part that unit tests cannot reach on their own, and it is
where a port drifts:

* a stop is tested **before** it trails, so a bar that makes a new extreme does
  not also stop out on its own pullback;
* the valuation price is resolved **before** a stop can rewrite the order
  price, so a stop fill does not retroactively change what the bar was valued
  at;
* conflicts are resolved **only** when an entry is involved;
* stops are cleared when a position closes and re-anchored when it reverses,
  but re-anchored on an increase only if the update mode says so.

Each case is one row. Array-valued parameters carry an encoding prefix, the
same dialect the ``from_orders`` generator uses::

    s:v          scalar
    c:a;b;c      one value per column
    r:a;b;c      one value per row
    f:...        a full matrix, column-major

Usage::

    python tests/parity/generate_from_signals.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
import math
import random
from pathlib import Path

import numpy as np
from vectorbt.portfolio.enums import (
    AccumulationMode,
    ConflictMode,
    Direction,
    DirectionConflictMode,
    OppositeEntryMode,
    SizeType,
    StopEntryPrice,
    StopExitMode,
    StopExitPrice,
    StopUpdateMode,
)
from vectorbt.portfolio.nb import simulate_from_signals_nb

from generate_execute_order import fmt_float
from generate_from_orders import Enc, c, f, r, s

SIDE_NAMES = {0: "Buy", 1: "Sell"}
DIRECTION_NAMES = {0: "LongOnly", 1: "ShortOnly", 2: "Both"}
SIZE_TYPE_NAMES = {0: "Amount", 1: "Value", 2: "Percent"}
ACCUMULATE_NAMES = {0: "Disabled", 1: "Both", 2: "AddOnly", 3: "RemoveOnly"}
CONFLICT_NAMES = {0: "Ignore", 1: "Entry", 2: "Exit", 3: "Adjacent", 4: "Opposite"}
DIR_CONFLICT_NAMES = {0: "Ignore", 1: "Long", 2: "Short", 3: "Adjacent", 4: "Opposite"}
OPPOSITE_NAMES = {0: "Ignore", 1: "Close", 2: "CloseReduce", 3: "Reverse", 4: "ReverseReduce"}
STOP_ENTRY_NAMES = {0: "ValPrice", 1: "Price", 2: "FillPrice", 3: "Close"}
STOP_EXIT_PRICE_NAMES = {0: "StopLimit", 1: "StopMarket", 2: "Price", 3: "Close"}
STOP_EXIT_NAMES = {0: "Close", 1: "CloseReduce", 2: "Reverse", 3: "ReverseReduce"}
STOP_UPDATE_NAMES = {0: "Keep", 1: "Override", 2: "OverrideNaN"}

ARRAY_PARAMS = [
    "open",
    "high",
    "low",
    "close",
    "entries",
    "exits",
    "direction",
    "long_entries",
    "long_exits",
    "short_entries",
    "short_exits",
    "size",
    "size_type",
    "price",
    "fees",
    "fixed_fees",
    "slippage",
    "min_size",
    "max_size",
    "size_granularity",
    "lock_cash",
    "allow_partial",
    "accumulate",
    "upon_long_conflict",
    "upon_short_conflict",
    "upon_dir_conflict",
    "upon_opposite_entry",
    "sl_stop",
    "sl_trail",
    "tp_stop",
    "stop_entry_price",
    "stop_exit_price",
    "upon_stop_exit",
    "upon_stop_update",
    "val_price",
]

OUTPUT_COLUMNS = ["outcome", "rec_idx", "rec_col", "rec_size", "rec_price", "rec_fees", "rec_side"]

COLUMNS = (
    ["case", "n_rows", "n_cols", "init_cash"]
    + ARRAY_PARAMS
    + ["use_stops", "ffill_val_price", "update_value"]
    + OUTPUT_COLUMNS
    + ["note"]
)


def fmt_bool(v) -> str:
    return "true" if v else "false"


FORMATTERS = {
    "direction": lambda v: DIRECTION_NAMES[int(v)],
    "size_type": lambda v: SIZE_TYPE_NAMES[int(v)],
    "accumulate": lambda v: ACCUMULATE_NAMES[int(v)],
    "upon_long_conflict": lambda v: CONFLICT_NAMES[int(v)],
    "upon_short_conflict": lambda v: CONFLICT_NAMES[int(v)],
    "upon_dir_conflict": lambda v: DIR_CONFLICT_NAMES[int(v)],
    "upon_opposite_entry": lambda v: OPPOSITE_NAMES[int(v)],
    "stop_entry_price": lambda v: STOP_ENTRY_NAMES[int(v)],
    "stop_exit_price": lambda v: STOP_EXIT_PRICE_NAMES[int(v)],
    "upon_stop_exit": lambda v: STOP_EXIT_NAMES[int(v)],
    "upon_stop_update": lambda v: STOP_UPDATE_NAMES[int(v)],
    "entries": fmt_bool,
    "exits": fmt_bool,
    "long_entries": fmt_bool,
    "long_exits": fmt_bool,
    "short_entries": fmt_bool,
    "short_exits": fmt_bool,
    "sl_trail": fmt_bool,
    "lock_cash": fmt_bool,
    "allow_partial": fmt_bool,
}

DTYPES = {
    "direction": np.int64,
    "size_type": np.int64,
    "accumulate": np.int64,
    "upon_long_conflict": np.int64,
    "upon_short_conflict": np.int64,
    "upon_dir_conflict": np.int64,
    "upon_opposite_entry": np.int64,
    "stop_entry_price": np.int64,
    "stop_exit_price": np.int64,
    "upon_stop_exit": np.int64,
    "upon_stop_update": np.int64,
    "entries": np.bool_,
    "exits": np.bool_,
    "long_entries": np.bool_,
    "long_exits": np.bool_,
    "short_entries": np.bool_,
    "short_exits": np.bool_,
    "sl_trail": np.bool_,
    "lock_cash": np.bool_,
    "allow_partial": np.bool_,
}

DEFAULTS = {
    "open": s(math.nan),
    "high": s(math.nan),
    "low": s(math.nan),
    "close": s(math.nan),
    "entries": s(False),
    "exits": s(False),
    "direction": s(Direction.LongOnly),
    "long_entries": s(False),
    "long_exits": s(False),
    "short_entries": s(False),
    "short_exits": s(False),
    "size": s(math.inf),
    "size_type": s(SizeType.Amount),
    "price": s(math.inf),
    "fees": s(0.0),
    "fixed_fees": s(0.0),
    "slippage": s(0.0),
    "min_size": s(0.0),
    "max_size": s(math.inf),
    "size_granularity": s(math.nan),
    "lock_cash": s(False),
    "allow_partial": s(True),
    "accumulate": s(AccumulationMode.Disabled),
    "upon_long_conflict": s(ConflictMode.Ignore),
    "upon_short_conflict": s(ConflictMode.Ignore),
    "upon_dir_conflict": s(DirectionConflictMode.Ignore),
    "upon_opposite_entry": s(OppositeEntryMode.ReverseReduce),
    "sl_stop": s(math.nan),
    "sl_trail": s(False),
    "tp_stop": s(math.nan),
    "stop_entry_price": s(StopEntryPrice.Close),
    "stop_exit_price": s(StopExitPrice.StopLimit),
    "upon_stop_exit": s(StopExitMode.Close),
    "upon_stop_update": s(StopUpdateMode.Override),
    "val_price": s(math.inf),
}


class Case:
    def __init__(
        self,
        name: str,
        n_rows: int,
        n_cols: int,
        init_cash: float | list[float] = 1000.0,
        *,
        use_stops: bool = True,
        ffill_val_price: bool = True,
        update_value: bool = False,
        note: str = "",
        **params: Enc,
    ) -> None:
        self.name = name
        self.n_rows = n_rows
        self.n_cols = n_cols
        self.init_cash = (
            init_cash if isinstance(init_cash, list) else [float(init_cash)] * n_cols
        )
        self.use_stops = use_stops
        self.ffill_val_price = ffill_val_price
        self.update_value = update_value
        self.note = note
        self.params = dict(DEFAULTS)
        for key, value in params.items():
            assert key in DEFAULTS, f"unknown parameter {key}"
            self.params[key] = value

    def array(self, name: str):
        return self.params[name].to_numpy(self.n_rows, self.n_cols, DTYPES.get(name, np.float64))

    def text(self, name: str) -> str:
        return self.params[name].text(FORMATTERS.get(name, fmt_float))


def run(case: Case) -> dict[str, str]:
    try:
        records, _ = simulate_from_signals_nb(
            target_shape=(case.n_rows, case.n_cols),
            group_lens=np.full(case.n_cols, 1),
            init_cash=np.asarray(case.init_cash, dtype=np.float64),
            call_seq=np.full((case.n_rows, case.n_cols), 0),
            **{name: case.array(name) for name in ARRAY_PARAMS},
            use_stops=case.use_stops,
            ffill_val_price=case.ffill_val_price,
            update_value=case.update_value,
            flex_2d=True,
        )
    except Exception as exc:  # noqa: BLE001 - the rejection is the fixture
        return {"outcome": "error", "note": type(exc).__name__}

    if len(records) == 0:
        return {
            "outcome": "ok",
            "rec_idx": "-",
            "rec_col": "-",
            "rec_size": "-",
            "rec_price": "-",
            "rec_fees": "-",
            "rec_side": "-",
        }
    return {
        "outcome": "ok",
        "rec_idx": ";".join(str(v) for v in records["idx"]),
        "rec_col": ";".join(str(v) for v in records["col"]),
        "rec_size": ";".join(fmt_float(v) for v in records["size"]),
        "rec_price": ";".join(fmt_float(v) for v in records["price"]),
        "rec_fees": ";".join(fmt_float(v) for v in records["fees"]),
        "rec_side": ";".join(SIDE_NAMES[int(v)] for v in records["side"]),
    }


def handpicked() -> list[Case]:
    out: list[Case] = []

    out.append(
        Case(
            "entry_then_exit",
            3,
            1,
            close=r([10.0, 11.0, 12.0]),
            entries=r([True, False, False]),
            exits=r([False, False, True]),
            size=s(5.0),
            use_stops=False,
            note="the simplest round trip",
        )
    )
    out.append(
        Case(
            "both_direction_exit_is_a_short_entry",
            2,
            1,
            close=r([10.0, 10.0]),
            entries=r([True, False]),
            exits=r([False, True]),
            direction=s(Direction.Both),
            size=s(5.0),
            use_stops=False,
            note="in a two-sided column the pair means long-or-short",
        )
    )
    out.append(
        Case(
            "ls_mode_is_ored_with_direction_mode",
            3,
            1,
            close=r([10.0, 10.0, 10.0]),
            entries=r([True, False, False]),
            short_entries=r([False, True, False]),
            direction=s(Direction.Both),
            size=s(5.0),
            use_stops=False,
            note="both calling conventions driving one column",
        )
    )

    # --- stops -----------------------------------------------------------
    out.append(
        Case(
            "stop_inside_the_bar",
            2,
            1,
            open=r([10.0, 9.8]),
            high=r([10.0, 10.0]),
            low=r([10.0, 8.0]),
            close=r([10.0, 9.5]),
            entries=r([True, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            note="the level sits in the range: fills at the level",
        )
    )
    out.append(
        Case(
            "stop_gapped_through",
            2,
            1,
            open=r([10.0, 7.0]),
            high=r([10.0, 8.0]),
            low=r([10.0, 7.0]),
            close=r([10.0, 7.5]),
            entries=r([True, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            note="the bar opened through it: fills at the open, worse than the stop",
        )
    )
    out.append(
        Case(
            "take_profit",
            2,
            1,
            open=r([10.0, 10.5]),
            high=r([10.0, 12.0]),
            low=r([10.0, 10.5]),
            close=r([10.0, 11.5]),
            entries=r([True, False]),
            size=s(5.0),
            tp_stop=s(0.1),
            note="a stop that triggers upward",
        )
    )
    out.append(
        Case(
            "trailing_stop_tested_before_it_trails",
            3,
            1,
            open=r([10.0, 11.0, 19.0]),
            high=r([10.0, 20.0, 19.0]),
            low=r([10.0, 11.0, 17.0]),
            close=r([10.0, 19.0, 17.5]),
            entries=r([True, False, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            sl_trail=s(True),
            note="a new high must not stop out on its own pullback",
        )
    )
    out.append(
        Case(
            "trailing_stop_on_a_short",
            4,
            1,
            open=r([10.0, 9.0, 5.5, 6.0]),
            high=r([10.0, 9.5, 6.0, 7.0]),
            low=r([10.0, 5.0, 5.0, 6.0]),
            close=r([10.0, 5.5, 5.5, 6.5]),
            short_entries=r([True, False, False, False]),
            direction=s(Direction.ShortOnly),
            size=s(5.0),
            sl_stop=s(0.1),
            sl_trail=s(True),
            note="the short case trails the low, not the high",
        )
    )
    out.append(
        Case(
            "sl_and_tp_together",
            3,
            1,
            open=r([10.0, 10.2, 9.5]),
            high=r([10.0, 11.5, 10.0]),
            low=r([10.0, 9.8, 8.5]),
            close=r([10.0, 11.0, 9.0]),
            entries=r([True, False, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            tp_stop=s(0.1),
            note="both armed; the stop-loss is tested first",
        )
    )
    out.append(
        Case(
            "stops_cleared_on_close_and_rearmed_on_reentry",
            4,
            1,
            close=r([10.0, 10.0, 10.0, 10.0]),
            open=r([10.0, 10.0, 10.0, 10.0]),
            high=r([10.0, 10.0, 10.0, 10.0]),
            low=r([10.0, 10.0, 10.0, 10.0]),
            entries=r([True, False, True, False]),
            exits=r([False, True, False, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            note="a surviving stop would stop the re-entry out at once",
        )
    )
    for mode, name in STOP_EXIT_NAMES.items():
        out.append(
            Case(
                f"stop_exit_mode_{name}",
                3,
                1,
                open=r([10.0, 8.5, 9.0]),
                high=r([10.0, 9.5, 9.5]),
                low=r([10.0, 8.0, 8.5]),
                close=r([10.0, 9.0, 9.0]),
                entries=r([True, False, False]),
                size=s(5.0),
                sl_stop=s(0.1),
                upon_stop_exit=s(mode),
                accumulate=s(AccumulationMode.Both),
                note=f"a fired stop under StopExitMode.{name}",
            )
        )
    for mode, name in STOP_EXIT_PRICE_NAMES.items():
        out.append(
            Case(
                f"stop_exit_price_{name}",
                2,
                1,
                open=r([10.0, 9.5]),
                high=r([10.0, 9.8]),
                low=r([10.0, 8.0]),
                close=r([10.0, 8.5]),
                entries=r([True, False]),
                size=s(5.0),
                sl_stop=s(0.1),
                slippage=s(0.01),
                stop_exit_price=s(mode),
                note=f"which price a stop fills at, and whether slippage applies",
            )
        )
    for mode, name in STOP_ENTRY_NAMES.items():
        out.append(
            Case(
                f"stop_entry_price_{name}",
                3,
                1,
                open=r([9.0, 9.0, 9.0]),
                high=r([11.0, 11.0, 11.0]),
                low=r([8.0, 8.0, 8.0]),
                close=r([10.0, 10.0, 10.0]),
                price=s(10.5),
                entries=r([True, False, False]),
                size=s(5.0),
                slippage=s(0.02),
                sl_stop=s(0.1),
                stop_entry_price=s(mode),
                note=f"which price the stop is anchored to under {name}",
            )
        )
    for mode, name in STOP_UPDATE_NAMES.items():
        out.append(
            Case(
                f"stop_update_{name}",
                4,
                1,
                open=r([10.0, 12.0, 12.0, 11.0]),
                high=r([10.5, 12.5, 12.5, 11.5]),
                low=r([9.5, 11.5, 10.0, 9.0]),
                close=r([10.0, 12.0, 12.0, 10.5]),
                entries=r([True, True, False, False]),
                size=s(2.0),
                accumulate=s(AccumulationMode.AddOnly),
                sl_stop=s(0.1),
                upon_stop_update=s(mode),
                note=f"adding to a position under StopUpdateMode.{name}",
            )
        )

    # Four cases below exist because a mutation survived without them, and
    # each needed its amplifier constructed rather than found: the difference
    # has to change *whether or where* a stop fires, not merely a number that
    # no branch reads.
    out.append(
        Case(
            "stop_with_no_high_or_low",
            2,
            1,
            open=r([10.0, 9.5]),
            close=r([10.0, 8.5]),
            entries=r([True, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            note="high and low reconstructed from open and close; the level "
            "sits between them, so a missing low would lose the stop entirely",
        )
    )
    out.append(
        Case(
            "partial_exit_does_not_rearm_the_stop",
            3,
            1,
            open=r([10.0, 12.0, 10.0]),
            high=r([10.0, 12.0, 10.5]),
            low=r([10.0, 12.0, 9.5]),
            close=r([10.0, 12.0, 10.0]),
            entries=r([True, False, False]),
            exits=r([False, True, False]),
            # The exit has to be *smaller* than the position, or it closes it
            # and takes the clearing branch instead of the reducing one. The
            # first attempt at this case used one size for both and proved
            # nothing.
            size=r([5.0, 2.0, 2.0]),
            init_cash=1000.0,
            accumulate=s(AccumulationMode.Both),
            sl_stop=s(0.1),
            note="reducing is neither opening nor adding, so the stop keeps "
            "its original anchor: re-anchoring at 12 would stop out on bar 2",
        )
    )
    out.append(
        Case(
            "adding_with_a_nan_stop_keeps_the_old_one",
            3,
            1,
            open=r([10.0, 12.0, 9.5]),
            high=r([10.0, 12.0, 9.8]),
            low=r([10.0, 12.0, 8.5]),
            close=r([10.0, 12.0, 9.0]),
            entries=r([True, True, False]),
            size=s(2.0),
            accumulate=s(AccumulationMode.AddOnly),
            sl_stop=r([0.1, math.nan, math.nan]),
            upon_stop_update=s(StopUpdateMode.Override),
            note="Override with a NaN stop must keep the existing one, not "
            "overwrite it with NaN and disarm the stop altogether",
        )
    )
    out.append(
        Case(
            "val_price_anchor_is_the_pre_stop_order_price",
            2,
            1,
            open=r([12.0, 10.5]),
            high=r([12.0, 11.0]),
            low=r([12.0, 10.2]),
            close=r([10.0, 10.6]),
            price=s(12.0),
            entries=r([True, False]),
            size=s(5.0),
            sl_stop=s(0.1),
            stop_entry_price=s(StopEntryPrice.ValPrice),
            note="the order price and the close differ, so anchoring to the "
            "wrong one moves the level from 10.8 to 9 and loses the stop",
        )
    )

    # --- accumulation and conflicts ---------------------------------------
    for mode, name in ACCUMULATE_NAMES.items():
        out.append(
            Case(
                f"accumulate_{name}",
                4,
                1,
                close=r([10.0, 10.0, 10.0, 10.0]),
                entries=r([True, True, False, False]),
                exits=r([False, False, True, False]),
                size=s(2.0),
                accumulate=s(mode),
                use_stops=False,
                note=f"AccumulationMode.{name}",
            )
        )
    for mode, name in CONFLICT_NAMES.items():
        out.append(
            Case(
                f"long_conflict_{name}",
                3,
                1,
                close=r([10.0, 10.0, 10.0]),
                entries=r([True, True, False]),
                exits=r([False, True, False]),
                size=s(2.0),
                upon_long_conflict=s(mode),
                use_stops=False,
                note=f"entry and exit on one bar under ConflictMode.{name}",
            )
        )
    for mode, name in DIR_CONFLICT_NAMES.items():
        out.append(
            Case(
                f"dir_conflict_{name}",
                3,
                1,
                close=r([10.0, 10.0, 10.0]),
                long_entries=r([True, True, False]),
                short_entries=r([False, True, False]),
                size=s(2.0),
                upon_dir_conflict=s(mode),
                use_stops=False,
                note=f"a long and a short entry together under {name}",
            )
        )
    for mode, name in OPPOSITE_NAMES.items():
        out.append(
            Case(
                f"opposite_entry_{name}",
                3,
                1,
                close=r([10.0, 10.0, 10.0]),
                long_entries=r([True, False, False]),
                short_entries=r([False, True, False]),
                size=s(2.0),
                upon_opposite_entry=s(mode),
                accumulate=s(AccumulationMode.Both),
                use_stops=False,
                note=f"a short entry while long under OppositeEntryMode.{name}",
            )
        )

    # --- sizing and prices ------------------------------------------------
    for st, name in SIZE_TYPE_NAMES.items():
        out.append(
            Case(
                f"size_type_{name}",
                3,
                1,
                close=r([10.0, 11.0, 12.0]),
                entries=r([True, False, False]),
                exits=r([False, False, True]),
                size=s(0.5 if name == "Percent" else 20.0),
                size_type=s(st),
                use_stops=False,
                note=f"SizeType.{name}",
            )
        )
    out.append(
        Case(
            "price_open_sentinel",
            3,
            1,
            open=r([9.0, 10.0, 11.0]),
            close=r([10.0, 11.0, 12.0]),
            price=s(-math.inf),
            entries=r([True, False, False]),
            exits=r([False, False, True]),
            size=s(5.0),
            use_stops=False,
            note="-inf is this bar's open here, not the previous close",
        )
    )
    out.append(
        Case(
            "price_open_sentinel_without_an_open",
            3,
            1,
            close=r([10.0, 11.0, 12.0]),
            price=s(-math.inf),
            entries=r([True, False, False]),
            exits=r([False, False, True]),
            size=s(5.0),
            use_stops=False,
            note="with no open it falls back to the previous close",
        )
    )
    out.append(
        Case(
            "infinite_size_goes_all_in",
            3,
            1,
            close=r([10.0, 11.0, 12.0]),
            entries=r([True, False, False]),
            exits=r([False, False, True]),
            use_stops=False,
            note="the default size",
        )
    )
    out.append(
        Case(
            "two_columns_diverge",
            4,
            2,
            close=f([10.0, 11.0, 12.0, 13.0, 10.0, 9.0, 8.0, 7.0]),
            entries=c([True, True]),
            exits=f([False] * 3 + [True] + [False] * 3 + [True]),
            size=s(3.0),
            use_stops=False,
            note="columns must not leak state into one another",
        )
    )
    out.append(
        Case(
            "per_column_stops",
            3,
            2,
            open=f([10.0, 9.0, 9.0] * 2),
            high=f([10.0, 9.5, 9.5] * 2),
            low=f([10.0, 8.0, 8.0] * 2),
            close=f([10.0, 9.0, 9.0] * 2),
            entries=c([True, True]),
            size=s(3.0),
            sl_stop=c([0.05, 0.5]),
            note="one column stops out and the other does not",
        )
    )
    out.append(
        Case(
            "no_signals_at_all",
            3,
            1,
            close=r([10.0, 11.0, 12.0]),
            note="an empty record set is a valid answer",
        )
    )
    out.append(
        Case(
            "negative_size_is_rejected",
            2,
            1,
            close=r([10.0, 11.0]),
            entries=r([True, False]),
            size=s(-5.0),
            use_stops=False,
            note="direction belongs to the signals, not the sign of the size",
        )
    )
    out.append(
        Case(
            "percent_reversal_is_rejected",
            3,
            1,
            close=r([10.0, 10.0, 10.0]),
            entries=r([True, False, False]),
            exits=r([False, True, False]),
            direction=s(Direction.Both),
            size=s(0.5),
            size_type=s(SizeType.Percent),
            use_stops=False,
            note="a percentage cannot say how much of what to reverse into",
        )
    )
    out.append(
        Case(
            "update_value_remarks_to_the_fill",
            4,
            1,
            close=r([10.0, math.nan, 12.0, 13.0]),
            price=r([10.0, 11.0, 12.0, 13.0]),
            entries=r([True, False, True, False]),
            exits=r([False, False, False, True]),
            size=s(20.0),
            size_type=s(SizeType.Value),
            accumulate=s(AccumulationMode.Both),
            use_stops=False,
            update_value=True,
            note="the flag only shows through a bar whose valuation is unusable",
        )
    )
    out.append(
        Case(
            "costs_on",
            4,
            1,
            open=r([10.0, 10.0, 9.0, 9.0]),
            high=r([10.5, 10.5, 9.5, 9.5]),
            low=r([9.5, 9.5, 8.0, 8.0]),
            close=r([10.0, 10.0, 9.0, 9.0]),
            entries=r([True, False, False, False]),
            size=s(5.0),
            fees=s(0.01),
            fixed_fees=s(0.5),
            slippage=s(0.002),
            sl_stop=s(0.08),
            note="fees and slippage through a stop exit",
        )
    )
    return out


def random_cases(seed: int, count: int) -> list[Case]:
    rng = random.Random(seed)
    npr = np.random.default_rng(seed)
    out = []
    for n in range(count):
        n_rows = rng.randint(5, 14)
        n_cols = rng.choice([1, 1, 2, 3])
        total = n_rows * n_cols

        base = npr.normal(20.0, 2.0, size=total)
        close = np.abs(base) + 5.0
        # Highs and lows bracket the close, so stop levels are reachable.
        high = close * (1.0 + npr.random(total) * 0.08)
        low = close * (1.0 - npr.random(total) * 0.08)
        open_ = low + (high - low) * npr.random(total)

        use_stops = rng.random() < 0.7
        out.append(
            Case(
                f"random_{n:03d}",
                n_rows,
                n_cols,
                init_cash=1000.0,
                open=f(list(open_)),
                high=f(list(high)),
                low=f(list(low)),
                close=f(list(close)),
                entries=f([bool(v) for v in npr.random(total) < 0.25]),
                exits=f([bool(v) for v in npr.random(total) < 0.25]),
                direction=s(rng.choice([Direction.LongOnly, Direction.ShortOnly, Direction.Both])),
                long_entries=f([bool(v) for v in npr.random(total) < 0.1]),
                short_entries=f([bool(v) for v in npr.random(total) < 0.1]),
                long_exits=f([bool(v) for v in npr.random(total) < 0.1]),
                short_exits=f([bool(v) for v in npr.random(total) < 0.1]),
                size=s(rng.choice([2.0, 5.0, math.inf])),
                fees=s(rng.choice([0.0, 0.01])),
                fixed_fees=s(rng.choice([0.0, 0.5])),
                slippage=s(rng.choice([0.0, 0.005])),
                accumulate=s(rng.choice(list(ACCUMULATE_NAMES))),
                upon_long_conflict=s(rng.choice(list(CONFLICT_NAMES))),
                upon_short_conflict=s(rng.choice(list(CONFLICT_NAMES))),
                upon_dir_conflict=s(rng.choice(list(DIR_CONFLICT_NAMES))),
                upon_opposite_entry=s(rng.choice(list(OPPOSITE_NAMES))),
                sl_stop=s(rng.choice([math.nan, 0.05, 0.1])),
                sl_trail=s(rng.random() < 0.4),
                tp_stop=s(rng.choice([math.nan, 0.05, 0.15])),
                stop_entry_price=s(rng.choice(list(STOP_ENTRY_NAMES))),
                stop_exit_price=s(rng.choice(list(STOP_EXIT_PRICE_NAMES))),
                upon_stop_exit=s(rng.choice(list(STOP_EXIT_NAMES))),
                upon_stop_update=s(rng.choice(list(STOP_UPDATE_NAMES))),
                use_stops=use_stops,
                note="seeded random case",
            )
        )
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "from_signals.tsv",
    )
    parser.add_argument("--seed", type=int, default=20240919)
    args = parser.parse_args()

    cases = handpicked() + random_cases(args.seed, 120)
    lines = ["\t".join(COLUMNS)]
    fills = 0
    errors = 0
    for case in cases:
        fields = {k: "" for k in COLUMNS}
        fields.update(
            {
                "case": case.name,
                "n_rows": str(case.n_rows),
                "n_cols": str(case.n_cols),
                "init_cash": "c:" + ";".join(fmt_float(v) for v in case.init_cash),
                "use_stops": fmt_bool(case.use_stops),
                "ffill_val_price": fmt_bool(case.ffill_val_price),
                "update_value": fmt_bool(case.update_value),
                "note": case.note,
            }
        )
        for name in ARRAY_PARAMS:
            fields[name] = case.text(name)
        result = run(case)
        # Only the outputs are overwritten, so an error case stays replayable.
        for key in OUTPUT_COLUMNS:
            if key in result:
                fields[key] = result[key]
        if result.get("note"):
            fields["note"] = result["note"]
        if result["outcome"] == "error":
            errors += 1
        elif fields["rec_idx"] != "-":
            fills += len(fields["rec_idx"].split(";"))
        lines.append("\t".join(fields[k] for k in COLUMNS))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")
    print(f"wrote {len(cases)} fixtures to {args.out}")
    print(f"  {fills} fills, {errors} rejection cases")


if __name__ == "__main__":
    main()
