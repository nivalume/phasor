"""Backtesting a model's predictions, against baselines that need no model.

A model that predicts returns is worth something only if trading on its
predictions beats trading without it. ``backtest`` runs the predictions as
signals and, beside them, the same run with baselines: holding throughout,
and random signals as many and as regularly spaced as the model's
(``phasor.signals.random_like``). ``compare`` puts their metrics side by
side.

The predictions must be made without the bar's own future: a prediction at
bar ``t`` may use what is known at ``t``'s close, and is traded from there.
Labels from ``phasor.labels`` are targets to train on, never predictions.
"""

from __future__ import annotations

from typing import Any

import numpy as np

__all__ = ["backtest", "compare"]


def backtest(close: Any, predictions: Any, *, enter_above: float = 0.0, exit_below: float | None = None,
             baselines: tuple[str, ...] = ("hold", "random"), seed: int = 0, **kwargs: Any) -> dict:
    """Portfolios by name: ``"model"`` and each of ``baselines``.

    The model enters where a prediction is above ``enter_above`` and exits
    where one is below ``exit_below`` (``enter_above`` by default). NaN
    predictions are neither. ``"hold"`` enters on the first bar and never
    exits; ``"random"`` is ``random_like(entries, keep="gaps")`` with exits
    at the model's holding periods, shuffled the same way. Every other
    keyword goes to ``from_signals`` for all of them.
    """
    from . import from_signals
    from .signals import random_like

    pred = np.asarray(predictions, dtype=np.float64)
    close = np.asarray(close, dtype=np.float64)
    if pred.shape != close.shape:
        raise ValueError(f"predictions must be shaped like close: {pred.shape} and {close.shape}")
    lower = enter_above if exit_below is None else exit_below
    with np.errstate(invalid="ignore"):
        entries = pred > enter_above
        exits = pred < lower
    out = {"model": from_signals(close, entries, exits, **kwargs)}
    for name in baselines:
        if name == "hold":
            hold = np.zeros_like(entries)
            hold[0] = True
            out[name] = from_signals(close, hold, np.zeros_like(entries), **kwargs)
        elif name == "random":
            out[name] = from_signals(
                close,
                random_like(entries, seed=seed, keep="gaps"),
                random_like(exits, seed=seed + 1, keep="gaps"),
                **kwargs,
            )
        else:
            raise ValueError(f"baselines are 'hold' and 'random'; got {name!r}")
    return out


def compare(portfolios: dict, metrics: tuple[str, ...] = ("total_return", "sharpe_ratio", "max_drawdown")) -> dict:
    """Each metric per portfolio: ``{metric: {name: value per account}}``."""
    return {m: {name: getattr(pf, m)() for name, pf in portfolios.items()} for m in metrics}
