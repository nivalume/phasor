"""phasor — a vectorized backtesting engine, in Rust.

The shell is deliberately thin. Everything that computes lives in Rust; this
module exists to do the two things a Rust extension cannot do for the caller:
put arrays in the layout the engine reads, and give back something a notebook
can print.

    import numpy as np, phasor

    close = np.array([[10.0], [11.0], [12.0], [11.0]])
    entries = np.array([[True], [False], [False], [False]])
    exits = np.array([[False], [False], [True], [False]])

    pf = phasor.from_signals(close, entries, exits, init_cash=100.0, fees=0.001)
    print(pf.total_return())     # one value per column
    print(pf.stats())            # the table, for column 0

Layout
------
The engine is column-major; NumPy is row-major by default. Every array is
passed through ``np.asfortranarray``, which is free when the array is already
in that order and a visible copy when it is not. The Rust side refuses a
C-ordered array rather than reading it as a transposed price series, so a
caller who reaches past this module gets an error instead of a plausible
backtest on scrambled data.

Shapes
------
A column is one symbol under one parameter set. By default columns have
independent cash. ``cash_sharing=True`` groups adjacent columns into accounts;
``threads`` parallelizes accounts. A 1-D array of prices is read as one column.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import numpy as np

from . import _phasor, data, events, labels, robust, signals, synthetic, validation
from ._phasor import InstrumentId, BarType
from ._phasor import Portfolio

__all__ = [
    "Expression",
    "FactorSet",
    "Portfolio",
    "alpha158",
    "alpha360",
    "data",
    "events",
    "labels",
    "robust",
    "signals",
    "synthetic",
    "validation",
    "InstrumentId",
    "BarType",
    "from_orders",
    "from_signals",
    "strategy",
    "factors",
    "crossed_above",
]
__version__ = "0.1.0"


def _matrix(value: Any, name: str) -> np.ndarray:
    """A 2-D, F-ordered, float64 array, whatever shape came in.

    A 1-D input is one column, not one row. That is the direction that matches
    "a column is a run": a bare price series is a single backtest, not a single
    bar across many symbols.
    """
    array = np.asarray(value, dtype=np.float64)
    if array.ndim == 1:
        array = array.reshape(-1, 1)
    if array.ndim != 2:
        raise ValueError(f"{name} must be 1-D or 2-D; got {array.ndim} dimensions")
    return np.asfortranarray(array)


def _factor_fields(
    fields: Mapping[str, Any],
) -> tuple[list[tuple[str, np.ndarray]], tuple[int, int]]:
    if not isinstance(fields, Mapping):
        raise TypeError("factor inputs must be a mapping of field names to arrays")
    if not fields:
        raise ValueError("factor inputs cannot be empty")

    prepared = []
    shape = None
    for name, value in fields.items():
        if not isinstance(name, str):
            raise TypeError("factor field names must be strings")
        array = _matrix(value, f"factor field {name!r}")
        if shape is None:
            shape = array.shape
        elif array.shape != shape:
            raise ValueError(
                f"factor field {name!r} has shape {array.shape}, expected {shape}"
            )
        prepared.append((name, array))
    assert shape is not None
    return prepared, shape


class FactorSet:
    """A compiled, reusable set of named factor expressions."""

    __slots__ = ("_inner",)

    def __init__(self, inner: _phasor._FactorSet):
        self._inner = inner

    @classmethod
    def compile(cls, factors: Mapping[str, str]) -> FactorSet:
        if not isinstance(factors, Mapping):
            raise TypeError("factors must be a mapping of names to expressions")
        pairs = []
        for name, source in factors.items():
            if not isinstance(name, str) or not isinstance(source, str):
                raise TypeError("factor names and expressions must be strings")
            pairs.append((name, source))
        return cls(_phasor._FactorSet.compile(pairs))

    @property
    def names(self) -> tuple[str, ...]:
        return tuple(self._inner.names)

    @property
    def required_fields(self) -> tuple[str, ...]:
        return tuple(self._inner.required_fields)

    @property
    def uses_future(self) -> bool:
        return self._inner.uses_future

    def evaluate(self, fields: Mapping[str, Any]) -> dict[str, np.ndarray]:
        prepared, shape = _factor_fields(fields)
        return {
            name: values.reshape(shape, order="F")
            for name, values in self._inner.evaluate(prepared).items()
        }


class Expression:
    """One compiled factor expression."""

    __slots__ = ("_factors",)

    def __init__(self, factors: FactorSet):
        self._factors = factors

    @classmethod
    def compile(cls, source: str) -> Expression:
        if not isinstance(source, str):
            raise TypeError("expression must be a string")
        return cls(FactorSet.compile({"value": source}))

    @property
    def required_fields(self) -> tuple[str, ...]:
        return self._factors.required_fields

    @property
    def uses_future(self) -> bool:
        return self._factors.uses_future

    def evaluate(self, fields: Mapping[str, Any]) -> np.ndarray:
        return self._factors.evaluate(fields)["value"]


def alpha158() -> FactorSet:
    """Returns Qlib's default 158 compiled expressions."""
    return FactorSet(_phasor.alpha158())


def alpha360() -> FactorSet:
    """Returns Qlib's default 360 compiled expressions."""
    return FactorSet(_phasor.alpha360())


def _signals(value: Any, name: str) -> np.ndarray:
    """A 2-D, F-ordered, bool array."""
    array = np.asarray(value, dtype=bool)
    if array.ndim == 1:
        array = array.reshape(-1, 1)
    if array.ndim != 2:
        raise ValueError(f"{name} must be 1-D or 2-D; got {array.ndim} dimensions")
    return np.asfortranarray(array)


def _market(value: Any, name: str) -> Any:
    """Market data beside the prices: ``None``, a number, or a matrix shaped like ``close``.

    A 1-D array is one column, as for ``close`` itself, not one value per column.
    """
    if value is None or isinstance(value, (bool, int, float)):
        return value
    return _matrix(value, name)


def _param(value: Any, name: str) -> Any:
    """A parameter, in whichever of the engine's four broadcast shapes it is.

    A number stays a number — the engine's ``FlexArray`` reads a scalar with no
    allocation at all, so converting one to an array here would spend memory to
    lose that. Arrays are converted; ``None`` means "the engine's default",
    which is where every default lives.
    """
    if value is None:
        return None
    if isinstance(value, (bool, int, float)):
        return value
    array = np.asarray(value, dtype=np.float64)
    if array.ndim == 0:
        # A NumPy scalar, or a 0-d array. Unwrapping it keeps the engine on its
        # allocation-free scalar path instead of building a one-element buffer.
        return float(array)
    if array.ndim == 2:
        return np.asfortranarray(array)
    if array.ndim == 1:
        return np.ascontiguousarray(array)
    raise ValueError(f"{name} must be a number, a 1-D array or a 2-D array")


def _flag(value: Any, name: str) -> Any:
    """A bool parameter: a scalar, a matrix, or ``None`` for the default."""
    if value is None:
        return None
    if isinstance(value, (bool, np.bool_)):
        return bool(value)
    array = np.asarray(value, dtype=bool)
    if array.ndim == 0:
        return bool(array)
    if array.ndim == 1:
        array = array.reshape(-1, 1)
    if array.ndim != 2:
        raise ValueError(f"{name} must be a bool or a 2-D bool array")
    return np.asfortranarray(array)


def _accumulate(value: Any) -> str:
    """The accumulation mode, accepting the reference's boolean spelling.

    The reference takes ``accumulate=True``, meaning "add and remove", and
    ``False``, meaning "ignore repeated signals". Those are two of the four
    modes, and someone porting a call over should not have to look up which
    two — so both spellings work here.
    """
    if isinstance(value, (bool, np.bool_)):
        return "both" if value else "disabled"
    return value


def _group_lens(value):
    if value is None:
        return None
    if isinstance(value, (str, bytes)):
        raise TypeError("group_lens must be a sequence of positive integers")
    lengths = list(value)
    if any(
        isinstance(n, (bool, np.bool_)) or not isinstance(n, (int, np.integer))
        for n in lengths
    ):
        raise TypeError("group_lens must contain integers")
    if not lengths or any(n <= 0 for n in lengths):
        raise ValueError("group_lens must contain positive lengths")
    return [int(n) for n in lengths]


def _account_type(value):
    if value is None or isinstance(value, str):
        return value
    try:
        names = list(value)
    except TypeError:
        names = None
    if names is None or not all(isinstance(name, str) for name in names):
        raise TypeError("account_type must be 'cash', 'margin', or one of them per column")
    return names


def _column_map(value):
    if value is None:
        return None
    a = np.asarray(value)
    if a.ndim != 1 or a.dtype.kind not in "iu" or a.size == 0 or np.any(a < 0):
        raise ValueError("column_map must be a nonempty one-dimensional array of nonnegative integers")
    return a.tolist()


def from_signals(
    close,
    entries,
    exits,
    *,
    open=None,
    high=None,
    low=None,
    init_cash=None,
    size=None,
    size_type="amount",
    price=None,
    direction="longonly",
    fees=None,
    fixed_fees=None,
    slippage=None,
    min_size=None,
    max_size=None,
    size_granularity=None,
    leverage=None,
    leverage_mode="lazy",
    account_type=None,
    initial_margin=None,
    margin_basis="notional",
    cash_limit=None,
    reject_prob=None,
    seed=0,
    lock_cash=None,
    allow_partial=None,
    raise_reject=None,
    log=None,
    accumulate="disabled",
    sl_stop=None,
    sl_trail=None,
    tp_stop=None,
    stop_entry_price=None,
    stop_exit_price=None,
    upon_stop_exit=None,
    upon_stop_update=None,
    val_price=None,
    cash_deposits=None,
    cash_dividends=None,
    cash_earnings=None,
    ffill_val_price=True,
    update_value=False,
    ann_factor=365.0,
    risk_free=0.0,
    trades="exit",
    incl_open=False,
    threads=1,
    cash_sharing=False,
    group_lens=None,
    call_seq="default",
    column_map=None,
) -> Portfolio:
    """Runs a signal-driven backtest and returns the portfolio.

    Parameters
    ----------
    close, entries, exits
        ``(n_bars, n_cols)``, or 1-D for a single column. These set the shape.
    column_map
        Logical run column -> source market column. Close and OHLC stay (T, S);
        signals and trading parameters use (T, len(column_map)).
    open, high, low
        The rest of the bar. Only needed for stops, and **required** for them:
        a stop level is tested against the bar's range, so without ``high`` and
        ``low`` no stop can fire and the run would silently report a stopless
        strategy. Passing ``sl_stop`` or ``tp_stop`` without them is an error
        rather than a surprise.
    init_cash, size, fees, fixed_fees, slippage, min_size, max_size, size_granularity
        A number, one value per column, or a full matrix. ``None`` takes the
        engine's own default, which is the only place a default is written down.
    size_type
        ``amount``, ``value`` or ``percent``. Targets require ``from_orders``.
    direction
        ``longonly``, ``shortonly`` or ``both``.
    accumulate
        ``disabled``, ``both``, ``addonly`` or ``removeonly`` — whether a
        repeated signal adds to or trims an open position rather than being
        ignored. ``True``/``False`` also work, as the reference spells them.
    ann_factor
        Periods per year, for the annualized metrics. 365 for daily crypto,
        252 for daily equities, 365*24 for hourly.
    trades
        Which trade view the statistics are scored over: ``exit``, ``entry`` or
        ``positions``. They give different trade counts and different per-trade
        PnL from identical fills, and none of the three is wrong — a win rate
        over entry trades answers a different question than one over exits.
    log
        ``True`` to record why every order ended the way it did, or a
        ``(n_bars, n_cols)`` mask to log one column or one row. Off by default:
        a log row is 45 fields against an order record's 7, written for every
        order *considered* rather than every one filled. Read them back with
        ``pf.logs()``, ``pf.log_reasons()`` and ``pf.reason_counts(col)``.
    raise_reject
        Turn a rejected order into an exception instead of a recorded refusal.
        Useful under ``log=True`` to stop at the first one.
    sl_stop, sl_trail, tp_stop
        Stop-loss and take-profit distances as fractions of the stop's anchor
        price (NaN disables one), and whether the stop-loss trails the best
        price reached. Need ``high`` and ``low``.
    stop_entry_price
        The anchor a stop is measured from when its position opens or is
        re-armed: ``close`` (the default), ``fillprice`` (the fill, slippage
        included), ``price`` (the requested order price) or ``valprice``.
    stop_exit_price
        What a triggered stop fills at: ``stoplimit`` (the default — the stop
        level, no slippage), ``stopmarket`` (the stop level plus slippage),
        ``price`` (the ``price`` parameter) or ``close``.
    upon_stop_exit
        What a triggered stop does: ``close`` (the default), ``reverse`` (close
        and open the opposite side — opening it needs ``direction="both"``),
        or ``closereduce``/``reversereduce``, which leave ``accumulate`` in
        force instead of switching it off for that order.
    upon_stop_update
        When a position is added to, whether the new ``sl_stop``/``tp_stop``
        re-anchors the live stop at the new price: ``override`` (the default;
        a NaN keeps the old stop), ``overridenan`` (a NaN clears it) or
        ``keep``. Averaging down with the original stop and averaging down
        with a moved one are different strategies.
    cash_sharing, group_lens
        Share capital across adjacent columns. None groups all columns when
        sharing is enabled; explicit positive lengths must sum to n_cols.
        Shared init_cash is a scalar or one value per group.
    call_seq
        "default" uses column order; "auto" stably sorts approximate order
        values, selling before buying. Neither mode allocates equal weights.
    val_price, ffill_val_price, update_value
        Valuation price (default: execution price), whether missing valuation
        prices retain the previous value, and whether each fill updates the
        account value before the next order. These do not shift signals.
    threads
        Splits independent account groups; symbols sharing cash execute sequentially.
    cash_deposits
        Cash added (positive) or withdrawn (negative) at each bar's open,
        before orders are sized, so a target-sized order on the same bar
        invests it. A number, one value per bar (1-D), or a matrix: one column
        per account — ``(n_bars, n_cols)`` unshared, ``(n_bars, n_groups)``
        with ``cash_sharing``. A withdrawal is capped at the free cash.
        Deposits are taken out of returns: each bar's return is measured from
        the previous value plus that bar's deposit.
    cash_limit
        The most cash one buy may spend, fees included. Applied before
        ``percent`` takes its share. Sells and shorts are not affected.
    leverage, leverage_mode
        How many times its own cash a buy may be worth: 1 or more, as a
        number, one value per column or a matrix. ``lazy`` (the default)
        spends own cash first and borrows only what it cannot cover;
        ``eager`` borrows ``1 - 1/leverage`` of every buy. Fees are paid from
        own cash, and ``percent`` sizes against the levered buying power. A
        short ties up its proceeds plus ``value / leverage`` (twice its value
        at 1). A buy with no own cash left is refused as
        ``NoCashLeverage``. Under leverage ``pf.cash()`` is own cash less what
        the longs borrowed and can go negative, ``pf.value()`` stays the
        equity, and ``pf.orders()`` gains ``debt`` and ``locked_cash``.
    account_type, initial_margin, margin_basis
        ``cash`` (the default) holds what it buys; ``margin`` trades like a
        futures or perpetual contract on the account's cash. A string for
        every column or one per column, so a spot leg and a perpetual leg can
        share one account with ``cash_sharing``. Opening either side of a
        margin position posts ``initial_margin`` from free cash: a fraction of
        the notional under ``margin_basis="notional"`` (``None`` means
        ``1 / leverage``), or cash per unit under ``"unit"``. Every bar the
        position's profit since the last bar is settled into cash, so a loss
        shrinks what new orders may post and cash can go below zero. Closing
        realizes against the average entry and releases the margin.
        ``leverage_mode`` and ``lock_cash`` do not apply to margin columns.
        ``pf.value()`` is the equity, ``pf.free_cash()`` the equity less the
        margin posted, and ``pf.orders()`` gains ``debt`` (the cost basis),
        ``locked_cash`` (the margin) and ``margin``.
    reject_prob, seed
        Probability that an order is rejected at random, for testing how a
        strategy degrades when fills fail. Each draw depends only on ``seed``,
        the bar and the column, so a run is reproducible on any thread count.
    cash_dividends
        Dividend per share, shaped like ``close`` (``column_map`` applies). Paid
        at each bar's open on the position held before that bar's orders; a
        short position pays it. Part of the return.
    cash_earnings
        Cash credited (positive) or charged (negative) at each bar's close, per
        column: interest, fees, a funding payment computed in advance. A
        number, one value per bar (1-D), or ``(n_bars, n_cols)``. Part of the
        return. A charge is capped at the free cash. ``pf.cash_records()``
        lists what was applied.

    Returns
    -------
    Portfolio
        The run's fills, plus everything derivable from them on demand.
    """
    return _phasor.from_signals(
        _matrix(close, "close"),
        _signals(entries, "entries"),
        _signals(exits, "exits"),
        open=None if open is None else _matrix(open, "open"),
        high=None if high is None else _matrix(high, "high"),
        low=None if low is None else _matrix(low, "low"),
        init_cash=_param(init_cash, "init_cash"),
        size=_param(size, "size"),
        size_type=size_type,
        price=_param(price, "price"),
        direction=direction,
        fees=_param(fees, "fees"),
        fixed_fees=_param(fixed_fees, "fixed_fees"),
        slippage=_param(slippage, "slippage"),
        min_size=_param(min_size, "min_size"),
        max_size=_param(max_size, "max_size"),
        size_granularity=_param(size_granularity, "size_granularity"),
        leverage=_param(leverage, "leverage"),
        leverage_mode=leverage_mode,
        account_type=_account_type(account_type),
        initial_margin=_param(initial_margin, "initial_margin"),
        margin_basis=margin_basis,
        cash_limit=_param(cash_limit, "cash_limit"),
        reject_prob=_param(reject_prob, "reject_prob"),
        seed=seed,
        lock_cash=_flag(lock_cash, "lock_cash"),
        allow_partial=_flag(allow_partial, "allow_partial"),
        raise_reject=_flag(raise_reject, "raise_reject"),
        log=_flag(log, "log"),
        accumulate=_accumulate(accumulate),
        sl_stop=_param(sl_stop, "sl_stop"),
        sl_trail=_flag(sl_trail, "sl_trail"),
        tp_stop=_param(tp_stop, "tp_stop"),
        stop_entry_price=stop_entry_price,
        stop_exit_price=stop_exit_price,
        upon_stop_exit=upon_stop_exit,
        upon_stop_update=upon_stop_update,
        val_price=_param(val_price, "val_price"),
        cash_deposits=_param(cash_deposits, "cash_deposits"),
        cash_dividends=_market(cash_dividends, "cash_dividends"),
        cash_earnings=_param(cash_earnings, "cash_earnings"),
        ffill_val_price=ffill_val_price,
        update_value=update_value,
        ann_factor=ann_factor,
        risk_free=risk_free,
        trades=trades,
        incl_open=incl_open,
        threads=threads,
        cash_sharing=cash_sharing,
        group_lens=_group_lens(group_lens),
        call_seq=call_seq,
        column_map=_column_map(column_map),
    )


def from_orders(
    close,
    size=None,
    *,
    size_type="amount",
    price=None,
    direction="both",
    init_cash=None,
    fees=None,
    fixed_fees=None,
    slippage=None,
    min_size=None,
    max_size=None,
    size_granularity=None,
    leverage=None,
    leverage_mode="lazy",
    account_type=None,
    initial_margin=None,
    margin_basis="notional",
    cash_limit=None,
    reject_prob=None,
    seed=0,
    lock_cash=None,
    allow_partial=None,
    raise_reject=None,
    log=None,
    val_price=None,
    cash_deposits=None,
    cash_dividends=None,
    cash_earnings=None,
    ffill_val_price=True,
    update_value=False,
    ann_factor=365.0,
    risk_free=0.0,
    trades="exit",
    incl_open=False,
    threads=1,
    cash_sharing=False,
    group_lens=None,
    call_seq="default",
    column_map=None,
) -> Portfolio:
    """Runs a backtest from one order request per element.

    The other shape of a vectorized run. Where ``from_signals`` resolves
    entries and exits into sizes, here ``size`` *is* the request, element by
    element: a rebalance is a matrix of target weights, a schedule is a matrix
    that is zero except on the bars that trade.

    All six size types work here, unlike the signal path — nothing has to be
    reconciled with a direction, so a target is unambiguous.

    Parameters
    ----------
    column_map
        Optional logical-column to source-column indices, as in from_signals.
        Trading parameters use the logical width; market prices retain source width.
    close
        ``(n_bars, n_cols)``, or 1-D for a single column. Sets the shape, and
        resolves the ``inf`` sentinel in ``price`` and ``val_price``.
    size
        What to order on each bar. A number, one value per column, or a full
        matrix. Default ``inf`` — as much as the cash allows, which with the
        default price makes a buy-and-hold on the first bar.
    size_type
        ``amount``, ``value``, ``percent``, ``targetamount``, ``targetvalue``
        or ``targetpercent``. A **target** type says where the position should
        end up rather than how much to trade, which is what makes a rebalance
        one line.
    direction
        ``both`` here, matching the reference, rather than ``longonly`` as in
        ``from_signals``: an order's sign already says which way it goes.
    price
        ``inf`` (the default) is this bar's close, ``-inf`` the previous one.
    val_price
        What the position is marked at when a value- or percent-sized order is
        resolved. ``inf`` (the default) is the order price.
    ffill_val_price
        Carry the last usable valuation price forward, as the reference does.
    update_value
        Re-mark to the filled price after each fill. In shared accounts later orders in the same bar see the updated value.
    log
        As in ``from_signals``: ``True``, or a mask, to record why each order
        ended the way it did.
    cash_sharing, group_lens
        Share capital across adjacent columns. None groups all columns when
        sharing is enabled; explicit positive lengths must sum to n_cols.
        Shared init_cash is a scalar or one value per group.
    call_seq
        "default" uses column order; "auto" stably sorts approximate order
        values, selling before buying. Neither mode allocates equal weights.
    threads
        Splits account groups, which are independent.
    cash_deposits
        Cash added (positive) or withdrawn (negative) at each bar's open,
        before orders are sized, so a target-sized order on the same bar
        invests it. A number, one value per bar (1-D), or a matrix: one column
        per account — ``(n_bars, n_cols)`` unshared, ``(n_bars, n_groups)``
        with ``cash_sharing``. A withdrawal is capped at the free cash.
        Deposits are taken out of returns: each bar's return is measured from
        the previous value plus that bar's deposit.
    cash_limit
        The most cash one buy may spend, fees included. Applied before
        ``percent`` takes its share. Sells and shorts are not affected.
    leverage, leverage_mode
        How many times its own cash a buy may be worth: 1 or more, as a
        number, one value per column or a matrix. ``lazy`` (the default)
        spends own cash first and borrows only what it cannot cover;
        ``eager`` borrows ``1 - 1/leverage`` of every buy. Fees are paid from
        own cash, and ``percent`` sizes against the levered buying power. A
        short ties up its proceeds plus ``value / leverage`` (twice its value
        at 1). A buy with no own cash left is refused as
        ``NoCashLeverage``. Under leverage ``pf.cash()`` is own cash less what
        the longs borrowed and can go negative, ``pf.value()`` stays the
        equity, and ``pf.orders()`` gains ``debt`` and ``locked_cash``.
    account_type, initial_margin, margin_basis
        ``cash`` (the default) holds what it buys; ``margin`` trades like a
        futures or perpetual contract on the account's cash. A string for
        every column or one per column, so a spot leg and a perpetual leg can
        share one account with ``cash_sharing``. Opening either side of a
        margin position posts ``initial_margin`` from free cash: a fraction of
        the notional under ``margin_basis="notional"`` (``None`` means
        ``1 / leverage``), or cash per unit under ``"unit"``. Every bar the
        position's profit since the last bar is settled into cash, so a loss
        shrinks what new orders may post and cash can go below zero. Closing
        realizes against the average entry and releases the margin.
        ``leverage_mode`` and ``lock_cash`` do not apply to margin columns.
        ``pf.value()`` is the equity, ``pf.free_cash()`` the equity less the
        margin posted, and ``pf.orders()`` gains ``debt`` (the cost basis),
        ``locked_cash`` (the margin) and ``margin``.
    reject_prob, seed
        Probability that an order is rejected at random, for testing how a
        strategy degrades when fills fail. Each draw depends only on ``seed``,
        the bar and the column, so a run is reproducible on any thread count.
    cash_dividends
        Dividend per share, shaped like ``close`` (``column_map`` applies). Paid
        at each bar's open on the position held before that bar's orders; a
        short position pays it. Part of the return.
    cash_earnings
        Cash credited (positive) or charged (negative) at each bar's close, per
        column: interest, fees, a funding payment computed in advance. A
        number, one value per bar (1-D), or ``(n_bars, n_cols)``. Part of the
        return. A charge is capped at the free cash. ``pf.cash_records()``
        lists what was applied.

    Returns
    -------
    Portfolio
        The run's fills, plus everything derivable from them on demand.
    """
    return _phasor.from_orders(
        _matrix(close, "close"),
        size=_param(size, "size"),
        size_type=size_type,
        price=_param(price, "price"),
        direction=direction,
        init_cash=_param(init_cash, "init_cash"),
        fees=_param(fees, "fees"),
        fixed_fees=_param(fixed_fees, "fixed_fees"),
        slippage=_param(slippage, "slippage"),
        min_size=_param(min_size, "min_size"),
        max_size=_param(max_size, "max_size"),
        size_granularity=_param(size_granularity, "size_granularity"),
        leverage=_param(leverage, "leverage"),
        leverage_mode=leverage_mode,
        account_type=_account_type(account_type),
        initial_margin=_param(initial_margin, "initial_margin"),
        margin_basis=margin_basis,
        cash_limit=_param(cash_limit, "cash_limit"),
        reject_prob=_param(reject_prob, "reject_prob"),
        seed=seed,
        lock_cash=_flag(lock_cash, "lock_cash"),
        allow_partial=_flag(allow_partial, "allow_partial"),
        raise_reject=_flag(raise_reject, "raise_reject"),
        log=_flag(log, "log"),
        val_price=_param(val_price, "val_price"),
        cash_deposits=_param(cash_deposits, "cash_deposits"),
        cash_dividends=_market(cash_dividends, "cash_dividends"),
        cash_earnings=_param(cash_earnings, "cash_earnings"),
        ffill_val_price=ffill_val_price,
        update_value=update_value,
        ann_factor=ann_factor,
        risk_free=risk_free,
        trades=trades,
        incl_open=incl_open,
        threads=threads,
        cash_sharing=cash_sharing,
        group_lens=_group_lens(group_lens),
        call_seq=call_seq,
        column_map=_column_map(column_map),
    )


from .strategy import crossed_above, factors, strategy
