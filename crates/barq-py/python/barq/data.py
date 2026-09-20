"""Read-only local Arrow sources; decoding and alignment run before backtesting.

Install ``barq[data]`` to use this module's sources. Importing barq and using
NumPy-only simulations does not require PyArrow.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import numpy as np
from . import _barq

InstrumentId = _barq.InstrumentId
BarType = _barq.BarType
DecodeContext = _barq.DecodeContext


@dataclass(frozen=True)
class DataSchema:
    """Batch decoder contract shared by CSV, Parquet, and catalog sources.

    input/output are PyArrow schemas. Decoder receives (RecordBatch, context).
    Time columns are uint64 UTC nanoseconds. instrument is 'source', a pair of
    decoded (symbol_column, venue_column), or None for global events.
    """
    name: str
    input: object
    output: object
    decode: object
    event_time: str = "event_time"
    available_time: str = "available_time"
    instrument: object = "source"

    def __post_init__(self):
        if not callable(self.decode):
            raise TypeError("decode must be callable")
        if self.instrument not in (None, "source"):
            if not isinstance(self.instrument, tuple) or len(self.instrument) != 2 or not all(isinstance(s, str) for s in self.instrument):
                raise ValueError("instrument must be 'source', (symbol_column, venue_column), or None")


@dataclass(frozen=True)
class BarPanel:
    time: np.ndarray
    bar_types: tuple
    open: np.ndarray
    high: np.ndarray
    low: np.ndarray
    close: np.ndarray
    volume: np.ndarray

    @classmethod
    def _from_native(cls, values):
        values["bar_types"] = tuple(values["bar_types"])
        shape = (len(values["time"]), len(values["bar_types"]))
        for name in ("open", "high", "low", "close", "volume"):
            values[name] = values[name].reshape(shape, order="F")
        return cls(**values)

    @property
    def instruments(self):
        return tuple(t.instrument for t in self.bar_types)

    @property
    def shape(self):
        return self.close.shape

    @property
    def n_rows(self):
        return self.shape[0]

    @property
    def n_cols(self):
        return self.shape[1]

    def coverage(self):
        return np.isfinite(self.close).sum(axis=0) / self.n_rows if self.n_rows else np.zeros(self.n_cols)


class Dataset:
    """Decoded Arrow batches with explicit numeric views onto a BarPanel."""
    def __init__(self, native):
        self._native = native

    @property
    def batches(self):
        """(DecodeContext, pyarrow.RecordBatch) pairs, including nonnumeric fields."""
        return self._native.batches

    def prepare(self):
        return BarPanel._from_native(self._native.prepare())

    def asof(self, panel, *, field):
        """Latest available value; missing history remains NaN. Ties use input order."""
        return self._native.align(panel.time, [str(i) for i in panel.instruments], field).reshape(panel.shape, order="F")

    def window(self, panel, *, field, window, agg="mean", empty=np.nan):
        """Aggregate (t-window, t] by availability. Count counts all event rows.

        Sum/mean ignore null and nonfinite values; empty supplies their result
        when there are no finite observations. Global events broadcast.
        """
        if isinstance(window, str):
            match = re.fullmatch(r"([1-9][0-9]*)(ns|us|ms|s|m|h|d|w)", window)
            if not match:
                raise ValueError("window must be positive nanoseconds or a duration such as '30m'")
            units = dict(ns=1, us=1000, ms=1000000, s=10**9, m=60*10**9, h=3600*10**9, d=86400*10**9, w=604800*10**9)
            window = int(match[1]) * units[match[2]]
        if isinstance(window, (bool, np.bool_)) or not isinstance(window, (int, np.integer)) or window <= 0:
            raise ValueError("window must be positive nanoseconds")
        return self._native.align(panel.time, [str(i) for i in panel.instruments], field, int(window), agg, empty).reshape(panel.shape, order="F")


class Source:
    """File references and a decoder; loading is explicit and does not write."""
    def __init__(self, items, schema):
        self.items = tuple((str(key), tuple(str(Path(p).expanduser()) for p in paths)) for key, paths in items)
        self.schema = schema

    def load(self, *, start=None, end=None):
        """Read [start,end) in decoded availability nanoseconds.

        Load auxiliary history before the target panel when using asof/windows;
        an explicit start discards earlier observations.
        """
        return Dataset(_barq.load_data(self.items, self.schema, start, end))

    def prepare(self, *, start=None, end=None):
        return self.load(start=start, end=end).prepare()


def files(items, *, schema):
    """Map source key -> path or ordered paths; filenames need no naming convention."""
    return Source([(key, [paths] if isinstance(paths, (str, Path)) else paths) for key, paths in items.items()], schema)


class Catalog:
    def __init__(self, root="~/.cache/barq/catalog", *, schemas):
        self.root = Path(root).expanduser()
        self.schemas = {}
        for schema in schemas:
            if schema.name in self.schemas:
                raise ValueError(f"duplicate Schema name: {schema.name}")
            self.schemas[schema.name] = schema

    def source(self, name, *, keys):
        schema = self.schemas[name]
        return Source(_barq.catalog_files(self.root, name, [str(key) for key in keys]), schema)
