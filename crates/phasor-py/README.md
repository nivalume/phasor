# phasor-py

Python bindings. PyO3 + rust-numpy, zero-copy in, NumPy arrays out.

```python
import numpy as np, phasor

pf = phasor.from_signals(close, entries, exits, init_cash=10_000, fees=0.001, threads=8)
print(pf.stats())                     # the 28-row table, for one column
best = int(np.nanargmax(pf.traded_only(pf.sharpe_ratio())))

# One order per element instead of signals: a rebalance is a weight matrix.
pf = phasor.from_orders(close, weights, size_type="targetpercent")

# And when a run traded less than expected, ask why.
pf = phasor.from_signals(close, entries, exits, init_cash=10_000, log=True)
print(pf.reason_counts(0))            # {'filled': 31, 'no cash to go long': 12}
```

## Shared accounts

Both `from_orders` and `from_signals` accept `cash_sharing=False`,
`group_lens=None`, and `call_seq="default"`. With sharing enabled and no
lengths, all columns form one account. Explicit lengths partition adjacent
columns; `[3, 3]` represents two parameter sets over three symbols each.
`init_cash` is a scalar or one balance per group, counted once per account.
Without sharing, existing per-column semantics are unchanged and explicit
`group_lens` is rejected.

```python
# weights has the same (time, symbol) shape as close.
# Use NaN to skip a rebalance and zero to close a position.
pf = phasor.from_orders(
    close, weights, size_type="targetpercent",
    cash_sharing=True, init_cash=100_000, call_seq="auto", fees=0.001,
)
print(pf.metrics())             # one result per account
print(pf.stats(group=0))
print(pf.cash(group=0))
print(pf.value(group=0))
print(pf.assets(col=0))         # symbol-level quantity
```

`call_seq="auto"` stably sorts approximate order values so sells release cash
before buys. Equal values retain column order. It does not allocate equal
weights: `size_type="percent"` consumes a fraction of the resources remaining
at each fill; `targetpercent` targets a fraction of the account's valuation.
Targets are supported by `from_orders`; signals accept amount, value and
percent. Both paths accept `val_price`, `ffill_val_price` and `update_value`;
with the default `update_value=False`, targets within a bar use the same
pre-fill account valuation. Auto sequencing assumes the Bar model's prices
remain available regardless of execution order.

`n_cols` counts logical simulation columns; `n_groups`, `group_lens` and `cash_sharing`
report account metadata. Metrics and trade counts are per group. Shared
account series require `group=`; `col=` continues to select positions and
record diagnostics. `stats()` defaults to account 0. Ungrouped positional
calls such as `value(0)` remain supported. Orders, trades and logs retain
original symbol columns; log `group` identifies the cash account. Shared
accounts do not infer a benchmark, so `Benchmark Return [%]` is `None`.
An unheld symbol's missing price contributes zero value to a shared account;
a missing held price propagates NaN in closing valuation.

Groups are independent and can run on separate threads. Symbols within one
account execute sequentially. `python examples/portfolio.py` demonstrates
allocation and signal parameter sweeps without network access. Signal delays
and execution prices remain explicit; no risk or automatic allocation layer
is added.

## Deposits and earnings

Both loops accept `cash_deposits`, `cash_dividends` and `cash_earnings`: cash
that moves without a fill.

- `cash_deposits` is booked at each bar's open, before orders are sized, so a
  `targetpercent` rebalance on the same bar invests it. Positive adds, negative
  withdraws. With `cash_sharing` a matrix is `(n_bars, n_groups)`, one column per
  account.
- `cash_earnings` is booked at each bar's close, per column, including bars with
  no signal: interest, fees, or a funding payment computed in advance.
- `cash_dividends` is per share and shaped like `close` (`column_map` applies).
  It is paid at each bar's open on the position held before that bar's orders,
  so buying on the ex-date does not earn it; a short position pays it.
- A 1-D array is one value per bar (a schedule), not one value per column.
- A withdrawal or a charge larger than the free cash is applied in part.
  `pf.cash_records()` returns what was applied (`col`, `idx`, `amount`, and
  `kind`: 0 deposit, 1 earning, 2 dividend).
- Deposits are not returns: each bar's return is measured from the previous
  value plus that bar's deposit, total return compounds the bars, and
  drawdowns ignore money moved in or out.

```python
# 1,000 on the first bar of every month, invested by the monthly rebalance.
deposits = np.where(month_start, 1_000.0, 0.0)
pf = phasor.from_orders(close, weights, size_type="targetpercent",
                      cash_sharing=True, cash_deposits=deposits)
```

## Order controls

`cash_limit` caps the cash one buy may spend, fees included; it applies before
`percent` takes its share and does not affect sells or shorts. `reject_prob`
rejects orders at random with that probability (status `RandomEvent` in the
logs), for testing how a strategy degrades when fills fail. Each draw depends
only on `seed`, the bar and the column, so a run is reproducible on any thread
count and under any grouping.

## Leverage

`leverage` (1 or more, per column) lets a buy be worth that many times its own
cash. `leverage_mode="lazy"`, the default, spends own cash first and borrows
only what it cannot cover; `"eager"` borrows `1 - 1/leverage` of every buy.
Fees come out of own cash, `percent` sizes against the levered buying power,
and a short ties up its proceeds plus `value / leverage` instead of twice its
value. A buy with no own cash left is refused (`NoCashLeverage` in the logs).

- `pf.value()` is the equity: positions at market, less what the longs owe.
- `pf.cash()` is own cash less that borrowing, so it goes negative on a
  levered long; `pf.free_cash()` also takes out what shorts tie up, and
  `pf.gross_exposure()` rises above 1 with the leverage.
- `pf.orders()` gains `debt` and `locked_cash` per fill, and the logs gain
  `locked_cash`, `new_locked_cash`, `req_leverage` and `req_leverage_mode`.
- There is no interest on the borrowing and no liquidation yet.

```python
# Twice the cash in the first bar's buy: 1,000 of own money, 1,000 borrowed.
pf = phasor.from_orders(close, size=np.inf, init_cash=1_000.0, leverage=2.0)
```

## Margin accounts

`account_type="margin"` makes a column trade the way futures, perpetuals and
CFDs do: either side only posts initial margin out of free cash, and the
position's profit or loss is settled into cash at every bar. Give one type for
every column, or a list with one per column, so a spot column and a perpetual
column in one group share their cash. `initial_margin` is a fraction of the
notional (`margin_basis="notional"`, the default; left unset it is
`1 / leverage`) or cash per unit of size (`margin_basis="unit"`, as futures
exchanges publish it per contract). Closing is always allowed; opening needs
free cash for its margin and fees, and is refused (`NoCashLong` /
`NoCashShort`) once a loss has used it up.

- `pf.value()` is the equity; `pf.free_cash()` is that equity less the margin
  posted, and `pf.gross_exposure()` counts the whole contract over it.
- `pf.orders()` gains `debt` (the cost basis), `locked_cash` (the margin) and
  `margin` per fill, and the logs gain `req_account_type`,
  `req_initial_margin` and `req_margin_basis`.
- There is no maintenance margin, liquidation or funding yet.

```python
# Spot long, perpetual short on 10% margin, one account: at 120 one gains
# what the other loses, so the value stays 1,000.
close = np.array([[100.0, 100.0], [120.0, 120.0]])
size = np.array([[5.0, -5.0], [np.nan, np.nan]])
pf = phasor.from_orders(close, size=size, init_cash=1_000.0, cash_sharing=True,
                        account_type=["cash", "margin"],
                        initial_margin=np.array([np.nan, 0.1]))
```

## Factor expressions

### Function strategies

Use a Python function to turn a `BarPanel` or field mapping into entry and exit
signals. A direct call returns raw, undelayed signals. `run` and `sweep` delay
them one bar by default, then call the existing `from_signals` engine. With its
default execution price, a decision on bar `t` fills at bar `t+1`'s close.

```python
@phasor.strategy
def ma_cross(data, fast=10, slow=50):
    f = phasor.factors(data, fast=f"Mean($close, {fast})",
                     slow=f"Mean($close, {slow})")
    return (phasor.crossed_above(f["fast"], f["slow"]),
            phasor.crossed_above(f["slow"], f["fast"]))

entries, exits = ma_cross({"close": close}, fast=10, slow=50)
pf = ma_cross.run({"close": close}, params={"fast": 10, "slow": 50}, fees=0.001)
pf = ma_cross.sweep(
    {"close": close},
    params=[{"fast": 10, "slow": 50}, {"fast": 20, "slow": 100}],
    fees=0.001,
)
```

The strategy returns two Boolean arrays of shape `(time, symbols)` (a 1-D
array also works for one symbol). Each parameter set runs over every symbol;
result columns follow parameter order, then source symbol order. `signal_delay`
accepts a nonnegative bar count, including zero. `phasor.factors` uses the existing
factor DSL and rejects expressions marked as using future data. It does not
inspect arbitrary Python strategy code for lookahead.

For one account per parameter set, pass `cash_sharing=True`; the wrapper groups
each set's symbols automatically, leaving sets independent. Execution options
such as `size`, `fees`, `slippage`, `init_cash`, and `threads` pass through to
`from_signals`. OHLC fields come from the input panel or mapping. The wrapper
owns the signal arrays, price column map, and group boundaries.

`phasor.crossed_above` uses the engine's crossing rules for equality and missing
values. These differ from the earlier examples' NumPy
comparison, so migrated strategies can produce different trades at those
boundaries.

Compile one custom expression once and evaluate it against any same-shaped
fields:

```python
trend = phasor.Expression.compile("$close / Mean($close, 20) - 1")
values = trend.evaluate({"close": close})
```

Compile several named factors together to share repeated subexpressions:

```python
factors = phasor.FactorSet.compile({
    "trend": "$close / Mean($close, 20) - 1",
    "volume_ratio": "$volume / Mean($volume, 5)",
})
values = factors.evaluate({"close": close, "volume": volume})
```

`phasor.alpha158()` and `phasor.alpha360()` return the built-in Qlib factor sets and
require explicit `open`, `high`, `low`, `close`, `volume`, and `vwap` fields.
Alpha360 returns the raw 60-row normalized history without Qlib's later data
handler processing. Custom field names such as `$funding_rate` work the same
way. The DSL supports arithmetic, comparisons, `If`, `Ref`, fixed and
expanding windows, regression, correlation/covariance, ranks, moments, WMA,
and EMA.

Expressions do not decide execution timing. Delay a close-derived factor
before turning it into a signal that fills on a later bar:

```python
entries = np.zeros_like(values["trend"], dtype=bool)
entries[1:] = values["trend"][:-1] > 0
```

## Local data and parameter sweeps

Install `pip install -e 'crates/phasor-py[data]'` for local CSV/Parquet sources.
NumPy-only backtests still work without PyArrow. Rust `phasor-data` owns reading,
validation, BarPanel construction and auxiliary alignment; Python supplies a
batch decoder through Arrow C Data capsules (no per-row Python conversion).

`InstrumentId.parse("ADAUSDC.BINANCE")` separates `symbol` and `venue`.
`BarType.parse("ADAUSDC.BINANCE-1-MINUTE-LAST-EXTERNAL")` additionally preserves
step, aggregation, price type and source. Namespace names do not identify bars:
a successfully parsed source BarType does. `prepare()` requires the five
float64 OHLCV fields and ignores additional decoded fields.

A `DataSchema(name, input, output, decode, event_time=..., available_time=...,
instrument=...)` applies identically to all readers. `input` and `output` are
PyArrow schemas; `decode(batch, context)` returns a PyArrow RecordBatch. Input
fields select Parquet columns and supply CSV types (CSV must have a header;
column order and extra columns do not matter). Output fields are validated by
name, dtype and nullability. The decoder owns vendor encodings and units.
`context` contains `key`, `path`, `instrument`, and parsed `bar_type` when present.
The two time columns must contain non-null uint64 UTC nanoseconds. They can be
the same column. `instrument="source"` uses the source key;
`instrument=("symbol", "venue")` uses decoded string columns; `instrument=None`
means global data, broadcast only when an aligned view is requested.

```python
# schemas contains user-defined DataSchema objects; see the runnable example.
catalog = phasor.data.Catalog(schemas=schemas)  # ~/.cache/phasor/catalog
bars = catalog.source("bars", keys=bar_types)
# Or the same Schema with standalone files, without a catalog hierarchy:
bars = phasor.data.files({bar_types[0]: "ada.csv", bar_types[1]: "eth.csv"},
                       schema=schemas[0])
panel = bars.prepare()
funding = catalog.source("funding", keys=instruments).load()
news = catalog.source("news", keys=instruments).load()
fields = {
    "close": panel.close,
    "funding_rate": funding.asof(panel, field="rate"),
    "news_sentiment": news.window(panel, field="sentiment", window="30m",
                                  agg="mean", empty=0.0),
}
```

Catalog paths are `root/data/{schema.name}/{key}/*.{parquet,csv}`. Files are
sorted within each key; requested keys retain their order. Standalone file
lists retain their explicit order. Readers never create directories, download,
or write caches. `load(start=..., end=...)` and `prepare(...)` filter decoded
availability timestamps in `[start,end)`. Arbitrary decoder transformations
prevent guessing a physical timestamp predicate, so range filtering follows
decoding; Parquet column projection happens before decoding. Include earlier
history when loading auxiliary sources for asof/window views.

`BarPanel.time` is the sorted union of available bar timestamps. Missing cells
remain NaN; duplicate timestamps within one series raise an error. Each OHLCV
field has Fortran-contiguous shape `[T,S]`; `bar_types` and `instruments` describe
its columns. Rolling factor windows count panel rows, including gaps, so choose
the decoder's availability clock deliberately when feeds have different clocks.
`Dataset.batches` exposes `(DecodeContext, RecordBatch)` pairs including text.

`asof` uses the latest available row at or before panel time; equal timestamps
use the last row in stable input order, and missing history produces NaN.
`window` includes `(t-window,t]`: sum/mean skip null/nonfinite observations and
use `empty` when none remain; count counts all rows and returns zero for an
empty window. Both views align by complete instrument identity, never bare
symbol. Funding is a factor input, not an automatic funding cash flow.

Both simulation entry points accept `column_map`. Market OHLC arrays retain
`[T,S]`; signals, orders, fees, sizes and other trading parameters use `[T,C]`,
where `C=len(column_map)`. For two instruments and four parameter sets:

```python
pf = phasor.from_signals(panel.close, entries, exits,
    column_map=[0, 1, 0, 1, 0, 1, 0, 1], threads=4)
assert pf.n_cols == 8
assert pf.price_nbytes == panel.close.nbytes
# Add cash_sharing=True, group_lens=[2]*4 for four two-instrument accounts.
```

The portfolio owns one source-price copy and the integer map. Execution and
statistics both read through the map; prices are never tiled by parameter.
Factor computation remains the existing FactorSet implementation. Threads split
independent simulation accounts; rows inside each account remain sequential.

Runnable examples (all except explicit fetch can run offline):

```sh
python examples/multi_data.py --source catalog
python examples/multi_data.py --source parquet
python examples/multi_data.py --source csv
python examples/crypto.py --demo
python examples/sweep.py
python examples/portfolio.py
# Full BTC/ETH/SOL/BNB USD-M history from Binance Vision:
python examples/download_binance_vision.py
# Optional real Binance download followed by read-only backtesting:
python examples/fetch.py --symbols BTCUSDT.BINANCE,ETHUSDT.BINANCE --interval 1d --from 2021-01-01
python examples/crypto.py --symbols BTCUSDT.BINANCE,ETHUSDT.BINANCE --interval 1d
```

`multi_data.py` supplies complete decoders and deterministic temporary samples;
`--dir` switches to existing files without writing samples. It prints stage
timings and source/expanded price bytes. Close-derived signals execute one row
later. Synthetic examples use `.SIM` venues or explicitly synthetic Binance
sample data.

Migration: `phasor.data.load_cached/align/fetch_*` have been removed. Use
`files(..., schema=...).prepare()` for old CSV inputs (the crypto example contains
the decoder). Download/resume/write helpers and interval annualization now live
only in `examples/binance.py`. New fetch filenames include the venue, e.g.
`BTCUSDT.BINANCE-1d.csv`; old files may be passed directly to `files` with an
explicit full BarType key. There is no automatic rename or cache migration.

## Build

```sh
python -m pip install --upgrade 'maturin>=1.9.4,<2.0'
maturin develop --release -m crates/phasor-py/Cargo.toml     # into the active venv
```

`abi3-py39`, so one wheel covers CPython 3.9 and up.

## Tests

```sh
python -m pip install -e 'crates/phasor-py[test,data]'
pytest crates/phasor-py/tests -q
```

The differential tests run the same call in `vectorbt` and here and
compare the records field by field — integers exactly, floats to 1e-12 — over 13
signal cases, 14 order cases, 4 stop configurations and 10 log cases, plus all 28
rows of the statistics table to 1e-9. The log comparison covers **all 37 stored
fields**, which is the widest check in the project: the account on both sides of
the order, the request as it was resolved, and the result. They skip when
`vectorbt` is not installed; the structural ones (layout, dtypes, bounds, error
messages, thread determinism, the log join) always run.

Shared accounts add 48 committed reference cases for both entry points, including
serial/parallel execution, all log fields, account series and statistics. These
fixtures run offline; installing the local `../vectorbt` also enables their live
regeneration check. See [fixture instructions](../../tests/parity/README.md#shared-account-python-boundary).

`cargo test -p phasor-py` covers the string-to-enum mappings, which are tested one
variant at a time because a table that maps `"targetvalue"` to `TargetAmount`
compiles, runs, and produces a merely different backtest.

## The one thing to know

**The engine is column-major; NumPy is row-major by default.** A `(3, 2)` array
of `0..6` is `[0,1,2,3,4,5]` in C order and `[0,2,4,1,3,5]` in F order, so
reading a C-ordered array as column-major silently backtests the *transposed*
price series — which runs, fills orders, and reports an ordinary-looking result.

Neither contiguity nor shape catches it: a C-ordered array is contiguous, and on
a square matrix the transposed reading has the same shape. So the Rust layer
refuses a C-ordered 2-D array and names the fix, and the Python shell calls
`np.asfortranarray` first — free when the array is already F-ordered, a visible
copy when it is not. The guard lives in Rust rather than only in the shell
because `phasor._phasor` is importable.

## What is exposed

Both vectorized loops — `from_signals` and `from_orders` — the `Portfolio` they
return, its metrics, its statistics table, and its order, trade, log and cash
records as NumPy arrays. `Expression`, `FactorSet`, `alpha158()`, and
`alpha360()` expose the Rust factor-expression engine through the same NumPy
boundary.
`phasor.data` supplies Binance Spot download, CSV caching and aligned OHLCV arrays
in Python.

The portfolio also answers how much was at work and how each trade went:
`pf.allocations()`, `pf.long_exposure()`, `pf.short_exposure()`,
`pf.net_exposure()` and `pf.gross_long_short()` return whole `(n_bars, …)`
matrices — per symbol column for allocations, per account for the rest —
and `pf.position_coverage()` the share of bars each column held a position.
`pf.excursions(high, low)` gives each trade's MAE and MFE, aligned with
`pf.trades()`, and `pf.edge_ratio(high, low)` one edge ratio per account. A
portfolio keeps only its closes, so the bar range is passed in; without it
both are measured on closes, which can only understate them.

`from_signals` takes the four stop modes beside `sl_stop`/`tp_stop`:
`stop_entry_price` (`close`, `fillprice`, `price`, `valprice`),
`stop_exit_price` (`stoplimit`, `stopmarket`, `price`, `close`),
`upon_stop_exit` (`close`, `reverse`, `closereduce`, `reversereduce`) and
`upon_stop_update` (`override`, `overridenan`, `keep`). `None` keeps the
engine's default.

Research tools sit in their own modules, beside the backtest rather than in it:

```python
from phasor import events, labels, robust, signals, synthetic

close = synthetic.gbm((1000, 20), mu=0.0, seed=1)    # a no-edge control
entries, exits = signals.random_entries_exits(close.shape, 10, seed=2)

study = events.forward_paths(close, entries, before=5, after=20)
study.mean_path(), study.hit_rate(20)

y, decided_at = labels.triple_barrier(close, up=0.03, down=0.02, max_hold=20)

r = returns_of_a_sweep                               # (n_bars, n_trials)
robust.pbo_cscv(r, n_splits=16)
robust.deflated_sharpe(r[:, best], per_period_sharpes)
```

`events` and `labels` read the future on purpose and must never feed a
simulation. `signals` and `synthetic` draw from `(seed, row, col)`, so a seed
reproduces and a column does not change when others are added. Sharpe ratios
in `robust` are per period, not annualized.

Not yet: the rolling metrics and drawdown records in Python. Each is
an addition on this scaffolding rather than a change to it.

### Logs

`log=True`, or a `(n_bars, n_cols)` mask to log one column of a sweep. Then
`pf.logs()` for the 38-field schema as arrays, `pf.log_reasons()` for the outcome
of each row in words, and `pf.reason_counts(col)` for the tally — which is the
first thing to reach for when a column traded far less than expected, because
there is almost always one reason dominating.

One row per order **considered**, filled or not. `from_orders` submits an order
per element and so logs every element; `from_signals` reaches the kernel only
when the signals resolved to a size, so a quiet bar gets no row. `order_id` joins
a row to the fill it produced, or is `-1`.

Logging is additional, never alternative: the same workload produces identical
order records with it on and off, and a test asserts that.

## The asymmetry between the two loops

`from_orders` accepts all six size types; `from_signals` accepts three. That is
not an oversight in either place — a *target* size says where the position should
end up, which is unambiguous per element and ambiguous against a signal that only
says which way to move. The reference refuses the same three, and a test pins
that so a release lifting the restriction shows up here rather than silently.

`direction` also defaults differently: `both` for orders, where the size's sign
already says which way it goes, and `longonly` for signals. Both match the
reference.

## Two traps this layer inherits

**A column that never traded has an infinite Sharpe.** Zero return over zero
volatility. That is the reference's answer, and it tops a naive `argmax` over a
sweep. `pf.traded_only(values)` masks those columns to `NaN`, which every
reduction skips.

**Each individual metric accessor rebuilds the derivation chain.** The engine's
`Portfolio` borrows its prices and caches its derivations; a `#[pyclass]` cannot
hold a borrow of itself, so the portfolio is rebuilt per call and the cache
starts empty each time. `pf.metrics()` and `pf.stats()` exist as bulk calls for
that reason — six scalars pulled one at a time is six rebuilds.
