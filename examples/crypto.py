"""Run a moving-average sweep over cached Binance Spot klines."""

import argparse

import barq
import numpy as np


def crossed_above(left, right):
    above = (left > right) & np.isfinite(left) & np.isfinite(right)
    crossed = np.zeros_like(above, dtype=bool)
    crossed[1:] = above[1:] & ~above[:-1]
    return crossed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--symbols", default="BTCUSDT,ETHUSDT,SOLUSDT")
    parser.add_argument("--interval", default="1d")
    parser.add_argument("--dir", default="data/binance")
    args = parser.parse_args()

    symbols = tuple(
        symbol.strip() for symbol in args.symbols.split(",") if symbol.strip()
    )
    series = []
    for symbol in symbols:
        bars = barq.data.load_cached(symbol, args.interval, directory=args.dir)
        if not bars:
            raise SystemExit(
                f"no cached data for {symbol}; run examples/fetch.py "
                f"--symbols {','.join(symbols)} --interval {args.interval} --dir {args.dir}"
            )
        series.append((symbol, bars))
    panel = barq.data.align(series)

    pairs = ((10, 50), (20, 50), (20, 100), (50, 200))
    expressions = {}
    for index, (fast, slow) in enumerate(pairs):
        expressions[f"fast_{index}"] = f"Mean($close, {fast})"
        expressions[f"slow_{index}"] = f"Mean($close, {slow})"
    values = barq.FactorSet.compile(expressions).evaluate({"close": panel.close})
    entries = np.concatenate(
        [
            crossed_above(values[f"fast_{i}"], values[f"slow_{i}"])
            for i in range(len(pairs))
        ],
        axis=1,
    )
    exits = np.concatenate(
        [
            crossed_above(values[f"slow_{i}"], values[f"fast_{i}"])
            for i in range(len(pairs))
        ],
        axis=1,
    )
    portfolio = barq.from_signals(
        np.tile(panel.close, (1, len(pairs))),
        entries,
        exits,
        init_cash=10_000.0,
        fees=0.001,
        slippage=0.0005,
        ann_factor=barq.data.bars_per_year(args.interval),
        threads=4,
    )

    labels = [(fast, slow, symbol) for fast, slow in pairs for symbol in symbols]
    sharpe = portfolio.traded_only(portfolio.sharpe_ratio())
    total = portfolio.total_return()
    drawdown = portfolio.max_drawdown()
    trades = portfolio.trade_count()
    print(f"{panel.n_rows} bars x {panel.n_cols} symbols, coverage={panel.coverage()}")
    for index, (fast, slow, symbol) in enumerate(labels):
        print(
            f"fast={fast:<3} slow={slow:<3} {symbol:<10} trades={trades[index]:<4} "
            f"return={total[index]:>8.1%} maxDD={drawdown[index]:>8.1%} "
            f"sharpe={sharpe[index]:>7.2f}"
        )


if __name__ == "__main__":
    main()
