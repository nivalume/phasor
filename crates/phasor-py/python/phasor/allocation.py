"""Portfolio weights on a rebalancing schedule, and their backtest.

A weight function sees the trailing window of returns, ``(lookback, n_assets)``,
and returns one weight per asset. :func:`weights` calls it on each
rebalancing bar and leaves every other bar ``NaN``; :func:`from_weights`
turns that matrix into target-percent orders on one shared-cash account::

    from phasor import allocation as al

    w = al.weights(close, al.min_variance, every=21, lookback=126)
    pf = al.from_weights(close, w, fees=0.0005)
    base = al.from_weights(close, al.weights(close, al.equal_weight, every=21))

    w = al.weights(close, al.from_pypfopt("max_sharpe"), freq="M", index=dates)

A weight function only ever sees rows before the bar it rebalances on (the
window ends at the previous bar), so the orders placed at a bar's close never
use that bar's return. Weights need not sum to one: the rest is cash, and a
negative weight is a short.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

import numpy as np

__all__ = [
    "equal_weight",
    "from_pypfopt",
    "from_riskfolio",
    "from_weights",
    "inverse_volatility",
    "max_sharpe",
    "min_variance",
    "rebalance_mask",
    "risk_parity",
    "universal_weights",
    "weights",
]

WeightFn = Callable[[np.ndarray], Any]


def _close(close: Any) -> np.ndarray:
    a = np.asarray(close, dtype=float)
    if a.ndim == 1:
        a = a[:, None]
    if a.ndim != 2:
        raise ValueError("close must be 1-D or 2-D")
    return a


def rebalance_mask(n: int, *, every: int | None = None, freq: str | None = None,
                   index: Any = None, offset: int = 0) -> np.ndarray:
    """Which of ``n`` bars rebalance: every ``every`` bars from ``offset``,
    or the first bar of each calendar period (``freq`` = ``W``, ``M``,
    ``Q``, ``Y``; needs ``index``)."""
    if (every is None) == (freq is None):
        raise ValueError("give exactly one of every and freq")
    mask = np.zeros(n, dtype=bool)
    if every is not None:
        if every < 1:
            raise ValueError("every must be at least 1")
        mask[offset::every] = True
        return mask
    if index is None:
        raise ValueError("freq needs index, the bars' times")
    from .validation import _period_ids, _times

    ids = _period_ids(_times(index), freq)
    if len(ids) != n:
        raise ValueError(f"index has {len(ids)} times for {n} bars")
    mask[np.r_[True, ids[1:] != ids[:-1]]] = True
    return mask


def weights(close: Any, fn: WeightFn, *, every: int | None = None, freq: str | None = None,
            index: Any = None, lookback: int = 60, min_periods: int | None = None) -> np.ndarray:
    """Target weights, ``(n_bars, n_assets)``: ``fn(window)`` on each
    rebalancing bar with at least ``min_periods`` (default ``lookback``) past
    returns, ``NaN`` elsewhere. ``window`` holds the simple returns of the
    ``lookback`` bars before this one, ``NaN`` where a price is missing."""
    c = _close(close)
    n, k = c.shape
    with np.errstate(divide="ignore", invalid="ignore"):
        ret = np.vstack([np.full((1, k), np.nan), c[1:] / c[:-1] - 1])
    mask = rebalance_mask(n, every=every, freq=freq, index=index)
    need = lookback if min_periods is None else min_periods
    out = np.full((n, k), np.nan)
    for t in np.flatnonzero(mask):
        # Returns 1..t-1 are known at bar t's open; bar t's own is not used.
        window = ret[max(1, t - lookback):t]
        if len(window) < need:
            continue
        w = np.asarray(fn(window), dtype=float).ravel()
        if w.shape != (k,):
            raise ValueError(f"the weight function returned {w.shape[0]} weights for {k} assets")
        out[t] = w
    return out


def from_weights(close: Any, weights: Any, **kwargs: Any) -> Any:
    """Backtests target weights: ``phasor.from_orders`` with
    ``size_type="targetpercent"`` on one shared-cash account, sells before
    buys (``call_seq="auto"``). ``NaN`` rows place no orders. Other keyword
    arguments go to ``from_orders``."""
    from . import from_orders

    c = _close(close)
    w = np.asarray(weights, dtype=float)
    if w.ndim == 1:
        w = w[:, None]
    if w.shape != c.shape:
        raise ValueError(f"weights {w.shape} and close {c.shape} differ in shape")
    kwargs.setdefault("call_seq", "auto")
    return from_orders(c, np.asfortranarray(w), size_type="targetpercent", cash_sharing=True,
                       group_lens=[c.shape[1]], **kwargs)


# ------------------------------------------------------------ weight functions


def _present(window: np.ndarray) -> np.ndarray:
    """Rows where every asset has a return."""
    return window[~np.isnan(window).any(axis=1)]


def _cov(window: np.ndarray) -> np.ndarray:
    r = _present(window)
    if len(r) < 2:
        raise ValueError("fewer than two complete rows of returns in the window")
    s = np.cov(r, rowvar=False, ddof=1)
    s = np.atleast_2d(s)
    # A small ridge keeps a singular estimate invertible.
    return s + np.eye(len(s)) * 1e-10 * max(np.trace(s) / len(s), 1e-300)


def equal_weight(window: np.ndarray) -> np.ndarray:
    """``1 / n`` each."""
    k = window.shape[1]
    return np.full(k, 1.0 / k)


def inverse_volatility(window: np.ndarray) -> np.ndarray:
    """Proportional to ``1 / std``, summing to one."""
    sd = np.nanstd(window, axis=0, ddof=1)
    inv = np.where(sd > 0, 1.0 / sd, 0.0)
    return inv / inv.sum() if inv.sum() > 0 else equal_weight(window)


def _solve_long_only(s: np.ndarray, b: np.ndarray, long_only: bool) -> np.ndarray:
    """``w ∝ S⁻¹ b`` on the active assets, dropping those that come out
    negative until none do (``long_only``); normalized to sum to one."""
    k = len(b)
    active = np.ones(k, dtype=bool)
    while True:
        w = np.zeros(k)
        w[active] = np.linalg.solve(s[np.ix_(active, active)], b[active])
        total = w.sum()
        if total <= 0:
            return np.zeros(k)
        w /= total
        if not long_only or (w >= -1e-15).all():
            return np.where(np.abs(w) < 1e-15, 0.0, w)
        active &= w > 0
        if not active.any():
            return np.zeros(k)


def min_variance(window: np.ndarray, long_only: bool = True) -> np.ndarray:
    """Minimum variance under the sample covariance, fully invested."""
    s = _cov(window)
    return _solve_long_only(s, np.ones(len(s)), long_only)


def max_sharpe(window: np.ndarray, risk_free: float = 0.0, long_only: bool = True) -> np.ndarray:
    """The tangency portfolio, ``w ∝ S⁻¹ (μ - rf)``, fully invested; all cash
    (zeros) when no asset beats ``risk_free``."""
    r = _present(window)
    mu = r.mean(axis=0) - risk_free
    if (mu <= 0).all():
        return np.zeros(window.shape[1])
    return _solve_long_only(_cov(window), mu, long_only)


def risk_parity(window: np.ndarray, iterations: int = 1000, tol: float = 1e-12) -> np.ndarray:
    """Equal risk contributions, ``w_i (S w)_i`` the same for every asset,
    long only, summing to one."""
    s = _cov(window)
    k = len(s)
    w = np.full(k, 1.0 / k)
    for _ in range(iterations):
        contrib = w * (s @ w)
        target = contrib.sum() / k
        new = w * np.sqrt(target / contrib)
        new /= new.sum()
        if np.abs(new - w).max() < tol:
            return new
        w = new
    return w


# ------------------------------------------------------------ library wrappers


def _frame(window: np.ndarray) -> Any:
    try:
        import pandas as pd
    except ImportError as e:  # pragma: no cover - depends on the environment
        raise ImportError("the optimizer wrappers need pandas") from e
    return pd.DataFrame(_present(window), columns=[f"a{i}" for i in range(window.shape[1])])


def from_pypfopt(method: str = "max_sharpe", *, frequency: int = 252, **method_kwargs: Any) -> WeightFn:
    """A weight function from PyPortfolioOpt: historical mean and sample
    covariance, then ``EfficientFrontier.<method>(**method_kwargs)``
    (``max_sharpe``, ``min_volatility``, ``efficient_risk``, ...)."""

    def fn(window: np.ndarray) -> np.ndarray:
        from pypfopt import EfficientFrontier, expected_returns, risk_models

        df = _frame(window)
        mu = expected_returns.mean_historical_return(df, returns_data=True, frequency=frequency)
        cov = risk_models.sample_cov(df, returns_data=True, frequency=frequency)
        ef = EfficientFrontier(mu, cov)
        getattr(ef, method)(**method_kwargs)
        w = ef.clean_weights()
        return np.array([w[c] for c in df.columns])

    return fn


def from_riskfolio(*, model: str = "Classic", rm: str = "MV", obj: str = "Sharpe",
                   rf: float = 0.0, **kwargs: Any) -> WeightFn:
    """A weight function from Riskfolio-Lib: historical estimates, then
    ``Portfolio.optimization(model, rm, obj, rf, ...)``."""

    def fn(window: np.ndarray) -> np.ndarray:
        import riskfolio as rp

        df = _frame(window)
        port = rp.Portfolio(returns=df)
        port.assets_stats(method_mu="hist", method_cov="hist")
        w = port.optimization(model=model, rm=rm, obj=obj, rf=rf, hist=True, **kwargs)
        if w is None:
            return np.full(window.shape[1], np.nan)
        return w.reindex(df.columns).to_numpy().ravel()

    return fn


def universal_weights(close: Any, algo: str = "OLMAR", **kwargs: Any) -> np.ndarray:
    """Weights from an online algorithm of the Universal Portfolios library
    (``universal.algos.<algo>``), one row per bar. These algorithms decide a
    bar's weights from prices up to the bar before, as the library runs
    them; pass the result to :func:`from_weights`."""
    import pandas as pd
    from universal import algos

    c = _close(close)
    result = getattr(algos, algo)(**kwargs).run(pd.DataFrame(c))
    return np.asarray(result.weights, dtype=float)
