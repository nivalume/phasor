"""The same user Schema must work through all local readers and Rust alignment."""
from dataclasses import replace
from pathlib import Path
import gc
import sys

import numpy as np
import pytest
import barq
pa = pytest.importorskip("pyarrow")
import pyarrow.csv as csv
import pyarrow.parquet as pq

ROOT = Path(__file__).parents[3]
sys.path.insert(0, str(ROOT / "examples"))
import multi_data

KEY = "ADAUSDC.BINANCE-1-MINUTE-LAST-EXTERNAL"
OTHER = "ETHUSDC.BINANCE-1-MINUTE-LAST-EXTERNAL"


def identity(batch, context):
    return batch


def schema(extra, *, name="custom", instrument="source"):
    fields = pa.schema([("event_time", pa.uint64()), ("available_time", pa.uint64()), *extra])
    return barq.data.DataSchema(name, fields, fields, identity, instrument=instrument)


def bar_schema():
    return schema([(k, pa.float64()) for k in multi_data.OHLCV])


def table(times, **columns):
    times = pa.array(times, type=pa.uint64())
    return pa.table({"event_time": times, "available_time": times, **columns})


def bar_table(times, prices=None):
    prices = prices if prices is not None else [10.] * len(times)
    return table(times, **{k: np.array(prices, dtype=float) for k in multi_data.OHLCV})


def source(tmp_path, definition, tables, extension="parquet"):
    paths = {}
    for i, (key, batch) in enumerate(tables.items()):
        path = tmp_path / f"input-{i}.{extension}"
        (csv.write_csv if extension == "csv" else pq.write_table)(batch, path)
        paths[key] = path
    return barq.data.files(paths, schema=definition)


def test_three_sources_have_identical_end_to_end_results_and_are_read_only(tmp_path):
    results = []
    for kind in ("catalog", "parquet", "csv"):
        root = tmp_path / kind
        multi_data.write_samples(root, kind)
        before = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
        result = multi_data.run(root, kind)
        assert {p: p.read_bytes() for p in root.rglob("*") if p.is_file()} == before
        results.append(result)
    first = results[0]
    for current in results[1:]:
        np.testing.assert_array_equal(first[0].time, current[0].time)
        for name in multi_data.OHLCV:
            np.testing.assert_allclose(getattr(first[0], name), getattr(current[0], name), rtol=1e-14)
        for index in (1, 2):
            for name in first[index]:
                np.testing.assert_allclose(first[index][name], current[index][name], equal_nan=True)
        for index in (3, 4):
            np.testing.assert_array_equal(first[index], current[index])
        for name, values in first[5].orders().items():
            np.testing.assert_allclose(values, current[5].orders()[name])
        np.testing.assert_allclose(first[5].total_return(), current[5].total_return())


@pytest.mark.parametrize("extension", ["csv", "parquet"])
def test_custom_namespace_bar_detection_projection_missing_and_duplicates(tmp_path, extension):
    definitions = bar_schema()
    # Physical fields may be reordered and contain entirely irrelevant fields.
    a = bar_table([30, 10], [3., 1.]).append_column("ignored", pa.array(["x", "y"]))
    a = a.select(list(reversed(a.schema.names)))
    b = bar_table([20], [2.])
    panel = source(tmp_path, definitions, {OTHER: b, KEY: a}, extension).prepare()
    assert [str(i) for i in panel.instruments] == ["ETHUSDC.BINANCE", "ADAUSDC.BINANCE"]
    assert str(panel.bar_types[1]) == KEY and panel.bar_types[1].step == 1
    np.testing.assert_array_equal(panel.time, [10, 20, 30])
    np.testing.assert_allclose(panel.close, [[np.nan,1.],[2.,np.nan],[np.nan,3.]], equal_nan=True)
    assert panel.close.flags.f_contiguous
    with pytest.raises(ValueError, match="duplicate bar"):
        source(tmp_path, definitions, {KEY: bar_table([1, 1])}, extension).prepare()
    with pytest.raises(ValueError, match="close"):
        source(tmp_path, definitions, {KEY: bar_table([1]).drop(["close"])}, extension).prepare()
    # Being named bars does not turn a plain instrument into a BarType.
    with pytest.raises(ValueError, match="STEP"):
        source(tmp_path, replace(definitions, name="bars"), {"ADAUSDC.BINANCE": bar_table([1])}, extension).prepare()


def test_availability_ties_window_boundaries_global_and_row_instruments(tmp_path):
    panel = source(tmp_path, bar_schema(), {KEY: bar_table([0,10,20,30,40]), OTHER:bar_table([0,10,20,30,40])}).prepare()
    definition = schema([("value", pa.float64()), ("headline",pa.string())])
    # Event times are early; rows become visible only at their availability times.
    t = table([10,20,20,35], value=[1.,2.,4.,8.], headline=["a","b","c","d"])
    t = t.set_column(0, "event_time", pa.array([0,0,0,0], type=pa.uint64()))
    data = source(tmp_path, definition, {"ADAUSDC.BINANCE": t}).load()
    np.testing.assert_allclose(data.asof(panel, field="value")[:,0], [np.nan,1,4,4,8], equal_nan=True)
    assert np.isnan(data.asof(panel, field="value")[:,1]).all()
    for agg, expected in [("sum", [0,1,7,6,8]), ("mean",[0,1,7/3,3,8]),("count",[0,1,3,2,1])]:
        np.testing.assert_allclose(data.window(panel, field="value", window=20, agg=agg, empty=0)[:,0], expected)
    assert data.batches[0][1]["headline"].to_pylist() == ["a","b","c","d"]
    # Lifetime survives the original tables and a full Python GC.
    del t
    gc.collect()
    assert data.batches[0][1]["value"].to_pylist() == [1.,2.,4.,8.]
    global_data = source(tmp_path, replace(definition, instrument=None), {"world":table([10], value=[7.], headline=["global"])}).load()
    aligned = global_data.asof(panel, field="value")
    np.testing.assert_allclose(aligned[:,0], aligned[:,1], equal_nan=True)
    row_schema = schema([("symbol",pa.string()),("venue",pa.string()),("value",pa.float64())], instrument=("symbol","venue"))
    rows = table([10,10], symbol=["ADAUSDC","ETHUSDC"], venue=["BINANCE","BINANCE"], value=[3.,9.])
    row_data = source(tmp_path, row_schema, {"arbitrary-filename": rows}).load()
    np.testing.assert_allclose(row_data.asof(panel, field="value")[1:], [[3.,9.]]*4)
    with pytest.raises(ValueError, match="numeric"):
        data.asof(panel, field="headline")


def test_decoded_range_validation_and_catalog_failures_do_not_create_files(tmp_path):
    data = source(tmp_path, bar_schema(), {KEY:bar_table([3,1,2])})
    np.testing.assert_array_equal(data.prepare(start=2,end=3).time, [2])
    assert data.prepare(start=10).shape == (0,1)
    with pytest.raises(ValueError, match="interval"):
        data.load(start=2,end=2)
    with pytest.raises(ValueError, match="cannot contain nulls"):
        source(tmp_path, bar_schema(), {KEY:bar_table([None])}).load()
    catalog = barq.data.Catalog(tmp_path / "absent", schemas=[bar_schema()])
    with pytest.raises(ValueError):
        catalog.source("custom", keys=[KEY])
    assert not (tmp_path / "absent").exists()
    with pytest.raises(ValueError, match="path components"):
        catalog.source("custom", keys=["../escape"])


@pytest.mark.parametrize("value", ["ADA", ".BINANCE", "ADA.", "A/B.X", "A X.Y"])
def test_invalid_instrument(value):
    with pytest.raises(ValueError):
        barq.InstrumentId.parse(value)


def test_identity_components_are_not_reinterpreted(tmp_path):
    with pytest.raises(ValueError):
        barq.InstrumentId("ADA", "BINANCE.SPOT")
    definition = schema([("symbol",pa.string()),("venue",pa.string()),("value",pa.float64())], instrument=("symbol","venue"))
    for symbol, venue in [("", "BINANCE"), ("ADA", "BINANCE.SPOT")]:
        with pytest.raises(ValueError):
            source(tmp_path, definition, {"events":table([1], symbol=[symbol], venue=[venue], value=[1.])}).load()


def test_decoder_controls_time_and_bar_payload(tmp_path):
    definition = replace(multi_data.schemas()[0], name="user_bars")
    raw = pa.table({"timestamp_ms":pa.array([1,2,3], type=pa.uint64()),
                    "published_ms":pa.array([2,3,4], type=pa.uint64()),
                    **{k: [1.,2.,3.] for k in multi_data.OHLCV}})
    data = source(tmp_path, definition, {KEY:raw}, "csv")
    panel = data.prepare(start=3_000_000, end=4_000_000)
    np.testing.assert_array_equal(panel.time, [3_000_000])
    np.testing.assert_array_equal(panel.close, [[2.]])
    definition = schema([(k, pa.float64()) for k in multi_data.OHLCV] + [("extra", pa.string())])
    data = source(tmp_path, definition, {KEY:bar_table([1]).append_column("extra",pa.array(["retained raw"]))}).load()
    assert data.prepare().close[0,0] == 10.
    assert data.batches[0][1]["extra"].to_pylist() == ["retained raw"]
