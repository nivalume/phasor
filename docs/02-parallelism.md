# Parallelism

## What actually runs in parallel

**Parameters and symbols are the same axis.** A sweep of 10 parameter
combinations over 3 symbols is not a 2-D problem — it is 30 columns. The
simulator never learns which is which; it runs `n_cols` independent simulations.
See [03 Column layout](03-column-layout.md).

Because symbols are backtested independently, every column shares no mutable
state with any other. Splitting the column range across threads is the whole
story: no serial sections, no reduction step.

**Cash sharing is the exception, and it is worth being precise about what it
costs.** A group of columns drawing on one balance is not N simulations — it is
one simulation over N instruments, and within each bar the columns run in
sequence because each fill changes what the next can afford. The parallel axis
therefore moves from the column to the *group*. Nothing else about the
decomposition changes: groups remain independent, each thread still owns its
buffers. Shared-group fills are emitted in execution order; the analysis
layer sorts its own copy by symbol and row before deriving trades.

| Workload | Units of work |
|---|---|
| A sweep of independent columns (the default) | one per column — unchanged |
| A sweep of *portfolios*: 200 parameter sets over a shared 5-symbol book | 200 groups of 5 — still 200 units |
| A single grouped portfolio | 1 — no parallelism at all |

The last row is an inherent limit, not an artefact to engineer around: a shared
cash balance is a sequential dependency across the whole group. It is also
rarely the shape that matters here, because the reason to run a sweep is to
have many of something.

The measured figures below are for the ungrouped case, which is what they were
taken on and what the default is. They predate the grouped signal implementation; they have not been remeasured
for this change. Regression tests pin identical results for ungrouped runs.

It runs on `std::thread::scope` rather than a work-stealing pool. Partly
because no external dependency is reachable in this environment, but mainly
because scoped threads are a stricter test of the claim: they require the whole
argument set to be `Sync` and every column's state to be owned locally. A pool
can replace it behind one function when it earns its place.

For phase 2's event-driven tier the unit becomes one complete backtest, and the
axis depends on the strategy:

| Strategy shape | Parallel axis |
|---|---|
| Each symbol traded on its own | Symbols **and** parameters |
| One strategy trading several symbols together (pairs, cross-sectional) | Parameters only — the symbols live inside one run |

The API has to make the user say which; it cannot be inferred.

## Why threads and not SIMD

They are not alternatives. They apply to different axes, and the simulation loop
happens to be the worst possible SIMD candidate.

**Along time, vectorization is impossible.** The loop carries a dependency:

```
state[i] = f(state[i-1], bar[i])       // cash, position, debt, free_cash
```

Bar `i`'s cash depends on bar `i-1`'s result. This is not an optimization
problem; there is no reordering that removes it.

**Across columns it is possible and a bad trade.** One could put 8 parameter
combinations in 8 AVX-512 lanes. Four reasons not to:

1. **Control flow diverges hard.** `execute_order` is a deep branch tree — a
   seven-stage order-sensitive size cascade, buy/sell dispatch, a back-solve
   when cash is short, granularity re-alignment after that back-solve, ~13
   invariant checks, and the stop engine's gap branch. Eight lanes take eight
   different paths. SIMD handles divergence by masking: compute both sides, then
   select. At this branch depth, masking eats most of the theoretical 8x.
2. **Fourteen data-dependent early exits.** Every rejection reason is a `return`.
   SIMD has no early exit; every lane runs to completion.
3. **Irregular writes.** Records are written only on a fill, and each column's
   record index advances independently. That is a scatter with per-lane
   counters.
4. **Too narrow anyway.** `f64` gives 8 lanes on AVX-512, 4 on AVX2. Sweeps run
   to hundreds or thousands of combinations. Threads give ~N× on N cores with
   none of the above problems.

The granularity fits well: tasks are fully independent, and each is an
entire column over the full history — tens of thousands to millions of
iterations. Scheduling overhead is noise.

**Empirical support.** vectorbt's own Rust-vs-Numba benchmark matrix:

| Function | Speedup | Why |
|---|---|---|
| `simulate_from_orders` | 0.73x – 1.15x | Loop-carried dependency, branchy — nothing to vectorize |
| `diff`, `pct_change`, `fshift`, `bshift` @ 1Kx100 | 6x – 10x | Elementwise, no dependency — vectorizes |

Same backend, same machine. The split is exactly along "does this vectorize".

Our own harness reproduces the first row independently: measured kernel to
kernel, vectorbt's Rust backend is **0.74x to 0.82x** its Numba one across our workloads
(range 0.60x – 1.70x). See `benchmarks/README.md`. Translating this loop into
Rust genuinely buys nothing; the threads do.

## Where SIMD does belong

The factor and analysis layers: `diff`, `pct_change`, `shift`, `fillna`,
`cum_returns`, rolling mean/std. These have no dependency or a trivially
parallelizable one.

The engineering order there is: get the memory layout right, let LLVM
autovectorize, then profile, then hand-write intrinsics for the two or three
kernels that justify it. `std::simd` is still unstable; the stable paths are
`wide` and `pulp`. For `diff`/`shift`/`fillna`, autovectorization on a good
layout gets most of the available win.

## The constraint this imposes

For N backtests to run concurrently, **every engine instance must be `Send`
and there must be no process-global state.**

This is precisely where the reference event-driven engine painted itself into a
corner. Its state is `Rc<RefCell<..>>` throughout, and `set_message_bus` installs
a process-level singleton — so `BacktestNode::run()` can only be a sequential
loop, and users sweep by spawning processes.

Therefore, in this codebase:

- `StrategyCore`, `Cache` and `Clock` use **arena + index** ownership. No `Rc`.
- Message routing, if it is built at all, is a field on the engine instance. Never a singleton.
- This is enforced from stage 1. Converting `Rc` to `Send` after the fact is a
  rewrite, not a refactor.

## Measured

Scaling is a claim the design makes, so it gets measured rather than asserted.
`cargo run --release --example scaling -p barq-sim`, on 4 cores, 20,000 bars ×
256 columns:

| Workload | 1 thread | 4 threads | Speedup | Efficiency |
|---|---:|---:|---:|---:|
| Sparse (5% of elements fill) | 33M elem/s | 121M elem/s | 3.62x | **90%** |
| Dense (every element fills) | 15M elem/s | 25M elem/s | 1.63x | 41% |

For calibration, a pure-compute loop with no allocation and no shared data
reaches 3.96x on the same machine. So 90% on the sparse workload says the
decomposition is sound — columns really are independent, and nothing is being
shared behind the engine's back.

**The dense case is memory-bound, not a parallelism defect.** With every element
filling, a run writes six record arrays of 5.12M `f64` each — about 245 MB — and
that is the ceiling, not the cores. It is also not a realistic sweep: a strategy
that trades on every bar of every parameter combination is not what anyone runs.
The number is kept because knowing *which* resource runs out is the point of
measuring.

Two things came out of doing this rather than assuming it:

- Per-thread record buffers are preallocated to the worst case (one fill per
  element, the same bound the reference's `max_orders` uses). Growing them by
  doubling put every thread in the allocator at once and cost roughly 15% of
  dense throughput at 4 threads.
- The first measurement attempt was meaningless: the compiler deleted the
  calibration loop, and "3.95x" was thread-spawn overhead divided by itself. A
  scaling number that looks right is worth one skeptical check.
