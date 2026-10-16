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

from . import _phasor, data, events, labels, rates, robust, signals, synthetic, validation
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
    "rates",
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


def _margin_mode(value):
    if value is None or isinstance(value, str):
        return value
    try:
        names = list(value)
    except TypeError:
        names = None
    if names is None or not all(isinstance(name, str) for name in names):
        raise TypeError("margin_mode must be 'cross', 'isolated', or one of them per column")
    return names


def _column_map(value):
    if value is None:
        return None
    a = np.asarray(value)
    if a.ndim != 1 or a.dtype.kind not in "iu" or a.size == 0 or np.any(a < 0):
        raise ValueError("column_map must be a nonempty one-dimensional array of nonnegative integers")
    return a.tolist()


def _multiplier(value: Any, column_map: Any) -> Any:
    """A contract multiplier: a number, or one value per column of ``close``.

    With a ``column_map`` the values are per source column and are mapped
    here, as the engine wants one per simulated column.
    """
    if value is None or isinstance(value, (bool, int, float)):
        return value
    array = np.asarray(value, dtype=np.float64)
    if array.ndim == 0:
        return float(array)
    if array.ndim != 1:
        raise ValueError("multiplier must be a number or one value per column")
    if column_map is not None:
        array = array[np.asarray(column_map, dtype=np.intp)]
    return np.ascontiguousarray(array)


_PRICE_OPTIONS = {
    "open": ("open", 0),
    "close": ("close", 0),
    "nextopen": ("open", 1),
    "nextclose": ("close", 1),
}


def _price_option(price: Any, from_ago: int) -> tuple[Any, Any, int]:
    """``price`` as the engine takes it: an array or number, or one of the
    bar's prices by name. ``nextopen`` and ``nextclose`` are the next bar's,
    which ``from_ago=1`` reads the order from the bar before."""
    if not isinstance(price, str):
        return _param(price, "price"), None, from_ago
    if price not in _PRICE_OPTIONS:
        raise ValueError(
            "price must be a number, an array, or 'open', 'close', 'nextopen' or 'nextclose'; "
            f"got {price!r}"
        )
    price_type, ago = _PRICE_OPTIONS[price]
    if ago and from_ago:
        raise ValueError(f"price={price!r} sets from_ago=1; do not pass from_ago with it")
    return None, price_type, from_ago or ago


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
    multiplier=None,
    from_ago=0,
    bid_price=None,
    ask_price=None,
    price_area_vio_mode=None,
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
    order_type=None,
    limit_delta=None,
    delta_format=None,
    limit_reverse=None,
    limit_order_price=None,
    limit_delay=None,
    limit_tif=None,
    limit_expiry=None,
    upon_adj_limit_conflict=None,
    upon_opp_limit_conflict=None,
    upon_adj_stop_conflict=None,
    upon_opp_stop_conflict=None,
    tsl_stop=None,
    tsl_th=None,
    td_stop=None,
    dt_stop=None,
    stop_ladder=None,
    stop_order_type=None,
    stop_limit_delta=None,
    val_price=None,
    cash_deposits=None,
    cash_dividends=None,
    cash_earnings=None,
    funding_rate=None,
    long_financing_rate=None,
    short_financing_rate=None,
    debt_rate=None,
    borrow_rate=None,
    mark_price=None,
    margin_mode=None,
    maintenance_margin=None,
    liquidation_fee=None,
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
        The rest of the bar. Needed for stops and for an intrabar liquidation
        test, and **required** for stops:
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
        a log row is 46 fields against an order record's 7, written for every
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
    order_type
        ``"market"`` (the default) executes a signal on its bar; ``"limit"``
        rests it as a limit order until the market reaches it. A string, or
        one per column. Stop exits are always market orders. A limit order
        pays no slippage; it fills at once if the order price already reaches
        it, and is otherwise tested from the next bar (from the rest of its
        own bar when placed at the open, ``price=-np.inf``). While it rests,
        a signal on the same side is ignored and one on the other side
        cancels it, as does a stop or a liquidation filling in its column.
    limit_delta, delta_format, limit_reverse
        Where it rests: ``limit_delta`` below the order price for a buy and
        above for a sell, as a fraction (``delta_format="percent"``, the
        default), an amount of price (``"absolute"``) or the limit price
        itself (``"target"``); NaN is at the order price. ``limit_reverse``
        turns both round, so a buy fills at or above its price: a breakout
        entry.
    limit_order_price
        What it fills at once reached: ``"limit"`` (the default; the open
        instead when a bar opens beyond the limit), ``"hardlimit"`` (the
        limit even then) or ``"close"``.
    limit_delay, limit_tif, limit_expiry
        ``limit_delay`` tests it from the next bar even when the price already
        reaches it. ``limit_tif`` is how many bars after the one it was placed
        on it may fill, and ``limit_expiry`` the row it is cancelled at; NaN
        is no limit. Both count bars: the engine has no timestamps.
    upon_adj_limit_conflict, upon_opp_limit_conflict
        What a new signal does to a resting limit order on the same side
        (``adj``) or the other (``opp``): ``"keepignore"`` (the ``adj``
        default) keeps it and ignores the signal, ``"cancelexecute"`` (the
        ``opp`` default) cancels it and executes the signal, and
        ``"keepexecute"`` and ``"cancelignore"`` the other two. A kept order
        keeps the size it was placed with; a new limit order takes its place.
    tsl_stop, tsl_th
        A trailing stop ``tsl_stop`` below the best price since the entry (above
        it for a short), live once the price has first moved ``tsl_th`` in the
        position's favour (NaN or 0: at once).
    td_stop, dt_stop
        Time stops: exit at the open ``td_stop`` bars after the entry bar, or
        at the open of row ``dt_stop``. Rows, not times.
    stop_ladder
        ``"uniform"`` or ``"weighted"`` makes ``sl_stop`` and ``tp_stop``
        ladders: a 1-D array of increasing steps for every column, or
        ``(n_steps, n_cols)``, NaN after the last. Each step closes an equal
        share of the position (``uniform``) or a share proportional to its
        distance from the step before (``weighted``); the last closes the rest.
    stop_order_type, stop_limit_delta
        ``"limit"`` turns a fired price stop into a limit order at its level,
        ``stop_limit_delta`` (in ``delta_format``) worse: a stop-limit order,
        which a gap past the limit leaves resting. A string or one per column.
    from_ago, bid_price, ask_price, price_area_vio_mode
        ``price`` may also name one of the bar's prices: ``"open"``,
        ``"close"``, or ``"nextopen"``/``"nextclose"``, the next bar's, which
        reads the order decided on the bar before (``from_ago=1``): a
        decision at one close executed at the following open uses nothing it
        could not have known. ``from_ago=n`` reads the order (its signals or
        size, size type and an explicit price) ``n`` bars back; a named price
        is still the executing bar's. ``bid_price`` and ``ask_price``, shaped
        like ``close``, are what a sell and a buy execute at where given.
        ``price_area_vio_mode`` decides what an order price outside the bar's
        low–high range does: ``"ignore"`` (the default), ``"cap"`` it to the
        range, or ``"error"``.
    upon_adj_stop_conflict, upon_opp_stop_conflict
        The same choice between a stop that fires and the bar's own signal.
        The default ``"keepignore"`` lets the stop replace the signal.
        ``"keepexecute"`` fills the stop, then the signal re-sized against
        what the stop left, on the same bar (a stop-out and a reversal);
        ``"cancelexecute"`` disarms the stop and executes the signal instead.
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
    multiplier
        What one unit of position is worth per unit of price: 50 for an
        E-mini S&P 500 future, 1 (the default) for a share. A number, or one
        value per column (per source column with ``column_map``); it cannot
        change within a column. A fill moves ``size * price * multiplier`` of
        cash, and fees, margin, dividends, holding charges and the
        liquidation test are all on that notional. Sizes, ``min_size``,
        ``size_granularity`` and a ``"unit"`` margin stay in contracts, and
        prices, stops and ``pf.orders()["price"]`` stay prices. A run with a
        multiplier other than 1 adds ``multiplier`` to ``pf.orders()``, as a
        levered run does; ``pf.trades()`` always has it, with ``pnl``
        multiplied and ``ret`` not.
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
    funding_rate
        A perpetual's funding rate per bar, shaped like ``close``
        (``column_map`` applies), ``NaN`` or 0 on bars that do not settle. At
        the bar's close, after its orders, a position pays ``position * price
        * rate``: a positive rate is paid by longs and received by shorts. Put
        each rate on the bar whose close is its funding time.
    long_financing_rate, short_financing_rate
        Overnight financing per bar for each side, shaped like ``close``: a
        position pays ``abs(position) * price * rate`` at the bar's close. A
        positive rate is a cost to either side, a negative one is received.
    debt_rate
        Interest per bar on what a cash column's long borrowed under
        ``leverage``, charged at the bar's close. A number, one value per bar
        (1-D) or ``(n_bars, n_cols)``.
    borrow_rate
        The fee per bar for the shares a cash column's short borrowed, shaped
        like ``close``: ``abs(position) * price * rate`` at the bar's close.
    mark_price
        The ``price`` of the charges above, shaped like ``close``. ``None``
        uses the close.

        Every rate is per bar, for the time until the next bar;
        ``phasor.rates.per_bar`` turns an annual rate into one from the bars'
        timestamps. A charge is owed whatever the balance, so unlike a
        ``cash_earnings`` charge it is not capped: any of these runs the
        accounts as ones that may borrow, cash can go below zero, after which
        only closing is allowed, and ``pf.orders()`` gains ``debt``,
        ``locked_cash`` and ``margin``. Margin columns pay funding and
        financing but no interest or borrow fee. ``pf.cash_flows("funding")``,
        ``"financing"``, ``"interest"`` and ``"borrowfee"`` give what each
        cost per bar and column; all are part of the return.
    margin_mode, maintenance_margin, liquidation_fee
        Liquidation of margin columns. ``maintenance_margin`` (in
        ``margin_basis``: a fraction of the notional below 1, or cash per unit;
        ``None`` or 0 carries none and never liquidates) is tested on every bar
        before its orders, along a path from the bar's open to its adverse
        extreme — the low for a long, the high for a short — or to
        ``mark_price`` where one is given. An account whose equity falls
        below the maintenance margin is closed where its path first crosses
        it, or at the open when it gaps through, paying ``liquidation_fee`` of
        the notional instead of its fees. ``margin_mode`` is ``"cross"`` (the
        default: the account's equity carries every cross position and they
        close together; cash columns count as collateral and are never closed)
        or ``"isolated"`` (the position is not settled into the account's
        cash, is tested on its own margin alone and can lose no more than it
        posted), a string for every column or one per column. A loss a flat
        cross account or an isolated position cannot pay is written off, as an
        exchange's insurance fund takes it: ``pf.cash_flows("liquidation")``.
        Fills that were liquidations are flagged in ``pf.orders()`` and
        ``pf.logs()`` (``liquidation``). Without ``high`` and ``low`` the test
        uses the close. Each leg of a hedge reaches its own extreme, so a
        hedged account wants a mark price, if only the close.

    Returns
    -------
    Portfolio
        The run's fills, plus everything derivable from them on demand.
    """
    price, price_type, from_ago = _price_option(price, from_ago)
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
        price=price,
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
        multiplier=_multiplier(multiplier, column_map),
        price_type=price_type,
        from_ago=from_ago,
        bid_price=_market(bid_price, "bid_price"),
        ask_price=_market(ask_price, "ask_price"),
        price_area_vio_mode=price_area_vio_mode,
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
        order_type=order_type,
        limit_delta=_param(limit_delta, "limit_delta"),
        delta_format=delta_format,
        limit_reverse=_flag(limit_reverse, "limit_reverse"),
        limit_order_price=limit_order_price,
        limit_delay=_flag(limit_delay, "limit_delay"),
        limit_tif=_param(limit_tif, "limit_tif"),
        limit_expiry=_param(limit_expiry, "limit_expiry"),
        upon_adj_limit_conflict=upon_adj_limit_conflict,
        upon_opp_limit_conflict=upon_opp_limit_conflict,
        upon_adj_stop_conflict=upon_adj_stop_conflict,
        upon_opp_stop_conflict=upon_opp_stop_conflict,
        tsl_stop=_param(tsl_stop, "tsl_stop"),
        tsl_th=_param(tsl_th, "tsl_th"),
        td_stop=_param(td_stop, "td_stop"),
        dt_stop=_param(dt_stop, "dt_stop"),
        stop_ladder=stop_ladder,
        stop_order_type=stop_order_type,
        stop_limit_delta=_param(stop_limit_delta, "stop_limit_delta"),
        val_price=_param(val_price, "val_price"),
        cash_deposits=_param(cash_deposits, "cash_deposits"),
        cash_dividends=_market(cash_dividends, "cash_dividends"),
        cash_earnings=_param(cash_earnings, "cash_earnings"),
        funding_rate=_market(funding_rate, "funding_rate"),
        long_financing_rate=_market(long_financing_rate, "long_financing_rate"),
        short_financing_rate=_market(short_financing_rate, "short_financing_rate"),
        debt_rate=_param(debt_rate, "debt_rate"),
        borrow_rate=_market(borrow_rate, "borrow_rate"),
        mark_price=_market(mark_price, "mark_price"),
        margin_mode=_margin_mode(margin_mode),
        maintenance_margin=_param(maintenance_margin, "maintenance_margin"),
        liquidation_fee=_param(liquidation_fee, "liquidation_fee"),
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
    open=None,
    high=None,
    low=None,
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
    multiplier=None,
    from_ago=0,
    bid_price=None,
    ask_price=None,
    price_area_vio_mode=None,
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
    funding_rate=None,
    long_financing_rate=None,
    short_financing_rate=None,
    debt_rate=None,
    borrow_rate=None,
    mark_price=None,
    margin_mode=None,
    maintenance_margin=None,
    liquidation_fee=None,
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
    open, high, low
        The rest of the bar, shaped like ``close``. Only read by the
        liquidation test, which follows a path from the open to the bar's
        adverse extreme; without them it tests the close.
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
    from_ago, bid_price, ask_price, price_area_vio_mode
        ``price`` may also name one of the bar's prices: ``"open"``,
        ``"close"``, or ``"nextopen"``/``"nextclose"``, the next bar's, which
        reads the order decided on the bar before (``from_ago=1``): a
        decision at one close executed at the following open uses nothing it
        could not have known. ``from_ago=n`` reads the order (its signals or
        size, size type and an explicit price) ``n`` bars back; a named price
        is still the executing bar's. ``bid_price`` and ``ask_price``, shaped
        like ``close``, are what a sell and a buy execute at where given.
        ``price_area_vio_mode`` decides what an order price outside the bar's
        low–high range does: ``"ignore"`` (the default), ``"cap"`` it to the
        range, or ``"error"``.
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
    multiplier
        What one unit of position is worth per unit of price: 50 for an
        E-mini S&P 500 future, 1 (the default) for a share. A number, or one
        value per column (per source column with ``column_map``); it cannot
        change within a column. A fill moves ``size * price * multiplier`` of
        cash, and fees, margin, dividends, holding charges and the
        liquidation test are all on that notional. Sizes, ``min_size``,
        ``size_granularity`` and a ``"unit"`` margin stay in contracts, and
        prices, stops and ``pf.orders()["price"]`` stay prices. A run with a
        multiplier other than 1 adds ``multiplier`` to ``pf.orders()``, as a
        levered run does; ``pf.trades()`` always has it, with ``pnl``
        multiplied and ``ret`` not.
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
    funding_rate
        A perpetual's funding rate per bar, shaped like ``close``
        (``column_map`` applies), ``NaN`` or 0 on bars that do not settle. At
        the bar's close, after its orders, a position pays ``position * price
        * rate``: a positive rate is paid by longs and received by shorts. Put
        each rate on the bar whose close is its funding time.
    long_financing_rate, short_financing_rate
        Overnight financing per bar for each side, shaped like ``close``: a
        position pays ``abs(position) * price * rate`` at the bar's close. A
        positive rate is a cost to either side, a negative one is received.
    debt_rate
        Interest per bar on what a cash column's long borrowed under
        ``leverage``, charged at the bar's close. A number, one value per bar
        (1-D) or ``(n_bars, n_cols)``.
    borrow_rate
        The fee per bar for the shares a cash column's short borrowed, shaped
        like ``close``: ``abs(position) * price * rate`` at the bar's close.
    mark_price
        The ``price`` of the charges above, shaped like ``close``. ``None``
        uses the close.

        Every rate is per bar, for the time until the next bar;
        ``phasor.rates.per_bar`` turns an annual rate into one from the bars'
        timestamps. A charge is owed whatever the balance, so unlike a
        ``cash_earnings`` charge it is not capped: any of these runs the
        accounts as ones that may borrow, cash can go below zero, after which
        only closing is allowed, and ``pf.orders()`` gains ``debt``,
        ``locked_cash`` and ``margin``. Margin columns pay funding and
        financing but no interest or borrow fee. ``pf.cash_flows("funding")``,
        ``"financing"``, ``"interest"`` and ``"borrowfee"`` give what each
        cost per bar and column; all are part of the return.
    margin_mode, maintenance_margin, liquidation_fee
        Liquidation of margin columns. ``maintenance_margin`` (in
        ``margin_basis``: a fraction of the notional below 1, or cash per unit;
        ``None`` or 0 carries none and never liquidates) is tested on every bar
        before its orders, along a path from the bar's open to its adverse
        extreme — the low for a long, the high for a short — or to
        ``mark_price`` where one is given. An account whose equity falls
        below the maintenance margin is closed where its path first crosses
        it, or at the open when it gaps through, paying ``liquidation_fee`` of
        the notional instead of its fees. ``margin_mode`` is ``"cross"`` (the
        default: the account's equity carries every cross position and they
        close together; cash columns count as collateral and are never closed)
        or ``"isolated"`` (the position is not settled into the account's
        cash, is tested on its own margin alone and can lose no more than it
        posted), a string for every column or one per column. A loss a flat
        cross account or an isolated position cannot pay is written off, as an
        exchange's insurance fund takes it: ``pf.cash_flows("liquidation")``.
        Fills that were liquidations are flagged in ``pf.orders()`` and
        ``pf.logs()`` (``liquidation``). Without ``high`` and ``low`` the test
        uses the close. Each leg of a hedge reaches its own extreme, so a
        hedged account wants a mark price, if only the close.

    Returns
    -------
    Portfolio
        The run's fills, plus everything derivable from them on demand.
    """
    price, price_type, from_ago = _price_option(price, from_ago)
    return _phasor.from_orders(
        _matrix(close, "close"),
        size=_param(size, "size"),
        open=None if open is None else _matrix(open, "open"),
        high=None if high is None else _matrix(high, "high"),
        low=None if low is None else _matrix(low, "low"),
        size_type=size_type,
        price=price,
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
        multiplier=_multiplier(multiplier, column_map),
        price_type=price_type,
        from_ago=from_ago,
        bid_price=_market(bid_price, "bid_price"),
        ask_price=_market(ask_price, "ask_price"),
        price_area_vio_mode=price_area_vio_mode,
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
        funding_rate=_market(funding_rate, "funding_rate"),
        long_financing_rate=_market(long_financing_rate, "long_financing_rate"),
        short_financing_rate=_market(short_financing_rate, "short_financing_rate"),
        debt_rate=_param(debt_rate, "debt_rate"),
        borrow_rate=_market(borrow_rate, "borrow_rate"),
        mark_price=_market(mark_price, "mark_price"),
        margin_mode=_margin_mode(margin_mode),
        maintenance_margin=_param(maintenance_margin, "maintenance_margin"),
        liquidation_fee=_param(liquidation_fee, "liquidation_fee"),
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
