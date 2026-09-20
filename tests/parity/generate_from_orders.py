#!/usr/bin/env python3
"""Generate differential fixtures for the ``from_orders`` simulation loop.

The kernel harnesses verify one order, and chains of orders, against the
reference. This one verifies the loop that drives them: price-sentinel
resolution, valuation-price forward-fill, per-element parameter broadcasting,
and the shape of the resulting record arrays.

Each case is one row. Array-valued parameters carry an encoding prefix, which
also exercises all four broadcasting paths:

===========  ==========================================================
``s:v``      scalar, constant everywhere
``c:a;b;c``  one value per column, constant down the rows
``r:a;b;c``  one value per row, constant across the columns
``f:...``    a full matrix, **column-major**, ``n_rows * n_cols`` values
===========  ==========================================================

Column-major for the full case because that is the engine's layout: each
column is an independent simulation scanned start to finish. NumPy is
row-major, so the generator reshapes with ``order="F"``.

Usage::

    python tests/parity/generate_from_orders.py

Setup is the same as the other generators; see the README in this directory.

Not covered: cash sharing and call sequencing. Symbols are backtested
independently here, so ``group_lens`` is all ones and no group ever needs its
sells ordered before its buys.
"""

from __future__ import annotations

import argparse
import math
import random
from pathlib import Path

import numpy as np
from vectorbt.portfolio.enums import Direction, SizeType
from vectorbt.portfolio.nb import simulate_from_orders_nb

from generate_execute_order import DIRECTION_NAMES, SIZE_TYPE_NAMES, fmt_float

SIDE_NAMES = {0: "Buy", 1: "Sell"}

# Parameters that take an encoded array. Order matters: it is the column order.
ARRAY_PARAMS = [
    "close",
    "size",
    "price",
    "size_type",
    "direction",
    "fees",
    "fixed_fees",
    "slippage",
    "min_size",
    "max_size",
    "size_granularity",
    "lock_cash",
    "allow_partial",
    "val_price",
]

COLUMNS = (
    ["case", "n_rows", "n_cols", "init_cash"]
    + ARRAY_PARAMS
    + [
        "ffill_val_price",
        "update_value",
        "outcome",
        "rec_col",
        "rec_idx",
        "rec_size",
        "rec_price",
        "rec_fees",
        "rec_side",
        "note",
    ]
)


class Enc:
    """An encoded array parameter: a kind plus its values."""

    def __init__(self, kind: str, values) -> None:
        assert kind in ("s", "c", "r", "f")
        self.kind = kind
        self.values = values if isinstance(values, (list, tuple)) else [values]

    def text(self, fmt) -> str:
        return f"{self.kind}:" + ";".join(fmt(v) for v in self.values)

    def to_numpy(self, n_rows: int, n_cols: int, dtype):
        """Builds the array the reference expects for this encoding.

        With ``flex_2d=True`` the reference reads a 1-D array as one value per
        column; a per-row array therefore has to be given shape
        ``(n_rows, 1)``.
        """
        vals = np.asarray(self.values, dtype=dtype)
        if self.kind == "s":
            return np.asarray(vals[0], dtype=dtype)
        if self.kind == "c":
            assert len(self.values) == n_cols, "per-column array must be n_cols long"
            return vals
        if self.kind == "r":
            assert len(self.values) == n_rows, "per-row array must be n_rows long"
            return vals.reshape((n_rows, 1))
        assert len(self.values) == n_rows * n_cols, "full array must be n_rows * n_cols"
        # The fixture stores column-major; NumPy defaults to row-major.
        return vals.reshape((n_rows, n_cols), order="F")


def s(v) -> Enc:
    return Enc("s", [v])


def c(vals) -> Enc:
    return Enc("c", list(vals))


def r(vals) -> Enc:
    return Enc("r", list(vals))


def f(vals) -> Enc:
    return Enc("f", list(vals))


# Formatters per parameter: enums render by name, bools as true/false.
def fmt_size_type(v) -> str:
    return SIZE_TYPE_NAMES[int(v)]


def fmt_direction(v) -> str:
    return DIRECTION_NAMES[int(v)]


def fmt_bool(v) -> str:
    return "true" if v else "false"


FORMATTERS = {
    "size_type": fmt_size_type,
    "direction": fmt_direction,
    "lock_cash": fmt_bool,
    "allow_partial": fmt_bool,
}

DTYPES = {
    "size_type": np.int64,
    "direction": np.int64,
    "lock_cash": np.bool_,
    "allow_partial": np.bool_,
}


class Case:
    """One simulation: a shape, a set of encoded parameters, and two flags."""

    def __init__(
        self,
        name: str,
        n_rows: int,
        n_cols: int,
        init_cash: float | list[float],
        *,
        ffill_val_price: bool = True,
        update_value: bool = False,
        **params: Enc,
    ) -> None:
        self.name = name
        self.n_rows = n_rows
        self.n_cols = n_cols
        # A list gives each column its own starting capital; a scalar is
        # shared. Per-column capital is part of the sweep shape and was
        # untested until mutation testing showed the loop could read column 0
        # for every column and nothing would notice.
        self.init_cash = init_cash if isinstance(init_cash, list) else [float(init_cash)] * n_cols
        assert len(self.init_cash) == n_cols, "init_cash must be scalar or one value per column"
        self.ffill_val_price = ffill_val_price
        self.update_value = update_value
        self.params: dict[str, Enc] = {
            "close": s(math.nan),
            "size": s(math.inf),
            "price": s(math.inf),
            "size_type": s(SizeType.Amount),
            "direction": s(Direction.Both),
            "fees": s(0.0),
            "fixed_fees": s(0.0),
            "slippage": s(0.0),
            "min_size": s(0.0),
            "max_size": s(math.inf),
            "size_granularity": s(math.nan),
            "lock_cash": s(False),
            "allow_partial": s(True),
            "val_price": s(math.inf),
        }
        unknown = set(params) - set(self.params)
        assert not unknown, f"unknown parameters: {sorted(unknown)}"
        self.params.update(params)


def ramp(n_rows: int, n_cols: int, start: float = 10.0, step: float = 1.0) -> list[float]:
    """A column-major price ramp: each column drifts from a different base."""
    out: list[float] = []
    for col in range(n_cols):
        for i in range(n_rows):
            out.append(start + col * 5.0 + i * step)
    return out


def cases() -> list[Case]:
    out: list[Case] = []

    # --- the defaults ----------------------------------------------------
    # Infinite size and infinite price: put all cash in at the first close.
    out.append(Case("buy_and_hold", 5, 1, 100.0, close=f([1.0, 2.0, 3.0, 4.0, 5.0])))

    # --- price sentinels -------------------------------------------------
    # -inf is the *previous* close, so the first bar cannot fill.
    out.append(
        Case(
            "price_prev_close",
            5,
            1,
            100.0,
            close=f([1.0, 2.0, 3.0, 4.0, 5.0]),
            price=s(-math.inf),
            size=s(1.0),
        )
    )
    out.append(
        Case(
            "price_explicit",
            5,
            1,
            100.0,
            close=f([1.0, 2.0, 3.0, 4.0, 5.0]),
            price=r([1.5, 2.5, 3.5, 4.5, 5.5]),
            size=s(1.0),
        )
    )
    # A NaN close means no price, so that bar is ignored rather than filled.
    out.append(
        Case(
            "close_with_gaps",
            6,
            1,
            100.0,
            close=f([1.0, math.nan, 3.0, math.nan, 5.0, 6.0]),
            size=s(1.0),
        )
    )

    # --- valuation price and forward fill --------------------------------
    # The valuation price must itself be able to go NaN, or the forward-fill
    # branch is never taken. It resolves to the order price, which defaults to
    # the close — so leaving `price` at its default is what makes a NaN close
    # produce a NaN valuation. An earlier version of these cases passed an
    # explicit `price`, which kept the valuation non-NaN and left ffill
    # completely untested; mutation testing found that.
    for ffill in (True, False):
        out.append(
            Case(
                f"val_price_ffill_{str(ffill).lower()}",
                8,
                1,
                1000.0,
                close=f([10.0, math.nan, math.nan, 13.0, math.nan, 15.0, 16.0, math.nan]),
                size=s(0.25),
                size_type=s(SizeType.TargetPercent),
                ffill_val_price=ffill,
            )
        )
        # Same, reached through the -inf (previous close) sentinel instead.
        out.append(
            Case(
                f"val_price_ffill_prev_close_{str(ffill).lower()}",
                8,
                1,
                1000.0,
                close=f([10.0, math.nan, math.nan, 13.0, math.nan, 15.0, 16.0, 17.0]),
                price=r([10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0]),
                val_price=s(-math.inf),
                size=s(0.25),
                size_type=s(SizeType.TargetPercent),
                ffill_val_price=ffill,
            )
        )
    out.append(
        Case(
            "val_price_prev_close",
            6,
            1,
            1000.0,
            close=f(ramp(6, 1)),
            val_price=s(-math.inf),
            size=s(0.5),
            size_type=s(SizeType.TargetPercent),
        )
    )

    # --- update_value ----------------------------------------------------
    # This only re-marks the valuation price to the *fill* price, so it does
    # nothing unless the fill price differs from the valuation price. Slippage
    # is what separates them: the valuation resolves to the pre-slippage order
    # price. Without it both settings agree and the flag is untested — another
    # gap mutation testing found.
    for upd in (True, False):
        out.append(
            Case(
                f"update_value_{str(upd).lower()}_slippage",
                8,
                1,
                1000.0,
                close=f(ramp(8, 1)),
                size=s(0.5),
                size_type=s(SizeType.TargetPercent),
                slippage=s(0.02),
                update_value=upd,
            )
        )
        # And with the valuation price pinned away from the order price.
        out.append(
            Case(
                f"update_value_{str(upd).lower()}_explicit_val",
                8,
                1,
                1000.0,
                close=f(ramp(8, 1)),
                price=r([10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0]),
                val_price=r([9.0, 10.5, 11.0, 13.5, 13.0, 16.0, 15.0, 18.0]),
                size=s(0.5),
                size_type=s(SizeType.TargetPercent),
                update_value=upd,
            )
        )
        # The intersection, which neither group above reaches on its own.
        #
        # After a fill, `update_value` re-marks the valuation price to the fill
        # price. That write is normally invisible, because the next bar
        # re-resolves the valuation price and overwrites it. It only survives
        # when the next bar's valuation is NaN *and* forward-fill is on — so
        # covering `update_value` and `ffill_val_price` separately leaves the
        # write-back itself untested. Mutation testing found this: deleting the
        # write-back entirely passed every other fixture.
        #
        # Slippage separates the fill price from the valuation price, and the
        # NaN closes create the gaps that let the written value survive.
        #
        # The order price must stay *valid* on the gap bars, or nothing fills
        # there and the difference never reaches a record. So the price is
        # given explicitly while the valuation price is taken from the previous
        # close via -inf — that is the only combination where a bar has a
        # usable price and an unusable valuation at the same time.
        gaps = [10.0, 11.0, math.nan, math.nan, 14.0, math.nan, math.nan, 17.0, math.nan, 19.0]
        explicit = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0]
        for ffill in (True, False):
            out.append(
                Case(
                    f"update_value_{str(upd).lower()}_ffill_{str(ffill).lower()}_gaps",
                    10,
                    1,
                    1000.0,
                    close=f(gaps),
                    price=r(explicit),
                    val_price=s(-math.inf),
                    size=s(0.5),
                    size_type=s(SizeType.TargetPercent),
                    slippage=s(0.02),
                    ffill_val_price=ffill,
                    update_value=upd,
                )
            )

    # --- size types across a price path ----------------------------------
    for st in SIZE_TYPE_NAMES:
        size = 0.5 if st in (SizeType.Percent, SizeType.TargetPercent) else 3.0
        out.append(
            Case(
                f"size_type_{SIZE_TYPE_NAMES[st].lower()}",
                8,
                1,
                1000.0,
                close=f(ramp(8, 1)),
                size=s(size),
                size_type=s(st),
            )
        )

    # --- directions ------------------------------------------------------
    for d in DIRECTION_NAMES:
        # Alternating signs, so each direction meets orders it must refuse.
        out.append(
            Case(
                f"direction_{DIRECTION_NAMES[d].lower()}",
                8,
                1,
                1000.0,
                close=f(ramp(8, 1)),
                size=r([2.0, -1.0, 3.0, -4.0, 1.0, -2.0, 2.0, -1.0]),
                direction=s(d),
            )
        )

    # --- costs -----------------------------------------------------------
    out.append(
        Case(
            "fees_and_slippage",
            8,
            1,
            1000.0,
            close=f(ramp(8, 1)),
            size=r([2.0, -1.0, 3.0, -4.0, 1.0, -2.0, 2.0, -1.0]),
            fees=s(0.01),
            fixed_fees=s(1.0),
            slippage=s(0.005),
        )
    )
    out.append(
        Case(
            "granularity",
            8,
            1,
            1000.0,
            close=f(ramp(8, 1)),
            size=r([2.7, -1.3, 3.9, -4.2, 1.1, -2.8, 2.5, -1.9]),
            size_granularity=s(1.0),
        )
    )
    out.append(
        Case(
            "min_and_max_size",
            8,
            1,
            1000.0,
            close=f(ramp(8, 1)),
            size=r([0.2, -1.0, 30.0, -4.0, 0.1, -2.0, 20.0, -1.0]),
            min_size=s(0.5),
            max_size=s(10.0),
        )
    )
    out.append(
        Case(
            "lock_cash_shorts",
            8,
            1,
            1000.0,
            close=f(ramp(8, 1)),
            size=r([-2.0, -3.0, 1.0, -4.0, 5.0, -2.0, 2.0, -1.0]),
            lock_cash=s(True),
        )
    )
    out.append(
        Case(
            "cash_exhaustion",
            8,
            1,
            50.0,
            close=f(ramp(8, 1)),
            size=s(2.0),
            fees=s(0.01),
        )
    )

    # --- the sweep shape -------------------------------------------------
    # This is what the whole column layout exists for: one price path per
    # symbol, a different parameter per column. Getting broadcasting wrong
    # here silently mixes columns up, and nothing else in the suite would
    # notice.
    out.append(
        Case(
            "sweep_per_column_size",
            10,
            4,
            1000.0,
            close=f(ramp(10, 4)),
            size=c([1.0, 2.0, 3.0, 4.0]),
        )
    )
    out.append(
        Case(
            "sweep_per_column_fees",
            10,
            4,
            1000.0,
            close=f(ramp(10, 4)),
            size=s(2.0),
            fees=c([0.0, 0.001, 0.01, 0.1]),
        )
    )
    out.append(
        Case(
            "sweep_per_column_direction",
            10,
            3,
            1000.0,
            close=f(ramp(10, 3)),
            size=r([2.0, -1.0, 3.0, -4.0, 1.0, -2.0, 2.0, -1.0, 1.0, -1.0]),
            direction=c([Direction.LongOnly, Direction.ShortOnly, Direction.Both]),
        )
    )
    out.append(
        Case(
            "sweep_per_column_init_cash_via_size",
            10,
            4,
            500.0,
            close=f(ramp(10, 4)),
            size=c([0.2, 0.4, 0.6, 0.8]),
            size_type=s(SizeType.TargetPercent),
        )
    )
    out.append(
        Case(
            "sweep_per_column_init_cash",
            10,
            4,
            [100.0, 500.0, 1000.0, 5000.0],
            close=f(ramp(10, 4)),
            size=s(2.0),
            fees=s(0.001),
        )
    )
    out.append(
        Case(
            "sweep_per_column_init_cash_target_percent",
            10,
            3,
            [250.0, 1000.0, 4000.0],
            close=f(ramp(10, 3)),
            size=s(0.4),
            size_type=s(SizeType.TargetPercent),
        )
    )
    out.append(
        Case(
            "sweep_per_column_init_cash_and_size",
            10,
            4,
            [100.0, 500.0, 1000.0, 5000.0],
            close=f(ramp(10, 4)),
            size=c([1.0, 2.0, 3.0, 4.0]),
        )
    )
    out.append(
        Case(
            "sweep_per_row_size",
            10,
            3,
            1000.0,
            close=f(ramp(10, 3)),
            size=r([1.0, -1.0, 2.0, -2.0, 3.0, -3.0, 1.0, -1.0, 2.0, -2.0]),
        )
    )
    out.append(
        Case(
            "sweep_full_matrix_size",
            6,
            3,
            1000.0,
            close=f(ramp(6, 3)),
            size=f([((i + col) % 5) - 2.0 for col in range(3) for i in range(6)]),
        )
    )

    return out


def random_cases(n: int, seed: int) -> list[Case]:
    """Seeded random simulations, mixing encodings and parameters."""
    rng = random.Random(seed)
    out: list[Case] = []
    size_types = list(SIZE_TYPE_NAMES)

    for i in range(n):
        n_rows = rng.randint(4, 20)
        n_cols = rng.randint(1, 4)
        base = rng.uniform(5.0, 150.0)

        close: list[float] = []
        for _ in range(n_cols):
            price = base
            for _ in range(n_rows):
                price = max(0.5, price * (1.0 + rng.gauss(0.0, 0.03)))
                close.append(price)

        stype = rng.choice(size_types)
        if stype in (SizeType.Percent, SizeType.TargetPercent):
            sizes = [rng.uniform(-1.0, 1.0) for _ in range(n_rows * n_cols)]
        else:
            sizes = [rng.uniform(-8.0, 8.0) for _ in range(n_rows * n_cols)]

        params: dict[str, Enc] = {
            "close": f(close),
            "size": f(sizes),
            "size_type": s(stype),
            # Both only: a LongOnly column raises as soon as a random chain
            # wanders short, which ends the case and wastes its remaining bars.
            "direction": s(Direction.Both),
            "fees": s(rng.choice([0.0, 0.001, 0.01])),
            "fixed_fees": s(rng.choice([0.0, 0.0, 1.0])),
            "slippage": s(rng.choice([0.0, 0.0, 0.001])),
            "min_size": s(rng.choice([0.0, 0.0, 0.5])),
            "max_size": s(rng.choice([math.inf, math.inf, 10.0])),
            "size_granularity": s(rng.choice([math.nan, math.nan, math.nan, 1.0])),
            "lock_cash": s(rng.random() < 0.2),
            "allow_partial": s(rng.random() < 0.85),
        }
        if rng.random() < 0.3:
            params["price"] = s(-math.inf)
        if rng.random() < 0.3:
            params["val_price"] = s(-math.inf)

        init_cash = (
            [rng.uniform(100.0, 5000.0) for _ in range(n_cols)]
            if rng.random() < 0.5
            else rng.uniform(100.0, 5000.0)
        )
        out.append(
            Case(
                f"rand_{i:03d}",
                n_rows,
                n_cols,
                init_cash,
                ffill_val_price=rng.random() < 0.8,
                update_value=rng.random() < 0.3,
                **params,
            )
        )

    return out


def run(case: Case) -> dict[str, str]:
    """Runs one simulation and flattens its records into TSV fields."""
    n_rows, n_cols = case.n_rows, case.n_cols
    kwargs = {
        name: enc.to_numpy(n_rows, n_cols, DTYPES.get(name, np.float64))
        for name, enc in case.params.items()
    }

    try:
        order_records, _ = simulate_from_orders_nb(
            target_shape=(n_rows, n_cols),
            # All ones: every column is its own group, so no cash sharing.
            group_lens=np.full(n_cols, 1, dtype=np.int64),
            init_cash=np.asarray(case.init_cash, dtype=np.float64),
            call_seq=np.zeros((n_rows, n_cols), dtype=np.int64),
            ffill_val_price=case.ffill_val_price,
            update_value=case.update_value,
            flex_2d=True,
            **kwargs,
        )
    except Exception as exc:  # noqa: BLE001 - informational only
        return {
            "outcome": "err",
            "rec_col": "-",
            "rec_idx": "-",
            "rec_size": "-",
            "rec_price": "-",
            "rec_fees": "-",
            "rec_side": "-",
            "note": str(exc).replace("\t", " ").replace("\n", " ")[:160],
        }

    def join(vals, fmt) -> str:
        return ";".join(fmt(v) for v in vals) if len(vals) else "-"

    return {
        "outcome": "ok",
        "rec_col": join(order_records["col"], lambda v: str(int(v))),
        "rec_idx": join(order_records["idx"], lambda v: str(int(v))),
        "rec_size": join(order_records["size"], fmt_float),
        "rec_price": join(order_records["price"], fmt_float),
        "rec_fees": join(order_records["fees"], fmt_float),
        "rec_side": join(order_records["side"], lambda v: SIDE_NAMES[int(v)]),
        "note": "-",
    }


def row_for(case: Case) -> list[str]:
    fields: dict[str, str] = {
        "case": case.name,
        "n_rows": str(case.n_rows),
        "n_cols": str(case.n_cols),
        "init_cash": (
            f"s:{fmt_float(case.init_cash[0])}"
            if len(set(case.init_cash)) == 1
            else "c:" + ";".join(fmt_float(v) for v in case.init_cash)
        ),
        "ffill_val_price": "true" if case.ffill_val_price else "false",
        "update_value": "true" if case.update_value else "false",
    }
    for name in ARRAY_PARAMS:
        fields[name] = case.params[name].text(FORMATTERS.get(name, fmt_float))
    fields.update(run(case))
    return [fields[col] for col in COLUMNS]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    repo_root = Path(__file__).resolve().parents[2]
    parser.add_argument("--out", type=Path, default=repo_root / "tests/fixtures/from_orders.tsv")
    parser.add_argument("--random-cases", type=int, default=250)
    parser.add_argument("--seed", type=int, default=20260918)
    args = parser.parse_args()

    import vectorbt

    all_cases = cases() + random_cases(args.random_cases, args.seed)
    rows = [row_for(case) for case in all_cases]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as fh:
        fh.write("# Generated by tests/parity/generate_from_orders.py -- do not edit by hand.\n")
        fh.write(f"# reference: vectorbt {vectorbt.__version__}, engine=numba\n")
        fh.write(f"# seed: {args.seed}, random cases: {args.random_cases}\n")
        fh.write("# Array encodings: s=scalar, c=per column, r=per row, f=full (column-major).\n")
        fh.write("\t".join(COLUMNS) + "\n")
        for row in rows:
            fh.write("\t".join(row) + "\n")

    errs = sum(1 for r in rows if r[COLUMNS.index("outcome")] == "err")
    fills = sum(
        0 if r[COLUMNS.index("rec_col")] == "-" else r[COLUMNS.index("rec_col")].count(";") + 1
        for r in rows
    )
    print(f"wrote {len(rows)} cases ({fills} fills, {errs} errors) to {args.out}")


if __name__ == "__main__":
    main()
