#!/usr/bin/env python3
"""Generate a margin-account fixture from NautilusTrader.

vectorbt has no margin account, so phasor's margin columns had nothing to be
checked against but hand-worked cases. NautilusTrader's ``MarginAccount`` is
an independent implementation of the same bookkeeping: a linear perpetual
(``BTCUSDT-PERP.BINANCE`` from its test kit, taker fee 0.018%) traded with
market orders on a schedule, positions netted, every fill realizing against
the average entry and paying its commission out of the balance.

For each bar the fixture holds the bar, the fill of the order the schedule
placed on it (Nautilus matches a market order placed in ``on_bar`` after the
call, against the bar's close: it fills at that close), and the account's
equity at the close before that order: the balance plus the position's
unrealized profit. phasor replays the fills on a margin column, each at its
bar's close, and its value on every bar must be that equity less the bar's
fee (``crates/phasor-analysis/tests/parity_nautilus.rs``).

Usage::

    uv venv .venv-nt --python 3.12 && uv pip install --python .venv-nt/bin/python \\
        nautilus_trader==1.231.0
    .venv-nt/bin/python tests/parity/generate_nautilus_margin.py

Output: ``tests/fixtures/nautilus_margin.tsv``.
"""

from __future__ import annotations

import random
from decimal import Decimal
from pathlib import Path

from nautilus_trader.backtest.engine import BacktestEngine, BacktestEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.model.currencies import USDT
from nautilus_trader.model.data import Bar, BarType
from nautilus_trader.model.enums import AccountType, OmsType, OrderSide
from nautilus_trader.model.identifiers import TraderId, Venue
from nautilus_trader.model.objects import Money, Price, Quantity
from nautilus_trader.test_kit.providers import TestInstrumentProvider
from nautilus_trader.trading.strategy import Strategy

VENUE = Venue("BINANCE")
CASH = 100_000.0
LEVERAGE = 10


class Schedule(Strategy):
    """Records the account's equity at each bar's close, then places the
    scheduled market order; records the fills."""

    def __init__(self, instrument, bar_type, orders):
        super().__init__()
        self.instrument = instrument
        self.bar_type = bar_type
        self.orders = orders
        self.bar = -1
        self.equity = []
        self.fills = {}

    def on_start(self):
        self.subscribe_bars(self.bar_type)

    def on_bar(self, bar):
        self.bar += 1
        account = self.portfolio.account(VENUE)
        unrealized = self.portfolio.unrealized_pnl(self.instrument.id)
        equity = account.balance_total(USDT).as_double()
        if unrealized is not None:
            equity += unrealized.as_double()
        self.equity.append(equity)
        qty = self.orders.get(self.bar)
        if qty:
            order = self.order_factory.market(
                self.instrument.id,
                OrderSide.BUY if qty > 0 else OrderSide.SELL,
                Quantity(abs(qty), self.instrument.size_precision),
            )
            self.submit_order(order)

    def on_order_filled(self, event):
        # Matched after `on_bar` returns, at the close of the bar it was
        # placed on, before the next bar.
        sign = 1.0 if event.order_side == OrderSide.BUY else -1.0
        qty, px, fee = self.fills.get(self.bar, (0.0, 0.0, 0.0))
        assert qty == 0.0, "one fill per bar"
        self.fills[self.bar] = (
            sign * event.last_qty.as_double(),
            event.last_px.as_double(),
            event.commission.as_double(),
        )


def bars(bar_type, n, rng, precision):
    out, p = [], 30_000.0
    for i in range(n):
        o = p * (1 + 0.004 * (rng.random() - 0.5))
        c = o * (1 + 0.02 * (rng.random() - 0.5))
        h = max(o, c) * (1 + 0.003 * rng.random())
        lo = min(o, c) * (1 - 0.003 * rng.random())
        p = c
        ts = (1_700_000_000 + 60 * i) * 1_000_000_000
        out.append(
            Bar(
                bar_type,
                Price(round(o, precision), precision),
                Price(round(h, precision), precision),
                Price(round(lo, precision), precision),
                Price(round(c, precision), precision),
                Quantity(1_000, 3),
                ts,
                ts,
            )
        )
    return out


def run(case, n, seed):
    rng = random.Random(seed)
    instrument = TestInstrumentProvider.btcusdt_perp_binance()
    bar_type = BarType.from_str(f"{instrument.id}-1-MINUTE-LAST-EXTERNAL")
    data = bars(bar_type, n, rng, instrument.price_precision)
    orders = {}
    for i in range(n - 1):
        if rng.random() < 0.35:
            orders[i] = round(rng.choice([-1, 1]) * rng.uniform(0.05, 2.0), 3)
    engine = BacktestEngine(
        BacktestEngineConfig(
            trader_id=TraderId("BACKTESTER-001"),
            logging=LoggingConfig(bypass_logging=True),
        )
    )
    engine.add_venue(
        venue=VENUE,
        oms_type=OmsType.NETTING,
        account_type=AccountType.MARGIN,
        base_currency=None,
        starting_balances=[Money(CASH, USDT)],
        default_leverage=Decimal(LEVERAGE),
        bar_execution=True,
    )
    engine.add_instrument(instrument)
    engine.add_data(data)
    strategy = Schedule(instrument, bar_type, orders)
    engine.add_strategy(strategy)
    engine.run()
    rows = []
    for i, bar in enumerate(data):
        qty, px, fee = strategy.fills.get(i, (0.0, float("nan"), 0.0))
        rows.append(
            [case, i, bar.open.as_double(), bar.high.as_double(), bar.low.as_double(),
             bar.close.as_double(), qty, px, fee, strategy.equity[i]]
        )
    engine.dispose()
    return rows


def main():
    header = ["case", "bar", "open", "high", "low", "close", "fill_qty", "fill_px", "fee", "equity"]
    rows = []
    for case, (n, seed) in enumerate([(120, 1), (120, 2), (200, 3), (60, 4)]):
        rows += run(case, n, seed)
    path = Path(__file__).resolve().parents[1] / "fixtures" / "nautilus_margin.tsv"
    with path.open("w") as f:
        f.write("# NautilusTrader 1.231.0 MarginAccount, BTCUSDT-PERP.BINANCE (test kit), "
                f"starting balance {CASH:g} USDT, leverage {LEVERAGE}, netting.\n")
        f.write("\t".join(header) + "\n")
        for r in rows:
            f.write("\t".join(repr(v) if isinstance(v, float) else str(v) for v in r) + "\n")
    print(f"wrote {len(rows)} rows to {path}")


if __name__ == "__main__":
    main()
