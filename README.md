# barq

A parallel, factor-driven vectorized backtesting engine in Rust.

**Built for three things:**

1. Parallel parameter sweeps.
2. Parallel backtesting across independent symbols or multi-asset accounts.
3. Factor expressions, so factor libraries can be backtested at scale.

Strategies are additionally meant to be writable in NautilusTrader's style —
source-level portability, not a code dependency.

## Why

Two findings from surveying the reference implementations shaped the design.

**Rewriting a simulation loop in Rust does not make it faster.** vectorbt ships
its own Rust backend; its published benchmarks put `simulate_from_orders` at
**0.73x–1.15x** against Numba — a wash, because Numba already emits LLVM machine
code and a branchy scalar loop gives Rust nothing to work with. The same backend
reaches **6–10x** on elementwise operations at 1000×100. One backend, two
outcomes, split exactly along "does this vectorize".

**The reference event-driven engine cannot sweep in parallel, structurally.**
NautilusTrader's `BacktestNode::run()` is a sequential `for` loop — no `rayon`,
no `par_iter`, no thread spawn anywhere in its backtest crate. Its state is
`Rc<RefCell<..>>` behind a process-global message bus, so one engine occupies one
process and users sweep by spawning processes.

So the opportunity is not "the same thing, in Rust". It is parallelism the
reference architecture has foreclosed on, with a factor pipeline feeding it.

## Where it stands

Measured against vectorbt on identical workloads, loop against loop (4-core
Xeon, `benchmarks/`):

| | median | range |
|---|---:|---:|
| ours vs their Numba | **~2.6x – 3.3x** | 2.0x – 3.6x within a sitting |
| ours vs their Rust | ~3.5x – 4.9x | 1.4x – 5.5x |
| ours on 4 threads vs their Numba | ~9.7x – 12.1x | 1.9x – 13.4x |
| *their Rust vs their Numba* | *0.74x – 0.82x* | *0.60x – 1.70x* |

Those are ranges rather than figures, and the reason is a correction. Three
separate sittings — each measuring **both** engines together, which is the
rule that keeps a ratio honest — gave medians of 2.61x, 2.79x and 3.34x for
the first row. A ratio should not care how busy the machine is, and this one
does: the reference's Numba loop loses more to a loaded machine than ours
does, so the quieter the box, the better we look. Quoting any single one of
those as *the* number implies a precision this benchmark does not have.

What survives the spread is the shape: several times faster than the Numba
loop, and the last row, which is the one that matters most.

That last row is the point. Their Rust backend is **slower** than their Numba
one — independently reproducing the figure they publish — so translating a
branchy scalar loop into Rust buys nothing on its own. The gap comes from design
differences rather than the compiler; which of them, in what proportion, is not
yet measured.

## Design

```
Factor expressions  →  factor panel (time × symbol)
                    →  signals (time × (symbol × parameter))
                    →  simulation   ← threaded across independent account groups
                    →  records      ← both tiers emit the same schemas
                    →  metrics per account, records per symbol
```

Parameters and symbols share a flat column axis. Columns are independent by
default; cash-sharing groups combine symbols into portfolios, with parameter
groups simulated in parallel. Python supports both orders and signals with
account-level results; see [shared accounts](crates/barq-py/README.md#shared-accounts).

A second, event-driven tier (phase 2) will run Nautilus-style strategies. It
takes a completely different execution path but emits the same records — so one
analysis layer serves both, and a vectorized sweep can be validated against an
event-driven replay column by column.

**Start with [`docs/`](docs/README.md).** [01 Architecture](docs/01-architecture.md)
and [02 Parallelism](docs/02-parallelism.md) cover the load-bearing decisions.

## Status

Phase 1, under construction. Bar/OHLCV only.

Implemented and pinned against the reference: the float tolerance layer,
`FlexArray`, multi-level column labels, the execution kernel, the `from_orders`
and `from_signals` loops with parallel execution across account groups, the stop
engine, order records, all three trade views, and the analysis layer — the
value/returns rebuild chain, return metrics, trade statistics and by-level
aggregation — plus the array layer (rolling, expanding and exponential
windows), the eight indicators with their parameter sweep, and the signal
layer that produces entries and exits rather than only consuming them. The factor DSL is
implemented through Alpha158 and Alpha360, and the Python package provides the
external market-data boundary. See the stage table in
[`docs/README.md`](docs/README.md).

### TODO

- **Batched factor output.** `FactorSet::evaluate` returns every factor as one
  `f64` matrix held in memory at once. Intermediates are released after their
  last use, so on Alpha158 at 2520×500 nearly all of the ~1.7 GB peak is the
  158 output matrices themselves; a full A-share panel (5000 symbols × 2520
  days) would need ~16 GB for the outputs alone. Evaluate in factor batches
  and hand each batch to a caller-supplied sink (or write it to Arrow/Parquet)
  so peak memory is bounded by the batch rather than the whole set.
- **Cross-sectional operators run in stages.** The native evaluator compiles a
  factor set into one dependency graph and runs it one symbol at a time,
  symbols in parallel, which is only valid while every operator reads a single
  symbol's series. Adding `CSRank`, `CSZScore` or neutralisation means
  splitting the graph at those nodes: symbols in parallel within a stage, one
  cross-sectional pass over every symbol between stages.

## Try it

```bash
python -m pip install -e crates/barq-py
python examples/sweep.py
```

Seven parameter pairs across four symbols: one run of twenty-eight columns,
threaded, with the result attributed back to the window and symbol that
produced it. The printed returns describe a synthetic generator, not a
strategy — the example says so, and says why that warning is there.

For shared cash across multiple symbols, run the offline
[portfolio example](examples/portfolio.py):

```bash
python examples/portfolio.py
```

It rebalances BTC/ETH/SOL in one account with `from_orders`, then compares two
moving-average parameter sets using `from_signals` and `group_lens=[3, 3]`.
Each parameter set gets an independent 100,000 account. The example prints
account equity, cash, positions and statistics, and checks that equity equals
cash plus the symbols' market value. Prices are synthetic; no downloads needed.

For Bars + funding + news from a catalog or standalone files:

```bash
python -m pip install -e 'crates/barq-py[data]'
python examples/multi_data.py --source catalog  # also parquet / csv
python examples/crypto.py --demo
# Full USD-M history from Binance Vision (writes ~/.cache/barq/catalog):
python examples/download_binance_vision.py
```

The offline data example uses two complete `symbol.venue` identities and four
parameter pairs. Prices stay `[T,2]`; signals are `[T,8]`, with `column_map`
sharing prices across parameters. See the [data API](crates/barq-py/README.md#local-data-and-parameter-sweeps).

## Developing

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
pytest crates/barq-py/tests -q
```

Local CSV/Parquet reading and alignment live in the optional `barq-data` Rust
crate. User Schemas decode Arrow batches before simulation; the compute crates
remain independent of file I/O. Binance HTTP and cache writing live only in
examples. Parallel account execution uses `std::thread::scope`.

## Licence

Apache-2.0. See [LICENSE](LICENSE).

Both reference projects are studied as specifications and used as test oracles;
no code is copied from either. NautilusTrader is LGPL-3.0-only and vectorbt is
Apache-2.0 with Commons Clause, so every line here is an independent
implementation written against public semantics. See
[09 Licensing](docs/09-licensing.md).
