# barq design docs

A parallel, factor-driven vectorized backtesting engine in Rust.

**What it is for.** Three things, in priority order:

1. Parallel parameter sweeps.
2. Parallel backtesting across independent symbols or multi-asset accounts.
3. Factor expressions, so factor libraries can be backtested at scale.

Strategies should additionally be writable in NautilusTrader's style — source-level
portability, not a code dependency.

## Read in this order

| Doc | What it settles |
|---|---|
| [01 Architecture](01-architecture.md) | The two tiers, and why they unify at the record layer rather than the simulation layer |
| [02 Parallelism](02-parallelism.md) | What runs in parallel, why threads and not SIMD, and the `Send` constraint that follows |
| [03 Column layout](03-column-layout.md) | Why parameters and symbols are the same axis, and the contiguity invariant |
| [04 Execution model](04-execution-model.md) | Order execution semantics: the size cascade, `debt`/`free_cash`, rejection vs. error |
| [05 Records](05-records.md) | The schemas both tiers emit, and the three trade views |
| [06 Factor expressions](06-factor-expressions.md) | Why the factor engine is a compiler, not a compute engine |
| [07 Nautilus compatibility](07-nautilus-compat.md) | What "compatible" means here, and the v2 API surface to target |
| [08 Verification](08-verification.md) | Differential testing as the acceptance criterion |
| [09 Licensing](09-licensing.md) | Why no code is taken from either reference project |

## Why this project exists

Two facts from surveying the reference implementations, both of which shaped
everything else:

**Rewriting a simulation loop in Rust does not make it faster.** vectorbt ships
its own Rust backend (`vectorbt-rust`, 15.6k lines, PyO3). Its own published
benchmarks put `simulate_from_orders` at **0.73x–1.15x** against the Numba
implementation — a wash. Numba already emits LLVM machine code, and a branchy
scalar loop gives Rust nothing to work with. The same backend hits **6–10x** on
`diff`, `pct_change`, `fshift`, `bshift` at 1000x100. One backend, two very
different outcomes, and the difference is whether the operation vectorizes.

**The reference event-driven engine cannot sweep in parallel, structurally.**
NautilusTrader's `BacktestNode::run()` is a sequential `for` loop. There is no
`rayon`, no `par_iter`, no thread spawn anywhere in its backtest crate. Its state
is `Rc<RefCell<..>>` behind a process-global message bus, so one engine occupies
one process; users are told to sweep by spawning processes. For calibration, it
costs roughly **1.2–1.9 µs per event** (~0.86M events/s replaying, ~0.54M/s
running a bar strategy).

So the opportunity is not "the same thing, in Rust". It is parallelism the
reference architecture has foreclosed on, plus a factor pipeline feeding it.

## Status

Phase 1, building. Bar/OHLCV only.

| Stage | Crate | State |
|---|---|---|
| 1 | `barq-core`, `barq-model` | Foundation done: tolerances, `FlexArray`, column labels, enums, value types |
| 2 | `barq-exec` | **Done**: `execute_order` ported; 474 single-call + 1504 sequence-step fixtures green |
| 3 | `barq-sim` | **Done**: `from_orders` and `from_signals` with the stop engine, cash sharing with the auto call order, parallel across groups, and a logged variant of each loop; 290 + 181 + 48 loop fixtures and 5088 exhaustive signal-pipeline fixtures green |
| 4 | `barq-records` | **Done**: order records, log records, all three trade views, the mapped-array reduction layer and drawdown records; 267 + 28,367 + 998 fixtures green, 32/34 mutations caught |
| 4.5 | `barq-generic` | **Done**: rolling, expanding and exponential windows, shifts, fills, crossovers; 1650 fixtures green, 24/25 mutations caught |
| 4.7 | `barq-signals` | **Done**: cleaning, exit generation, ranking, ranges; 5782 exhaustive fixtures green, 24/25 mutations caught |
| 5 | `barq-analysis` | **Done**: the `Portfolio` type (lazy, `Sync`, caches each derivation once), rebuild chain, all return metrics in scalar and rolling form, benchmark-relative metrics, trade statistics, by-level aggregation, and the 28-row tag-filtered stats registry; 37 + 2,047 + 72 fixtures green, 43/47 mutations caught |
| 4.6 | `barq-indicators` | **Done**: the eight reference indicators plus the parameter sweep; 480 fixtures green, 24/24 mutations caught |
| 6 | `barq-expr` | **Core implemented**: Qlib DSL, Alpha158, Alpha360, matrix bridge, committed Qlib numeric fixtures and factor-to-portfolio test |
| 7 | `barq-py` | **Done for the Python boundary**: PyO3 + rust-numpy for simulation, shared accounts with grouped analysis, and expressions, plus Python `barq.data` for Binance paging, resumable CSV caching and outer-join alignment. Differential tests compare records and all 37 log fields against the reference. See [crates/barq-py/README.md](../crates/barq-py/README.md). |

Phase 2 — the event-driven tier with the Nautilus-style strategy API — is out of
scope for now, but phase 1 carries two constraints on its behalf: the execution
kernel must be callable for a single bar in isolation, and record writes go
through a `RecordSink` trait. Both are cheap now and expensive to retrofit.

## Developing

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Parallel scaling, which the design claims and therefore measures
cargo run --release --example scaling -p barq-sim

# Performance against vectorbt, and against our own committed baseline
cargo run --release --example bench -p barq-sim
python benchmarks/compare.py                    # head-to-head
python benchmarks/compare.py --check-baseline   # regression gate

# The Python bindings
python -m pip install -e 'crates/barq-py[test]'
pytest crates/barq-py/tests -q

# Public examples
python examples/sweep.py
python examples/fetch.py --symbols BTCUSDT,ETHUSDT --interval 1d --from 2021-01-01
python examples/crypto.py --symbols BTCUSDT,ETHUSDT --interval 1d
```

**The engine crates have no external dependencies.** Parallelism uses
`std::thread::scope`, which needs none and is a stricter test of the design:
scoped threads require the argument set to be `Sync` and every column's state to
be owned locally. A work-stealing pool can replace it behind one function when it
earns its place. Factor expressions reuse the engine's column-major matrices and
rolling kernels, so they do not add another runtime dependency.

`barq-py` is the one exception: PyO3 and rust-numpy are the binding, and its
Python shell owns market-data I/O. It is a leaf crate, so the engine stays free
of HTTP and cache dependencies whether or not the package is built.
