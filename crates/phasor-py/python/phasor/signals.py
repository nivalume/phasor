"""Random signals, for null-model comparison.

A strategy whose entries beat :func:`random_n` entries with the same count
has an entry rule; one that does not has a sample.

Every draw is a function of ``(seed, row, col)`` alone, not of a random
stream: the same seed gives the same signals, bit for bit, and a column's
signals do not change when other columns are generated beside it. They are
not NumPy's draws and do not try to be.

``shape`` is ``n`` for one 1-D series or ``(n_rows, n_cols)`` for a panel.
"""

from __future__ import annotations

from typing import Any

import numpy as np

from . import _phasor
from ._columns import columns as _columns, restore, shape as _shape

__all__ = ["generate_ohlc_stop_exits", "random_by_prob", "random_entries_exits", "random_n"]


def random_by_prob(shape: Any, prob: float, *, seed: int = 0) -> np.ndarray:
    """Each bar fires independently with probability ``prob``.

    The count per column is binomial, not fixed; use :func:`random_n` when it
    must match another signal's.
    """
    n_rows, n_cols, one_d = _shape(shape)
    return restore(_phasor.random_by_prob(n_rows, n_cols, float(prob), int(seed)), one_d)


def random_n(shape: Any, n: int, *, seed: int = 0) -> np.ndarray:
    """Exactly ``n`` signals per column, at distinct rows drawn uniformly."""
    n_rows, n_cols, one_d = _shape(shape)
    return restore(_phasor.random_n(n_rows, n_cols, int(n), int(seed)), one_d)


def random_entries_exits(shape: Any, n: int, *, seed: int = 0) -> tuple[np.ndarray, np.ndarray]:
    """``n`` entry/exit pairs per column, strictly alternating.

    ``2n`` distinct rows are drawn, sorted, and dealt entry, exit, entry,
    exit — so every entry has its exit before the next entry and no bar is
    both. Returns ``(entries, exits)``.
    """
    n_rows, n_cols, one_d = _shape(shape)
    entries, exits = _phasor.random_entries_exits(n_rows, n_cols, int(n), int(seed))
    return restore(entries, one_d), restore(exits, one_d)


def _stop(value: Any, name: str) -> Any:
    """A stop input: a number, one value per column (1-D) or a matrix."""
    if value is None or isinstance(value, (int, float)):
        return value
    array = np.asarray(value, dtype=np.float64)
    if array.ndim == 0:
        return float(array)
    if array.ndim == 1:
        return np.ascontiguousarray(array)
    if array.ndim == 2:
        return np.asfortranarray(array)
    raise ValueError(f"{name} must be a number, a 1-D array or a 2-D array")


def generate_ohlc_stop_exits(
    entries: Any,
    close: Any,
    *,
    open: Any = None,
    high: Any = None,
    low: Any = None,
    sl_stop: Any = None,
    sl_trail: bool = False,
    tsl_stop: Any = None,
    tsl_th: Any = None,
    tp_stop: Any = None,
    td_stop: Any = None,
    dt_stop: Any = None,
    short: bool = False,
    wait: int = 1,
    until_next: bool = True,
    skip_until_exit: bool = False,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Where each entry's stops would fire, without running a simulation.

    Each entry is taken at its bar's close and its stops — fractions of that
    close, NaN off; ``td_stop`` in bars after the entry, ``dt_stop`` a row —
    are tested on the bars after it with the simulation's own test. Returns
    the exits, the price each fired at (NaN elsewhere) and which stop fired
    (0 stop-loss, 1 trailing, 2 take-profit, 3 time, -1 elsewhere), shaped
    like ``close``.

    ``until_next`` ends an entry's search at the next entry; ``skip_until_exit``
    ignores the entries before an exit. With ``until_next=False`` and
    ``skip_until_exit=True`` the exits are the stop exits ``from_signals``
    fills for the same entries when it ignores an entry while holding. ``wait``
    is how many bars after the entry the first test is, at least 1.
    """
    close_2d, one_d = _columns(close, "close")
    entries_2d, _ = _columns(entries, "entries", dtype=bool)
    bar = lambda a, name: None if a is None else _columns(a, name)[0]  # noqa: E731
    exits, price, kind = _phasor.generate_ohlc_stop_exits(
        entries_2d, close_2d, open=bar(open, "open"), high=bar(high, "high"),
        low=bar(low, "low"), sl_stop=_stop(sl_stop, "sl_stop"), sl_trail=bool(sl_trail),
        tsl_stop=_stop(tsl_stop, "tsl_stop"), tsl_th=_stop(tsl_th, "tsl_th"),
        tp_stop=_stop(tp_stop, "tp_stop"), td_stop=_stop(td_stop, "td_stop"),
        dt_stop=_stop(dt_stop, "dt_stop"), short=bool(short), wait=int(wait),
        until_next=bool(until_next), skip_until_exit=bool(skip_until_exit),
    )
    return restore(exits, one_d), restore(price, one_d), restore(kind, one_d)
