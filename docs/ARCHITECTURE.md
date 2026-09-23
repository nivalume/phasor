# barq architecture

[中文版](ARCHITECTURE.zh-CN.md)

This is the single architecture document for barq. It reads top down: why the
project exists, the shape of the whole system, then each stage of the pipeline
in the order data flows through it, then the concerns that cut across stages
(parallelism, the Python boundary, verification, performance), and finally what
is planned and what is known to be missing.

Section numbers are identical in the English and Chinese versions, so a
reference such as `docs/ARCHITECTURE.md §7.3` in a code comment means the same
section in either file.

**Source of truth.** The code is authoritative. Every claim here was checked
against the workspace at commit `be1f38c` (branch `claude/zealous-bardeen-7bz0l5`).
Measured numbers carry the machine and the conditions they were measured under;
they are observations, not guarantees.

---

## Contents

0. [Conventions](#0-conventions)
1. [Goals and motivation](#1-goals-and-motivation)
2. [System overview](#2-system-overview)
3. [Foundations: barq-core and barq-model](#3-foundations-barq-core-and-barq-model)
4. [Data layer: barq-data and barq.data](#4-data-layer-barq-data-and-barqdata)
5. [Factor expressions: barq-expr](#5-factor-expressions-barq-expr)
6. [Signals](#6-signals)
7. [Execution kernel: barq-exec](#7-execution-kernel-barq-exec)
8. [Simulation loops: barq-sim](#8-simulation-loops-barq-sim)
9. [Records: barq-records](#9-records-barq-records)
10. [Analysis: barq-analysis](#10-analysis-barq-analysis)
11. [Parallelism](#11-parallelism)
12. [Python layer: barq-py](#12-python-layer-barq-py)
13. [Research layer: research/barq_lab](#13-research-layer-researchbarq_lab)
14. [Verification](#14-verification)
15. [Benchmarks and performance](#15-benchmarks-and-performance)
16. [Development workflow](#16-development-workflow)
17. [Roadmap and known gaps](#17-roadmap-and-known-gaps)
- [Appendix A. Licensing and provenance](#appendix-a-licensing-and-provenance)
- [Appendix B. Glossary](#appendix-b-glossary)
- [Appendix C. File index](#appendix-c-file-index)

---

## 0. Conventions

- **Rows are time, columns are symbols** (or symbol × parameter combinations).
  `n_rows` is the number of bars; `n_cols` the number of columns.
- **Column-major storage.** A 2-D array is one flat `f64` buffer and element
  `(row, col)` lives at `data[col * n_rows + row]`. One column is one contiguous
  slice (§3.2).
- Paths are relative to the repository root. `crate::module::item` names Rust
  items; `path.rs:function` points at a function in a file.
- "Reference" means the project a behaviour is pinned against: vectorbt 1.1.0 for
  execution, simulation, records and analysis; Qlib for factor expressions.
- "Phase 1" is the vectorized tier that exists today. "Phase 2" is the planned
  event-driven tier (§2.2, §17.1); nothing of phase 2 is implemented.

---

## 1. Goals and motivation

### 1.1 What barq is for

In priority order:

1. **Parallel parameter sweeps.** Thousands of parameter combinations run as
   independent columns of one simulation, spread over all cores.
2. **Parallel backtests across symbols or multi-asset accounts.** Symbols are
   columns too; a multi-symbol account is a contiguous group of columns sharing
   cash (§8.3).
3. **Factor expressions at scale.** A Qlib-style expression language (§5)
   evaluates factor libraries such as Alpha158 over whole panels and feeds the
   result into signals and simulation without leaving the engine.

A secondary aim is that strategies can later be written in NautilusTrader's
style — source-level portability, not a code dependency (§17.1).

### 1.2 Two findings that shaped the design

**Rewriting a simulation loop in Rust does not make it faster.** vectorbt ships
its own Rust backend (`vectorbt-rust`, about 15.6k lines, PyO3). Its published
benchmarks put `simulate_from_orders` at **0.73×–1.15×** against Numba: Numba
already emits LLVM machine code, and a branchy scalar loop gives Rust nothing
extra to exploit. The same backend reaches **6–10×** on `diff`, `pct_change`,
`fshift`, `bshift` at 1000×100 — operations that vectorize. One backend, two
outcomes, split exactly along "does this vectorize". barq's own harness
reproduces the first figure: vectorbt's Rust engine runs at 0.74×–0.82× of its
Numba engine (§15.1).

**The reference event-driven engine cannot sweep in parallel, structurally.**
NautilusTrader's `BacktestNode::run()` is a sequential `for` loop — no rayon, no
`par_iter`, no thread spawn anywhere in its backtest crate. Its state is
`Rc<RefCell<..>>` behind a process-global message bus, so one engine occupies one
process and users sweep by spawning processes. It costs about 1.2–1.9 µs per
event (about 0.86 M events/s replaying, 0.54 M/s running a bar strategy).

So the opportunity is not "the same thing, in Rust". It is parallelism the
reference architectures have foreclosed on, with a factor pipeline feeding it.
The measured outcome (§15.1): the loop is 2.6×–3.3× faster than vectorbt's Numba
loop single-threaded and 9.7×–12.1× on 4 threads. The gap comes from design
choices (column-major scans, stack-local state, `FlexArray`'s scalar fast path,
threads), not from the compiler; which of them contributes how much has not been
measured.

### 1.3 What barq is not

- Not an event-driven engine (yet). There is no order book, no tick data and no
  latency model; execution happens on bars (§2.4).
- Not a data vendor. Library crates never download or write data; network code
  lives only in `examples/` (§4).
- Not an allocator or risk system. Sizing is whatever the caller asks for
  (§7.3); there is no automatic portfolio construction.

---

## 2. System overview

### 2.1 The pipeline

```
                    local CSV / Parquet / catalog
                                │  barq-data (§4)
                                ▼
            BarPanel: time × symbol OHLCV (+ aligned auxiliary series)
                                │  barq-expr (§5)
                                │  parse → compile to graph → evaluate per symbol
                                ▼
            factor matrices: one Matrix (time × symbol) per factor
                                │  signal rules, delay, cleaning (§6)
                                ▼
       entries/exits or target sizes: time × (parameter × symbol) columns
                                │  barq-sim over barq-exec (§7, §8)
                                │  parallel across independent accounts
                                ▼
       records: orders, logs  → trades, positions, drawdowns (§9)
                                │  barq-analysis (§10)
                                ▼
       per-account cash, value, returns, metrics, stats, sweep aggregation
```

Each arrow is a function call on in-memory arrays. There is no dataframe
between stages: factors are `barq_core::matrix::Matrix`, signals and parameters
are borrowed through `FlexArray` (§3.3), records are struct-of-arrays (§9).

### 2.2 Two tiers, unified by records and one execution kernel

A vectorized engine is fast because it has no events, no per-bar objects and no
message bus. An event-driven engine is realistic because it has them. Both
cannot be had on one code path: the vectorized loop costs nanoseconds per column
per bar, the event-driven one microseconds per event — three orders of magnitude,
and the gap *is* the realism.

- **Tier A (phase 1, implemented):** vectorized. Static arrays in, records out.
  Parameters and symbols are columns.
- **Tier B (phase 2, planned):** event-driven, NautilusTrader-style `Strategy`
  objects against a simulated venue, parallel across runs rather than within one.

What unifies the tiers is the output, not the computation. Both write the same
record schemas (§9), so the analysis layer is written once, a vectorized sweep
and an event-driven replay can be compared column by column, and users can
prototype fast and then validate at fidelity. That only holds if both tiers call
the same `barq_exec::execute_order` (§7): if execution forks, Tier A stops being
a proxy for Tier B. Phase 1 therefore keeps two constraints for phase 2: the
kernel is callable for a single bar in isolation, and record writes go through
the `RecordSink` trait (§9.3).

A connecting observation: vectorbt's Rust backend refuses callback-accepting
functions (`_engine.py`: "Rust engine is not implemented for callback-accepting
functions"), so `from_order_func` stays in Numba. vectorbt's
`order_func_nb(OrderContext) -> Order` and Nautilus's `on_bar(&Bar) { submit_order }`
have the same shape; in Rust that is a monomorphized trait call with no
indirection. A Nautilus-style strategy API can therefore double as the native
implementation of `from_order_func`.

### 2.3 Crates

Thirteen workspace crates (`Cargo.toml` `[workspace] members`):

| Crate | Role | Workspace dependencies |
|---|---|---|
| `barq-core` | `Matrix`, `FlexArray`, column labels, float tolerances, `Error` (§3) | none (no external deps either) |
| `barq-model` | execution enums, ids, `BarType`, `OhlcvPanel`, fixed-point `Price`/`Quantity` (§3.6) | core |
| `barq-generic` | rolling/expanding/EWM windows, shifts, fills, crossovers (§5.5, §6) | core |
| `barq-signals` | signal cleaning, exit generation, ranks, ranges (§6.2) | core |
| `barq-indicators` | MA, MSTD, BBANDS, RSI, STOCH, MACD, ATR, OBV with parameter sweeps (§6.4) | core, generic |
| `barq-expr` | factor expression language: parser, graph compiler, native and Polars backends, Alpha158/360 (§5) | core, generic, model |
| `barq-exec` | the order execution kernel (§7) | core, model |
| `barq-records` | record schemas, sinks, trade/drawdown derivation, mapped arrays (§9) | core, exec, model |
| `barq-sim` | `from_orders` / `from_signals` loops, stops, groups, parallel runners (§8) | core, exec, model, records |
| `barq-analysis` | `Portfolio`, value/returns chain, metrics, stats, aggregation (§10) | core, generic, model, records, sim |
| `barq-data` | CSV/Parquet/catalog → Arrow → panels, point-in-time alignment (§4) | model |
| `barq` | façade: re-exports every crate, `prelude`, `SignalSweep` (§6.1) | all of the above; `data` feature pulls in barq-data |
| `barq-py` | PyO3 extension `barq._barq` + the Python package `barq` (§12) | data, analysis, core, expr, generic, model, records, sim |

Layering, bottom to top: `core → model → exec → records → sim → analysis`, with
side branches `generic → indicators / expr`, `signals` (on core only) and `data`
(on model only). `barq-py` does not depend on `barq-signals`, `barq-indicators`
or the `barq` façade; it reaches the kernel through `barq-sim`.

External runtime dependencies are deliberately few:

| Crate | External dependency | Why |
|---|---|---|
| `barq-expr` | `rayon` | per-symbol parallel factor evaluation (§5.4) |
| `barq-expr` (feature `polars`) | `polars` 0.46 | optional second backend (§5.6) |
| `barq-data` | `arrow-*` 59, `parquet` 59, `csv` | reading local data (§4) |
| `barq-py` | `pyo3` 0.29 (abi3-py39), `numpy` 0.29, arrow `ffi` | the Python boundary (§12) |

The simulator itself (`barq-sim`) uses only `std::thread::scope` (§11.2).

### 2.4 Scope decisions

- **Accounts are independent by default.** Every column is its own account with
  its own cash. `cash_sharing=True` with `group_lens` makes contiguous columns
  share one account, so one parameter set can hold several symbols while
  parameter sets stay isolated and parallel (§8.3).
- **Orders are sequential within an account.** Within a bar, a shared account
  executes its columns in a call sequence; `call_seq="auto"` sorts by estimated
  order value so sells come before buys (§8.3).
- **Metrics are per account; records stay per symbol.** Returns and metrics are
  computed from account equity; orders, trades and logs keep their symbol
  column (§10).
- **Bars only.** Execution happens at bar prices. Funding, news and other event
  datasets can be factor inputs through point-in-time alignment (§4.4); ticks
  and order books are out of scope.
- **Rust engine, Python boundary.** Dynamic schemas, strings and Python
  callbacks stay outside the simulation loops. User decoders run while loading
  data (§4.2), before any simulation.
- **Sweeps do not copy prices.** Source OHLCV stays `[T, S]`; logical sweep
  columns are mapped onto source columns with `FlexArray::MappedColumns`
  (`column_map` in Python) in both execution and analysis (§3.3, §12.3).

---

## 3. Foundations: barq-core and barq-model

### 3.1 The column axis

Parameters and symbols share one flat column axis:

```
col = param_idx * n_symbols + symbol_idx
```

Three MA windows (10, 20, 50) over BTC and ETH give six columns:

| col | 0 | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| window | 10 | 10 | 20 | 20 | 50 | 50 |
| symbol | BTC | ETH | BTC | ETH | BTC | ETH |

- Parameter labels are **repeated** (`aabbcc`, `Level::repeat`) and symbol labels
  **tiled** (`abcabc`, `Level::tile`) in `crates/barq-core/src/columns.rs`.
- **Contiguity.** One parameter combination occupies adjacent columns, so a
  shared-cash account is a contiguous slice: `group_lens=[2,2,2]` gives three
  accounts of two symbols each.
- **Uniformity.** The simulator only knows the shape and the group lengths; it
  never needs to know which column is which parameter.
- The layout is a convention set by constructors (`Level::repeat`/`tile`,
  `barq_indicators::sweep::column_of`, `barq::sweep::SignalSweep`,
  `barq.strategy.sweep`), not checked at runtime. A test in `columns.rs`
  (`one_parameter_combination_occupies_contiguous_columns`) pins it.

**Labels.** `Columns { levels, n_cols }` carries named levels. A `Label` is
`Int | Float | Str | Bool` (floats compare by bit pattern). `Columns::group_indices(level)`
returns `(label, [cols])` in first-seen order and is the basis of by-parameter
aggregation (§10.5): 2,000 ungrouped columns produce 2,000 Sharpe ratios, and
`group_by(columns, "window", values, Reduce::Mean)` reduces them per window;
`best_column` reports the winner as `window=20, symbol=BTC`. `Columns::level`
errors with the list of available levels; `new` rejects levels of different
widths. Labels live in the foundation crate because factor screening depends on
them.

### 3.2 Memory layout

- `barq_core::matrix::Matrix { data: Vec<f64>, n_rows, n_cols }` is column-major:
  `column(c)` is a contiguous `&[f64]` with no copy.
- Why column-major: a column is the unit of work. An ungrouped simulation scans
  one column from the first bar to the last; a factor is evaluated one symbol at
  a time (§5.4); a thread takes a range of columns without touching another
  thread's cache lines. A shared account advances a group of adjacent columns
  bar by bar.
- `barq_model::OhlcvPanel { open, high, low, close, volume, n_rows, n_cols }` is
  struct-of-arrays: each field is its own column-major buffer, because the stop
  engine and factor inputs read fields separately. An empty field means "not
  provided".
- NumPy is row-major by default, so every array that crosses the Python
  boundary must be Fortran-ordered (§12.2). Fixture generators reshape with
  `order="F"` for the same reason.

### 3.3 Broadcasting with `FlexArray`

Price, fees, slippage, stops, sizes and most other parameters may be a scalar,
one value per column, one value per row, or a full matrix. Materializing
everything would cost `n_rows × n_cols × 8` bytes per parameter — 1.6 GB for
100k bars × 2,000 columns, and there are a dozen such parameters. Instead
`barq_core::flex::FlexArray<'a, T>` borrows what it was given:

| Variant | Meaning | `get(row, col)` reads |
|---|---|---|
| `Scalar(T)` | one value everywhere | the value (no memory access) |
| `PerColumn(&[T])` | one value per column | `values[col]` |
| `PerRow(&[T])` | one value per bar | `values[row]` |
| `MappedColumns { data, n_rows, columns }` | logical column → source column of a shared matrix | `data[columns[col] * n_rows + row]` |
| `Full { data, n_rows }` | a full column-major matrix | `data[col * n_rows + row]` |

- `get` is one `#[inline]` match. `varies_by_row()` lets loops hoist per-column
  constants out of the bar loop (§8.1).
- Checked constructors (`scalar`, `per_column`, `per_row`, `full`, `mapped`)
  validate lengths and return `Error::FlexLen` on mismatch.
- `MappedColumns` is what makes parameter sweeps cheap: 50 parameter sets over
  100 symbols need 5,000 logical price columns, but only the 100 source columns
  exist in memory. Python exposes it as `column_map` (§12.3), and
  `barq.strategy.sweep` builds it automatically (§6.3).

### 3.4 The float tolerance layer

`crates/barq-core/src/math.rs` is small but it is the most important piece of
numerical policy in the workspace:

- `REL_TOL = 1e-9`, `ABS_TOL = 1e-12`, matching the reference.
- `is_close(a, b)` follows PEP 485 `isclose`, with one deliberate deviation:
  **NaN and any infinity are never close, not even `inf` to `inf`.** Infinity is
  a live sentinel in the kernel — `size = +inf` means "as much as possible",
  `price = +inf` means "use the close" — and it must never compare as a normal
  number.
- `is_close_or_less(a, b)` is `is_close || a < b`; `is_less(a, b)` is
  `!is_close && a < b`. Affordability decisions use these, never raw `<=` (§7.5).
- `add(a, b)` and `is_addition_zero` return exactly `0.0` when `a + b` is close to
  zero (comparing magnitudes when signs differ). Every running-balance update in
  the kernel uses `add`, so cash and position never carry `1e-17` dust.
- The workspace lint `clippy::float_cmp = "deny"` forbids `==` on floats;
  intentional exact comparisons carry a local `#[allow]` with a reason.

### 3.5 Errors versus rejections

`barq_core::error::Error` has five variants: `Broadcast`, `FlexLen`,
`UnknownLevel`, `LevelWidth` and `Invariant(String)`. An `Err` means an
invariant was violated — negative cash, a non-finite position, a long-only column
holding a short, an invalid parameter — and the run cannot continue meaningfully.

**A rejected order is never an `Err`.** An order that cannot be filled (no
cash, below the minimum size, a NaN price) returns an `OrderResult` whose
`status` is `Ignored` or `Rejected` and whose `status_info` says why (§7.6). "Why
didn't it trade?" is the first question a user asks, so every reason is
distinct and recorded (§9.4).

### 3.6 Domain model (barq-model)

- **Execution enums** (`crates/barq-model/src/enums.rs`) are `#[repr(i8)]` with
  discriminants pinned by a test, because records store them as integers:
  `SizeType {Amount, Value, Percent, TargetAmount, TargetValue, TargetPercent}`,
  `Direction {LongOnly, ShortOnly, Both}`, `OrderSide {Buy, Sell}`,
  `OrderStatus {Filled, Ignored, Rejected}`, and `StatusInfo` with 14 reasons
  (§7.6). Signal-pipeline enums (`AccumulationMode`, `ConflictMode`,
  `DirectionConflictMode`, `OppositeEntryMode`), stop enums (`StopEntryPrice`,
  `StopExitPrice`, `StopExitMode`, `StopUpdateMode`) and record enums
  (`TradeDirection`, `TradeStatus`, `TradesType`) follow vectorbt's semantics.
- **Identifiers** (`ids.rs`): `Symbol`, `Venue` (both `Arc<str>` newtypes) and
  `InstrumentId { symbol, venue }`, displayed as `SYMBOL.VENUE`. Parsing splits
  on the last dot and rejects whitespace, control characters and path
  separators. They are cheap to clone but not interned yet (§17.4).
- **`BarType`** (`bar.rs`) formats as `SYMBOL.VENUE-STEP-AGGREGATION-PRICE-SOURCE`,
  for example `BTCUSDT.BINANCE-1-MINUTE-LAST-EXTERNAL`. Parsing uses
  `rsplitn(5, '-')`, so symbols may contain `-`. It is the key format of the data
  catalog (§4.1).
- **Fixed-point values** (`types.rs`): `Price { raw: i64, precision }` and
  `Quantity { raw: u64, precision }` with 9 decimal places (`PRECISION = 9`),
  following NautilusTrader's shape. The raw types are aliases so a later
  16-decimal `i128` variant is a type change, not a redesign. They exist for
  phase 2; the vectorized tier works in `f64` throughout.
- The two lineages are deliberate: execution semantics follow vectorbt, value and
  identifier types follow NautilusTrader (§17.1).

---

## 4. Data layer: barq-data and barq.data

`crates/barq-data/src/lib.rs` turns local files into panels. It never downloads
and never writes; download and cache code lives in `examples/`
(`binance.py`, `fetch.py`, `download_binance_vision.py`).

### 4.1 Catalog layout

```
<root>/data/<name>/<key>/*.csv | *.parquet
```

- `name` is a dataset (`bars`, `funding`, `news`, …); `key` is a `BarType`
  string for bars (`BTCUSDT.BINANCE-1-HOUR-LAST-EXTERNAL`) or an `InstrumentId`
  for other data. Both must be a single path component.
- `barq_data::catalog(root, name, keys)` lists the files, sorted. The default
  Python root is `~/.cache/barq/catalog`; `examples/download_binance_vision.py`
  fills it with monthly Parquet files from Binance's public archive.

### 4.2 Schema and loading

A `DataSchema { name, input, output, event_time, available_time, instrument }`
describes one dataset:

- `input` — the Arrow schema to read from files (the projection).
- `output` — the schema after decoding. `event_time` and `available_time` must be
  `UInt64` UTC nanoseconds. `available_time` is when the value became known and
  is the time every filter and join uses — this is what prevents look-ahead
  with late-published data.
- `instrument` — `Source` (the key names the instrument), `Columns { symbol, venue }`
  (per-row instrument columns, `Utf8`) or `Global` (applies to every
  instrument).

`load(schema, items, start, end, decode)` then, for each file:

1. streams `RecordBatch`es of 65,536 rows (Parquet with a projection mask; CSV
   with typed fields from `input`, falling back to `Utf8`);
2. projects to `input` and passes each batch to the caller's `decode` closure —
   native Rust, or a Python callback through the Arrow C Data Interface (§12.4);
3. projects to `output`, checks that time columns have no nulls and that row
   instruments agree with the key's `BarType`;
4. keeps rows with `start <= available_time < end`.

The result is a `Dataset { schema, keys, batches }`.

### 4.3 Bars to a panel

`Dataset::prepare()` builds a `BarPanel { time, bar_types, values: OhlcvPanel }`:

- every key must parse as a `BarType`; OHLCV columns must be `Float64`;
- the time axis is the sorted union of all `available_time` values;
- each symbol's bars are written column-major at `col * n_rows + row`; a missing
  bar is `NaN`; a duplicate bar is an error.

### 4.4 Point-in-time alignment of auxiliary data

`Dataset::align(time, instruments, field, window, empty)` joins any numeric
field onto a target time axis (usually the bar panel's):

- The field is cast to `f64` and grouped by instrument (the global key for
  `Global` data), sorted by `available_time`.
- **Without a window** it is an as-of join: at time `t`, the last value with
  `available_time <= t`.
- **With `(width, agg)`** it aggregates the window `(t - width, t]` with `Sum`,
  `Count` or `Mean`. Sum and mean skip non-finite values; `empty` fills windows
  with no finite value.
- Output is column-major `time.len() × instruments.len()`; global events are
  broadcast to every instrument.

In Python these are `Dataset.asof(panel, field=…)` and
`Dataset.window(panel, field=…, window="30m", agg="mean")`.
`examples/multi_data.py` joins funding (as-of) and news (windowed mean) onto
1-minute bars and uses them in an entry gate.

### 4.5 The Python data API

`crates/barq-py/python/barq/data.py` wraps the Rust layer:

- `DataSchema(name, input=pa.schema, output=pa.schema, decode=fn, event_time, available_time, instrument)`;
- `files({key: path | [paths]}, schema=…)` and
  `Catalog(root, schemas=[…]).source(name, keys=…)` → `Source` →
  `.load(start, end)` → `Dataset`, or `.prepare()` → `BarPanel`;
- `BarPanel` exposes `time`, `bar_types`, `instruments`, `open … volume` as
  `(T, S)` Fortran-ordered arrays, `shape`, `coverage()`.

Loading, `prepare` and `align` release the GIL (§12.5); a Python decoder
re-acquires it per batch.

---

## 5. Factor expressions: barq-expr

`barq-expr` computes factor matrices from named expressions:

```rust
let factors = FactorSet::compile([("trend", "$close / Mean($close, 20) - 1")])?;
let values = factors.evaluate(&FactorInputs::from_ohlcv(&panel)?)?;
let trend: &Matrix = values.get("trend").unwrap();   // time × symbol
```

A factor is information, not a decision. Turning it into orders — thresholds,
ranks, a one-bar delay, cleaning — is the job of §6. Factor matrices can also be
exported for model training outside the crate; a model's output is just another
score matrix.

### 5.1 The language

- **Fields** are `$name`, lower-cased: `$open $high $low $close $volume` come from
  `FactorInputs::from_ohlcv`; any other field (`$vwap`, funding, …) is added with
  `FactorInputs::with_field(name, &[f64])` and must have `n_rows × n_cols`
  values. A missing field is an error at evaluation time.
- **Numbers** are decimal or scientific literals.
- **Infix operators**, lowest to highest precedence: `|`, `&`, comparisons
  (`> < >= <= == !=`), `+ -`, `* /`, unary `+ -`, `**` (right-associative).
  Comparisons and boolean operators return `1.0`/`0.0`.

Functions (arity checked by `parse.rs:validate`, semantics in `graph.rs` and
`eval.rs`):

| Group | Functions | Notes |
|---|---|---|
| Elementwise | `Abs Sign Log Not` (1 arg); `Add Sub Mul Div Power Gt Ge Lt Le Eq Ne And Or` (2); `Greater Less` (2); `If(cond, a, b)` | `Greater`/`Less` are elementwise max/min and return NaN if either side is NaN; function and operator spellings are the same node |
| Time shift | `Ref(x, n)`, `Delta(x, n)` | `Ref` is row-based (also on irregular calendars); negative `n` reads the future and is reported by `FactorSet::uses_future()`; `Delta` requires `n >= 0` |
| Smoothing | `EMA(x, p)` | `p < 1` is the alpha; `p >= 1` a span (`alpha = 2/(p+1)`); `p = 0` is an expanding EMA |
| Rolling `(x, N)` | `Mean Sum Std Var Max Min Med Mad Count Skew Kurt WMA Rank IdxMax IdxMin Slope Rsquare Resi` | `Rank` is the percentile of the newest value within its own trailing window (time-series, not cross-sectional); `IdxMax`/`IdxMin` return the 1-based position; `Slope/Rsquare/Resi` regress on positions `1..=N` |
| Rolling `(x, N, q)` | `Quantile` | `q` in `0..=1`, linear interpolation |
| Pairwise `(x, y, N)` | `Corr Cov` | |

Window rules: `min_periods = 1` (a window with at least one observation
produces a value), `N = 0` means the whole history (expanding), and missing
observations stay `NaN`. Window lengths, offsets, quantiles and EMA parameters
must be numeric literals.

**Libraries.** `alpha158()` and `alpha360()` compile Qlib's `Alpha158DL` and
`Alpha360DL` feature sets. Both need `open/high/low/close/volume` plus an
explicit `vwap` field. They return the raw features; Qlib's `DataHandlerLP`
normalization, fill and labels are out of scope.

**Out of scope today:** cross-sectional operators (`CSRank`, `CSZScore`,
neutralization), cross-instrument references, masks and resampling (§5.7).

### 5.2 Parsing

`crates/barq-expr/src/parse.rs` lexes the source (at most 16,384 bytes) and parses
it with a Pratt parser (nesting depth at most 128) into
`Expression { Number | Field | Unary | Binary | Call }`. `validate` rejects
unknown functions and wrong arity. `FactorSet::compile` rejects empty or duplicate
factor names and empty sets. `Expression` is kept on the `FactorSet` for
`required_fields()`, `uses_future()` and the Polars backend.

### 5.3 Compiling to a dependency graph

`FactorSet::compile` then builds one `graph::Graph` for the whole set
(`crates/barq-expr/src/graph.rs`):

1. **Hash-consing.** Every node is interned on `(Op, [input node ids])`. A
   subexpression shared by many factors — `Ref($close, 1)` appears in dozens of
   Alpha158 factors — becomes one node, and so do different spellings of the same
   thing (`a+b` and `Add(a, b)`, `+x` and `x`).
2. **Compile-time validation.** Literal parameters are parsed into typed
   operators: `Ref(i64)`, `Delta(usize)`, `Ema(Option<alpha>)`,
   `Rolling(kind, Window)`, `Pair { corr, window }`. A fractional or negative
   window, a quantile outside `0..=1`, a negative EMA span, a negative `Delta`
   offset or a non-literal parameter fails here, and the message names the
   factor. `Window::All` stands for `N = 0`.
3. **Constant folding.** An elementwise operator whose operands are all literals
   becomes a literal (`-(1+2)*3` → `-9`).
4. **Dependency order.** Nodes are appended after their inputs, so the node list
   is already a valid evaluation order.
5. **Scratch-slot scheduling** (`Graph::schedule`). Each computed node gets a
   scratch slot. A slot is released after the last node that reads it and reused
   by a later node; a node is never given the slot of one of its own inputs.
   Fields and literals take no slot (they are read in place or broadcast).

Alpha158 compiles to 357 nodes that need 18 slots. A symbol's whole factor set is
therefore evaluated with 18 series of scratch, whatever the number of factors.

### 5.4 Native evaluation

`FactorSet::evaluate` (the `NativeBackend`) runs `eval.rs:evaluate`:

1. **Resolve fields.** Look up every field the graph reads; a missing one is an
   error before any work starts. The parallel section below has no error path.
2. **Allocate outputs.** One `n_rows × n_cols` buffer per factor, allocated
   separately so the pages are zeroed lazily by the threads that write them (a
   single cloned zero buffer cost 350 ms serially at 2520×200×158).
3. **Split by column.** Each output is cut into per-column `&mut [f64]` slices and
   regrouped so that task `c` owns column `c` of every output — disjoint writes
   with no locks and no `unsafe`.
4. **Evaluate symbols in parallel.** `columns.into_par_iter().enumerate().for_each_init(…)`
   (rayon). Each worker thread gets its own scratch (`slots` series of length
   `n_rows`), reused across the columns it processes. For one column,
   `run_column` walks the nodes in order:
   - a field is a slice of the caller's input (no copy); a literal is a scalar;
   - elementwise nodes run one tight loop per operator and operand shape
     (`binary_series` dispatches the operator outside the loop);
   - time-series nodes call the kernels of §5.5;
   - when a node is some factor's output, its series is copied into that
     factor's column (several factors can share one node).
5. **Assemble** `FactorValues` (names plus one `Matrix` per factor).

Consequences: parallelism is over symbols, one region per evaluation. Every
intermediate is one symbol's series (about 20 KB at 2,520 rows) instead of a
panel, so peak memory is essentially the outputs. A panel with fewer columns
than cores cannot use every core (§11.1). A test
(`each_symbol_evaluates_exactly_as_it_would_alone`) checks that each column of a
multi-symbol evaluation is bit-identical to evaluating that symbol alone.

### 5.5 Kernels

| Operators | Kernel | Cost per row |
|---|---|---|
| `Mean` | prefix sums, `barq_generic::rolling::rolling_mean_1d` | O(1) |
| `Max Min` | monotonic deque, `barq_generic::rolling` | amortized O(1) |
| `Sum Std Var Skew Kurt WMA Slope Rsquare Resi Corr Cov` | incremental accumulators, `window.rs` | O(1) |
| `Quantile Med Rank Mad` | Fenwick tree over sorted positions, `order.rs` | O(log n) |
| `IdxMax IdxMin` | monotonic queue, `order.rs:rolling_arg_extreme` | amortized O(1) |
| `Count` | `rolling_apply_1d` | O(window) |
| `EMA` | adjusted recurrence; expanding form | O(1); O(n) per row for `EMA(x, 0)` |

**Incremental accumulators** (`window.rs`) add the row entering the window and
subtract the row leaving it. Three measures keep that numerically honest:

- **Periodic rebuild.** The sums are recomputed from the window every `window`
  rows, so add/subtract rounding never accumulates over more than two windows.
  The rebuild costs O(window) once per window rows, so the total stays O(n).
- **Shifted sums.** Values are summed as offsets from a shift (the first value
  pushed into an empty window), so the variance of a price near 100 is not the
  small difference of two large squares.
- **Exact constant windows.** Qlib's reference makes exact zero checks
  (`Corr`'s `va == 0`, `Rsquare`'s `yvar <= 0`). Running sums over identical
  values leave a rounding residue instead of zero, so `Moments` counts trailing
  identical observations and reports exactly zero variance when the whole window
  is constant.

`NaN` rows are skipped: they occupy a window slot but add nothing.

**Order statistics** (`order.rs`) sort each column once, give every row a fixed
position in that order, and keep a Fenwick tree over positions marking which rows
are in the window. Entering and leaving is a point update, the k-th smallest
value is a tree descent, and "how many values are below x" is a prefix query.
Counts and positions are integers, so `Quantile`, `Med` and `Rank` are
bit-identical to sorting the window. `Mad` also keeps partial sums and reports
exactly zero for a constant window.

**Exactness.** Unit tests compare every incremental kernel with a full rescan of
each window on stress series (NaN runs, all-NaN stretches, constant runs, large
levels with small moves, noise) for window lengths from 1 to beyond the series,
at `1e-9` relative tolerance (`incremental_windows_match_a_full_rescan`,
`incremental_pairs_match_a_full_rescan`). The Qlib fixtures (§14.3) pin the end
result at `1e-6` (`1e-4` for `CORR*`/`CORD*`).

### 5.6 The Polars backend

With the `polars` feature, `set.evaluate_with(&inputs, &PolarsBackend)` evaluates
the same set with Polars (`crates/barq-expr/src/polars_backend.rs`). Backend
choice is always explicit; `ComputeBackend` is the extension point.

- The panel is converted to a long `DataFrame` with a `__barq_expr_symbol`
  column. Each factor's `Expression` (not the graph) is compiled to a Polars
  lazy expression evaluated `.over(symbol)`, in batches of 64 factors.
- Standard rolling operations map to native Polars rolling functions; Qlib-specific
  ones (`Rank`, `IdxMax/IdxMin`, `Slope/Rsquare/Resi`, `Skew/Kurt`, `Mad`,
  `Count`, `WMA`) use `rolling_map` with the reference rescan implementations.

It is much slower than the native backend (measured in this workspace, best of 3,
4 threads):

| Panel | Workload | Native | Polars | Polars / Native |
|---|---|---|---|---|
| 1024 × 16 | 4 elementwise factors | 0.3 ms | 5.0 ms | 15× |
| 2520 × 200 | 6 rolling factors (window 20) | 31 ms | 102 ms | 3.3× |
| 2520 × 200 | Alpha158 | 0.60 s | 6.2 s | 10× |
| 2520 × 200 | Alpha360 | 0.41 s | 2.7 s | 6.7× |
| 2520 × 500 | Alpha158 | 1.5 s | 19.4 s | 13× |

The costs are `.over(symbol)` regrouping on every operator, O(window) UDFs where
native kernels are O(1) or O(log n), batch boundaries that prevent sharing, and
`Option<f64>` conversions. The backend's value today is as an independent
implementation for cross-checking: the Qlib fixtures run through both backends.

### 5.7 Memory, limits and planned work

Measured on the 4-core container used for development (noisy; relative
comparisons were made by alternating builds in one sitting):

| Alpha158 | Result |
|---|---|
| 2520 × 200, 4 threads | about 0.8–1.0 s depending on machine load |
| 2520 × 16 | about 75 ms (was 160 ms before the graph evaluator) |
| peak RSS at 2520 × 500 | about 1.6 GB, almost entirely the 158 output matrices |

Two limits follow from the design:

- **All outputs are held at once.** A full A-share panel (5,000 symbols × 2,520
  days) would need about 16 GB for Alpha158's outputs alone. Planned: evaluate in
  factor batches and hand each batch to a caller-supplied sink or Arrow/Parquet
  writer (§17.2).
- **Cross-sectional operators need stages.** Per-symbol parallel evaluation is
  valid only while every operator reads a single symbol's series. `CSRank` and
  friends would split the graph at those nodes: symbols in parallel within a
  stage, one cross-sectional pass over all symbols between stages (§17.2).

---

## 6. Signals

Signals turn factor matrices (or anything else) into per-bar entry/exit booleans
over the sweep column axis, which `from_signals` consumes (§8.2).

**The one rule that matters: delay.** A factor computed from bar `t`'s close is
known only after bar `t` closes. A signal that trades on bar `t` using it is
look-ahead. Every helper below either delays by default or makes the delay an
explicit parameter. `crates/barq/tests/factor_path.rs` shows the manual path:
factor → one-bar delay → thresholds → `clean_enex` → `Portfolio::from_signals`.

### 6.1 `SignalSweep` (Rust)

`barq::sweep::SignalSweep::from_factor((name, &Matrix), symbols, cases, delay, rule)`
(`crates/barq/src/sweep.rs`):

- evaluates `rule(factor[row - delay, symbol], &params) -> (entry, exit)` for
  every case × symbol;
- the first `delay` rows and non-finite scores emit no signal; signals are left
  uncleaned (the simulator's conflict rules handle them);
- columns are labelled with levels `factor` and `case` (repeated) and `symbol`
  (tiled): `col = case_idx * n_symbols + symbol_idx`;
- `expand(&close)` repeats close per case, and `args(&swept_close)` returns a
  `FromSignals` with shape, close, entries and exits filled — the caller adds the
  rest with struct-update syntax.

`crates/barq/examples/factor_sweep.rs` runs momentum → 2 cases × 3 symbols →
`Portfolio::from_signals` with `available_parallelism()` threads.

### 6.2 barq-signals

Flat column-major `&[bool]` utilities (`crates/barq-signals/src`):

- `clean_enex(entries, exits, n_rows, n_cols, FirstSignal)` produces a strictly
  alternating entry/exit sequence; a bar with both produces neither.
- `generate_exits`, `exit_after(entries, bars, …)`, `exit_on(entries, condition, …)`
  generate exits from entries with a chosen `ExitWindow`; `generate_alternating`.
- `signal_rank`, `partition_rank`, `nth_index`, `normalized_average_index`;
  `partition_ranges`, `between_ranges`, `between_partition_ranges` produce
  `RangeRecords`.
- Random signal generators are deliberately absent.

### 6.3 Python function strategies

`crates/barq-py/python/barq/strategy.py`:

```python
@barq.strategy
def ma_cross(data, fast, slow):
    f = barq.factors(data, fast=f"Mean($close, {fast})", slow=f"Mean($close, {slow})")
    return barq.crossed_above(f["fast"], f["slow"]), barq.crossed_above(f["slow"], f["fast"])

pf = ma_cross.sweep(panel, params=[{"fast": 5, "slow": 20}, …], threads=4)
```

- `factors(data, **expressions)` compiles, **rejects expressions that read the
  future** (`uses_future`), pulls required fields from a mapping or `BarPanel` and
  evaluates.
- `.sweep(data, params, signal_delay=1, **options)` calls the function once per
  parameter set, checks the boolean shapes, delays signals by `signal_delay`
  bars, concatenates columns (`param_idx * S + symbol`), passes
  `column_map = tile(arange(S), n_params)` so prices are never copied, forwards
  OHLC for stops, sets `group_lens = [S] * n_params` when `cash_sharing=True`
  (one shared account per parameter set), and calls `from_signals`.
- `.run(data, params=None)` is a one-parameter sweep.

### 6.4 Indicators

`barq-indicators` computes `ma`, `mstd`, `bbands`, `rsi`, `stoch`, `macd`,
`true_range`/`atr` and `obv` over parameter grids. `sweep_columns` and
`column_of(comb, input_col, n_input) = comb * n_input + input_col` give outputs
the column layout of §3.1, and a per-indicator cache computes shared
intermediates once. Indicators are Rust-only and single-threaded.

---

## 7. Execution kernel: barq-exec

`barq_exec::execute_order(state, order, reject_draw)` (`crates/barq-exec/src/kernel.rs`)
executes one order against one account state at one bar. It is the semantic core
of the engine: both simulation loops call it, and phase 2 will too (§2.2).

### 7.1 State and request

- `ExecuteOrderState { cash, position, debt, free_cash }` — `position` is signed
  (negative is short).
- **`debt` and `free_cash`.** `debt` is the proceeds of open shorts at their
  average entry price. Opening a short moves `free_cash` by
  `proceeds - 2 * short_value`: the proceeds are credited but twice the short
  value is reserved as collateral. Covering unwinds proportionally through
  `avg_entry_price = debt / |position|`. This reservation is path-dependent, so
  it cannot be derived from cash and position alone — hence explicit state.
- `ProcessOrderState` adds `val_price`, `value`, and the order and log counters
  (`oidx`, `lidx`) used by the loops.
- `Order { size, price, size_type, direction, fees, fixed_fees, slippage, min_size, max_size, size_granularity, reject_prob, lock_cash, allow_partial, raise_reject, log }`
  with defaults `size = +inf`, `price = +inf`, `Amount`, `Both`,
  `max_size = +inf`, `size_granularity = NaN`, `allow_partial = true`,
  `min_size = 0.0`. (The simulation loops default `min_size` to `1e-8`, §8.4.)
- `OrderResult { size, price, fees, side, status, status_info }`.

### 7.2 Validation

1. **Zero-snap.** `cash`, `position`, `debt`, `free_cash`, `val_price` and `value`
   are snapped to exactly zero if `is_close` to zero (§3.4).
2. **Ignored before validation:** NaN size → `Ignored/SizeNaN`; NaN price →
   `Ignored/PriceNaN`.
3. **Bar validation** (`validate_bar`): cash finite and `>= 0`, position finite,
   debt finite and `>= 0`, free cash not NaN, price finite and `> 0`, no short
   position in a long-only column and no long position in a short-only column.
4. **Parameter validation** (`param_violation`): fees and fixed fees finite,
   slippage `>= 0`, `min_size >= 0`, `max_size > 0`, granularity NaN or finite
   `> 0`, `reject_prob` in `[0, 1]`.

Any violation is `Err(Error::Invariant)`. `ValidatedOrder` caches the parameter
half so a loop validates a column's constant parameters once and only swaps the
per-bar fields (`set_bar`). The kernel's comments measure this at 12–15%; the
benchmark attribution in `benchmarks/README.md` records +30% on sparse and +22%
on dense workloads.

### 7.3 The size cascade

Sizes are normalized to a signed amount in a fixed order:

1. `ShortOnly` flips the sign of `size`.
2. `TargetPercent × value → TargetValue`. NaN value → `Ignored/ValueNaN`; value
   `<= 0` → `Rejected/ValueZeroNeg`.
3. `Value / val_price → Amount`, `TargetValue / val_price → TargetAmount`. An
   infinite or non-positive `val_price` is an error; NaN → `Ignored/ValPriceNaN`.
4. `TargetAmount - position → Amount`.
5. `-inf` under `ShortOnly`/`Both` becomes `Percent(-1)`.
6. `Percent` becomes `percent = |size|`, `size = ±inf`, type `Amount`.
7. The sign dispatches: `size > 0` → buy; otherwise (including 0 and NaN) → sell,
   as in the reference.

### 7.4 Buying and selling

**Buy** (`kernel.rs:buy`):

- `adj_price = price * (1 + slippage)`.
- The cash limit is `cash`, or with `lock_cash` a closed form over `free_cash`
  that also covers the short being closed (three branches), capped by `percent`.
- `ShortOnly` caps the size at `-position` (cover only).
- Fees sit inside the cash cap: `req_cash = size * adj_price`,
  `fees_paid = req_cash * fees + fixed_fees`. If
  `is_close_or_less(req_cash + fees_paid, cash_limit)` fails, the affordable
  notional is back-solved as `(cash_limit - fixed_fees) / (1 + fees)`
  (`CantCoverFees` if `<= 0`), and `size_granularity` is applied again after the
  resize — granularity is applied twice.
- Then `max_size` (reject or clamp with `allow_partial`), `SizeZero`,
  `MinSizeNotReached` (`is_less`) and `PartialFill`.
- Covering a short releases debt proportionally and adds `2 * debt_diff` back to
  `free_cash`.

**Sell** (`kernel.rs:sell`):

- `adj_price = price * (1 - slippage)`; `LongOnly` caps the size at the position.
- With `lock_cash` or an infinite percent, the largest short is bounded by
  `free_cash` plus long proceeds (`NoCashShort`, `CantCoverFees`).
- `max_size`, `NoOpenPosition`, granularity, `SizeZero`, `MinSizeNotReached`,
  `PartialFill`, and `CantCoverFees` when proceeds net of fees are negative.
- A new short adds `debt += short_value` and reserves `2 * short_value` of
  `free_cash`.

### 7.5 Why the tolerance matters here

Affordability is decided by `is_close_or_less(total_req_cash, cash_limit)`, not
`<=`. The two differ only inside the `1e-9` band, and a mutation replacing one
with the other initially survived the tests. `size_granularity` turns that
invisible difference into a whole unit (5.0 filled versus 4.0), so fixtures built
with `math.nextafter` at the boundary now catch it (§14.4).

### 7.6 Outcomes and rejection reasons

`OrderStatus` is `Filled`, `Ignored` or `Rejected`. `StatusInfo` has 14 reasons:
`SizeNaN`, `PriceNaN`, `ValPriceNaN`, `ValueNaN`, `ValueZeroNeg`, `SizeZero`,
`NoCashShort`, `NoCashLong`, `NoOpenPosition`, `MaxSizeExceeded`, `RandomEvent`,
`CantCoverFees`, `MinSizeNotReached`, `PartialFill`. With `raise_reject` a
rejection becomes an error in the loops.

**Random rejection** (`reject_prob`) is supported by the kernel with an inverted
RNG: the caller supplies the uniform draw, so parallel runs stay deterministic.
The draw is compared after the fill is computed; a hit returns
`Rejected/RandomEvent` with the original state. Neither simulation loop wires it
up yet (§17.4).

---

## 8. Simulation loops: barq-sim

`barq-sim` runs the kernel over a whole panel. There are two loops — orders and
signals — each in plain, logged, serial and parallel variants.

### 8.1 `from_orders`

`FromOrders<'a>` (`crates/barq-sim/src/from_orders.rs`) holds `n_rows`, `n_cols`
and every input as a `FlexArray`: `init_cash`, `close`, `size`, `price`,
`size_type`, `direction`, `fees`, `fixed_fees`, `slippage`, `min_size`,
`max_size`, `size_granularity`, `lock_cash`, `allow_partial`, `raise_reject`,
`val_price`, `log`, plus `ffill_val_price`, `update_value`, `groups` and
`call_seq`. One element is one order request (NaN size = no order).

Per account group (`simulate_group`):

1. **Once per column:** build a `ValidatedOrder` from the column's parameters;
   rebuild per bar only if a parameter varies by row.
2. **Per bar, pass 1 — prices.** For every column in the group resolve the order
   price (`+inf` → this bar's close, `-inf` → previous close) and the valuation
   price (`+inf` → the order price, `-inf` → previous close, forward-filled when
   `ffill_val_price`).
3. **Group value.** For a shared group, compute the account value once at bar
   open: cash plus `Σ position × last_val_price`. Every order in the bar is sized
   against this bar-open value; it is threaded through the bar, not re-summed
   after each fill.
4. **Call sequence.** Resolve the execution order for the bar (§8.3).
5. **Pass 2 — execution.** Execute each column through `execute_validated`; with
   `update_value`, re-mark the value after each fill. Push fills to the record
   sink and, if `log` is set, a log row (§9).

Entry points: `simulate_from_orders`, `simulate_from_orders_logged`,
`simulate_from_orders_parallel(args, n_threads)`,
`simulate_from_orders_logged_parallel`. `Portfolio::from_orders(&args, threads)`
runs one and wraps the records (§10).

### 8.2 `from_signals`

`FromSignals<'a>` adds OHLC, signals, conflict modes and stops. Defaults
(`FromSignals::default()`): `init_cash = 100`, `LongOnly`, `size = +inf`,
`Amount`, `price = +inf`, `min_size = 1e-8`, `accumulate = Disabled`, all
conflict modes `Ignore`, `upon_opposite_entry = ReverseReduce`, stops off
(`NaN`), `stop_entry_price = Close`, `stop_exit_price = StopLimit`,
`upon_stop_exit = Close`, `upon_stop_update = Override`.

**Two signal conventions**, OR-merged by `read_signals`:

- `entries`/`exits` + `direction`. In `Both` mode an exit signal is a short
  entry.
- `long_entries`, `long_exits`, `short_entries`, `short_exits` explicitly.

**Per bar, per column** (`prepare_order`, run for every column in the group
before any fill):

1. Resolve the order price (`-inf` means this bar's open, falling back to the
   previous close) and the valuation price.
2. **Stops**, if armed: reconstruct missing open/high/low from close/open, then
   test stop-loss first, then take-profit, with
   `stop_price(position, anchor, stop, open, low, high, hit_below)`:
   - if the open is already through the level (a gap), the fill is at the open;
   - else if the bar's range touches the level, the fill is at the level;
   - otherwise no stop.
   A trailing stop updates its anchor to the bar's high (longs) or low (shorts)
   **after** the test, so a bar cannot trigger its own trailing move.
3. **Signals.** If a stop fired, `Signals::from_stop` replaces the bar's signals;
   the fill price follows `StopExitPrice` (`StopLimit` also zeroes slippage) and
   the exit follows `StopExitMode` (close, reduce, reverse). Otherwise
   `read_signals`, and if any entry is present, `resolve_signal_conflict` (long,
   then short), `resolve_dir_conflict`, `resolve_opposite_entry`.
4. **Size.** `signals_to_size(position, signals, size, size_type, accumulate, val_price)`.
   Only `Amount`, `Value` and `Percent` are allowed; a negative size is an error.
5. Build a `ValidatedOrder`, or skip if the size is zero.

Execution then follows §8.1 (group value, call sequence, `execute_validated`).
After each fill the stops are re-armed: flat → cleared; opened or reversed →
re-anchored at the new entry (per `StopEntryPrice`); added to → re-anchored
according to `StopUpdateMode` (`Keep`, `Override`, `OverrideNaN`).

The stages in `crates/barq-sim/src/signals.rs` are pure functions and are tested
exhaustively on their own (§14.3).

### 8.3 Groups, cash sharing and call order

- `Groups { lens, starts, of_col }` (`groups.rs`). `Groups::ungrouped(n)` is the
  default (one column per account). `Groups::from_lens(lens, n_cols)` groups
  adjacent columns and validates that the lengths cover all columns.
- A group's cash comes from `init_cash` at its first column.
- `CallSeq` sets the execution order inside a bar: `Default` (layout order),
  `Auto` (sort by `approx_order_value` ascending with a stable insertion sort, NaN
  last — sells before buys, so freed cash funds the buys in the same bar), or
  `Explicit(&[usize])`.
- A shared account is inherently sequential. The parallel unit becomes the group
  (§11.2); a single portfolio holding every symbol is one unit and runs on one
  thread.
- Shared-group fills are emitted in execution order; analysis sorts its own copy
  by `(col, idx)` before deriving trades (§10.1).

### 8.4 Defaults, logging and sinks

- `barq_sim::DEFAULT_MIN_SIZE = 1e-8` matches the reference. The kernel's
  `Order::default().min_size` is `0.0`; exposing logs is how the difference was
  found (§9.4).
- Logged variants write one log row per order *considered* — every element for
  `from_orders`, every bar whose signals resolve to a nonzero size for
  `from_signals` (the reference's rule, pinned row by row). Logging is off by
  default because a log row is about five times wider than an order record and
  written far more often. A test asserts that turning logging on does not change
  the orders.
- Both loops are generic over a `SinkBuffer` (§9.3), so the plain and logged
  variants share one implementation.

---

## 9. Records: barq-records

Records are the contract between simulation and analysis, and between the two
tiers (§2.2). All record types are struct-of-arrays.

### 9.1 Schemas

| Record | Stored fields | Notes |
|---|---|---|
| `OrderRecords` | `col, idx, size, price, fees, side` | one row per fill |
| `LogRecords` | 37 fields: before-state (`group, col, idx, cash, position, debt, free_cash, val_price, value`), the full request (`req_*`), after-state (`new_*`), result (`res_size, res_price, res_fees, res_side, res_status, res_status_info`), `order_id` | one row per order considered |
| `TradeRecords` | `col, size, entry_idx, entry_price, entry_fees, exit_idx, exit_price, exit_fees, pnl, ret, direction, status, parent_id` | derived (§9.5) |
| `DrawdownRecords` | `col, peak_idx, start_idx, valley_idx, end_idx, peak_val, valley_val, end_val, status` | derived from value (§9.6) |
| `RangeRecords` | start/end/status per column | from `barq-signals` ranges |

**Ids are implicit.** A record's id is its position after merging and is not
stored (`OrderRecords::ids()`). Counted with the implicit id, logs have 38 fields,
trades 14 and drawdowns 10.

**Ordering contract.** Records are grouped by column with ascending rows —
column-major by construction for ungrouped runs. Shared groups emit in execution
order and are re-sorted by the analysis layer.

### 9.2 Merging per-thread buffers

Each worker thread writes into its own buffer. Buffers are concatenated in
chunk order (`OrderRecords::concat`, `Journal::concat`). `order_id` in logs is
buffer-relative and re-based on merge, like ids — a global counter would make
ids depend on thread interleaving. The merged result is therefore identical
whatever the thread count.

### 9.3 Sinks

- `RecordSink::push_order(row, col, size, price, fees, side)`.
- `LogSink` receives log entries; it is a no-op for plain `OrderRecords`.
- `SinkBuffer: RecordSink + LogSink + Send` adds `with_capacity` and `merge`, and
  is implemented by `OrderRecords` and `Journal { orders, logs }`.

Writes go through traits so the phase-2 tier can reuse the record layer without
inheriting the vectorized tier's preallocated storage. Per-thread buffers are
preallocated to the worst case (one fill per element, like the reference's
`max_orders`); letting them grow by doubling cost about 15% of dense throughput
at 4 threads.

### 9.4 Logs and what exposing them found

`LogRecords::reasons()` and `reason_counts(col, n_cols)` answer "why didn't it
trade" per column. Two logs are rarely `==` because NaN `res_*` fields make a
derived `PartialEq` a trap; compare field by field. When logs were first exposed
and diffed against the reference, they found two defects no other test had: the
default `min_size` was `0.0` instead of `1e-8` (a `5e-9` order was filled here and
rejected there), and `order_id` was not re-based across groups in the serial
path.

### 9.5 Three views of trades

`crates/barq-records/src/trades.rs`, each taking `(orders, close, n_rows, n_cols)`:

- `entry_trades` — one record per entry fill; exits are allocated first in, first
  out.
- `exit_trades` — one record per exit fill; entries are averaged.
- `positions(trades, n_cols)` — one record per position (trades grouped by
  `parent_id`).

They legitimately give different counts, PnL splits and win rates for the same
fills. Open trades are marked at the last close. Trade derivation requires sorted
input and errors otherwise.

### 9.6 Mapped arrays and drawdowns

- `ColMap::build(col_arr, n_cols)` is a stable counting sort. It detects the
  common case where records are already contiguous per column and then hands out
  zero-copy subslices; a general path handles merged or filtered records, with
  identical results.
- `MappedArray` reduces per column: `count`, `sum`, `mean`, `min`, `max`,
  `std(ddof)`, `idxmax`, `idxmin`, `reduce`, `top_n_mask`, `value_counts`,
  `stack_expand` (back to a `Matrix`). Reductions skip missing values like the
  reference; empty columns get the fill value.
- `find_drawdowns(ts, n_rows, n_cols)` produces one record per drawdown episode
  (peak, valley, end, `Active`/`Recovered`) with derived durations, drawdown and
  recovery return.
- **Duration convention** (trades, drawdowns and signal ranges alike): a closed
  range lasts `end - start`, an open one `end - start + 1`. Using one convention
  for both would bias averages by up to a bar.

---

## 10. Analysis: barq-analysis

### 10.1 `Portfolio`

`barq_analysis::Portfolio<'a>` (`portfolio.rs`) stores only the order records,
the borrowed close prices (`FlexArray`), the shape, the groups, per-group initial
cash, an optional benchmark and the `Scoring` settings. Everything else is derived
lazily and cached once in `OnceLock`s, so a `Portfolio` is `Sync` and each series
is computed at most once.

Constructors:

- `from_records(orders, close, init_cash, n_rows, n_cols, scoring)`;
- `from_grouped_records(…, groups)` — re-sorts records by `(col, idx)` when groups
  are shared;
- `from_orders(&FromOrders, threads)` and `from_signals(&FromSignals, threads)` —
  run the simulation (in parallel when `threads > 1`) and wrap the result.

`Scoring { ann_factor, risk_free, trades_type, incl_open }` controls
annualization, the risk-free rate, which trade view metrics use (§9.5) and whether
open trades count.

### 10.2 The rebuild chain

`series.rs` rebuilds balances from records alone:

```
orders → asset_flow → assets ─────────────┐
       → cash_flow  → cash (per group) ────┴→ value → returns
                                    close ─┘
```

`asset_value = assets × close`; `value = cash + asset_value` per group;
`returns` are period returns of `value` (the first against initial cash).
`gross_exposure` and `free_cash` are derived the same way.

### 10.3 Metrics

Per account (group), NaN-skipping, from `metrics.rs`:

- return metrics: `total_return`, `annualized_return`, `annualized_volatility`,
  `max_drawdown`, `sharpe_ratio`, `sortino_ratio`, `calmar_ratio`,
  `omega_ratio`, `tail_ratio`, `downside_risk`, `value_at_risk`,
  `cond_value_at_risk` (`DAILY = 365`, `VAR_CUTOFF = 0.05`);
- benchmark-relative (`benchmark.rs`): `information_ratio`, `beta`, `alpha`,
  `capture`, `up_capture`, `down_capture`;
- rolling versions of each (`rolling.rs`), generated over
  `barq_generic::rolling::rolling_apply_1d`;
- `TradeStats { count, winning, losing, win_rate, profit_factor, expectancy, sqn, total_pnl, best, worst }`.

### 10.4 The stats registry

`stats.rs` holds 28 default metrics (`DEFAULT_METRICS`), each with a key, a
title, tags and a compute function: start, end, period, start/end value, total
return, benchmark return, max gross exposure, fees paid, max drawdown and its
duration, trade counts, open-trade PnL, win rate, best/worst trade, average
winning/losing trade and duration, profit factor, expectancy, and the Sharpe,
Calmar, Omega and Sortino ratios; `benchmark_metrics()` adds beta, alpha,
information ratio and capture. `Select::{all, with_tags, keys, without_tags}`
filters by `Tag` (Portfolio, Orders, Trades, Closed, Open, Winning, Losing,
Drawdowns, Duration, Returns, …). Titles and order match vectorbt's
`Portfolio.stats()`, and a fixture pins every row (§14.3).

### 10.5 Aggregating sweeps

- `traded_only(values)` masks accounts that never traded to NaN. **Pitfall:** a
  column that never trades has zero volatility and can score a `+inf` Sharpe
  ratio, so a naive argmax picks it. `crates/barq/tests/sweep.rs` pins this.
- `argmax` / `argmin` skip NaN but not `+inf`.
- `group_by(columns, level, values, Reduce::{Mean, Median, Max, Min, Count})`
  reduces per label of a level (§3.1); `best_column(columns, values)` returns the
  winning column and its labels.

---

## 11. Parallelism

### 11.1 Every parallel site

| Where | Mechanism | Unit of work |
|---|---|---|
| `barq-sim` `from_orders.rs`, `from_signals.rs` (`simulate_into_parallel`) | `std::thread::scope`, contiguous chunks of groups | an account group (one column when ungrouped) |
| `barq-expr` `eval.rs:evaluate` | rayon `into_par_iter().for_each_init` | one symbol's column through the whole factor graph |
| `barq-expr` `polars_backend.rs` (feature) | Polars' own thread pool | Polars lazy frames |
| `barq-py` `expr.rs`, `data.rs` | `py.detach` (releases the GIL) | not parallelism by itself (§12.5) |

Nothing else is parallel: records, analysis, indicators, generic, signals and
data run on one thread. `Portfolio` is `Sync` but nothing fans out over it yet.

### 11.2 The simulator

- `n_threads` is clamped to `[1, n_groups]`. Groups are split into contiguous
  chunks of `ceil(n_groups / n_threads)`; each scoped thread owns its sink and
  counters. There is no shared mutable state, no serial section and no reduction.
- Results are merged in chunk order and ids re-based (§9.2); if several chunks
  fail, the lowest chunk's error wins. Output is identical whatever the thread
  count, and tests assert bit-identical results between serial and threaded runs.
- The parallel unit is the account. Independent columns give one unit per column;
  200 parameter sets × a shared 5-symbol book give 200 units; a single portfolio
  holding every symbol is one unit and cannot be parallelized.
- Scoped threads rather than a work-stealing pool: they force `Sync` arguments
  and column-local state, which is the property that matters. Tasks are whole
  columns or groups (10⁴–10⁶ iterations), so scheduling overhead is noise. A pool
  could replace the scope behind one function if load balancing ever matters.

Measured (`cargo run --release --example scaling -p barq-sim`, 20,000 bars × 256
columns, 4-core Xeon, ungrouped; recorded before grouped signals existed):

| Workload | 1 thread | 4 threads | Speedup | Efficiency |
|---|---|---|---|---|
| sparse (1 fill in 20 bars) | 33 M elem/s | 121 M elem/s | 3.62× | 90% |
| dense (every bar fills) | 15 M elem/s | 25 M elem/s | 1.63× | 41% |

Dense is memory-bound: six record arrays of 5.12 M `f64` each are about 245 MB of
writes. A pure-compute calibration loop scales 3.96×. (The first calibration
measurement was invalid — the compiler deleted the loop and "3.95×" was spawn
overhead divided by itself.)

### 11.3 Factor evaluation

Per-symbol parallelism with rayon (§5.4). One parallel region per evaluation;
each worker keeps its scratch across the columns it handles; outputs are written
into disjoint column slices. Scaling is limited by the number of symbols (a
16-symbol panel feeds at most 16 tasks) and by uneven per-symbol work (work
stealing balances at whole-column granularity).

### 11.4 Why threads and not SIMD for the loops

- **Along time it is impossible:** `state[t] = f(state[t-1], bar[t])` for cash,
  position, debt and free cash.
- **Across columns it is possible but a bad trade:** the kernel has divergent
  control flow (a seven-stage size cascade, buy/sell dispatch, a fee back-solve,
  granularity re-alignment, about 13 invariant checks, the stop gap branch), 14
  data-dependent early exits (the rejection reasons), irregular scatter writes to
  records, and SIMD is too narrow anyway (4–8 lanes versus thousands of columns).

SIMD belongs in the factor and analysis layers (`diff`, `pct_change`, shifts,
rolling statistics), in the order: layout first, then LLVM autovectorization,
then profiling, and only then intrinsics for two or three kernels (`std::simd` is
unstable; `wide`/`pulp` are the stable options).

### 11.5 The `Send` constraint

Every engine instance must be `Send` and there must be no process-global state.
NautilusTrader's `Rc<RefCell<…>>` and message-bus singleton are exactly what force
its sequential `BacktestNode::run()`. The workspace has no `Rc`, no `RefCell` and
no globals (only `OnceLock` caches inside `Portfolio`); phase-2 components are to
use arena-and-index ownership, with any message routing as an engine field, never
a singleton.

---

## 12. Python layer: barq-py

### 12.1 Packaging

`crates/barq-py` builds one wheel with two layers: the Rust extension
`barq._barq` (PyO3 0.29, `abi3-py39`, built by maturin) and the Python package
`crates/barq-py/python/barq/`. Extras: `data` (pyarrow) and `test` (pytest,
vectorbt 1.1.0). In the repository's uv setup, `uv sync` builds and installs it
editable into `.venv` (§16.2).

### 12.2 Arrays cross without copies — and must be Fortran-ordered

`array.rs:columns_of` borrows a NumPy 2-D `float64` array as a column-major
slice. It **rejects C-ordered arrays** (unless one of the dimensions is 1), because
reading a row-major buffer as column-major silently transposes time and symbols
and produces an ordinary-looking wrong backtest. Non-contiguous views are rejected
too. The Python wrappers (`_matrix`, `_signals`, `_param`) convert with
`np.asfortranarray` before calling Rust, and the tests assert that borrowed arrays
are never written.

A 1-D array is ambiguous (one value per row or per column?), so every call site
states which it means (`OneDim::{PerRow, PerColumn}`); simulation parameters
treat 1-D as per column.

### 12.3 Simulation entry points

`barq.from_signals(close, entries, exits, *, open, high, low, init_cash, size, size_type, price, direction, fees, fixed_fees, slippage, min_size, max_size, size_granularity, lock_cash, allow_partial, raise_reject, log, accumulate, sl_stop, sl_trail, tp_stop, val_price, ffill_val_price, update_value, ann_factor=365, risk_free, trades="exit", incl_open, threads=1, cash_sharing=False, group_lens=None, call_seq="default", column_map=None)`
and `barq.from_orders(close, *, size, size_type, …)` mirror the Rust structs:

- every parameter may be a scalar, a 1-D per-column array or a full `(T, C)` array;
- `cash_sharing=True` without `group_lens` makes all columns one account;
- `call_seq` accepts `"default"` and `"auto"`;
- `column_map` maps logical columns to source price columns (§3.3) — prices stay
  `[T, S]` while signals are `[T, C]`; `Portfolio.price_nbytes` reports the
  retained price memory;
- stops require `high` and `low`.

### 12.4 Other bindings

- `barq.FactorSet.compile({...})`, `.evaluate({field: (T, S) array})` →
  `{name: (T, S) array}`, `names`, `required_fields`, `uses_future`;
  `barq.Expression` (a single factor); `barq.alpha158()`, `barq.alpha360()`. Only
  the native backend is exposed.
- `barq.crossed_above(a, b)`, `barq.factors`, `@barq.strategy` (§6.3).
- `barq.data` (§4.5). Python decoders receive `pyarrow.RecordBatch`es through the
  Arrow C Data Interface (`arrow.rs`, the only `unsafe` code, scoped with
  `#[allow(unsafe_code)]`), and only exact pyarrow types are accepted.
- `barq.InstrumentId`, `barq.BarType`.

### 12.5 `barq.Portfolio` and the GIL

- The Python `Portfolio` owns the records, a copy of close at source width, the
  column map, groups and scoring. Each method rebuilds a borrowing Rust
  `Portfolio` and computes what was asked, so the Rust cache does not persist
  across Python calls; `metrics()` and `stats()` compute many values in one call
  for that reason.
- Methods: `stats(col=None, *, group=None)`, `metrics()`, `total_return`,
  `sharpe_ratio`, `max_drawdown`, `trade_count`, `traded_only`,
  `value/returns/cash(col, *, group)`, `assets`, `orders()`, `trades()`,
  `logs()`, `log_reasons()`, `reason_counts(col)`, plus `n_rows`, `n_cols`,
  `n_groups`, `group_lens`, `n_fills`, `n_logs`, `price_nbytes`. Records come
  back as dicts of NumPy arrays.
- **GIL.** Factor evaluation, data loading, `prepare` and `align` release the
  GIL. `from_signals` / `from_orders` do not: their worker threads run in parallel
  but other Python threads wait for the call to finish (§17.4).
- Not exposed yet: rolling metrics, drawdown records, indicators, barq-signals
  utilities, `CallSeq::Explicit`, the Polars backend.

---

## 13. Research layer: research/barq_lab

`research/` is a separate Python package (`barq-lab`) that uses barq to run a
**pre-registered factor research protocol**: a language model writes hypotheses
and factor specifications, and a fixed protocol decides what survives. The model
never sees any result, so it cannot optimize against the data. A healthy pipeline
mostly rejects.

### 13.1 Components

- **Ledger** (`protocol/ledger.py`): an append-only `ledger.jsonl` in its own git
  repository (default `../barq-ledger`). Each entry carries a SHA-256 hash of
  itself and of the previous entry; reading verifies the chain and every append is
  committed. Entry kinds: `legacy`, `registered`, `run`, `control`, `oos_batch`,
  `forward`. The ledger counts in-sample tests (N) and out-of-sample batches used.
- **Policy** (`policy.py`, versioned by content hash): periods (in-sample
  2021-01-01..2025-01-01, out-of-sample 2025-01-01..2026-09-01), data (Binance 1h,
  BTC/ETH/SOL/BNB USDT), costs (fees 0.001, slippage 0.0005, funding not
  modelled), harness settings, spec limits, gate thresholds, OOS budget (at most 3
  batches, 90 days apart) and forward-tracking rules.
- **Pipeline version** (`versions.py`): a hash of the policy and the judging code.
  Changing a prompt does not invalidate earlier results; changing judging code
  marks them stale.
- **Cards and specs** (`cards.py`, `types/factor_ts/spec.py`): a card states the
  mechanism, counterparty, horizon, expected behaviour, failure conditions and a
  fixed direction; dates, years and named events are rejected because the model's
  training data covers the evaluation period. A spec is a barq expression grid
  with a signal rule (`sign`, `zscore`, `band`), side and selection (`ensemble` or
  pre-declared `walk_forward`). The spec parser mirrors
  `crates/barq-expr/src/parse.rs:validate` (a test checks they agree). Limits
  bound grid size, node count, free-parameter ratio and similarity to already
  tried structures.

### 13.2 Workflow

1. `propose` — the model writes cards and specs to `pending/` (it sees only titles
   of what was tried, never results); format problems are sent back up to three
   times.
2. `review` / `show` — a human reads and edits.
3. `approve` — regulation plus a **contract check** on synthetic bars: the spec
   compiles, is not NaN-dominated or constant, and is **prefix-consistent**
   (appending bars never changes earlier values — no look-ahead); then a smoke
   backtest. Approved specs are registered in the ledger.
4. `run` — each registered spec runs once in-sample. Failed runs still count
   toward N, because the data was read.
5. `oos` — current in-sample passes are tested out-of-sample in batches, within
   the batch budget and spacing.
6. `forward` — survivors are shadow-tracked on bars newer than both registration
   and the OOS period; retired if the z-score of daily returns falls below −2.
7. `report`, `calibrate` (negative controls — random, shuffled and a deliberate
   future leak that must be rejected; positive controls recorded only),
   `import-legacy` (earlier trials count toward N).

### 13.3 The harness and its use of barq

`types/factor_ts/harness.py` evaluates a spec with
`barq.FactorSet.compile(...)`, rejects `uses_future`, evaluates on
Fortran-ordered fields, turns member factors into quantized target weights
(decided at `t`, executed at `t + 1`, sent only on change) and runs **one**
`barq.from_orders(close, targets, size_type="targetpercent", cash_sharing=True, group_lens=…, call_seq="auto", column_map=…, ann_factor=8760, threads=4)`
covering every member × cost multiplier (0×, 1×, 2× fees) as separate shared
accounts. The gate requires a net t-statistic above a Bonferroni threshold that
rises with N (`Φ⁻¹(1 − α / 2N)`), a t-statistic not suspiciously large (≤ 8),
positive Sharpe at twice the costs, and consistency across members and assets.
The closest Alpha158 factor (`barq.alpha158()`) is reported for reference only.

---

## 14. Verification

### 14.1 Differential testing against committed fixtures

The acceptance criterion for every layer is agreement with the reference on the
same inputs:

1. Python generators in `tests/parity/generate_*.py` run the reference and write
   fixtures to `tests/fixtures/`.
2. Fixtures are committed. Rust tests replay them, so `cargo test` needs neither
   Python nor the reference.
3. Inputs are hand-picked edge cases plus seeded random cases (seed `20260918`),
   not property-based: a regenerated fixture's diff is itself a finding.
4. Record arrays are compared field by field, never through aggregates. Floats
   use the reference tolerances (`agree`: both NaN, same-sign infinities, or
   `is_close`); integers (`col`, `idx`, sides, statuses, ids) must match exactly.

Fixture format: TSV with a header row, optional `#` comment lines recording
generator, reference version and seed; arrays `;`-separated, multiple outputs
`|`-separated; broadcast prefixes `s:` scalar, `c:` per column, `r:` per row,
`f:` full column-major matrix. `shared_accounts.json` holds whole cases.

### 14.2 Granularity: one call, a sequence, a pipeline

- `crates/barq-exec/tests/parity.rs` — one fixture is one kernel call, so a
  failure names the cascade stage that broke.
- `crates/barq-exec/tests/parity_sequence.rs` — chains calls, feeding the
  kernel's own outputs forward (only initial cash comes from the fixture); this
  catches state threading and residue that single calls cannot.
- `crates/barq-analysis/tests/parity_analysis.rs` — from orders through value and
  returns to a dozen scalar metrics; built last because it cannot localize a
  failure.

### 14.3 Parity tests by layer

| Test | Fixture (rows) | Reference | Tolerance |
|---|---|---|---|
| `barq-exec/tests/parity.rs` | `execute_order.tsv` (474) | vectorbt `execute_order_nb` | `agree` (1e-9 rel / 1e-12 abs) |
| `barq-exec/tests/parity_sequence.rs` | `order_sequence.tsv` (1,504 steps) | chained `execute_order_nb` | `agree` |
| `barq-sim/tests/parity_from_orders.rs` | `from_orders.tsv` (290) | `simulate_from_orders_nb` | `agree`, records field by field |
| `barq-sim/tests/parity_from_signals.rs` | `from_signals.tsv` (181) | from_signals loop and stop engine | `agree` |
| `barq-sim/tests/parity_signals.rs` | `signals.tsv` (5,088, exhaustive) | signal-resolution stages | `is_close` |
| `barq-sim/tests/parity_cash_sharing.rs` | `cash_sharing.tsv` (48) | grouped from_orders, default and auto order | 1e-9 relative |
| `barq-records/tests/parity_trades.rs` | `trades.tsv` (267) | Entry/Exit trades, positions | `is_close`, integers exact |
| `barq-records/tests/parity_drawdowns.rs` | `drawdowns.tsv` (998) | drawdowns | 1e-12 relative |
| `barq-records/tests/parity_mapped.rs` | `mapped.tsv` (28,367) | MappedArray reductions | 1e-12 relative |
| `barq-generic/tests/parity_generic.rs` | `generic.tsv` (1,650) | `vectorbt.generic.nb` | `is_close` |
| `barq-indicators/tests/parity_indicators.rs` | `indicators.tsv` (480) | `vectorbt.indicators.nb` | `is_close`; sweeps bit-identical to single runs |
| `barq-signals/tests/parity_signals_gen.rs` | `signals_gen.tsv` (5,782, exhaustive) | `vectorbt.signals.nb` | exact / 1e-12 |
| `barq-analysis/tests/parity_analysis.rs` | `analysis.tsv` (37) | end to end | `is_close` |
| `barq-analysis/tests/parity_metrics.rs` | `metrics.tsv` (2,047) | `vectorbt.returns.nb` | 1e-9 relative |
| `barq-analysis/tests/parity_stats.rs` | `stats.tsv` (72) | `Portfolio.stats()`, all 28 rows | 1e-9 relative; titles and order exact |
| `barq-expr/tests/parity_qlib.rs` | `qlib_alpha158.tsv` (240), `qlib_alpha360.tsv` (64) | Qlib `Alpha158DL`/`Alpha360DL` | 1e-6 relative (1e-4 for `CORR*`/`CORD*`); native and Polars backends |

Structural tests complement parity: `crates/barq/tests/sweep.rs` (a sweep is one
run; threaded = serial bit for bit; labels lead back to parameters; the `+inf`
Sharpe pitfall), `crates/barq/tests/factor_sweep.rs`,
`crates/barq/tests/factor_path.rs`, `crates/barq-analysis/tests/shared_accounts.rs`
(hand-calculated shared accounts), and the barq-expr unit tests of §5.5.

### 14.4 Mutation testing

Each layer was checked by applying mutations by hand and counting how many the
tests catch; every survivor is argued as unreachable or equivalent. The matrix is
in `tests/parity/README.md` (for example 24/25 caught for generic, 24/24 for
indicators, 43/47 for analysis).

The kernel's lesson generalizes: of five kernel mutations, four were caught; the
survivor replaced `is_close_or_less` with `<=`, which differs only below `1e-9`.
The fix was to aim fixtures at the boundary and find a discrete amplifier —
`size_granularity` turns the invisible difference into a whole unit (§7.5). The
zero-snap has two redundant mechanisms (the entry snap and `add`); removing either
alone is undetectable by design, and removing both is caught only by the sequence
harness (negative cash turns a rejection into an error).

### 14.5 Python tests

`crates/barq-py/tests/`:

- `test_bindings.py` — differential tests that run vectorbt in the same
  interpreter (13 signal cases, 14 order cases, 4 stop configurations, 10 log
  cases, every stats row; integers exact, floats 1e-12, stats 1e-9), plus
  structural tests (thread determinism, C-order refusal, shape and enum errors,
  broadcasting, untraded masking, record dtypes, no writes through borrowed
  arrays, logging and `reason_counts`).
- `test_shared_accounts.py` — the committed `shared_accounts.json` oracle (48
  cases, also re-run with 3 threads) and a live-vectorbt check.
- `test_column_map.py`, `test_data.py`, `test_examples.py`, `test_expressions.py`,
  `test_strategy.py`.

The tests that need vectorbt skip when it is not installed; the repository's uv
environment (Python 3.14) does not include it, so run them from an environment
with `pip install -e 'crates/barq-py[test]'`.

### 14.6 Regenerating fixtures

```bash
python -m venv .venv-parity
.venv-parity/bin/pip install "vectorbt==1.1.0" "plotly<6"
for g in tests/parity/generate_*.py; do .venv-parity/bin/python "$g"; done
cargo test --workspace
```

The Qlib generators run in a Qlib environment
(`conda run -n rdagent4qlib python tests/parity/generate_qlib_alpha158.py`) and
replace only Qlib's leaf feature provider with a deterministic panel (irregular
dates, injected NaNs), so no market data is downloaded.
`generate_shared_accounts.py` uses an editable vectorbt checkout. After
regenerating, read the diff: any changed line is a finding.
`tests/parity/README.md` has the full procedure.

---

## 15. Benchmarks and performance

### 15.1 Execution loop against vectorbt

`benchmarks/` runs identical workloads through barq and both vectorbt engines:

```bash
cargo run --release --example bench -p barq-sim      # → benchmarks/results/barq.tsv
.venv-parity/bin/python benchmarks/bench_vectorbt.py  # → benchmarks/results/vectorbt.tsv
python benchmarks/compare.py                          # head to head
python benchmarks/compare.py --check-baseline         # regression gate
```

- `workloads.tsv` defines ten `from_orders` workloads varying fill rate (dense is
  memory-bound, sparse compute-bound), aspect ratio and per-bar work.
- **Same work or no comparison.** Both runners generate inputs from the same
  formula, implemented twice; each reports a checksum of its arrays and the fill
  count, and `compare.py` aborts if they disagree.
- **Kernel against kernel.** vectorbt's dispatch wrapper validates and prepares
  about eighteen arrays before its loop and costs about 2×; comparing through it
  once reported 5.3× instead of 2.7×. The headline tier times vectorbt's
  `simulate_from_orders_nb` directly.
- Best of 7 runs at 1, 2 and all threads; the run asserts that the thread count
  does not change results.
- **Regression gate:** 10% on single-threaded rows only (single-thread noise was
  1.3% median, 3.7% worst; four-thread noise up to 47%); a uniform shift across
  every workload is flagged as likely machine load.

Reported on a 4-core Xeon at 2.10 GHz, kernel tier, three sittings:

| Comparison | Median range |
|---|---|
| barq vs vectorbt Numba | 2.6×–3.3× |
| barq vs vectorbt Rust | 3.5×–4.9× |
| barq on 4 threads vs vectorbt Numba | 9.7×–12.1× |
| vectorbt Rust vs vectorbt Numba | 0.74×–0.82× |

The ranges are ranges because a ratio moved by 28% between sittings: the Numba
loop loses more to a busy machine than barq's does. Quote the shape, not a single
number.

### 15.2 Other performance tools

- `cargo run --release --example scaling -p barq-sim` — thread scaling (§11.2).
- `cargo run --release --example scale -p barq-expr -- <rows> <cols>` — wall-clock
  per operator and Alpha158 at realistic panel sizes; this is where an O(window)
  kernel or an unbounded cache shows up.
- `cargo bench -p barq-expr --features polars` — criterion comparison of the
  native and Polars backends at 1024 × 16 (§5.6).

### 15.3 Factor engine history

The factor engine was reworked in five steps on this branch; alternating old and
new builds in one sitting measured each (Alpha158, 4 threads):

| Change | Effect |
|---|---|
| O(1) rolling kernels, column-parallel operators, GIL released | 2520×200: 7.87 s → 0.96 s |
| intermediates released after last use, literals broadcast, fields borrowed | peak RSS 2520×500: 3.5 GB → 1.7 GB |
| O(log n) order statistics, monotonic queues, power-sum skew/kurtosis | 2520×200: 860 → 629 ms |
| dependency graph, per-symbol parallel evaluation | 2520×200 about −20%; 2520×16 160 → 75 ms; RSS 1695 → 1591 MB |

---

## 16. Development workflow

### 16.1 Toolchain and configuration

- `mise.toml`: Rust 1.96.0, Python 3.14.6, uv 0.11.33; `.venv` is created
  automatically; `mise run sync` runs `uv sync`.
- Root `Cargo.toml`: edition 2024, `rust-version = 1.85`, Apache-2.0. Lints:
  `unsafe_code = "deny"`, `missing_debug_implementations = "warn"`,
  `clippy::float_cmp = "deny"`, `clippy::too_many_arguments = "allow"` (faithful
  ports of reference loops). Release profile: `lto = true`, `codegen-units = 1`,
  `opt-level = 3` (debug builds of the kernels are about 50× slower and
  meaningless for timing).
- Features: `barq-expr/polars`; `barq/data`, `barq/polars`.
- `rustfmt.toml`: `max_width = 100`. Root `pyproject.toml` is a uv workspace
  (`barq-dev`) depending on the editable `barq[data]`, with a `research`
  dependency group; ruff and pyright are configured for `examples`, `research`
  and the Python package.

### 16.2 Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test -p barq-expr --features polars          # includes Polars parity

uv sync                                            # build + install barq into .venv
uv run --with pytest python -m pytest crates/barq-py/tests -q
uv sync --reinstall-package barq                   # after changing Rust code
```

There is no CI configuration in the repository; these are run by hand.

### 16.3 Examples

| Example | Shows |
|---|---|
| `examples/sweep.py` | `@barq.strategy` MA cross, 7 parameter pairs × 4 synthetic symbols, 28 columns on 4 threads |
| `examples/portfolio.py` | shared-cash rebalancing with `from_orders(targetpercent, cash_sharing, call_seq="auto")`; per-parameter shared accounts with `group_lens` |
| `examples/multi_data.py` | bars + funding + news from catalog/Parquet/CSV, as-of and windowed alignment, `column_map` sweep without price copies |
| `examples/crypto.py` | MA sweep on cached Binance klines with a custom decoder (`--demo` for synthetic data) |
| `examples/fetch.py`, `binance.py`, `download_binance_vision.py` | example-only downloaders that write the CSV cache or the Parquet catalog |
| `crates/barq/examples/factor_sweep.rs` | Rust: factor → `SignalSweep` → `Portfolio::from_signals` |

---

## 17. Roadmap and known gaps

### 17.1 Phase 2: an event-driven tier with a NautilusTrader-compatible API

Nothing of this exists yet; it is the design target.

- **Compatibility means source-level portability**: change imports, keep method
  names and signatures, no linking (Appendix A).
- **Target NautilusTrader v2** (the Rust rewrite), not v1. Notable renames:
  `on_quote_tick` → `on_quote`, `on_trade_tick` → `on_trade`, `on_order_book*` →
  `on_book*`; `cache.quote_tick(s)` → `cache.quote(s)`; `on_event` split into
  `on_time_event` / `on_order_event` / `on_position_event`; `self.msgbus` replaced
  by `publish_message` / `subscribe_topic` / `unsubscribe_topic`; `modify_order`
  takes a `ClientOrderId`; `cancel_all_orders` defaults to strategy-only;
  `is_running` and `state` are methods; `BarType` is an enum.
- **Shape:** a struct holding `core: StrategyCore` plus user state, registered
  with a `barq_strategy!` macro, implementing `DataActor` with `on_start`
  (subscribe to bars) and `on_bar` (`submit_order`).
- **Minimum surface:** `Strategy::new(config)`, `self.config`,
  `cache.instrument()`, `order_factory.market()`, `submit_order`,
  `close_all_positions`, `portfolio.is_net_flat/long/short`, `subscribe_bars`,
  `register_indicator_for_bars`, `indicators_initialized`,
  `on_start`/`on_bar`/`on_stop`.
- **Parallel sweep API:** `ParamGrid::new().sweep(…).per_instrument([...])` —
  `per_instrument` puts symbols on the parallel axis, while an `instruments` form
  runs one strategy over all symbols (pairs, cross-sectional) and parallelizes
  over parameters only. `Portfolio::from_strategy::<S>(&data, grid).run()`
  returns the same `Portfolio` as `from_signals`.
- **Bar matching,** if it follows NautilusTrader: an L1 book; each bar becomes
  four ticks O → H → L → C, with whichever of high/low is nearer the open first;
  volume split across the legs; the finest subscribed timeframe wins.
- **Constraints already honoured in phase 1:** the kernel runs for a single bar
  in isolation (§7); records go through `RecordSink` (§9.3); no `Rc`/globals
  (§11.5). Open-position lookup must be indexed, never a linear scan
  (NautilusTrader v2 loses 1.64× to v1 on a netting position with thousands of
  fills because of one).

### 17.2 Factor engine

- **Batched output** — evaluate in factor batches into a sink or Arrow/Parquet so
  peak memory is bounded by the batch (§5.7).
- **Cross-sectional operators in stages** — `CSRank`, `CSZScore`,
  neutralization, winsorization; split the graph at those nodes (§5.7).
- **More elementwise operators** — `Exp`, `Sqrt`, `Clip`, `FillNa`, `IsNan`,
  masks.
- **Parallelism within a symbol** for narrow panels (fewer symbols than cores):
  independent graph nodes could be spread over threads.
- **Polars backend** — only worth optimizing if a downstream Polars pipeline
  needs it (§5.6).

### 17.3 Ideas to evaluate

Not scheduled; to be assessed now that function strategies exist (§6.3):

- automatic parameter search with an objective and a separate validation period;
- cross-run factor caching (data identity, invalidation, memory limits — first
  measure reuse within one sweep);
- declarative trading rules that reference factors by name and reuse the
  expression language.

### 17.4 Known issues

Found while writing this document; none affects correctness of results.

- `crates/barq-sim/examples/bench.rs` labels its implementation `vbt`, while
  `benchmarks/compare.py` and `baseline.tsv` expect `barq`, so the regression
  gate cannot currently match barq's rows.
- `benchmarks/README.md` and `bench_vectorbt.py` still say barq has no Python
  layer.
- `reject_prob` is implemented in the kernel but neither loop supplies a draw
  (§7.6).
- `CallSeq::Explicit` exists in Rust but Python accepts only `"default"` and
  `"auto"`.
- Python `from_signals` / `from_orders` hold the GIL for the duration of the
  simulation (§12.5).
- The Python `Portfolio` rebuilds the Rust `Portfolio` on every call, so its cache
  does not persist between calls (§12.5).
- `Symbol`/`Venue` are `Arc<str>` without an interner, although a module comment
  describes interned handles (§3.6).
- `from_orders` and `from_signals` differ in small implementation details: serial
  preallocation (`n_rows × n_cols` vs growth) and worker panics (mapped to
  `Error::Invariant` vs propagated).
- Some doc comments are stale: `barq-analysis`'s crate docs say cash sharing is
  not implemented (it is, §10.1), and `from_orders.rs` justifies scoped threads by
  an unavailable crates.io mirror (rayon is now a dependency, §11.2).
- `research/barq_lab/propose.py` refers to `tests/test_propose.py`; the check it
  describes lives in `research/tests/test_pipeline.py::test_proposals_never_see_results`.
- The two Qlib fixtures were generated with different Qlib/NumPy/pandas versions
  (recorded in their headers).

---

## Appendix A. Licensing and provenance

barq is Apache-2.0 (`LICENSE`, workspace `license = "Apache-2.0"`). Both
reference projects are used as specifications and test oracles; **no code is
copied from either**, and every line is an independent implementation written
against public semantics.

- **NautilusTrader is LGPL-3.0-only.** Linking it would bring LGPL obligations
  into the project, and because Rust links statically, §4 of the licence would
  require shipping relinkable objects. barq does not link it. API names and method
  signatures are not protected, so a clean-room API with the same method names is
  permitted.
- **vectorbt is Apache-2.0 with the Commons Clause**, which forbids selling the
  software (including hosting or consulting whose value derives from it). barq
  copies none of its code; reimplementing documented behaviour is permitted.
- **The plug-in route is closed.** `nautilus-plugin` is a guest SDK only (a
  manifest, `nautilus_plugin_init`, zero-sized host tables whose tests assert
  `size_of::<HostVTable>() == 0`), and the live node refuses with "requires
  host-side plug-in support; nautilus-plugin is the guest SDK only". Its component
  contract ships strategies *into* NautilusTrader; it cannot replace the engine.
- **Rules.** Read, do not copy. Fixtures may be derived from upstream tests as
  data (inputs and expected outputs), never as vendored test files. Documentation
  cites paths and short quotes only.
- The references run only in external environments (`.venv-parity`, a Qlib conda
  environment, an editable vectorbt checkout) to generate committed fixtures.
  Neither is a dependency of the crates or the wheel; vectorbt appears only in the
  optional `test` extra.

## Appendix B. Glossary

| Term | Meaning |
|---|---|
| column | one simulated series: a symbol, or a (parameter set, symbol) pair (§3.1) |
| account / group | columns that share cash; ungrouped means one account per column (§8.3) |
| call sequence | the order in which a shared account's columns execute within a bar |
| fill | an executed order; one `OrderRecords` row |
| log row | one order considered, filled or not, with before/after state (§9.1) |
| rejection | an unfillable order, returned as a status, not an error (§3.5) |
| factor | a named expression producing a time × symbol matrix (§5) |
| node / slot | a deduplicated operation in the factor graph / its reusable scratch series (§5.3) |
| `column_map` | logical column → source price column, so sweeps do not copy prices (§3.3) |
| phase 1 / phase 2 | the vectorized tier (exists) / the event-driven tier (planned) |

## Appendix C. File index

| Topic | Files | Section |
|---|---|---|
| matrix, broadcasting, labels, tolerances, errors | `crates/barq-core/src/{matrix,flex,columns,math,error}.rs` | §3 |
| enums, ids, bars, fixed point | `crates/barq-model/src/{enums,ids,bar,types}.rs` | §3.6 |
| data loading and alignment | `crates/barq-data/src/lib.rs`, `crates/barq-py/python/barq/data.py` | §4 |
| expressions | `crates/barq-expr/src/{parse,graph,eval,window,order,polars_backend,alpha158,alpha360}.rs` | §5 |
| signals, sweeps, indicators | `crates/barq/src/sweep.rs`, `crates/barq-signals/src`, `crates/barq-indicators/src`, `crates/barq-py/python/barq/strategy.py` | §6 |
| execution kernel | `crates/barq-exec/src/{types,kernel}.rs` | §7 |
| simulation | `crates/barq-sim/src/{from_orders,from_signals,signals,groups}.rs` | §8 |
| records | `crates/barq-records/src/{lib,logs,trades,mapped,drawdowns}.rs` | §9 |
| analysis | `crates/barq-analysis/src/{portfolio,series,metrics,benchmark,rolling,stats,trade_stats,aggregate}.rs` | §10 |
| Python bindings | `crates/barq-py/src/{lib,array,params,parse,expr,data,arrow}.rs`, `crates/barq-py/python/barq/` | §12 |
| research protocol | `research/barq_lab/` | §13 |
| parity harness | `tests/parity/`, `tests/fixtures/`, `crates/*/tests/` | §14 |
| benchmarks | `benchmarks/`, `crates/barq-sim/examples/{bench,scaling}.rs`, `crates/barq-expr/{examples/scale.rs,benches/backends.rs}` | §15 |
