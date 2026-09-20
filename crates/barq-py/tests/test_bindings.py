"""Tests for the Python bindings.

Two kinds, and the split matters:

**Differential** — the same call made in the reference and here, compared field
by field. These are the tests that say the binding is correct, and they skip
when ``vectorbt`` is not installed, which is most environments.

**Structural** — layout, shapes, dtypes, error messages, thread determinism.
These need no reference and always run. They are not a substitute for the
differential tests: they say the binding is *consistent*, not that it agrees
with anything.

Run from the repository root::

    python -m pip install -e 'crates/barq-py[test]'
    pytest crates/barq-py/tests
"""

from __future__ import annotations

import numpy as np
import pytest

import barq

try:
    import vectorbt as _reference
except ImportError:  # pragma: no cover - depends on the environment
    _reference = None

needs_reference = pytest.mark.skipif(
    _reference is None, reason="vectorbt is not installed in this environment"
)

SEED = 20260919


def bars(n_rows: int, n_cols: int, seed: int = SEED):
    """A bar series with a consistent high >= close >= low."""
    rng = np.random.default_rng(seed)
    close = np.abs(rng.normal(100.0, 6.0, (n_rows, n_cols))) + 5.0
    high = close * (1.0 + np.abs(rng.normal(0.0, 0.01, close.shape)))
    low = close * (1.0 - np.abs(rng.normal(0.0, 0.01, close.shape)))
    opn = low + (high - low) * rng.random(close.shape)
    entries = rng.random(close.shape) < 0.15
    exits = rng.random(close.shape) < 0.15
    return opn, high, low, close, entries, exits


# --------------------------------------------------------------------------
# Differential: the same call, both engines
# --------------------------------------------------------------------------

CASES = [
    ("plain", {"init_cash": 100.0}),
    ("fees and slippage", {"init_cash": 1000.0, "fees": 0.001, "slippage": 0.0005}),
    ("fixed size", {"init_cash": 1000.0, "size": 2.0}),
    ("percent", {"init_cash": 1000.0, "size": 0.5, "size_type": "percent"}),
    ("value", {"init_cash": 1000.0, "size": 300.0, "size_type": "value"}),
    ("short only", {"init_cash": 1000.0, "direction": "shortonly"}),
    ("both directions", {"init_cash": 1000.0, "direction": "both"}),
    ("granularity", {"init_cash": 1000.0, "size_granularity": 1.0}),
    ("accumulate", {"init_cash": 1000.0, "size": 1.0, "accumulate": "both"}),
    ("fixed fees", {"init_cash": 1000.0, "size": 1.0, "fixed_fees": 1.0}),
    ("min size", {"init_cash": 1000.0, "size": 1.0, "min_size": 0.5}),
    ("max size", {"init_cash": 1000.0, "size": 5.0, "max_size": 2.0}),
    ("lock cash", {"init_cash": 1000.0, "direction": "both", "lock_cash": True}),
]

STOP_CASES = [
    ("stop loss", {"sl_stop": 0.02}),
    ("take profit", {"tp_stop": 0.02}),
    ("trailing stop", {"sl_stop": 0.02, "sl_trail": True}),
    ("both stops", {"sl_stop": 0.03, "tp_stop": 0.02}),
]


def assert_same_fills(reference_records, mine):
    """Integer fields exactly, float fields to 1e-12.

    Records rather than metrics, because an aggregate hides a sign error and an
    off-by-one in the record it came from.
    """
    assert len(reference_records) == len(mine["col"]), "different number of fills"
    for field in ("col", "idx", "side"):
        np.testing.assert_array_equal(reference_records[field], mine[field])
    for field in ("size", "price", "fees"):
        np.testing.assert_allclose(
            reference_records[field], mine[field], rtol=1e-12, atol=1e-12
        )


@needs_reference
@pytest.mark.parametrize("name,kwargs", CASES, ids=[c[0] for c in CASES])
def test_fills_match_the_reference(name, kwargs):
    _, _, _, close, entries, exits = bars(120, 3)
    reference = _reference.Portfolio.from_signals(
        close, entries, exits, freq="1D", engine="numba", **kwargs
    )
    mine = barq.from_signals(close, entries, exits, **kwargs)
    assert_same_fills(reference.order_records, mine.orders())


@needs_reference
@pytest.mark.parametrize("name,kwargs", STOP_CASES, ids=[c[0] for c in STOP_CASES])
def test_stops_match_the_reference(name, kwargs):
    opn, high, low, close, entries, exits = bars(150, 3, seed=770519)
    reference = _reference.Portfolio.from_signals(
        close, entries, exits, open=opn, high=high, low=low,
        init_cash=1000.0, fees=0.001, freq="1D", engine="numba", **kwargs,
    )
    mine = barq.from_signals(
        close, entries, exits, open=opn, high=high, low=low,
        init_cash=1000.0, fees=0.001, **kwargs,
    )
    assert_same_fills(reference.order_records, mine.orders())


@needs_reference
def test_every_stats_row_matches_the_reference():
    _, _, _, close, entries, exits = bars(250, 1, seed=99)
    reference = _reference.Portfolio.from_signals(
        close, entries, exits, init_cash=1000.0, fees=0.001, freq="1D", engine="numba"
    )
    mine = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.001)
    theirs, ours = reference.stats(), mine.stats()

    assert len(ours) == len(theirs)
    compared = 0
    for title, their_value in theirs.items():
        assert title in ours, f"{title} is missing here"
        our_value = ours[title]
        assert our_value is not None, f"{title} came back as Missing"
        # Durations come back as bar counts here and as Timedeltas there.
        if hasattr(their_value, "total_seconds"):
            their_value = their_value.total_seconds() / 86400.0
        their_value, our_value = float(their_value), float(our_value)
        if np.isnan(their_value):
            assert np.isnan(our_value), f"{title}: NaN there, {our_value} here"
        else:
            np.testing.assert_allclose(
                our_value, their_value, rtol=1e-9, atol=1e-9, err_msg=title
            )
        compared += 1
    # Guards against the loop silently comparing nothing.
    assert compared == 28


# --------------------------------------------------------------------------
# Structural: no reference needed
# --------------------------------------------------------------------------

@pytest.mark.parametrize(
    "size_type", ["targetamount", "targetvalue", "targetpercent"]
)
def test_a_target_size_is_refused_in_the_signal_path(size_type):
    """Only three of the six size types mean anything here.

    A target size says where the position should end up; a signal says which
    way to move. Combining them leaves the size ambiguous on a reversal, so the
    signal pipeline takes Amount, Value and Percent only — and, deliberately,
    the group value it would need for TargetPercent is not computed correctly
    in this loop, because nothing can reach that branch. Accepting one here
    would quietly reach it.
    """
    _, _, _, close, entries, exits = bars(60, 1)
    with pytest.raises(ValueError, match="Amount, Value and Percent"):
        barq.from_signals(close, entries, exits, size=1.0, size_type=size_type)


@needs_reference
@pytest.mark.parametrize(
    "size_type", ["targetamount", "targetvalue", "targetpercent"]
)
def test_the_reference_refuses_the_same_three(size_type):
    """Pinned, so that a reference release lifting the restriction shows up
    here as a failure rather than as a silently diverging engine."""
    _, _, _, close, entries, exits = bars(60, 1)
    reference_type = {
        "targetamount": _reference.portfolio.enums.SizeType.TargetAmount,
        "targetvalue": _reference.portfolio.enums.SizeType.TargetValue,
        "targetpercent": _reference.portfolio.enums.SizeType.TargetPercent,
    }[size_type]
    with pytest.raises(ValueError, match="are supported"):
        _reference.Portfolio.from_signals(
            close, entries, exits, size=1.0, size_type=reference_type,
            freq="1D", engine="numba",
        )




def test_threads_do_not_change_a_single_fill():
    """The reason columns are the parallel axis: they do not interact."""
    _, _, _, close, entries, exits = bars(300, 40, seed=4)
    one = barq.from_signals(close, entries, exits, init_cash=1000.0, threads=1).orders()
    many = barq.from_signals(close, entries, exits, init_cash=1000.0, threads=8).orders()
    assert len(one["col"]) > 100, "too few fills for this to prove anything"
    for field in one:
        np.testing.assert_array_equal(one[field], many[field], err_msg=field)


def test_a_c_ordered_array_is_refused_rather_than_transposed():
    """The trap the whole array module exists for.

    A square matrix is used deliberately: the transposed reading has the same
    shape, so nothing downstream could notice.
    """
    _, _, _, close, entries, exits = bars(40, 40)
    with pytest.raises(ValueError, match="C-ordered"):
        barq._barq.from_signals(
            np.ascontiguousarray(close),
            np.asfortranarray(entries),
            np.asfortranarray(exits),
        )


def test_the_wrapper_accepts_a_c_ordered_array_by_converting_it():
    _, _, _, close, entries, exits = bars(40, 4)
    c_order = np.ascontiguousarray(close)
    assert not c_order.flags["F_CONTIGUOUS"]
    a = barq.from_signals(c_order, entries, exits, init_cash=1000.0).orders()
    b = barq.from_signals(np.asfortranarray(close), entries, exits, init_cash=1000.0).orders()
    for field in a:
        np.testing.assert_array_equal(a[field], b[field], err_msg=field)


def test_a_non_contiguous_slice_is_refused():
    _, _, _, close, entries, exits = bars(40, 8)
    view = np.asfortranarray(close)[:, ::2]
    assert not view.flags["F_CONTIGUOUS"]
    with pytest.raises(ValueError):
        barq._barq.from_signals(
            view, np.asfortranarray(entries[:, ::2]), np.asfortranarray(exits[:, ::2])
        )


def test_a_stop_without_a_bar_range_is_refused():
    """A stop with no high and low can never fire, and would look like a run
    where the strategy simply never stopped out."""
    _, _, _, close, entries, exits = bars(60, 1)
    with pytest.raises(ValueError, match="high and low"):
        barq.from_signals(close, entries, exits, sl_stop=0.02)
    with pytest.raises(ValueError, match="high and low"):
        barq.from_signals(close, entries, exits, tp_stop=0.02)


def test_a_one_dimensional_series_is_one_column():
    _, _, _, close, entries, exits = bars(60, 1)
    flat = barq.from_signals(close.ravel(), entries.ravel(), exits.ravel(), init_cash=100.0)
    shaped = barq.from_signals(close, entries, exits, init_cash=100.0)
    assert flat.n_cols == shaped.n_cols == 1
    assert flat.n_rows == shaped.n_rows == 60
    np.testing.assert_array_equal(flat.orders()["idx"], shaped.orders()["idx"])


def test_a_mismatched_shape_names_both_shapes():
    _, _, _, close, entries, exits = bars(60, 4)
    with pytest.raises(ValueError, match="60x2 array against a 60x4 run"):
        barq.from_signals(close, entries[:, :2], exits)


@pytest.mark.parametrize(
    "kwargs,message",
    [
        ({"direction": "sideways"}, "longonly"),
        ({"size_type": "Amount"}, "targetpercent"),
        ({"accumulate": "add"}, "addonly"),
        ({"trades": "exits"}, "'exit'"),
    ],
)
def test_an_unknown_enum_name_lists_the_alternatives(kwargs, message):
    _, _, _, close, entries, exits = bars(20, 1)
    with pytest.raises(ValueError, match=message):
        barq.from_signals(close, entries, exits, **kwargs)


def test_parameters_broadcast_from_a_scalar_and_from_a_column_vector():
    """One value per column must mean the same as that value everywhere."""
    _, _, _, close, entries, exits = bars(80, 3)
    scalar = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.002)
    per_col = barq.from_signals(
        close, entries, exits, init_cash=1000.0, fees=np.full(3, 0.002)
    )
    for field in scalar.orders():
        np.testing.assert_allclose(
            scalar.orders()[field], per_col.orders()[field], err_msg=field
        )


def test_per_column_parameters_actually_differ_per_column():
    """The previous test would pass if the vector were ignored entirely."""
    _, _, _, close, entries, exits = bars(80, 3)
    mixed = barq.from_signals(
        close, entries, exits, init_cash=1000.0, fees=np.array([0.0, 0.01, 0.05])
    )
    orders = mixed.orders()
    by_col = [orders["fees"][orders["col"] == c].sum() for c in range(3)]
    assert by_col[0] == 0.0
    assert by_col[1] < by_col[2], "a higher fee rate produced no more fees"


def test_untraded_columns_are_masked_rather_than_ranked_first():
    """An untraded column's Sharpe is inf, which tops a naive ranking."""
    _, _, _, close, entries, exits = bars(80, 3)
    quiet = entries.copy()
    quiet[:, 1] = False  # column 1 never enters
    pf = barq.from_signals(close, quiet, exits, init_cash=1000.0)
    sharpe = pf.sharpe_ratio()
    assert not np.isfinite(sharpe[1]), "an untraded column should not score finitely"
    masked = pf.traded_only(sharpe)
    assert np.isnan(masked[1])
    assert int(np.nanargmax(masked)) != 1


def test_records_come_back_as_arrays_with_the_right_dtypes():
    _, _, _, close, entries, exits = bars(120, 2)
    pf = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.001)
    orders, trades = pf.orders(), pf.trades()
    assert pf.n_fills == len(orders["col"])
    for field in ("col", "idx", "side"):
        assert orders[field].dtype == np.int64, field
    for field in ("size", "price", "fees"):
        assert orders[field].dtype == np.float64, field
    for field in ("col", "entry_idx", "exit_idx", "direction", "status", "parent_id"):
        assert trades[field].dtype == np.int64, field
    # Every trade belongs to a fill, so there cannot be more of them.
    assert len(trades["col"]) <= len(orders["col"])


def test_series_are_one_column_and_bounds_are_checked():
    _, _, _, close, entries, exits = bars(120, 3)
    pf = barq.from_signals(close, entries, exits, init_cash=1000.0)
    assert pf.value(0).shape == (120,)
    assert pf.returns(2).shape == (120,)
    with pytest.raises(IndexError, match="outside 0..3"):
        pf.value(3)


def test_the_metrics_bundle_agrees_with_the_individual_accessors():
    """The bundle exists to save rebuilds, not to compute something else."""
    _, _, _, close, entries, exits = bars(150, 4)
    pf = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.001)
    bundle = pf.metrics()
    np.testing.assert_array_equal(bundle["total_return"], pf.total_return())
    np.testing.assert_array_equal(bundle["sharpe_ratio"], pf.sharpe_ratio())
    np.testing.assert_array_equal(bundle["max_drawdown"], pf.max_drawdown())
    np.testing.assert_array_equal(bundle["trade_count"], pf.trade_count())


def test_the_trade_view_changes_what_the_statistics_score():
    """Three views of the same fills, and they are not interchangeable."""
    _, _, _, close, entries, exits = bars(200, 1, seed=7)
    counts = {}
    for view in ("exit", "entry", "positions"):
        pf = barq.from_signals(
            close, entries, exits, init_cash=1000.0, accumulate="both", size=1.0,
            trades=view,
        )
        counts[view] = int(pf.trade_count()[0])
    assert len(set(counts.values())) > 1, f"all three views agreed: {counts}"


def test_the_engine_does_not_write_through_a_borrowed_array():
    """Inputs are borrowed, not copied, so a stray write would reach into the
    caller's own array. The borrow is readonly on the Rust side; this checks
    the guarantee from the outside, where it is actually observable."""
    _, _, _, close, entries, exits = bars(60, 2)
    close = np.asfortranarray(close)
    before = close.copy()
    barq.from_signals(close, entries, exits, init_cash=1000.0)
    # Not proof of borrowing on its own, but it does prove the engine does not
    # write through the borrow, which is the risk a readonly borrow takes.
    np.testing.assert_array_equal(close, before)


# --------------------------------------------------------------------------
# from_orders, and the log records
# --------------------------------------------------------------------------

LOG_INT_FIELDS = (
    "group", "col", "idx", "req_size_type", "req_direction",
    "res_side", "res_status", "res_status_info", "order_id",
)
LOG_FLOAT_FIELDS = (
    "cash", "position", "debt", "free_cash", "val_price", "value",
    "req_size", "req_price", "req_fees", "req_fixed_fees", "req_slippage",
    "req_min_size", "req_max_size", "req_size_granularity", "req_reject_prob",
    "new_cash", "new_position", "new_debt", "new_free_cash", "new_val_price",
    "new_value", "res_size", "res_price", "res_fees",
)
LOG_BOOL_FIELDS = ("req_lock_cash", "req_allow_partial", "req_raise_reject", "req_log")


def assert_same_logs(theirs, ours):
    """All 37 stored fields, the same rules as the order records.

    A log is the widest thing either engine emits: the account on both sides of
    the order, the request as it was resolved, and the result. Comparing the
    whole row is what makes the request fields — which no order record carries —
    part of the contract. ``req_min_size`` alone caught a default that differed
    from the reference's and that no fixture had ever exercised.
    """
    assert len(theirs) == len(ours["col"]), "different number of log rows"
    for field in LOG_INT_FIELDS + LOG_BOOL_FIELDS:
        np.testing.assert_array_equal(theirs[field], ours[field], err_msg=field)
    for field in LOG_FLOAT_FIELDS:
        np.testing.assert_allclose(
            np.asarray(theirs[field], dtype=float),
            np.asarray(ours[field], dtype=float),
            rtol=1e-12, atol=1e-12, equal_nan=True, err_msg=field,
        )


def orders_sizes(shape, size_type, rng):
    """Sizes in the scale the size type reads them at."""
    if size_type in ("targetpercent",):
        size = rng.uniform(-0.5, 0.9, shape)
    elif size_type == "percent":
        # Sell-weighted, so the cash balance does not walk to zero. A percent
        # buy against no cash is an invariant violation in both engines, which
        # `test_both_engines_refuse_a_percent_order_with_no_cash` covers.
        size = rng.uniform(-0.8, 0.4, shape)
    elif size_type in ("targetvalue", "value"):
        size = rng.uniform(-200.0, 400.0, shape)
    else:
        size = rng.uniform(-3.0, 4.0, shape)
    size[rng.random(shape) < 0.3] = 0.0
    return size


REFERENCE_ENUM = {
    "targetpercent": ("SizeType", "TargetPercent"),
    "targetvalue": ("SizeType", "TargetValue"),
    "targetamount": ("SizeType", "TargetAmount"),
    "percent": ("SizeType", "Percent"),
    "value": ("SizeType", "Value"),
    "amount": ("SizeType", "Amount"),
    "longonly": ("Direction", "LongOnly"),
    "shortonly": ("Direction", "ShortOnly"),
    "both": ("Direction", "Both"),
}


def as_reference_kwargs(kwargs):
    """The same call, spelled the way the reference spells its enums."""
    out = dict(kwargs)
    for key in ("size_type", "direction"):
        if key in out:
            enum, variant = REFERENCE_ENUM[out[key]]
            out[key] = getattr(getattr(_reference.portfolio.enums, enum), variant)
    return out


ORDER_CASES = [
    ("plain", {}),
    ("fees", {"fees": 0.002, "fixed_fees": 0.5, "slippage": 0.001}),
    ("target percent", {"size_type": "targetpercent"}),
    ("target value", {"size_type": "targetvalue"}),
    ("target amount", {"size_type": "targetamount"}),
    ("value", {"size_type": "value"}),
    ("percent", {"size_type": "percent"}),
    ("long only", {"direction": "longonly"}),
    ("short only", {"direction": "shortonly"}),
    ("granularity", {"size_granularity": 1.0}),
    ("min and max size", {"min_size": 0.5, "max_size": 3.0}),
    ("lock cash", {"lock_cash": True}),
    ("no partial fills", {"allow_partial": False}),
    ("update value", {"update_value": True, "size_type": "targetpercent"}),
]


@needs_reference
@pytest.mark.parametrize("name,kwargs", ORDER_CASES, ids=[c[0] for c in ORDER_CASES])
def test_from_orders_matches_the_reference(name, kwargs):
    rng = np.random.default_rng(31337)
    close = np.abs(rng.normal(100.0, 8.0, (40, 3))) + 5.0
    size = orders_sizes(close.shape, kwargs.get("size_type", "amount"), rng)

    try:
        reference = _reference.Portfolio.from_orders(
            close, size, init_cash=1000.0, log=True, freq="1D", engine="numba",
            **as_reference_kwargs(kwargs),
        )
    except ValueError as invariant:
        # An invariant violation, not a rejection: a repeated percent order
        # walks the cash balance to zero and then has nothing to take a
        # percentage of. Both engines must refuse the same input — a run that
        # quietly continued where the reference stopped is the more dangerous
        # divergence, because it produces a number.
        with pytest.raises(ValueError):
            barq.from_orders(close, size, init_cash=1000.0, log=True, **kwargs)
        assert "cash" in str(invariant)
        return

    mine = barq.from_orders(close, size, init_cash=1000.0, log=True, **kwargs)
    # Fills first: a log comparison that passes over wrong fills says nothing.
    assert_same_fills(reference.order_records, mine.orders())
    assert_same_logs(reference.log_records, mine.logs())


SIGNAL_LOG_CASES = [
    ("plain", {}),
    ("fees", {"fees": 0.002, "fixed_fees": 0.5, "slippage": 0.001}),
    ("sized", {"size": 2.0}),
    ("percent", {"size": 0.4, "size_type": "percent"}),
    ("both directions", {"direction": "both"}),
    ("short only", {"direction": "shortonly"}),
    ("accumulate", {"size": 1.0, "accumulate": True}),
    ("no cash", {"init_cash": 1.0}),
    ("granularity", {"size_granularity": 1.0}),
    ("min size refuses", {"size": 1.0, "min_size": 2.0}),
]


@needs_reference
@pytest.mark.parametrize(
    "name,kwargs", SIGNAL_LOG_CASES, ids=[c[0] for c in SIGNAL_LOG_CASES]
)
def test_from_signals_logs_match_the_reference(name, kwargs):
    """The log path in the signal loop is new, and its one design question —
    which bars get a row — is settled here rather than by judgement."""
    rng = np.random.default_rng(31337)
    close = np.abs(rng.normal(100.0, 8.0, (50, 3))) + 5.0
    entries = rng.random(close.shape) < 0.2
    exits = rng.random(close.shape) < 0.2
    kwargs = dict(kwargs)
    init_cash = kwargs.pop("init_cash", 1000.0)

    reference = _reference.Portfolio.from_signals(
        close, entries, exits, init_cash=init_cash, log=True, freq="1D",
        engine="numba", **as_reference_kwargs(kwargs),
    )
    mine = barq.from_signals(close, entries, exits, init_cash=init_cash, log=True, **kwargs)
    assert_same_fills(reference.order_records, mine.orders())
    assert_same_logs(reference.log_records, mine.logs())


@needs_reference
def test_the_default_minimum_size_matches_the_reference():
    """`min_size` defaults to 1e-8 there and used to default to 0 here.

    Observable, and invisible to every fixture: a randomly generated size is
    never dust. The log's `req_min_size` field is what surfaced it.
    """
    setting = _reference._settings.settings["portfolio"]["min_size"]
    close = np.full((2, 1), 10.0)
    size = np.array([[5e-9], [0.0]])
    reference = _reference.Portfolio.from_orders(
        close, size, init_cash=100.0, log=True, freq="1D", engine="numba"
    )
    mine = barq.from_orders(close, size, init_cash=100.0, log=True)
    assert mine.logs()["req_min_size"][0] == setting
    assert len(reference.order_records) == 0, "the reference rejects dust"
    assert mine.n_fills == 0, "and so must this"
    # With the guard off it fills, which is what makes the default a decision.
    assert barq.from_orders(close, size, init_cash=100.0, min_size=0.0).n_fills == 1


# --- structural, no reference needed ---------------------------------------


def test_a_quiet_bar_gets_no_log_row_from_signals():
    """One row per order *considered*, not per bar.

    `from_orders` submits an order for every element, so it logs every element.
    `from_signals` only reaches the kernel when the signals resolved to a size,
    so a bar with no signal has nothing to explain — and a row per bar would
    bury the interesting ones under the bars where the strategy had no opinion.
    """
    close = np.array([[10.0], [11.0], [12.0], [11.0], [13.0]])
    entries = np.array([[True], [False], [False], [True], [False]])
    exits = np.array([[False], [False], [True], [False], [False]])
    signals = barq.from_signals(close, entries, exits, init_cash=100.0, log=True)
    assert signals.n_logs == 3
    np.testing.assert_array_equal(signals.logs()["idx"], [0, 2, 3])

    orders = barq.from_orders(close, np.ones((5, 1)), init_cash=100.0, log=True)
    assert orders.n_logs == 5, "from_orders considers every element"


def test_logging_is_off_by_default_and_changes_nothing_when_on():
    """Additional, never alternative. An investigation that changed the run
    would be investigating something else."""
    _, _, _, close, entries, exits = bars(120, 3)
    quiet = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.001)
    loud = barq.from_signals(close, entries, exits, init_cash=1000.0, fees=0.001, log=True)
    assert quiet.n_logs == 0, "logging should be off unless asked for"
    assert loud.n_logs > 0
    for field in quiet.orders():
        np.testing.assert_array_equal(quiet.orders()[field], loud.orders()[field], err_msg=field)


def test_a_log_mask_logs_only_what_it_selects():
    """The useful shape on a sweep: one column under investigation."""
    _, _, _, close, entries, exits = bars(80, 4)
    mask = np.zeros(close.shape, dtype=bool)
    mask[:, 2] = True
    pf = barq.from_signals(close, entries, exits, init_cash=1000.0, log=mask)
    assert pf.n_logs > 0, "nothing was logged, so this proves nothing"
    assert set(np.unique(pf.logs()["col"])) == {2}


def test_every_log_row_joins_to_its_own_fill():
    """`order_id` is an index into the order records, and the index is re-based
    when per-thread buffers merge. A mistake there names a real fill in another
    column — in range, wrong trade, and invisible to a length check."""
    _, _, _, close, entries, exits = bars(120, 6)
    for threads in (1, 4):
        pf = barq.from_signals(
            close, entries, exits, init_cash=1000.0, log=True, threads=threads
        )
        logs, orders = pf.logs(), pf.orders()
        joined = logs["order_id"] >= 0
        assert joined.sum() > 3, "too few fills to prove anything"
        at = logs["order_id"][joined]
        np.testing.assert_array_equal(orders["col"][at], logs["col"][joined])
        np.testing.assert_array_equal(orders["idx"][at], logs["idx"][joined])
        # And an unfilled row points at nothing rather than at fill zero.
        unfilled = logs["res_status"][~joined]
        assert np.all(unfilled != 0), "a row with no order claims it filled"


def test_threads_do_not_change_the_logs():
    _, _, _, close, entries, exits = bars(200, 12)
    one = barq.from_signals(close, entries, exits, init_cash=1000.0, log=True, threads=1)
    many = barq.from_signals(close, entries, exits, init_cash=1000.0, log=True, threads=6)
    assert one.n_logs > 20, "too few rows to prove anything"
    a, b = one.logs(), many.logs()
    assert set(a) == set(b)
    for field in a:
        np.testing.assert_array_equal(
            np.asarray(a[field], dtype=float), np.asarray(b[field], dtype=float),
            err_msg=field,
        )


def test_reason_counts_tallies_one_column():
    close = np.full((6, 2), 100.0)
    size = np.array([[1.0, 0.0]] * 6)  # column 1 never orders anything
    pf = barq.from_orders(close, size, init_cash=250.0, log=True)
    first = pf.reason_counts(0)
    assert sum(first.values()) == 6, "every considered order should be tallied"
    assert "filled" in first
    # Commonest first, which is what makes it a diagnostic rather than a dump.
    counts = list(first.values())
    assert counts == sorted(counts, reverse=True)
    assert set(pf.reason_counts(1)) == {"size is zero"}
    with pytest.raises(IndexError, match="outside 0..2"):
        pf.reason_counts(2)


def test_log_reasons_agree_with_the_status_fields():
    """The words are a lookup on the fields, so they must not disagree."""
    close = np.full((6, 1), 100.0)
    size = np.array([[1.0], [0.0], [1000.0], [1.0], [0.0], [-2.0]])
    pf = barq.from_orders(close, size, init_cash=250.0, log=True, allow_partial=False)
    reasons, logs = pf.log_reasons(), pf.logs()
    assert len(reasons) == pf.n_logs == 6
    for i, reason in enumerate(reasons):
        filled = logs["res_status"][i] == 0
        assert (reason == "filled") == filled, f"row {i}: {reason!r}"
    assert "size is zero" in reasons


def test_from_orders_with_no_size_is_buy_and_hold():
    """The default size is `inf` and the default price is the close, so the
    whole balance goes in on the first bar — the reference's own doc example."""
    close = np.array([[10.0], [11.0], [12.0], [13.0]])
    pf = barq.from_orders(close, init_cash=100.0)
    assert pf.n_fills == 1
    np.testing.assert_array_equal(pf.orders()["idx"], [0])
    np.testing.assert_allclose(pf.total_return(), [0.3])


def test_from_orders_accepts_the_target_sizes_that_from_signals_refuses():
    """The asymmetry is real, not an oversight: a target size says where the
    position should end up, which is unambiguous per element and ambiguous
    against a signal that says which way to move."""
    close = np.full((5, 1), 100.0)
    weights = np.full((5, 1), 0.5)
    pf = barq.from_orders(close, weights, size_type="targetpercent", init_cash=1000.0)
    assert pf.n_fills >= 1
    with pytest.raises(ValueError, match="Amount, Value and Percent"):
        barq.from_signals(
            close, np.ones((5, 1), dtype=bool), np.zeros((5, 1), dtype=bool),
            size=0.5, size_type="targetpercent",
        )


def test_raise_reject_turns_a_refusal_into_an_exception():
    close = np.full((3, 1), 100.0)
    size = np.full((3, 1), 1000.0)  # far more than the cash allows
    refused = barq.from_orders(close, size, init_cash=10.0, allow_partial=False, log=True)
    assert refused.n_fills == 0
    assert "filled" not in refused.log_reasons()
    with pytest.raises(ValueError, match="rejected"):
        barq.from_orders(
            close, size, init_cash=10.0, allow_partial=False, raise_reject=True
        )
