"""Generate the committed Alpha158 oracle with the local Qlib installation."""

from __future__ import annotations

import csv
import math
from pathlib import Path

import numpy as np
import pandas as pd
import qlib
from qlib.contrib.data.loader import Alpha158DL
from qlib.data.base import Feature
from qlib.data.data import LocalExpressionProvider


OUT = Path(__file__).parents[1] / "fixtures" / "qlib_alpha158.tsv"
SYMBOLS = ("AAA", "BBB", "CCC")
ROWS = 80
INPUTS = ("open", "high", "low", "close", "volume", "vwap")


def panel() -> tuple[pd.DatetimeIndex, dict[str, dict[str, pd.Series]]]:
    dates = pd.bdate_range("2020-01-02", periods=86).delete([3, 17, 40, 65, 72, 81])
    data: dict[str, dict[str, pd.Series]] = {}
    for symbol_index, symbol in enumerate(SYMBOLS):
        row = np.arange(ROWS, dtype=np.float64)
        base = 80.0 + symbol_index * 23.0 + row * (0.071 + symbol_index * 0.013)
        wave = ((row * 17.0 + symbol_index * 11.0) % 19.0 - 9.0) * 0.033
        close = base + wave
        open_ = close + np.sin(row * 0.37 + symbol_index) * 0.31
        high = np.maximum(open_, close) + 0.45 + (row % 5.0) * 0.025
        low = np.minimum(open_, close) - 0.40 - (row % 7.0) * 0.02
        volume = 1_000.0 + symbol_index * 317.0 + row * 4.7 + (row % 9.0) * 13.0
        vwap = (open_ + high + low + 2.0 * close) / 5.0
        fields = {
            "open": open_,
            "high": high,
            "low": low,
            "close": close,
            "volume": volume,
            "vwap": vwap,
        }
        # Missing observations exercise Qlib's partial-window and NaN rules.
        fields["close"][[7 + symbol_index, 34, 67 - symbol_index]] = np.nan
        fields["volume"][[12, 41 + symbol_index]] = np.nan
        fields["open"][[23 + symbol_index]] = np.nan
        fields["high"][[51]] = np.nan
        fields["low"][[52]] = np.nan
        fields["vwap"][[28, 63]] = np.nan
        data[symbol] = {name: pd.Series(values, index=dates) for name, values in fields.items()}
    return dates, data


def format_float(value: float) -> str:
    if math.isnan(value):
        return "nan"
    if math.isinf(value):
        return "inf" if value > 0 else "-inf"
    return format(value, ".17g")


def main() -> None:
    qlib.init()
    dates, data = panel()

    # Keep Qlib's parser, expression tree, cache, and operators. Only replace
    # the leaf provider so this fixture is self-contained and deterministic.
    def load_feature(self: Feature, instrument: str, start: int, end: int, _freq: str) -> pd.Series:
        return data[instrument][self._name].iloc[start : end + 1]

    Feature._load_internal = load_feature
    fields, names = Alpha158DL.get_feature_config()
    assert len(fields) == len(names) == 158
    provider = LocalExpressionProvider(time2idx=False)
    output: dict[str, dict[str, np.ndarray]] = {}
    for symbol in SYMBOLS:
        output[symbol] = {}
        for expression, name in zip(fields, names, strict=True):
            values = provider.get_expression_instance(expression).load(symbol, 0, ROWS - 1, "day")
            output[symbol][name] = values.to_numpy(dtype=np.float64)

    with OUT.open("w", newline="") as handle:
        handle.write(f"# qlib={qlib.__version__}\n")
        handle.write(f"# numpy={np.__version__} pandas={pd.__version__}\n")
        handle.write("# layout=symbol-major rows=80 symbols=AAA,BBB,CCC\n")
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(["symbol", "row", "date", *INPUTS, *names])
        for symbol in SYMBOLS:
            for row, date in enumerate(dates):
                writer.writerow(
                    [
                        symbol,
                        row,
                        date.date().isoformat(),
                        *(format_float(float(data[symbol][name].iloc[row])) for name in INPUTS),
                        *(format_float(float(output[symbol][name][row])) for name in names),
                    ]
                )

    print(f"wrote {OUT} ({len(SYMBOLS) * ROWS} rows, {len(names)} factors)")


if __name__ == "__main__":
    main()
