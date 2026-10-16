"""``cash_limit``, ``reject_prob`` and ``leverage`` through the Python API."""

import phasor
import numpy as np
import pytest


def test_cash_limit_caps_a_buy():
    pf = phasor.from_orders(np.array([10.0]), size=np.inf, cash_limit=30.0)
    np.testing.assert_array_equal(pf.orders()["size"], [3.0])


def test_reject_prob_is_reproducible_and_seeded():
    close = np.full((500, 4), 10.0, order="F")
    kwargs = dict(size=1.0, init_cash=1e9, reject_prob=0.4)
    first = phasor.from_orders(close, seed=1, **kwargs).orders()
    again = phasor.from_orders(close, seed=1, threads=3, **kwargs).orders()
    other = phasor.from_orders(close, seed=2, **kwargs).orders()
    np.testing.assert_array_equal(first["idx"], again["idx"])
    assert not np.array_equal(first["idx"], other["idx"])
    assert abs(len(first["idx"]) / 2000 - 0.6) < 0.05


def test_an_out_of_range_reject_prob_is_refused():
    with pytest.raises(ValueError, match="reject_prob"):
        phasor.from_orders(np.array([10.0]), size=1.0, reject_prob=1.5)


def test_a_levered_buy_borrows_and_value_stays_the_equity():
    close = np.array([[100.0], [120.0]])
    pf = phasor.from_orders(close, size=np.array([[15.0], [np.nan]]), init_cash=1000.0, leverage=2.0)
    orders = pf.orders()
    np.testing.assert_array_equal(orders["size"], [15.0])
    np.testing.assert_array_equal(orders["debt"], [500.0])
    np.testing.assert_array_equal(orders["locked_cash"], [1000.0])
    np.testing.assert_allclose(np.ravel(pf.cash()), [-500.0, -500.0])
    np.testing.assert_allclose(np.ravel(pf.value()), [1000.0, 1300.0])


def test_eager_leverage_puts_up_its_share_and_logs_the_request():
    pf = phasor.from_orders(
        np.array([100.0]), size=5.0, init_cash=1000.0,
        leverage=2.0, leverage_mode="eager", log=True,
    )
    np.testing.assert_array_equal(pf.orders()["debt"], [250.0])
    logs = pf.logs()
    np.testing.assert_array_equal(logs["req_leverage"], [2.0])
    np.testing.assert_array_equal(logs["req_leverage_mode"], [1])
    np.testing.assert_array_equal(logs["new_locked_cash"], [250.0])


def test_an_unlevered_run_has_no_balance_columns():
    pf = phasor.from_orders(np.array([100.0]), size=5.0, init_cash=1000.0)
    assert "debt" not in pf.orders()


def test_a_levered_signal_entry_buys_twice_the_cash():
    pf = phasor.from_signals(
        np.array([100.0, 110.0]), np.array([True, False]), np.array([False, True]),
        init_cash=1000.0, leverage=2.0,
    )
    np.testing.assert_array_equal(pf.orders()["size"], [20.0, 20.0])


@pytest.mark.parametrize("bad", [0.5, 0.0, -1.0, np.nan])
def test_a_leverage_below_one_is_refused(bad):
    with pytest.raises(ValueError, match="leverage"):
        phasor.from_orders(np.array([10.0]), size=1.0, leverage=bad)


def test_an_unknown_leverage_mode_is_refused():
    with pytest.raises(ValueError, match="leverage_mode"):
        phasor.from_orders(np.array([10.0]), size=1.0, leverage=2.0, leverage_mode="greedy")
