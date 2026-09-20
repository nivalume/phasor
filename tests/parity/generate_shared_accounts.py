"""Generate complete shared-account fixtures using the local vectorbt package.

Run with an environment containing ../vectorbt and NumPy. No market data needed.
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np

OUT = Path(__file__).parents[1] / "fixtures" / "shared_accounts.json"


def cases():
    rng = np.random.default_rng(92723)
    close = 100 * np.cumprod(1 + rng.normal(0, 0.065, (12, 4)), axis=0)
    entries = rng.random(close.shape) < 0.35
    exits = rng.random(close.shape) < 0.25
    entries[0] = True
    exits[0] = False
    entries[-1] = False
    exits[-1] = True
    valuation = close.copy()
    valuation[4:6, 0] = np.nan
    base = {
        "close": close,
        "init_cash": [1000.0, 1500.0],
        "group_lens": [2, 2],
        "fees": 0.001,
        "slippage": 0.0005,
        "log": True,
    }
    signal_variants = {
        "amount": {"size": 2.0},
        "value": {"size": 250.0, "size_type": "value"},
        "percent": {"size": 0.35, "size_type": "percent"},
        "reverse": {"size": 2.0, "direction": "both"},
        "short": {"size": 2.0, "direction": "shortonly", "lock_cash": True},
        "accumulate": {"size": 1.0, "accumulate": True},
        "stop_loss": {"size": 2.0, "sl_stop": 0.04},
        "take_profit": {"size": 2.0, "tp_stop": 0.04},
        "trailing": {"size": 2.0, "sl_stop": 0.05, "sl_trail": True},
        "no_partial": {"size": 100.0, "allow_partial": False},
        "update_value": {"size": 250.0, "size_type": "value", "update_value": True},
        "previous_valuation": {
            "size": 250.0,
            "size_type": "value",
            "val_price": -np.inf,
        },
        "missing_valuation_ffill": {
            "size": 250.0,
            "size_type": "value",
            "val_price": valuation,
        },
        "missing_valuation_no_ffill": {
            "size": 250.0,
            "size_type": "value",
            "val_price": valuation,
            "ffill_val_price": False,
        },
        "no_signals": {
            "size": 2.0,
            "entries": np.zeros_like(entries),
            "exits": np.zeros_like(exits),
        },
    }
    for mode in ("default", "auto"):
        for name, options in signal_variants.items():
            args = dict(
                base,
                entries=entries,
                exits=exits,
                open=close * 0.99,
                high=close * 1.03,
                low=close * 0.96,
                call_seq=mode,
            )
            args.update(options)
            yield f"signals_{name}_{mode}", "signals", args
        for size_type, sizes in (
            ("targetpercent", rng.uniform(0.1, 0.45, close.shape)),
            ("targetvalue", rng.uniform(50, 450, close.shape)),
            ("amount", rng.uniform(-4, 4, close.shape)),
            ("percent", rng.uniform(-0.5, 0.2, close.shape)),
        ):
            for update in (False, True):
                yield (
                    f"orders_{size_type}_{mode}_{update}",
                    "orders",
                    dict(
                        base,
                        size=sizes,
                        size_type=size_type,
                        call_seq=mode,
                        update_value=update,
                    ),
                )
    missing = close.copy()
    missing[:, 1] = np.nan  # Never held; must not poison the first account.
    missing[7, 2] = np.nan  # Held-price NaN retains existing valuation semantics.
    size = np.full_like(close, np.nan)
    size[0, [0, 2, 3]] = 1.0
    yield (
        "orders_missing",
        "orders",
        dict(base, close=missing, size=size, call_seq="auto"),
    )
    yield (
        "singletons",
        "signals",
        dict(
            base,
            entries=entries,
            exits=exits,
            size=2.0,
            group_lens=[1, 1, 1, 1],
            init_cash=[1000.0, 1200.0, 1500.0, 1800.0],
            call_seq="auto",
        ),
    )


def reference(kind, inputs):
    import vectorbt
    from vectorbt.portfolio import enums

    args = dict(inputs)
    lens = args.pop("group_lens")
    groups = np.repeat(np.arange(len(lens)), lens)
    names = {
        "amount": "Amount",
        "value": "Value",
        "percent": "Percent",
        "targetamount": "TargetAmount",
        "targetvalue": "TargetValue",
        "targetpercent": "TargetPercent",
        "both": "Both",
        "longonly": "LongOnly",
        "shortonly": "ShortOnly",
    }
    for key, enum in (("size_type", enums.SizeType), ("direction", enums.Direction)):
        if key in args:
            args[key] = getattr(enum, names[args[key]])
    return getattr(vectorbt.Portfolio, f"from_{kind}")(
        **args,
        cash_sharing=True,
        group_by=groups,
        freq="1D",
        engine="numba",
        fillna_close=False,
    )


def records(array):
    return {name: array[name].tolist() for name in array.dtype.names if name != "id"}


def snapshot(pf, n_groups):
    tables = []
    for group in range(n_groups):
        table = {}
        for name, value in pf.stats(column=group).items():
            if hasattr(value, "total_seconds"):
                value = value.total_seconds() / 86400
            table[name] = float(value)
        tables.append(table)
    return {
        "orders": records(pf.order_records),
        "logs": records(pf.log_records),
        "trades": records(pf.trades.records_arr),
        "cash": np.asarray(pf.cash()),
        "assets": np.asarray(pf.assets()),
        "value": np.asarray(pf.value()),
        "returns": np.asarray(pf.returns()),
        "metrics": {
            name: np.atleast_1d(getattr(pf, name)())
            for name in (
                "total_return",
                "annualized_return",
                "max_drawdown",
                "sharpe_ratio",
                "sortino_ratio",
                "calmar_ratio",
            )
        },
        "trade_count": np.atleast_1d(pf.trades.count()),
        "stats": tables,
    }


def encode(value):
    if isinstance(value, np.ndarray):
        return value.tolist()
    if isinstance(value, np.generic):
        return value.item()
    raise TypeError(type(value).__name__)


def main():
    import vectorbt

    output = []
    for name, kind, inputs in cases():
        pf = reference(kind, inputs)
        output.append(
            {
                "name": name,
                "kind": kind,
                "inputs": inputs,
                "expected": snapshot(pf, len(inputs["group_lens"])),
            }
        )
    OUT.write_text(
        json.dumps(
            {"vectorbt": vectorbt.__version__, "cases": output},
            default=encode,
            separators=(",", ":"),
        )
        + "\n"
    )
    print(f"wrote {OUT}: {len(output)} shared-account cases")


if __name__ == "__main__":
    main()
