"""``cash_limit`` and ``reject_prob`` through the Python API."""

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
