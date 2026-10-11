"""Time-series cross-validation: which rows a parameter choice may see.

A backtest that picks its parameters on the same bars it reports is measuring
its own fit, not its edge. The splitters here return row indices, not data, so
they compose with anything that takes an index: slicing a price matrix before
``barq.from_signals``, a pandas ``iloc``, or a model's ``fit``.

    import numpy as np
    from barq import validation

    for split in validation.rolling(n=1000, train=500, test=100):
        close[split.train], close[split.test]

Every splitter keeps time order: test rows always come from a contiguous block,
and for ``rolling`` and ``expanding`` every train row precedes every test row.
``purged_kfold`` is the one exception, because k-fold uses the future to train
on the past; it pays for that by dropping the rows where leakage concentrates.

Splitters are explicit about their sizes rather than accepting a fraction or a
count of splits, so the same call on a longer series gives more splits of the
same shape, not a different shape.
"""

from __future__ import annotations

import math
from collections.abc import Callable, Hashable, Iterable, Mapping
from dataclasses import dataclass
from typing import Any

import numpy as np

__all__ = ["Split", "rolling", "expanding", "purged_kfold", "walk_forward"]


@dataclass(frozen=True, eq=False)
class Split:
    """One train/test pair of integer row indices.

    Attributes
    ----------
    train : np.ndarray
        Sorted ``int64`` row indices to fit or select parameters on.
    test : np.ndarray
        Sorted ``int64`` row indices to evaluate the chosen parameters on.
        Never overlaps ``train``.
    """

    train: np.ndarray
    test: np.ndarray

    def __repr__(self) -> str:
        return f"Split(train={_span(self.train)}, test={_span(self.test)})"


def _span(index: np.ndarray) -> str:
    """A compact description: a range when contiguous, a count otherwise."""
    if index.size == 0:
        return "[]"
    if index[-1] - index[0] + 1 == index.size:
        return f"[{index[0]}:{index[-1] + 1}]"
    return f"<{index.size} rows in [{index[0]}:{index[-1] + 1}]>"


def _int(name: str, value: Any, minimum: int) -> int:
    """``value`` as an int no smaller than ``minimum``, or a ``ValueError``.

    ``bool`` is refused even though it is an ``int`` subclass: ``test=True``
    is a mistake, not a window of one row.
    """
    if isinstance(value, bool) or not isinstance(value, (int, np.integer)):
        raise ValueError(f"{name} must be an integer; got {value!r}")
    value = int(value)
    if value < minimum:
        raise ValueError(f"{name} must be >= {minimum}; got {value}")
    return value


def _range(start: int, stop: int) -> np.ndarray:
    return np.arange(start, stop, dtype=np.int64)


def rolling(n: int, train: int, test: int, step: int | None = None) -> list[Split]:
    """Fixed-length train windows, each followed by a test window.

    Split ``i`` trains on rows ``[s, s + train)`` and tests on
    ``[s + train, s + train + test)`` where ``s = i * step``. A fixed train
    length means every parameter choice is made on the same amount of history,
    so a drift in the chosen parameters reflects the market, not a growing
    sample.

    Parameters
    ----------
    n : int
        Number of rows in the series.
    train : int
        Rows in each train window (>= 1).
    test : int
        Rows in each test window (>= 1).
    step : int, optional
        Rows to advance between splits. Defaults to ``test``, which makes the
        test windows tile the series without overlap, so their results can be
        concatenated into one out-of-sample record.

    Returns
    -------
    list of Split
        In time order. Trailing rows that cannot fill a whole test window are
        left out rather than tested on a short window.

    Raises
    ------
    ValueError
        If an argument is not a positive integer, or ``n < train + test``.
    """
    n = _int("n", n, 0)
    train = _int("train", train, 1)
    test = _int("test", test, 1)
    step = test if step is None else _int("step", step, 1)
    if train + test > n:
        raise ValueError(
            f"train + test ({train} + {test}) exceeds n ({n}); no split fits"
        )
    return [
        Split(_range(s, s + train), _range(s + train, s + train + test))
        for s in range(0, n - train - test + 1, step)
    ]


def expanding(n: int, min_train: int, test: int, step: int | None = None) -> list[Split]:
    """Train windows anchored at row 0 that grow by ``step`` each split.

    Split ``i`` trains on rows ``[0, e)`` and tests on ``[e, e + test)`` where
    ``e = min_train + i * step``. Use this when older history stays relevant
    and more of it should only sharpen the estimate; use ``rolling`` when old
    regimes should age out.

    Parameters
    ----------
    n : int
        Number of rows in the series.
    min_train : int
        Rows in the first (shortest) train window (>= 1).
    test : int
        Rows in each test window (>= 1).
    step : int, optional
        Rows the train window grows by between splits. Defaults to ``test``,
        so test windows tile the series without overlap.

    Returns
    -------
    list of Split
        In time order; every ``train`` starts at row 0.

    Raises
    ------
    ValueError
        If an argument is not a positive integer, or ``n < min_train + test``.
    """
    n = _int("n", n, 0)
    min_train = _int("min_train", min_train, 1)
    test = _int("test", test, 1)
    step = test if step is None else _int("step", step, 1)
    if min_train + test > n:
        raise ValueError(
            f"min_train + test ({min_train} + {test}) exceeds n ({n}); no split fits"
        )
    return [
        Split(_range(0, e), _range(e, e + test))
        for e in range(min_train, n - test + 1, step)
    ]


def purged_kfold(n: int, k: int, purge: int = 0, embargo: int = 0) -> list[Split]:
    """K contiguous test folds; train on the rest, minus the leaky edges.

    The series is cut into ``k`` contiguous folds (sizes differ by at most one
    row; the first ``n % k`` folds get the extra row). Each fold is a test set
    once, and trains on every other row except:

    * **purge** -- the ``purge`` rows immediately *before* the test fold. A
      label at row ``t`` usually looks forward (a return over the next ``h``
      bars, a triple-barrier outcome), so a train row just before the test
      fold has a label computed from test-period prices. Training on it fits
      the answer. Set ``purge`` to the label horizon.
    * **embargo** -- the ``embargo`` rows immediately *after* the test fold.
      Features usually look backward (moving averages, rolling volatility), so
      a train row just after the test fold has features built from test-period
      prices, and serial correlation makes its label close to the last test
      labels. Set ``embargo`` to the longest feature lookback, or a small
      fraction of ``n``.

    This follows López de Prado, *Advances in Financial Machine Learning*
    (2018), ch. 7, with the label horizon given as a fixed row count.

    Parameters
    ----------
    n : int
        Number of rows in the series.
    k : int
        Number of folds (2 <= k <= n).
    purge : int, default 0
        Rows dropped from train before each test fold.
    embargo : int, default 0
        Rows dropped from train after each test fold.

    Returns
    -------
    list of Split
        One per fold, in time order. ``train`` is generally not contiguous.

    Raises
    ------
    ValueError
        If an argument is out of range, or ``purge`` and ``embargo`` leave
        some fold with no train rows.
    """
    n = _int("n", n, 0)
    k = _int("k", k, 2)
    purge = _int("purge", purge, 0)
    embargo = _int("embargo", embargo, 0)
    if k > n:
        raise ValueError(f"k ({k}) exceeds n ({n}); some fold would be empty")
    rows = _range(0, n)
    splits = []
    for fold in np.array_split(rows, k):
        start, stop = int(fold[0]), int(fold[-1]) + 1
        keep = (rows < start - purge) | (rows >= stop + embargo)
        if not keep.any():
            raise ValueError(
                f"purge ({purge}) and embargo ({embargo}) leave no train rows "
                f"for the test fold [{start}:{stop}]"
            )
        splits.append(Split(rows[keep], fold.copy()))
    return splits


def _argmax(scores: Mapping[Hashable, float]) -> Hashable:
    """The key with the highest score; NaN scores never win, ties go first."""
    best_key, best = None, -math.inf
    found = False
    for key, score in scores.items():
        score = float(score)
        if math.isnan(score):
            continue
        if not found or score > best:
            best_key, best, found = key, score, True
    if not found:
        raise ValueError("every train score is NaN; nothing to select")
    return best_key


def walk_forward(
    fn: Callable[[np.ndarray], Mapping[Hashable, float]],
    n: int,
    splits: Iterable[Split],
    *,
    select: Callable[[Mapping[Hashable, float]], Hashable] | None = None,
) -> list[dict[str, Any]]:
    """Select parameters on each train window, report them on its test window.

    ``fn`` evaluates a whole parameter grid on a set of rows and returns
    ``{param_key: score}``. For each split, ``fn(split.train)`` scores the
    grid, ``select`` picks one key, then ``fn(split.test)`` scores the grid
    out of sample and only the chosen key's score is kept. The other test
    scores are discarded so they cannot influence the choice: reporting the
    best *test* score would be the in-sample fit this exists to avoid.

    Nothing here runs a backtest, so ``fn`` can wrap ``barq.from_signals``
    over a parameter grid (one column per parameter set) or anything else::

        def fn(rows):
            pf = barq.from_signals(close[rows], entries[rows], exits[rows])
            return dict(zip(params, pf.total_return()))

    Parameters
    ----------
    fn : callable
        ``fn(rows) -> Mapping[key, float]`` where ``rows`` is an ``int64``
        index array. Must return the chosen key on the test rows too.
    n : int
        Number of rows in the series; every split index must lie in
        ``[0, n)``. Catches splits built for a different series.
    splits : iterable of Split
        From ``rolling``, ``expanding``, ``purged_kfold``, or hand-built.
    select : callable, optional
        ``select(train_scores) -> key``. Defaults to the key with the highest
        score (NaN never wins; ties go to the first key in ``fn``'s order).
        Pass e.g. ``lambda s: min(s, key=s.get)`` to minimize a loss.

    Returns
    -------
    list of dict
        One per split, in order, with keys ``split`` (the position of the
        split in ``splits``), ``chosen`` (the selected key), ``train_score``
        and ``test_score`` (that key's score on train and on test).

    Raises
    ------
    ValueError
        If ``splits`` is empty, a split's indices fall outside ``[0, n)`` or
        overlap, ``fn`` returns no scores, or ``select`` picks a key missing
        from the train or test scores.
    """
    if not callable(fn):
        raise ValueError(f"fn must be callable; got {fn!r}")
    n = _int("n", n, 1)
    select = _argmax if select is None else select
    splits = list(splits)
    if not splits:
        raise ValueError("splits is empty; nothing to walk forward over")

    results = []
    for i, split in enumerate(splits):
        if not isinstance(split, Split):
            raise ValueError(f"splits[{i}] must be a Split; got {type(split).__name__}")
        for name in ("train", "test"):
            rows = getattr(split, name)
            if rows.size == 0:
                raise ValueError(f"splits[{i}].{name} is empty")
            if rows.min() < 0 or rows.max() >= n:
                raise ValueError(f"splits[{i}].{name} has indices outside [0, {n})")
        if np.intersect1d(split.train, split.test).size:
            raise ValueError(f"splits[{i}] has rows in both train and test")

        train_scores = fn(split.train)
        if not train_scores:
            raise ValueError(f"fn returned no scores for splits[{i}].train")
        chosen = select(train_scores)
        if chosen not in train_scores:
            raise ValueError(f"select chose {chosen!r}, which is not a train score key")
        test_scores = fn(split.test)
        if chosen not in test_scores:
            raise ValueError(
                f"fn did not score the chosen key {chosen!r} on splits[{i}].test"
            )
        results.append(
            {
                "split": i,
                "chosen": chosen,
                "train_score": train_scores[chosen],
                "test_score": test_scores[chosen],
            }
        )
    return results
