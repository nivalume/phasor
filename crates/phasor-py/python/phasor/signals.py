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
from ._columns import restore, shape as _shape

__all__ = ["random_by_prob", "random_entries_exits", "random_n"]


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
