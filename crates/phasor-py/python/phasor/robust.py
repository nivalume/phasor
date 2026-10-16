"""Overfitting statistics: putting a sweep's best result back in context.

A sweep that tries a thousand parameter sets and reports the best Sharpe has
not measured a strategy; it has measured the maximum of a thousand noisy
estimates. These follow Bailey & López de Prado:

- :func:`probabilistic_sharpe` — the probability that the true Sharpe ratio
  beats a benchmark, given the sample length, skew and fat tails.
- :func:`expected_max_sharpe` — what the best of ``N`` worthless trials would
  show by luck alone.
- :func:`deflated_sharpe` — the probabilistic Sharpe ratio against that
  luck-only maximum instead of against zero.
- :func:`pbo_cscv` — how often the in-sample winner of a sweep lands at or
  below the median out of sample.

**Sharpe ratios here are per period**: mean over the ``ddof=1`` standard
deviation of the bar returns, no risk-free rate, not annualized. That is not
what :meth:`Portfolio.sharpe_ratio` returns. Per-period ratios of a sweep's
columns are ``np.nanmean(r, axis=0) / np.nanstd(r, axis=0, ddof=1)``.

NaN is a missing bar and is skipped. Degenerate inputs — fewer than two bars,
a column that never moved — give NaN rather than a confident probability.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from . import _phasor
from ._columns import columns, per_column

__all__ = ["deflated_sharpe", "expected_max_sharpe", "pbo_cscv", "probabilistic_sharpe"]


def probabilistic_sharpe(returns: Any, benchmark_sr: float = 0.0) -> Any:
    """``P(true Sharpe > benchmark_sr)`` for each column of ``returns``.

    ``returns`` is per-bar returns, ``(bars,)`` for one strategy — which gives
    one float — or ``(bars, strategies)``, which gives one per column.
    ``benchmark_sr`` is per period, like everything in this module.
    """
    arr, one_d = columns(returns, "returns")
    return per_column(_phasor.probabilistic_sharpe(arr, float(benchmark_sr)), one_d)


def expected_max_sharpe(n_trials: int, sr_variance: float) -> float:
    """The Sharpe ratio the best of ``n_trials`` zero-edge trials shows by luck.

    ``sr_variance`` is the variance of the trial Sharpe ratios across the
    sweep. One trial has nothing to be lucky against and gives 0.
    """
    return _phasor.expected_max_sharpe(int(n_trials), float(sr_variance))


def deflated_sharpe(returns: Any, trial_sharpes: Any) -> Any:
    """:func:`probabilistic_sharpe` against the sweep's luck-only maximum.

    ``trial_sharpes`` is the per-period Sharpe ratio of every trial in the
    sweep, the one being judged included. Its NaN-free count and ``ddof=1``
    variance set the benchmark; fewer than two trials give NaN.
    """
    arr, one_d = columns(returns, "returns")
    trials = np.ascontiguousarray(trial_sharpes, dtype=np.float64)
    if trials.ndim != 1:
        raise ValueError(f"trial_sharpes must be 1-D; got {trials.ndim} dimensions")
    return per_column(_phasor.deflated_sharpe(arr, trials), one_d)


def pbo_cscv(perf: Any, n_splits: int = 16) -> float:
    """Probability of backtest overfitting, by combinatorially symmetric
    cross-validation.

    ``perf`` is per-bar returns, ``(bars, strategies)``, at least two
    strategies. The bars are cut into ``n_splits`` contiguous blocks; for
    every choice of half of them as in-sample, the column with the best
    in-sample mean return is ranked out of sample. The result is the fraction
    of splits where it lands at or below the median.

    ``n_splits`` must be even, at most the number of bars, and at most 16 —
    ``C(16, 8) = 12,870`` splits, past which the cost grows fourfold per pair
    of blocks while the estimate stops improving. 16 is the paper's choice.
    """
    arr, _ = columns(perf, "perf")
    return _phasor.pbo_cscv(arr, int(n_splits))
