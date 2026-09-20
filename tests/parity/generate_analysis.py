#!/usr/bin/env python3
"""Generate differential fixtures for the analysis layer, end to end.

This is the widest harness in the project: one case runs a simulation, derives
the value curve from its records, and reduces that to a dozen scalars — so a
single row exercises the execution kernel, the ``from_orders`` loop, the record
layer, the rebuild chain and the metrics together. A failure here is diagnosed
by the *narrower* harnesses, which is why they were built first.

It is also the end-to-end smoke test in fixture form. The only piece of the
intended pipeline still missing is the factor expression that would produce
the signals; everything from the orders onward is covered.

Each case stores:

* the workload that produced it, so the simulation is reproducible from the
  fixture alone;
* the whole value and returns series, per column, because a metric that agrees
  on a series that disagrees is a coincidence;
* the scalar metrics, which are where the degenerate cases live — a column
  that never traded, one that never moved, one with no drawdown.

Usage::

    python tests/parity/generate_analysis.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
from vectorbt.portfolio.enums import Direction, SizeType
from vectorbt.portfolio.nb import (
    asset_value_nb,
    cash_flow_nb,
    cash_nb,
    get_exit_trades_nb,
    simulate_from_orders_nb,
    value_nb,
)
from vectorbt.portfolio.nb import asset_flow_nb
from vectorbt.records.nb import col_map_nb
from vectorbt.returns import nb as ret_nb

from generate_execute_order import fmt_float

ANN_FACTOR = 365.0

METRICS = [
    "total_return",
    "annualized_return",
    "annualized_volatility",
    "max_drawdown",
    "sharpe_ratio",
    "sortino_ratio",
    "calmar_ratio",
    "downside_risk",
]

TRADE_STATS = [
    "trade_count",
    "win_rate",
    "profit_factor",
    "expectancy",
    "total_pnl",
]

COLUMNS = [
    "case",
    "n_rows",
    "n_cols",
    "init_cash",
    "fees",
    "fixed_fees",
    "slippage",
    "size_type",
    "direction",
    "close",
    "size",
    "value",
    "returns",
    *METRICS,
    *TRADE_STATS,
    "note",
]


class Case:
    """A workload: a shape, prices, sizes and cost parameters."""

    def __init__(
        self,
        name: str,
        close: np.ndarray,
        size: np.ndarray,
        init_cash: float = 1000.0,
        fees: float = 0.0,
        fixed_fees: float = 0.0,
        slippage: float = 0.0,
        size_type: int = SizeType.Amount,
        direction: int = Direction.Both,
        note: str = "",
    ) -> None:
        self.name = name
        self.close = close
        self.size = size
        self.init_cash = init_cash
        self.fees = fees
        self.fixed_fees = fixed_fees
        self.slippage = slippage
        self.size_type = size_type
        self.direction = direction
        self.note = note
        self.n_rows, self.n_cols = close.shape


def flat(a: np.ndarray) -> str:
    """Serializes a 2-D array column-major, matching the engine's layout."""
    return ";".join(fmt_float(v) for v in a.flatten(order="F"))


def run(case: Case) -> dict[str, str]:
    shape = (case.n_rows, case.n_cols)
    init_cash = np.full(case.n_cols, case.init_cash)

    records, _ = simulate_from_orders_nb(
        target_shape=shape,
        group_lens=np.full(case.n_cols, 1),
        init_cash=init_cash,
        call_seq=np.full(shape, 0),
        size=case.size,
        price=case.close,
        size_type=np.asarray(case.size_type),
        direction=np.asarray(case.direction),
        fees=np.asarray(case.fees),
        fixed_fees=np.asarray(case.fixed_fees),
        slippage=np.asarray(case.slippage),
        flex_2d=True,
    )
    col_map = col_map_nb(records["col"], case.n_cols)

    assets = np.cumsum(asset_flow_nb(shape, records, col_map, Direction.Both), axis=0)
    cash = cash_nb(cash_flow_nb(shape, records, col_map, False), init_cash)
    value = value_nb(cash, asset_value_nb(case.close, assets))
    returns = ret_nb.returns_nb(value, init_cash)

    out = {
        "value": flat(value),
        "returns": flat(returns),
        "total_return": ";".join(fmt_float(v) for v in ret_nb.cum_returns_final_nb(returns, 0.0)),
        "annualized_return": ";".join(
            fmt_float(v) for v in ret_nb.annualized_return_nb(returns, ANN_FACTOR)
        ),
        "annualized_volatility": ";".join(
            fmt_float(v) for v in ret_nb.annualized_volatility_nb(returns, ANN_FACTOR)
        ),
        "max_drawdown": ";".join(fmt_float(v) for v in ret_nb.max_drawdown_nb(returns)),
        "sharpe_ratio": ";".join(
            fmt_float(v) for v in ret_nb.sharpe_ratio_nb(returns, ANN_FACTOR, 0.0)
        ),
        "sortino_ratio": ";".join(
            fmt_float(v) for v in ret_nb.sortino_ratio_nb(returns, ANN_FACTOR, 0.0)
        ),
        "calmar_ratio": ";".join(fmt_float(v) for v in ret_nb.calmar_ratio_nb(returns, ANN_FACTOR)),
        "downside_risk": ";".join(
            fmt_float(v) for v in ret_nb.downside_risk_nb(returns, ANN_FACTOR, 0.0)
        ),
    }

    # Trade statistics, over the exit-trade view. Computed here rather than
    # through the Portfolio wrapper so the comparison stays at the same tier as
    # everything else in this file.
    trades = get_exit_trades_nb(records, case.close, col_map)
    counts, rates, factors, expectancies, pnls = [], [], [], [], []
    for col in range(case.n_cols):
        pnl = trades["pnl"][trades["col"] == col]
        n = len(pnl)
        wins, losses = pnl[pnl > 0], pnl[pnl < 0]
        counts.append(str(n))
        if n == 0:
            rates.append(fmt_float(np.nan))
            factors.append(fmt_float(np.nan))
            expectancies.append(fmt_float(np.nan))
            pnls.append(fmt_float(0.0))
            continue
        rate = len(wins) / n
        avg_win = wins.mean() if len(wins) else 0.0
        avg_loss = losses.mean() if len(losses) else 0.0
        with np.errstate(divide="ignore", invalid="ignore"):
            factors.append(fmt_float(wins.sum() / abs(losses.sum())))
        rates.append(fmt_float(rate))
        expectancies.append(fmt_float(rate * avg_win - (1 - rate) * abs(avg_loss)))
        pnls.append(fmt_float(pnl.sum()))

    out["trade_count"] = ";".join(counts)
    out["win_rate"] = ";".join(rates)
    out["profit_factor"] = ";".join(factors)
    out["expectancy"] = ";".join(expectancies)
    out["total_pnl"] = ";".join(pnls)
    return out


def cases(seed: int) -> list[Case]:
    rng = np.random.default_rng(seed)
    out: list[Case] = []

    # A rising market bought once and held: the simplest possible curve.
    close = np.linspace(10.0, 20.0, 12).reshape(-1, 1)
    size = np.full((12, 1), np.nan)
    size[0, 0] = 10.0
    out.append(Case("buy_and_hold", close, size, note="one entry, held to the end"))

    # The same, falling. Sharpe and Sortino must go negative together.
    out.append(
        Case("buy_and_hold_falling", close[::-1].copy(), size, note="a losing hold")
    )

    # A flat market: zero volatility, so Sharpe is infinite and Calmar is NaN.
    flat_close = np.full((12, 1), 10.0)
    out.append(
        Case(
            "flat_market",
            flat_close,
            size,
            note="zero volatility: Sharpe inf, Calmar NaN, no drawdown",
        )
    )

    # A column that never trades at all.
    out.append(
        Case(
            "no_trades",
            close,
            np.full((12, 1), np.nan),
            note="every metric on an untouched column",
        )
    )

    # One column trading, one not: the metric vector must stay aligned.
    two_close = np.hstack([close, close])
    two_size = np.full((12, 2), np.nan)
    two_size[0, 0] = 10.0
    out.append(
        Case(
            "one_active_one_idle",
            two_close,
            two_size,
            note="an idle column must not disturb its neighbour",
        )
    )

    # Round trips, so there are closed trades to count.
    rt_size = np.full((12, 1), np.nan)
    rt_size[[0, 3, 6, 9], 0] = [5.0, -5.0, 5.0, -5.0]
    out.append(Case("round_trips", close, rt_size, note="two closed winning trades"))

    wobble = np.array([10, 12, 9, 14, 8, 15, 11, 13, 10, 16, 12, 18], dtype=float).reshape(-1, 1)
    out.append(
        Case(
            "round_trips_mixed",
            wobble,
            rt_size,
            note="wins and losses, so win rate is neither 0 nor 1",
        )
    )

    # Costs on, so fees show up in the curve and in every trade's PnL.
    out.append(
        Case(
            "with_costs",
            wobble,
            rt_size,
            fees=0.01,
            fixed_fees=0.5,
            slippage=0.001,
            note="fees and slippage flow all the way into expectancy",
        )
    )

    # Shorts: the value curve can fall below zero-ish territory and the return
    # sign convention for a negative base has to hold.
    short_size = np.full((12, 1), np.nan)
    short_size[[0, 5], 0] = [-8.0, 8.0]
    out.append(
        Case(
            "short_round_trip",
            wobble,
            short_size,
            note="a short that covers; tests the signed-return convention",
        )
    )

    # A price gap to NaN: a bar with no information must not poison the column.
    gapped = wobble.copy()
    gapped[4, 0] = np.nan
    out.append(
        Case(
            "nan_price_bar",
            gapped,
            rt_size,
            note="a NaN bar is skipped by the reductions, not propagated",
        )
    )

    # Target-percent sizing revalues every bar, exercising a different path
    # into the same records.
    tp_size = np.full((12, 2), 0.5)
    out.append(
        Case(
            "target_percent",
            two_close,
            tp_size,
            size_type=SizeType.TargetPercent,
            note="rebalanced every bar: many small trades",
        )
    )

    # A short that goes badly enough for the portfolio value to turn
    # negative. This is the only way to reach the sign-flip branch in
    # `period_return`: a return measured from a negative base has to read as a
    # further loss, not a gain.
    spike = np.array([10, 10, 14, 18, 22, 26, 30, 30, 30, 30, 30, 30], dtype=float).reshape(-1, 1)
    blowup = np.full((12, 1), np.nan)
    blowup[0, 0] = -100.0
    out.append(
        Case(
            "short_into_a_spike",
            spike,
            blowup,
            note="portfolio value goes negative: the signed-return branch",
        )
    )

    # A portfolio with no money and nothing to do. Value is zero on every bar,
    # which is the other branch of `period_return` — and one that must give
    # zero rather than NaN.
    out.append(
        Case(
            "no_capital",
            close,
            np.full((12, 1), np.nan),
            init_cash=0.0,
            note="a zero value base: returns are 0, not NaN",
        )
    )

    # Seeded random sweeps, the bulk of the coverage.
    for n in range(24):
        n_rows, n_cols = 20, 3
        c = np.abs(rng.normal(20.0, 4.0, size=(n_rows, n_cols))) + 1.0
        s = rng.normal(0.0, 3.0, size=(n_rows, n_cols))
        s[rng.random(s.shape) < 0.55] = np.nan
        out.append(
            Case(
                f"random_{n:02d}",
                c,
                s,
                fees=float(rng.choice([0.0, 0.001, 0.01])),
                fixed_fees=float(rng.choice([0.0, 0.5])),
                slippage=float(rng.choice([0.0, 0.005])),
                direction=int(rng.choice([Direction.Both, Direction.LongOnly])),
                note="seeded random sweep",
            )
        )
    return out


DIRECTION_NAMES = {0: "LongOnly", 1: "ShortOnly", 2: "Both"}
SIZE_TYPE_NAMES = {
    0: "Amount",
    1: "Value",
    2: "Percent",
    3: "TargetAmount",
    4: "TargetValue",
    5: "TargetPercent",
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "analysis.tsv",
    )
    parser.add_argument("--seed", type=int, default=20240918)
    args = parser.parse_args()

    lines = ["\t".join(COLUMNS)]
    for case in cases(args.seed):
        fields = {k: "" for k in COLUMNS}
        fields.update(
            {
                "case": case.name,
                "n_rows": str(case.n_rows),
                "n_cols": str(case.n_cols),
                "init_cash": fmt_float(case.init_cash),
                "fees": fmt_float(case.fees),
                "fixed_fees": fmt_float(case.fixed_fees),
                "slippage": fmt_float(case.slippage),
                "size_type": SIZE_TYPE_NAMES[int(case.size_type)],
                "direction": DIRECTION_NAMES[int(case.direction)],
                "close": flat(case.close),
                "size": flat(case.size),
                "note": case.note,
            }
        )
        fields.update(run(case))
        lines.append("\t".join(fields[k] for k in COLUMNS))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")
    print(f"wrote {len(lines) - 1} fixtures to {args.out}")


if __name__ == "__main__":
    main()
