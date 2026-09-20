#!/usr/bin/env python3
"""Generate differential fixtures for cash sharing and the call order.

This is the feature phase 1 deliberately left out, because it is the one place
columns stop being independent: a group drawing on one cash balance is a
single simulation over several instruments, and within a bar each fill changes
what the next column can afford.

What that makes testable, and what these fixtures aim at:

* **Order matters.** The same bar executed left-to-right and sorted by order
  value produces different fills, because a buy arriving before the sell that
  funds it is rejected for want of cash — silently. Every case is generated
  both ways so the difference is visible rather than assumed.
* **The group's value is shared.** A ``TargetPercent`` order is a fraction of
  the *group's* value, not the column's, so one column's position changes
  another column's order size. Half the cases use target sizing for that
  reason.
* **The value moves within the bar.** After each fill the group is worth
  something different, and the next column in the sequence must see that.
* **A group of one must behave exactly as an ungrouped column.** Cases with
  all-ones group lengths are generated so the regression is pinned, not
  assumed from the other harnesses passing.

Shorting is included because free cash — which the call-order estimate reads —
diverges from plain cash only when something is short.

Usage::

    python tests/parity/generate_cash_sharing.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
from vectorbt.portfolio.enums import Direction, SizeType
from vectorbt.portfolio.nb import simulate_from_orders_nb

from generate_execute_order import fmt_float

COLUMNS = [
    "case",
    "n_rows",
    "n_cols",
    "group_lens",
    "init_cash",
    "auto_call_seq",
    "update_value",
    "size_type",
    "direction",
    "fees",
    "close",
    "size",
    "n_fills",
    "fill_col",
    "fill_idx",
    "fill_size",
    "fill_price",
    "fill_fees",
    "fill_side",
    "note",
]


def flat(a: np.ndarray) -> str:
    return ";".join(fmt_float(v) for v in np.asarray(a, dtype=float).flatten(order="F"))


def ints(a) -> str:
    return ";".join(str(int(v)) for v in np.asarray(a).ravel())


def build_call_seq(n_rows: int, group_lens: np.ndarray) -> np.ndarray:
    """`CallSeqType.Default`: each group's columns numbered from zero.

    The auto sort permutes this in place per row, so it has to start as the
    identity *within each group* — a plain zero array, which is what the
    reference's own examples pass, silently disables the sort's first element.
    """
    n_cols = int(group_lens.sum())
    out = np.empty((n_rows, n_cols), dtype=np.int64)
    from_col = 0
    for length in group_lens:
        out[:, from_col : from_col + length] = np.arange(length)[None, :]
        from_col += length
    return out


class Case:
    def __init__(
        self,
        name: str,
        close: np.ndarray,
        size: np.ndarray,
        group_lens: list[int],
        init_cash: list[float],
        size_type: int = SizeType.Amount,
        direction: int = Direction.Both,
        fees: float = 0.0,
        update_value: bool = False,
        note: str = "",
    ) -> None:
        self.name = name
        self.close = close
        self.size = size
        self.group_lens = np.array(group_lens, dtype=np.int64)
        self.init_cash = np.array(init_cash, dtype=np.float64)
        self.size_type = size_type
        self.direction = direction
        self.fees = fees
        self.update_value = update_value
        self.note = note
        self.n_rows, self.n_cols = close.shape


def run(case: Case, auto: bool) -> dict[str, str]:
    shape = (case.n_rows, case.n_cols)
    records, _ = simulate_from_orders_nb(
        target_shape=shape,
        group_lens=case.group_lens,
        init_cash=case.init_cash,
        call_seq=build_call_seq(case.n_rows, case.group_lens),
        size=case.size,
        price=case.close,
        size_type=np.asarray(case.size_type),
        direction=np.asarray(case.direction),
        fees=np.asarray(case.fees),
        auto_call_seq=auto,
        update_value=case.update_value,
        flex_2d=True,
    )
    # Records come out in group order; within a group, in execution order.
    # Sorting by (col, idx) is what the port's caller does before comparing,
    # since the ordering contract is by column.
    order = np.lexsort((records["idx"], records["col"]))
    r = records[order]
    return {
        "case": case.name,
        "n_rows": str(case.n_rows),
        "n_cols": str(case.n_cols),
        "group_lens": ints(case.group_lens),
        "init_cash": flat(case.init_cash),
        "auto_call_seq": str(auto),
        "update_value": str(case.update_value),
        "size_type": str(int(case.size_type)),
        "direction": str(int(case.direction)),
        "fees": fmt_float(case.fees),
        "close": flat(case.close),
        "size": flat(case.size),
        "n_fills": str(len(r)),
        "fill_col": ints(r["col"]),
        "fill_idx": ints(r["idx"]),
        "fill_size": ";".join(fmt_float(v) for v in r["size"]),
        "fill_price": ";".join(fmt_float(v) for v in r["price"]),
        "fill_fees": ";".join(fmt_float(v) for v in r["fees"]),
        "fill_side": ints(r["side"]),
        "note": case.note,
    }


def col(*values: list[float]) -> np.ndarray:
    """Columns given one per list, stacked column-major."""
    return np.array(values, dtype=np.float64).T


def cases() -> list[Case]:
    rising = [10.0, 11.0, 12.0, 13.0, 14.0]
    falling = [10.0, 9.0, 8.0, 7.0, 6.0]
    choppy = [10.0, 12.0, 9.0, 11.0, 10.0]

    out: list[Case] = []

    # A buy and a sell on the same bar, in the same group. Left to right the
    # buy is starved; sorted, the sell funds it.
    out.append(
        Case(
            "funded rebalance",
            col(rising, falling),
            col([0.0, 50.0, 0.0, 0.0, 0.0], [10.0, -10.0, 0.0, 0.0, 0.0]),
            [2],
            [200.0],
            note="a buy that only fits if the sell runs first",
        )
    )
    out.append(
        Case(
            "funded rebalance, reversed layout",
            col(falling, rising),
            col([10.0, -10.0, 0.0, 0.0, 0.0], [0.0, 50.0, 0.0, 0.0, 0.0]),
            [2],
            [200.0],
            note="the same trade with the columns swapped: layout order decides",
        )
    )

    # Target-percent sizing over a shared book: every column is a fraction of
    # the group's value, so one column's position resizes another's order.
    out.append(
        Case(
            "target percent over a shared book",
            col(rising, falling, choppy),
            col(
                [0.3, 0.3, 0.5, 0.2, 0.2],
                [0.3, 0.4, 0.2, 0.4, 0.4],
                [0.3, 0.2, 0.2, 0.3, 0.3],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetPercent,
            note="each order is a fraction of the group's value, not the column's",
        )
    )
    out.append(
        Case(
            "target percent, with costs",
            col(rising, falling, choppy),
            col(
                [0.3, 0.3, 0.5, 0.2, 0.2],
                [0.3, 0.4, 0.2, 0.4, 0.4],
                [0.3, 0.2, 0.2, 0.3, 0.3],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetPercent,
            fees=0.002,
            note="costs shrink what each fill leaves for the next",
        )
    )

    # Two groups, so a thread boundary has something to sit on and the groups
    # must not see each other's cash.
    out.append(
        Case(
            "two groups",
            col(rising, falling, choppy, rising),
            col(
                [5.0, -5.0, 5.0, 0.0, 0.0],
                [5.0, 5.0, -5.0, 0.0, 0.0],
                [0.0, 3.0, -3.0, 3.0, 0.0],
                [3.0, -3.0, 0.0, 3.0, 0.0],
            ),
            [2, 2],
            [150.0, 150.0],
            note="groups must not see each other's cash",
        )
    )

    # Shorting, where free cash and plain cash diverge and the call-order
    # estimate reads the free one.
    out.append(
        Case(
            "shorting inside a group",
            col(rising, falling),
            col([-5.0, 0.0, 5.0, 0.0, 0.0], [0.0, 8.0, -8.0, 0.0, 0.0]),
            [2],
            [200.0],
            note="free cash reserves the short's margin",
        )
    )
    out.append(
        Case(
            "percent sizing with a short",
            col(rising, falling),
            col([-0.5, 0.0, 0.5, 0.0, 0.0], [0.0, 0.5, -0.5, 0.0, 0.0]),
            [2],
            [200.0],
            size_type=SizeType.Percent,
            note="the percent branch of the order-value estimate",
        )
    )

    # A group of one must be exactly an ungrouped column.
    out.append(
        Case(
            "groups of one",
            col(rising, falling),
            col([5.0, -5.0, 5.0, 0.0, 0.0], [5.0, 5.0, -5.0, 0.0, 0.0]),
            [1, 1],
            [100.0, 100.0],
            note="must match the ungrouped simulation exactly",
        )
    )

    # A group that runs out of cash: the later columns in the sequence are
    # rejected, and which ones depends entirely on the order.
    out.append(
        Case(
            "not enough to go round",
            col(rising, rising, rising),
            col(
                [5.0, 0.0, 0.0, 0.0, 0.0],
                [5.0, 0.0, 0.0, 0.0, 0.0],
                [5.0, 0.0, 0.0, 0.0, 0.0],
            ),
            [3],
            [120.0],
            note="three buys, cash for two and a bit",
        )
    )

    # Every column wants everything.
    out.append(
        Case(
            "everyone wants all of it",
            col(rising, falling, choppy),
            col([np.inf] * 5, [np.inf] * 5, [np.inf] * 5),
            [3],
            [300.0],
            note="infinite size: the first in the sequence takes the lot",
        )
    )

    # Long-only and short-only, which change the order-value estimate's sign
    # handling.
    for name, direction in (("long only", Direction.LongOnly), ("short only", Direction.ShortOnly)):
        out.append(
            Case(
                f"{name} group",
                col(rising, falling),
                col([0.4, 0.2, 0.4, 0.2, 0.0], [0.2, 0.4, 0.2, 0.4, 0.0]),
                [2],
                [500.0],
                size_type=SizeType.TargetPercent,
                direction=direction,
                note="direction changes how an order's value is estimated",
            )
        )

    # --- reaching the order-value estimate's branches --------------------
    #
    # The auto sort only shows up in the output when it actually reorders the
    # columns *and* the reordering changes what fills. A mutation sweep found
    # the estimate's position, direction and target-percent branches were not
    # reached by the cases above, so these exist to reach them: each holds a
    # position before the bar that rebalances, so `asset_value` is nonzero and
    # the estimate has something to subtract.
    out.append(
        Case(
            "rebalance out of existing positions",
            col(rising, falling, choppy),
            col(
                [0.5, 0.1, 0.6, 0.1, 0.3],
                [0.4, 0.7, 0.1, 0.6, 0.3],
                [0.1, 0.2, 0.3, 0.3, 0.4],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetPercent,
            note="every bar reverses the weights, so the sort reorders every bar",
        )
    )
    out.append(
        Case(
            "short-only rebalance",
            col(rising, falling, choppy),
            col(
                [0.5, 0.1, 0.6, 0.1, 0.3],
                [0.4, 0.7, 0.1, 0.6, 0.3],
                [0.1, 0.2, 0.3, 0.3, 0.4],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetPercent,
            direction=Direction.ShortOnly,
            note="a short-only column states its size positively and means the other way",
        )
    )
    out.append(
        Case(
            "target value rebalance",
            col(rising, falling, choppy),
            col(
                [400.0, 100.0, 500.0, 100.0, 300.0],
                [300.0, 600.0, 100.0, 500.0, 300.0],
                [100.0, 200.0, 300.0, 300.0, 300.0],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetValue,
            note="the estimate subtracts the position's value rather than scaling it",
        )
    )
    out.append(
        Case(
            "target amount rebalance",
            col(rising, falling, choppy),
            col(
                [30.0, 5.0, 40.0, 5.0, 20.0],
                [20.0, 50.0, 5.0, 40.0, 20.0],
                [5.0, 15.0, 25.0, 25.0, 25.0],
            ),
            [3],
            [1000.0],
            size_type=SizeType.TargetAmount,
            note="the estimate prices the delta, not the target",
        )
    )

    # --- update_value, which only becomes observable with cash sharing ----
    #
    # The group's value is threaded through the bar rather than re-derived, so
    # with `update_value` on a fill moves the value the *next* column in the
    # sequence is sized against. Without a grouped fixture that sets it, the
    # whole value-carrying path is untestable — which is exactly what it was
    # before this feature existed.
    for name, st in (("target percent", SizeType.TargetPercent), ("percent", SizeType.Percent)):
        out.append(
            Case(
                f"update_value with {name}",
                col(rising, falling, choppy),
                col(
                    [0.3, 0.3, 0.5, 0.2, 0.2],
                    [0.3, 0.4, 0.2, 0.4, 0.4],
                    [0.3, 0.2, 0.2, 0.3, 0.3],
                ),
                [3],
                [1000.0],
                size_type=st,
                fees=0.002,
                update_value=True,
                note="a fill re-marks the group value the next column is sized against",
            )
        )

    # --- amplifiers for the order-value estimate --------------------------
    #
    # A sort only shows up in the output when it reorders the columns *and*
    # cash is tight enough that the order decides who gets filled. Several of
    # the estimate's terms survived a mutation sweep because neither condition
    # held: scaling every column's value by a common factor leaves the order
    # alone, and an order that changes nothing when cash is ample changes
    # nothing at all.
    #
    # These pair a cheap instrument with an expensive one, so that ranking by
    # raw size and ranking by size-times-price disagree, and set the starting
    # cash just above one leg — so whichever column runs second is the one
    # that goes short of funds.
    cheap = [1.0, 1.0, 1.0, 1.0, 1.0]
    dear = [100.0, 100.0, 100.0, 100.0, 100.0]
    out.append(
        Case(
            "cheap and dear, by amount",
            col(dear, cheap),
            col([2.0, 0.0, 0.0, 0.0, 0.0], [50.0, 0.0, 0.0, 0.0, 0.0]),
            [2],
            [210.0],
            note="ranking by size and by size-times-price disagree, and cash binds",
        )
    )
    out.append(
        Case(
            "cheap and dear, by target amount",
            col(dear, cheap),
            col([2.0, 0.0, 0.0, 0.0, 0.0], [50.0, 0.0, 0.0, 0.0, 0.0]),
            [2],
            [210.0],
            size_type=SizeType.TargetAmount,
            note="the same disagreement, one size type along",
        )
    )
    out.append(
        Case(
            "cheap and dear, short only",
            col(dear, cheap),
            col([1.0, 0.0, 2.0, 0.0, 0.0], [30.0, 0.0, 60.0, 0.0, 0.0]),
            [2],
            [150.0],
            direction=Direction.ShortOnly,
            note="a short-only size means the other way, which reverses the ranking",
        )
    )

    # Short-only needed a second look. Opening a short *raises* cash, so a
    # group of them is never starved and the call order cannot matter however
    # the estimate ranks them. Closing one spends cash, so the order only
    # bites when some column in the group is buying its short back while
    # another opens one — which is what this case is.
    out.append(
        Case(
            "short only, one closing one opening",
            col(dear, cheap),
            col([2.0, 0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0, 0.0]),
            [2],
            [260.0],
            size_type=SizeType.TargetAmount,
            direction=Direction.ShortOnly,
            note="opening a short raises cash; closing one spends it",
        )
    )
    out.append(
        Case(
            "short only, unwinding under pressure",
            col(dear, cheap),
            col([2.0, 0.0, 0.2, 0.0, 0.0], [100.0, 0.0, 10.0, 0.0, 0.0]),
            [2],
            [205.0],
            size_type=SizeType.TargetAmount,
            direction=Direction.ShortOnly,
            note="both columns cut their shorts at once, and cash is tight",
        )
    )

    # `Value` sizing, which the estimate passes through unscaled — the one
    # branch that must *not* be multiplied by the valuation price.
    out.append(
        Case(
            "value sizing",
            col(cheap, dear),
            col([200.0, 0.0, -100.0, 0.0, 0.0], [50.0, 0.0, 120.0, 0.0, 0.0]),
            [2],
            [210.0],
            size_type=SizeType.Value,
            note="the cheap column asks for the larger cash amount, so pricing "
            "the size again would rank them the other way round",
        )
    )

    # `Percent` with a negative size, which is the branch that reserves twice
    # a short's value. Long-only so the reference stays solvent.
    out.append(
        Case(
            "percent sells",
            col(dear, cheap),
            col([0.5, 0.0, -0.5, 0.0, 0.0], [0.4, 0.0, -0.4, 0.0, 0.0]),
            [2],
            [400.0],
            size_type=SizeType.Percent,
            direction=Direction.LongOnly,
            note="a negative percent scales the position, not the cash",
        )
    )

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "cash_sharing.tsv",
    )
    args = parser.parse_args()

    data = []
    skipped = []
    for c in cases():
        try:
            pair = [run(c, auto) for auto in (False, True)]
        except ValueError as e:
            # The kernel refuses some states outright — a group driven to
            # negative cash, say. Those are invariant violations, already
            # covered by the execute_order harness, and not what this file is
            # about.
            skipped.append((c.name, str(e)))
            continue
        data.extend(pair)
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    differing = 0
    for i in range(0, len(data), 2):
        if data[i]["fill_size"] != data[i + 1]["fill_size"]:
            differing += 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name, err in skipped:
        print(f"  skipped {name!r}: {err}")
    print(f"  {differing} of {len(data) // 2} cases change with the call order")


if __name__ == "__main__":
    main()
