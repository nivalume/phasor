# Architecture

## The pipeline

```
Factor expressions  (qlib-style DSL)
        │  parse → lower → CSE / fusion → evaluate
        ▼
Factor panel  (time × symbol)                          [Arrow]
        │  threshold / rank / quantile
        ▼
Signal matrix  (time × (symbol × parameter))           ← one axis, not two
        │  Portfolio::from_signals — threaded across independent accounts
        ▼
Records  (orders / trades / positions / logs)          ← the unification point
        │
        ▼
Analysis  →  account cash / equity / returns  →  account metrics
```

## The central tension, and how it resolves

A vectorized engine is fast because it has no events, no per-bar objects and no
message bus — just arrays and tight loops. An event-driven engine is realistic
because it has exactly those things. You cannot have both on one execution path.
Measured: the vectorized loop runs in nanoseconds per column per bar, the
event-driven one in **1.2–1.9 µs per event**. That is three orders of magnitude,
and no amount of engineering closes it, because the gap *is* the realism.

So there are two tiers, and they will never share a simulation loop:

- **Tier A — vectorized.** Static arrays in, records out. Parameters and symbols
  are both just columns. This is phase 1.
- **Tier B — event-driven.** A Nautilus-style `Strategy` reacting to bars,
  submitting orders against a simulated venue. Slower per run, parallel *across*
  runs. This is phase 2.

**What unifies them is the output, not the computation.** Both tiers emit the
same record schemas, so:

- The analysis layer is written once and serves both.
- A vectorized sweep and an event-driven replay of the same strategy can be
  compared column by column.
- Users prototype at vectorized speed and validate at event-driven fidelity.

This only works if the fills agree, which is why both tiers call the same
`execute_order` in `barq-exec` rather than each having its own. If that ever
forks, Tier A stops being a usable proxy for Tier B and the whole two-tier
design loses its point.

## The insight that connects the two goals

vectorbt's Rust backend refuses to port any function taking a callback —
`_engine.py` returns a flat *"Rust engine is not implemented for
callback-accepting functions."* So `from_orders` and `from_signals` (static,
array-driven) are native, while `from_order_func` — the one that lets a user
write arbitrary per-bar logic — stays in Numba.

But look at the shapes:

```
vectorbt:   order_func_nb(OrderContext) -> Order
Nautilus:   on_bar(&Bar)  { ... self.submit_order(order) }
```

They are the same thing: observe state at a bar, emit orders. In Numba that
callback cannot cross into native code. In Rust it is a monomorphized trait with
no indirection at all.

**So the Nautilus-style strategy API is not a second system bolted alongside the
vectorized one — it can be the native implementation of `from_order_func`.** One
mechanism serves "replicate vectorbt" and "write strategies Nautilus-style"
simultaneously, and it fills a hole the upstream project explicitly left open.

## Scope decisions

**Accounts are independent.** By default each column has its own cash. With
`cash_sharing=True`, contiguous columns described by `group_lens` share one
account. One parameter set can therefore hold multiple symbols, while parameter
sets remain isolated and run in parallel. Within an account, orders execute
sequentially; `call_seq="auto"` stably sorts estimated order values, selling
before buying. Returns and metrics come from account equity, while positions,
trades and logs retain their original symbol columns. No automatic allocation
or risk policy is applied.

**Bar execution.** Generic funding/news/event datasets may provide factor inputs;
tick and L2 order book execution are deferred. Bar execution alone
already carries enough semantics to get wrong (see
[04 Execution model](04-execution-model.md)).

**Rust engine, Python boundary.** Simulation, expressions and analysis live in
Rust. `barq-data` reads local CSV/Parquet and prepares OHLCV panels or numeric
views of user-defined Arrow datasets. User decoders run before simulation.
Python exports these interfaces and supplies examples. HTTP and cache writing
live only in examples; catalog reads never manage or write data.

Parameter sweeps retain source OHLCV as `[T,S]`. `FlexArray::MappedColumns` maps
logical simulation columns back to source columns in both execution and
analysis. Signals and account state use `[T,C]`. Dynamic Schemas, strings and
Python callbacks stay outside the simulation loops.

## Crates

| Crate | Responsibility |
|---|---|
| `barq-core` | `FlexArray`, column labels, float tolerances, errors |
| `barq-model` | Fixed-point values, instrument/BarType identities, bars, execution enums |
| `barq-data` | Local CSV/Parquet readers, user Schema decoding, BarPanel and auxiliary alignment |
| `barq-exec` | `execute_order` — shared by both tiers |
| `barq-sim` | Vectorized loops; where threading is applied |
| `barq-records` | Record schemas, trade derivation, `RecordSink` |
| `barq-analysis` | Return metrics, drawdowns, stats registry, level aggregation |
| `barq-expr` | Factor expression DSL |
| `barq` | Façade and prelude |
| `barq-py` | PyO3 shell plus the Python `barq.data` boundary |

Phase 2 adds `barq-strategy` (the `Strategy` trait and macro) and
`barq-indicators` (indicator factory with parameter sweeps).
