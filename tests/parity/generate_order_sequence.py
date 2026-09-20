#!/usr/bin/env python3
"""Generate sequence-level differential fixtures for the execution kernel.

Single-call fixtures (``generate_execute_order.py``) pin down one
``execute_order`` invocation. They cannot see anything that only goes wrong
when state is threaded from one call to the next — residue accumulating across
steps, a field silently dropped between iterations, a discrete branch that
flips only because an earlier step left a value a few ULPs off zero.

This generator chains orders: a column starts flat, runs N orders against a
price path, and the state after every step is recorded. The Rust side replays
the chain feeding *its own* outputs forward, so a divergence at step 3 shows up
at step 3 rather than being washed out.

Two uses:

1. It closes the gap left by stage 2, where mutation testing showed the
   zero-snapping machinery was unverified.
2. It is the harness stage 3 needs anyway — a simulation loop is exactly this
   chaining, plus bar data and record writing.

Chaining rule, which the Rust replay must match exactly::

    state = flat(init_cash)
    for each step i:
        val_price = price_path[i]
        value     = state.cash + state.position * val_price
        state, result = execute_order(state, order[i])

``val_price`` and ``value`` are not outputs of the kernel; a simulation loop
derives them from the bar. Fixing the rule here keeps the two sides honest.

Usage::

    python tests/parity/generate_order_sequence.py

Setup is the same as the single-call generator; see the README in this
directory.
"""

from __future__ import annotations

import argparse
import math
import random
from pathlib import Path

from vectorbt.portfolio.enums import Direction, Order, ProcessOrderState, SizeType
from vectorbt.portfolio.nb import execute_order_nb

from generate_execute_order import (  # reuse the single-call vocabulary
    DIRECTION_NAMES,
    SIDE_NAMES,
    SIZE_TYPE_NAMES,
    STATUS_INFO_NAMES,
    STATUS_NAMES,
    fmt_float,
    order,
)

COLUMNS = [
    "seq",
    "step",
    "init_cash",
    "val_price",
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


class Sequence:
    """A chain of orders run against a price path from a flat start."""

    def __init__(self, name: str, init_cash: float, steps: list[tuple[float, Order]]) -> None:
        self.name = name
        self.init_cash = init_cash
        # Each step is (val_price, order). The order's own `price` may differ
        # from val_price -- valuation and execution prices are separate.
        self.steps = steps


def sequences() -> list[Sequence]:
    """Hand-built chains, each aimed at something single-call fixtures miss."""
    out: list[Sequence] = []

    # --- the position lifecycle ------------------------------------------
    # State has to thread correctly through every transition. A dropped
    # `debt` or a stale `free_cash` shows up here and nowhere else.
    out.append(
        Sequence(
            "long_lifecycle",
            100.0,
            [
                (10.0, order(size=5.0, price=10.0)),  # open
                (11.0, order(size=2.0, price=11.0)),  # add
                (12.0, order(size=-3.0, price=12.0)),  # reduce
                (12.0, order(size=-4.0, price=12.0)),  # close
                (12.0, order(size=-1.0, price=12.0)),  # go short
            ],
        )
    )
    out.append(
        Sequence(
            "short_lifecycle",
            100.0,
            [
                (10.0, order(size=-5.0, price=10.0)),  # open short
                (9.0, order(size=-2.0, price=9.0)),  # extend
                (8.0, order(size=3.0, price=8.0)),  # partially cover
                (8.0, order(size=4.0, price=8.0)),  # fully cover
                (8.0, order(size=2.0, price=8.0)),  # go long
            ],
        )
    )
    out.append(
        Sequence(
            "short_lifecycle_fees",
            100.0,
            [
                (10.0, order(size=-5.0, price=10.0, fees=0.01, fixed_fees=1.0)),
                (9.0, order(size=-2.0, price=9.0, fees=0.01, fixed_fees=1.0)),
                (8.0, order(size=7.0, price=8.0, fees=0.01, fixed_fees=1.0)),
            ],
        )
    )

    # --- the zero-snapping amplifier -------------------------------------
    # This is the sequence stage 2's mutation testing asked for.
    #
    # The tolerant affordability check lets a buy through when the required
    # cash sits a few ULPs *above* the cash on hand. `add()` then snaps the
    # resulting balance to exactly 0.0. Plain `+` would leave about -1.8e-15
    # -- a NEGATIVE cash balance, which is an invariant violation, so the very
    # next step turns from a normal rejection into an error.
    #
    # That is the discrete amplifier: a residue far below the 1e-12 comparison
    # floor becomes an ok/err difference one step later.
    eps_price = math.nextafter(3.0, math.inf)
    out.append(
        Sequence(
            "snap_amplifier_negative_cash",
            15.0,
            [
                (3.0, order(size=5.0, price=eps_price)),  # spends a hair over 15.0
                (3.0, order(size=1.0, price=3.0)),  # only legal if cash snapped to 0
                (3.0, order(size=-1.0, price=3.0)),  # and again from the other side
            ],
        )
    )
    eps_price_10 = math.nextafter(10.0, math.inf)
    out.append(
        Sequence(
            "snap_amplifier_negative_cash_10",
            100.0,
            [
                (10.0, order(size=10.0, price=eps_price_10)),
                (10.0, order(size=1.0, price=10.0)),
            ],
        )
    )
    # Same amplifier reached through a percentage rather than an exact size.
    out.append(
        Sequence(
            "snap_amplifier_percent",
            100.0,
            [
                (7.0, order(size=1.0, price=7.0, size_type=SizeType.Percent)),
                (7.0, order(size=1.0, price=7.0)),
                (7.0, order(size=-1.0, price=7.0)),
            ],
        )
    )
    # The same amplifier on the *position* path rather than the cash path.
    #
    # Buying twice by value performs two independent divisions; selling the
    # combined notional performs a third. The three do not cancel exactly:
    # the position ends about 7.1e-15 long instead of flat. `add()` snaps that
    # to a true zero. Left alone, the residue makes the column nominally long,
    # and a ShortOnly order afterwards is an invariant violation rather than a
    # normal short.
    #
    # Note the buy and the sell must come from *different* computations. An
    # earlier attempt here closed with the same number it opened with, which
    # cancels exactly and amplifies nothing.
    for tag, v1, v2 in [("a", 100.0, 10.0), ("b", 10.0, 1.0)]:
        out.append(
            Sequence(
                f"snap_amplifier_position_{tag}",
                10_000.0,
                [
                    (3.0, order(size=v1, price=3.0, size_type=SizeType.Value)),
                    (3.0, order(size=v2, price=3.0, size_type=SizeType.Value)),
                    (3.0, order(size=-(v1 + v2), price=3.0, size_type=SizeType.Value)),
                    # Only legal from a genuinely flat position.
                    (3.0, order(size=1.0, price=3.0, direction=Direction.ShortOnly)),
                ],
            )
        )

    # --- rebalancing across a price path ---------------------------------
    # Target sizing re-reads `value` every step, so a drifting balance
    # compounds into a different target rather than staying a rounding error.
    path = [10.0, 11.0, 9.5, 12.0, 11.5, 13.0, 12.5, 10.0]
    out.append(
        Sequence(
            "target_percent_rebalance",
            1000.0,
            [(p, order(size=0.5, price=p, size_type=SizeType.TargetPercent)) for p in path],
        )
    )
    out.append(
        Sequence(
            "target_percent_rebalance_fees",
            1000.0,
            [
                (p, order(size=0.5, price=p, size_type=SizeType.TargetPercent, fees=0.001))
                for p in path
            ],
        )
    )
    out.append(
        Sequence(
            "target_amount_walk",
            1000.0,
            [
                (p, order(size=t, price=p, size_type=SizeType.TargetAmount))
                for p, t in zip(path, [10.0, 20.0, 5.0, 0.0, -5.0, -10.0, 0.0, 15.0], strict=True)
            ],
        )
    )

    # --- exhausting cash, then trading on empty --------------------------
    out.append(
        Sequence(
            "spend_everything_then_keep_trying",
            100.0,
            [
                (10.0, order(size=math.inf, price=10.0)),
                (10.0, order(size=1.0, price=10.0)),  # no cash left
                (10.0, order(size=-1.0, price=10.0)),  # but can still sell
                (10.0, order(size=1.0, price=10.0)),  # and buy again
            ],
        )
    )
    out.append(
        Sequence(
            "granularity_walk",
            100.0,
            [
                (10.0, order(size=3.7, price=10.0, size_granularity=1.0)),
                (10.0, order(size=2.9, price=10.0, size_granularity=1.0)),
                (10.0, order(size=-4.4, price=10.0, size_granularity=1.0)),
                (10.0, order(size=math.inf, price=10.0, size_granularity=1.0)),
            ],
        )
    )
    out.append(
        Sequence(
            "lock_cash_walk",
            100.0,
            [
                (10.0, order(size=-5.0, price=10.0, lock_cash=True)),
                (11.0, order(size=-2.0, price=11.0, lock_cash=True)),
                (9.0, order(size=math.inf, price=9.0, lock_cash=True)),
            ],
        )
    )

    return out


def random_sequences(n: int, steps: int, seed: int) -> list[Sequence]:
    """Seeded random chains, for the combinations hand-built ones miss."""
    rng = random.Random(seed)
    out: list[Sequence] = []
    size_types = list(SIZE_TYPE_NAMES)

    for i in range(n):
        init_cash = rng.uniform(100.0, 2000.0)
        price = rng.uniform(5.0, 200.0)
        chain: list[tuple[float, Order]] = []
        for _ in range(steps):
            # A random walk, so valuation moves under the position.
            price = max(0.5, price * (1.0 + rng.gauss(0.0, 0.03)))
            stype = rng.choice(size_types)
            if stype in (SizeType.Percent, SizeType.TargetPercent):
                size = rng.uniform(-1.0, 1.0)
            else:
                size = rng.uniform(-20.0, 20.0)
            chain.append(
                (
                    price,
                    order(
                        size=size,
                        price=price,
                        size_type=stype,
                        # Direction.Both only: LongOnly/ShortOnly raise as soon
                        # as the chain wanders to the wrong sign, which ends
                        # the sequence and wastes the remaining steps.
                        direction=Direction.Both,
                        fees=rng.choice([0.0, 0.001, 0.01]),
                        fixed_fees=rng.choice([0.0, 0.0, 1.0]),
                        slippage=rng.choice([0.0, 0.0, 0.001]),
                        min_size=rng.choice([0.0, 0.0, 0.5]),
                        max_size=rng.choice([math.inf, math.inf, 10.0]),
                        size_granularity=rng.choice([math.nan, math.nan, math.nan, 1.0]),
                        lock_cash=rng.random() < 0.2,
                        allow_partial=rng.random() < 0.85,
                    ),
                )
            )
        out.append(Sequence(f"rand_{i:03d}", init_cash, chain))

    return out


def run_sequence(seq: Sequence) -> list[dict[str, str]]:
    """Runs a chain, returning one row of fields per step.

    Stops at the first error: the reference cannot continue past one either,
    so there is no further state to record.
    """
    rows: list[dict[str, str]] = []
    cash, position, debt, free_cash = seq.init_cash, 0.0, 0.0, seq.init_cash

    for step, (val_price, od) in enumerate(seq.steps):
        value = cash + position * val_price
        st = ProcessOrderState(
            cash=cash,
            position=position,
            debt=debt,
            free_cash=free_cash,
            val_price=val_price,
            value=value,
            oidx=0,
            lidx=0,
        )

        fields = {
            "seq": seq.name,
            "step": str(step),
            "init_cash": fmt_float(seq.init_cash),
            "val_price": fmt_float(val_price),
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

        try:
            exec_state, result = execute_order_nb(st, od)
        except Exception as exc:  # noqa: BLE001 - informational only
            fields.update({k: "-" for k in OUTPUT_COLUMNS})
            fields["outcome"] = "err"
            fields["note"] = str(exc).replace("\t", " ").replace("\n", " ")[:160]
            rows.append(fields)
            break

        fields.update(
            {
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
        )
        rows.append(fields)

        cash, position = exec_state.cash, exec_state.position
        debt, free_cash = exec_state.debt, exec_state.free_cash

    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    repo_root = Path(__file__).resolve().parents[2]
    parser.add_argument("--out", type=Path, default=repo_root / "tests/fixtures/order_sequence.tsv")
    parser.add_argument("--random-sequences", type=int, default=120)
    parser.add_argument("--steps", type=int, default=12)
    parser.add_argument("--seed", type=int, default=20260918)
    args = parser.parse_args()

    import vectorbt

    all_seqs = sequences() + random_sequences(args.random_sequences, args.steps, args.seed)
    rows: list[dict[str, str]] = []
    for seq in all_seqs:
        rows.extend(run_sequence(seq))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as fh:
        fh.write("# Generated by tests/parity/generate_order_sequence.py -- do not edit by hand.\n")
        fh.write(f"# reference: vectorbt {vectorbt.__version__}, engine=numba\n")
        fh.write(f"# seed: {args.seed}, random sequences: {args.random_sequences}\n")
        fh.write("# Chaining rule: state starts flat at init_cash; each step sets\n")
        fh.write("# value = cash + position * val_price before executing.\n")
        fh.write("\t".join(COLUMNS) + "\n")
        for row in rows:
            fh.write("\t".join(row[c] for c in COLUMNS) + "\n")

    errs = sum(1 for r in rows if r["outcome"] == "err")
    print(
        f"wrote {len(all_seqs)} sequences / {len(rows)} steps to {args.out} "
        f"({errs} ended in an error)"
    )


if __name__ == "__main__":
    main()
