"""A synthetic moving-average parameter sweep through the Python API."""

import barq
import numpy as np


def crossed_above(left, right):
    above = (left > right) & np.isfinite(left) & np.isfinite(right)
    crossed = np.zeros_like(above, dtype=bool)
    crossed[1:] = above[1:] & ~above[:-1]
    return crossed


def main():
    symbols = ("BTC", "ETH", "SOL", "AVAX")
    pairs = ((5, 20), (5, 50), (10, 50), (10, 100), (20, 100), (20, 200), (20, 1900))
    rng = np.random.default_rng(20260920)
    close = 100.0 * np.cumprod(
        1.0 + rng.normal(0.0, 0.011, (2000, len(symbols))), axis=0
    )

    expressions = {}
    for index, (fast, slow) in enumerate(pairs):
        expressions[f"fast_{index}"] = f"Mean($close, {fast})"
        expressions[f"slow_{index}"] = f"Mean($close, {slow})"
    values = barq.FactorSet.compile(expressions).evaluate({"close": close})

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
    swept_close = np.tile(close, (1, len(pairs)))
    portfolio = barq.from_signals(
        swept_close,
        entries,
        exits,
        init_cash=10_000.0,
        fees=0.001,
        threads=4,
    )

    labels = [(fast, slow, symbol) for fast, slow in pairs for symbol in symbols]
    sharpe = portfolio.traded_only(portfolio.sharpe_ratio())
    total = portfolio.total_return()
    trades = portfolio.trade_count()
    best = int(np.nanargmax(sharpe))
    fast, slow, symbol = labels[best]

    print(f"{close.shape[0]} bars x {len(symbols)} symbols -> {len(labels)} columns")
    print(
        f"best: fast={fast}, slow={slow}, symbol={symbol}, "
        f"sharpe={sharpe[best]:.2f}, return={total[best]:.1%}, trades={trades[best]}"
    )
    print(
        "synthetic prices: these numbers demonstrate the pipeline, not a strategy result"
    )


if __name__ == "__main__":
    main()
