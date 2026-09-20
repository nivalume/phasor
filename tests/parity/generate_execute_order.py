#!/usr/bin/env python3
"""Generate differential-test fixtures for the order execution kernel.

Runs the reference implementation (vectorbt's ``execute_order_nb``) over a set
of cases and writes inputs plus expected outputs to a TSV that the Rust test
suite replays. Fixtures are committed, so CI never needs Python or vectorbt.

Why this granularity: one fixture is one ``execute_order`` call. Diffing whole
simulations tells you *that* something drifted; diffing single calls tells you
*which* stage of the size cascade did it.

Usage::

    python tests/parity/generate_execute_order.py
    python tests/parity/generate_execute_order.py --out path/to/other.tsv

Requires the reference implementation::

    python -m venv .venv-parity
    .venv-parity/bin/pip install "vectorbt==1.1.0" "plotly<6"

The ``plotly<6`` pin is load-bearing: vectorbt 1.1.0 references the
``scattermapbox`` trace, which plotly 6 removed, and the import fails without
it.

Deliberately not covered here: ``reject_prob > 0``. It consumes NumPy's global
RNG, so the outcome is not reproducible across implementations. Random
rejection is tested on the Rust side against an injected generator instead.
"""

from __future__ import annotations

import argparse
import math
import random
from pathlib import Path

from vectorbt.portfolio.enums import Direction, Order, ProcessOrderState, SizeType
from vectorbt.portfolio.nb import execute_order_nb

SIZE_TYPE_NAMES = {
    SizeType.Amount: "Amount",
    SizeType.Value: "Value",
    SizeType.Percent: "Percent",
    SizeType.TargetAmount: "TargetAmount",
    SizeType.TargetValue: "TargetValue",
    SizeType.TargetPercent: "TargetPercent",
}

DIRECTION_NAMES = {
    Direction.LongOnly: "LongOnly",
    Direction.ShortOnly: "ShortOnly",
    Direction.Both: "Both",
}

SIDE_NAMES = {0: "Buy", 1: "Sell", -1: "none"}

STATUS_NAMES = {0: "Filled", 1: "Ignored", 2: "Rejected"}

STATUS_INFO_NAMES = {
    -1: "none",
    0: "SizeNaN",
    1: "PriceNaN",
    2: "ValPriceNaN",
    3: "ValueNaN",
    4: "ValueZeroNeg",
    5: "SizeZero",
    6: "NoCashShort",
    7: "NoCashLong",
    8: "NoOpenPosition",
    9: "MaxSizeExceeded",
    10: "RandomEvent",
    11: "CantCoverFees",
    12: "MinSizeNotReached",
    13: "PartialFill",
}

COLUMNS = [
    "name",
    "in_cash",
    "in_position",
    "in_debt",
    "in_free_cash",
    "in_val_price",
    "in_value",
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
    "outcome",
    "out_cash",
    "out_position",
    "out_debt",
    "out_free_cash",
    "res_size",
    "res_price",
    "res_fees",
    "res_side",
    "res_status",
    "res_status_info",
    "note",
]

# Only these are cleared when a case raises. Blanking the input columns too
# would leave the error cases unreplayable.
OUTPUT_COLUMNS = [
    "out_cash",
    "out_position",
    "out_debt",
    "out_free_cash",
    "res_size",
    "res_price",
    "res_fees",
    "res_side",
    "res_status",
    "res_status_info",
]


def fmt_float(x: float) -> str:
    """Shortest round-trippable text for a float.

    Python's ``repr`` round-trips exactly, and Rust's ``f64::from_str`` accepts
    ``nan``, ``inf`` and ``-inf``, so the two agree with no loss.
    """
    return repr(float(x))


def state(
    cash: float = 100.0,
    position: float = 0.0,
    debt: float = 0.0,
    free_cash: float | None = None,
    val_price: float = 10.0,
    value: float | None = None,
) -> ProcessOrderState:
    """A ``ProcessOrderState`` with sensible defaults.

    ``free_cash`` defaults to ``cash`` (true when no short is open) and
    ``value`` to ``cash + position * val_price``.
    """
    if free_cash is None:
        free_cash = cash
    if value is None:
        value = cash + position * val_price
    return ProcessOrderState(
        cash=cash,
        position=position,
        debt=debt,
        free_cash=free_cash,
        val_price=val_price,
        value=value,
        oidx=0,
        lidx=0,
    )


def order(**kwargs) -> Order:
    """An ``Order`` with the reference implementation's defaults."""
    params = dict(
        size=math.inf,
        price=math.inf,
        size_type=SizeType.Amount,
        direction=Direction.Both,
        fees=0.0,
        fixed_fees=0.0,
        slippage=0.0,
        min_size=0.0,
        max_size=math.inf,
        size_granularity=math.nan,
        reject_prob=0.0,
        lock_cash=False,
        allow_partial=True,
        raise_reject=False,
        log=False,
    )
    params.update(kwargs)
    return Order(**params)


def cases() -> list[tuple[str, ProcessOrderState, Order]]:
    """Hand-picked cases, one per behaviour worth pinning down."""
    out: list[tuple[str, ProcessOrderState, Order]] = []

    def add(name: str, st: ProcessOrderState, od: Order) -> None:
        out.append((name, st, od))

    # --- the simple path -------------------------------------------------
    add("buy_plain", state(), order(size=5.0, price=10.0))
    add("sell_plain", state(position=5.0), order(size=-3.0, price=10.0))
    add("buy_all_cash", state(), order(size=math.inf, price=10.0))

    # --- fees and slippage ----------------------------------------------
    add("buy_pct_fees", state(), order(size=5.0, price=10.0, fees=0.01))
    add("buy_fixed_fees", state(), order(size=5.0, price=10.0, fixed_fees=1.0))
    add("buy_both_fees", state(), order(size=5.0, price=10.0, fees=0.01, fixed_fees=1.0))
    add("sell_both_fees", state(position=5.0), order(size=-5.0, price=10.0, fees=0.01, fixed_fees=1.0))
    # Slippage is directional: buys pay up, sells receive less.
    add("buy_slippage", state(), order(size=5.0, price=10.0, slippage=0.01))
    add("sell_slippage", state(position=5.0), order(size=-5.0, price=10.0, slippage=0.01))

    # --- insufficient cash: the fee back-solve ---------------------------
    # Fees are inside the cash cap, not added on top.
    add("buy_short_of_cash", state(cash=100.0), order(size=50.0, price=10.0))
    add("buy_short_of_cash_fees", state(cash=100.0), order(size=50.0, price=10.0, fees=0.1, fixed_fees=1.0))
    add("buy_fixed_fees_exceed_cash", state(cash=1.0), order(size=5.0, price=10.0, fixed_fees=2.0))
    add("buy_no_cash", state(cash=0.0), order(size=5.0, price=10.0))

    # --- granularity, applied before *and* after the cash constraint -----
    add("buy_granularity", state(), order(size=5.7, price=10.0, size_granularity=1.0))
    add(
        "buy_granularity_capped",
        state(cash=100.0),
        order(size=50.0, price=10.0, size_granularity=1.0, fees=0.1, fixed_fees=1.0),
    )
    add("sell_granularity", state(position=5.7), order(size=-5.7, price=10.0, size_granularity=1.0))

    # --- size bounds -----------------------------------------------------
    add("buy_below_min_size", state(), order(size=1.0, price=10.0, min_size=2.0))
    add("buy_at_min_size", state(), order(size=2.0, price=10.0, min_size=2.0))
    add("buy_above_max_size", state(), order(size=5.0, price=10.0, max_size=3.0))
    add("buy_above_max_size_no_partial", state(), order(size=5.0, price=10.0, max_size=3.0, allow_partial=False))
    add("buy_partial_disallowed", state(cash=10.0), order(size=5.0, price=10.0, allow_partial=False))
    # An infinite request cannot be a partial fill, by definition.
    add("buy_inf_partial_disallowed", state(cash=10.0), order(size=math.inf, price=10.0, allow_partial=False))

    # --- size types ------------------------------------------------------
    add("size_value", state(), order(size=50.0, price=10.0, size_type=SizeType.Value))
    add("size_percent", state(), order(size=0.5, price=10.0, size_type=SizeType.Percent))
    add("size_target_amount_up", state(position=2.0), order(size=5.0, price=10.0, size_type=SizeType.TargetAmount))
    add("size_target_amount_down", state(position=8.0), order(size=5.0, price=10.0, size_type=SizeType.TargetAmount))
    add("size_target_amount_flat", state(position=5.0), order(size=5.0, price=10.0, size_type=SizeType.TargetAmount))
    add("size_target_value", state(position=2.0), order(size=50.0, price=10.0, size_type=SizeType.TargetValue))
    add("size_target_percent", state(position=2.0), order(size=0.5, price=10.0, size_type=SizeType.TargetPercent))
    add(
        "size_target_percent_zero_value",
        state(cash=0.0, position=0.0, value=0.0),
        order(size=0.5, price=10.0, size_type=SizeType.TargetPercent),
    )
    add(
        "size_target_percent_nan_value",
        state(value=math.nan),
        order(size=0.5, price=10.0, size_type=SizeType.TargetPercent),
    )
    add(
        "size_value_nan_val_price",
        state(val_price=math.nan),
        order(size=50.0, price=10.0, size_type=SizeType.Value),
    )

    # --- shorts: open, extend, reduce, close, reverse --------------------
    add("short_open", state(), order(size=-5.0, price=10.0))
    add("short_extend", state(position=-5.0, debt=50.0, free_cash=0.0), order(size=-2.0, price=10.0))
    add("short_reduce", state(cash=150.0, position=-5.0, debt=50.0, free_cash=50.0), order(size=2.0, price=10.0))
    add("short_close", state(cash=150.0, position=-5.0, debt=50.0, free_cash=50.0), order(size=5.0, price=10.0))
    add("short_reverse", state(cash=150.0, position=-5.0, debt=50.0, free_cash=50.0), order(size=8.0, price=10.0))
    add("long_reverse_to_short", state(position=5.0), order(size=-8.0, price=10.0))
    # The -inf special case: "100% to short".
    add("short_inf", state(), order(size=-math.inf, price=10.0))

    # --- lock_cash: the short-margin reservation -------------------------
    add("lock_cash_buy_flat", state(), order(size=5.0, price=10.0, lock_cash=True))
    add(
        "lock_cash_cover_affordable",
        state(cash=150.0, position=-5.0, debt=50.0, free_cash=50.0),
        order(size=5.0, price=10.0, lock_cash=True),
    )
    add(
        "lock_cash_cover_unaffordable",
        state(cash=60.0, position=-5.0, debt=50.0, free_cash=1.0),
        order(size=math.inf, price=10.0, lock_cash=True),
    )
    add("lock_cash_sell", state(position=5.0), order(size=-math.inf, price=10.0, lock_cash=True))
    add("lock_cash_sell_percent", state(position=5.0), order(size=-0.5, price=10.0, size_type=SizeType.Percent, lock_cash=True))

    # --- directions ------------------------------------------------------
    add("long_only_buy", state(), order(size=5.0, price=10.0, direction=Direction.LongOnly))
    add("long_only_sell_to_zero", state(position=5.0), order(size=-8.0, price=10.0, direction=Direction.LongOnly))
    add("long_only_sell_no_position", state(), order(size=-5.0, price=10.0, direction=Direction.LongOnly))
    # ShortOnly flips the sign of the request.
    add("short_only_open", state(), order(size=5.0, price=10.0, direction=Direction.ShortOnly))
    add("short_only_cover", state(cash=150.0, position=-5.0, debt=50.0, free_cash=50.0), order(size=-5.0, price=10.0, direction=Direction.ShortOnly))
    add("short_only_no_position", state(), order(size=-5.0, price=10.0, direction=Direction.ShortOnly))

    # --- ignored inputs (checked before state validation) ----------------
    add("size_nan", state(), order(size=math.nan, price=10.0))
    add("price_nan", state(), order(size=5.0, price=math.nan))
    add("size_zero", state(), order(size=0.0, price=10.0))
    # NaN size wins even when the state would otherwise be rejected.
    add("size_nan_beats_bad_state", state(cash=-1.0), order(size=math.nan, price=10.0))

    # --- invariant violations (expected to raise) ------------------------
    add("err_negative_cash", state(cash=-1.0), order(size=5.0, price=10.0))
    add("err_nan_cash", state(cash=math.nan), order(size=5.0, price=10.0))
    add("err_infinite_position", state(position=math.inf), order(size=5.0, price=10.0))
    add("err_negative_debt", state(debt=-1.0), order(size=5.0, price=10.0))
    add("err_nan_free_cash", state(free_cash=math.nan), order(size=5.0, price=10.0))
    add("err_zero_price", state(), order(size=5.0, price=0.0))
    add("err_infinite_price", state(), order(size=5.0, price=math.inf))
    add("err_long_only_with_short", state(position=-5.0, debt=50.0), order(size=5.0, price=10.0, direction=Direction.LongOnly))
    add("err_short_only_with_long", state(position=5.0), order(size=-5.0, price=10.0, direction=Direction.ShortOnly))
    add("err_negative_slippage", state(), order(size=5.0, price=10.0, slippage=-0.1))
    add("err_negative_min_size", state(), order(size=5.0, price=10.0, min_size=-1.0))
    add("err_zero_max_size", state(), order(size=5.0, price=10.0, max_size=0.0))
    add("err_zero_granularity", state(), order(size=5.0, price=10.0, size_granularity=0.0))
    add("err_infinite_fees", state(), order(size=5.0, price=10.0, fees=math.inf))
    add("err_infinite_buy", state(cash=math.inf), order(size=math.inf, price=10.0))

    # --- the tolerance boundary ------------------------------------------
    # The affordability test is `is_close_or_less(total_req_cash, cash_limit)`,
    # not `<=`. These cases put the required cash a few ULPs *above* the
    # limit, so the two spellings take different branches.
    #
    # Without granularity the branches differ only in the last bits, which is
    # below the comparison tolerance and therefore invisible. With granularity
    # set, the "insufficient cash" branch floors 4.999...  down to 4.0 while
    # the tolerant branch fills 5.0 — a whole unit. Mutation testing found
    # this gap: drop these and replacing is_close_or_less with <= passes.
    eps_price = math.nextafter(3.0, math.inf)  # 3.0 + 1 ulp
    add(
        "tolerance_boundary_granularity",
        state(cash=15.0),
        order(size=5.0, price=eps_price, size_granularity=1.0),
    )
    add(
        "tolerance_boundary_plain",
        state(cash=15.0),
        order(size=5.0, price=eps_price),
    )
    add(
        "tolerance_boundary_granularity_fees",
        state(cash=15.15),
        order(size=5.0, price=eps_price, fees=0.01, size_granularity=1.0),
    )
    # Same idea one lot up, to catch an off-by-one in the floor.
    eps_price_10 = math.nextafter(10.0, math.inf)
    add(
        "tolerance_boundary_granularity_10",
        state(cash=100.0),
        order(size=10.0, price=eps_price_10, size_granularity=1.0),
    )
    # And a case genuinely beyond tolerance, so the partial fill is real.
    add(
        "beyond_tolerance_granularity",
        state(cash=14.0),
        order(size=5.0, price=3.0, size_granularity=1.0),
    )

    return out


def random_cases(n: int, seed: int) -> list[tuple[str, ProcessOrderState, Order]]:
    """Randomized cases over the combinations hand-written ones miss.

    Seeded, so the fixture file is stable across runs and shows up in diffs
    only when the reference behaviour actually changes.
    """
    rng = random.Random(seed)
    out: list[tuple[str, ProcessOrderState, Order]] = []

    size_types = list(SIZE_TYPE_NAMES)
    directions = list(DIRECTION_NAMES)

    i = 0
    while len(out) < n:
        i += 1
        direction = rng.choice(directions)
        position = rng.choice([0.0, 0.0, rng.uniform(0.1, 20.0), -rng.uniform(0.1, 20.0)])
        # Keep the state self-consistent: the reference raises on a long-only
        # column holding a short, and those are covered by the hand-written
        # error cases rather than by random ones.
        if direction == Direction.LongOnly and position < 0:
            position = -position
        if direction == Direction.ShortOnly and position > 0:
            position = -position

        val_price = rng.uniform(1.0, 100.0)
        cash = rng.choice([0.0, rng.uniform(1.0, 1000.0)])
        debt = -position * val_price if position < 0 else 0.0
        free_cash = cash - 2 * debt if debt > 0 else cash

        st = ProcessOrderState(
            cash=cash,
            position=position,
            debt=debt,
            free_cash=free_cash,
            val_price=val_price,
            value=cash + position * val_price,
            oidx=0,
            lidx=0,
        )

        size_type = rng.choice(size_types)
        if size_type == SizeType.Percent:
            size = rng.uniform(-1.0, 1.0)
        elif size_type == SizeType.TargetPercent:
            size = rng.uniform(-1.5, 1.5)
        else:
            size = rng.choice([rng.uniform(-50.0, 50.0), rng.uniform(-5.0, 5.0)])

        od = order(
            size=size,
            price=val_price,
            size_type=size_type,
            direction=direction,
            fees=rng.choice([0.0, 0.001, 0.01]),
            fixed_fees=rng.choice([0.0, 0.0, 1.0]),
            slippage=rng.choice([0.0, 0.0, 0.001]),
            min_size=rng.choice([0.0, 0.0, 0.5]),
            max_size=rng.choice([math.inf, math.inf, 10.0]),
            size_granularity=rng.choice([math.nan, math.nan, math.nan, 1.0]),
            lock_cash=rng.random() < 0.25,
            allow_partial=rng.random() < 0.8,
        )
        out.append((f"rand_{i:04d}", st, od))

    return out


def run(st: ProcessOrderState, od: Order) -> dict[str, str]:
    """Runs one case and flattens the outcome into TSV fields."""
    try:
        exec_state, result = execute_order_nb(st, od)
    except Exception as exc:  # noqa: BLE001 - the message is informational only
        blank = {k: "-" for k in OUTPUT_COLUMNS}
        blank["outcome"] = "err"
        # Recorded for humans; the Rust side asserts only that it errored.
        # Matching messages across implementations would be over-fitting.
        blank["note"] = str(exc).replace("\t", " ").replace("\n", " ")[:160]
        return blank

    return {
        "outcome": "ok",
        "out_cash": fmt_float(exec_state.cash),
        "out_position": fmt_float(exec_state.position),
        "out_debt": fmt_float(exec_state.debt),
        "out_free_cash": fmt_float(exec_state.free_cash),
        "res_size": fmt_float(result.size),
        "res_price": fmt_float(result.price),
        "res_fees": fmt_float(result.fees),
        "res_side": SIDE_NAMES[int(result.side)],
        "res_status": STATUS_NAMES[int(result.status)],
        "res_status_info": STATUS_INFO_NAMES[int(result.status_info)],
        "note": "-",
    }


def row_for(name: str, st: ProcessOrderState, od: Order) -> list[str]:
    fields = {
        "name": name,
        "in_cash": fmt_float(st.cash),
        "in_position": fmt_float(st.position),
        "in_debt": fmt_float(st.debt),
        "in_free_cash": fmt_float(st.free_cash),
        "in_val_price": fmt_float(st.val_price),
        "in_value": fmt_float(st.value),
        "size": fmt_float(od.size),
        "price": fmt_float(od.price),
        "size_type": SIZE_TYPE_NAMES[int(od.size_type)],
        "direction": DIRECTION_NAMES[int(od.direction)],
        "fees": fmt_float(od.fees),
        "fixed_fees": fmt_float(od.fixed_fees),
        "slippage": fmt_float(od.slippage),
        "min_size": fmt_float(od.min_size),
        "max_size": fmt_float(od.max_size),
        "size_granularity": fmt_float(od.size_granularity),
        "lock_cash": "true" if od.lock_cash else "false",
        "allow_partial": "true" if od.allow_partial else "false",
    }
    fields.update(run(st, od))
    return [fields[c] for c in COLUMNS]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    repo_root = Path(__file__).resolve().parents[2]
    parser.add_argument("--out", type=Path, default=repo_root / "tests/fixtures/execute_order.tsv")
    parser.add_argument("--random-cases", type=int, default=400)
    parser.add_argument("--seed", type=int, default=20260918)
    args = parser.parse_args()

    import vectorbt

    all_cases = cases() + random_cases(args.random_cases, args.seed)
    rows = [row_for(name, st, od) for name, st, od in all_cases]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as fh:
        fh.write("# Generated by tests/parity/generate_execute_order.py -- do not edit by hand.\n")
        fh.write(f"# reference: vectorbt {vectorbt.__version__}, engine=numba\n")
        fh.write(f"# seed: {args.seed}, random cases: {args.random_cases}\n")
        fh.write("\t".join(COLUMNS) + "\n")
        for row in rows:
            fh.write("\t".join(row) + "\n")

    ok = sum(1 for r in rows if r[COLUMNS.index("outcome")] == "ok")
    print(f"wrote {len(rows)} cases to {args.out} ({ok} ok, {len(rows) - ok} err)")


if __name__ == "__main__":
    main()
