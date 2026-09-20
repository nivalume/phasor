# Verification

Differential testing is the acceptance criterion. Unit tests confirm the code
does what it was written to do; only differential testing confirms it does what
the reference engine does — and "replicates vectorbt" is a claim about the
latter.

## Differential harness

Generate fixtures by running the reference implementation, commit them, replay
them from Rust. `cargo test` then needs neither Python nor vectorbt. Setup and
regeneration are in [`tests/parity/README.md`](../tests/parity/README.md).

**One fixture is one kernel call, not one simulation.** Diffing whole
simulations tells you *that* something drifted; diffing single `execute_order`
calls tells you *which* stage of the size cascade did it.

Inputs are hand-picked cases plus seeded random ones, covering: price series
with embedded `NaN`s, every `SizeType` × `Direction` combination,
fee/slippage/granularity settings, `min_size`/`max_size` boundaries, `lock_cash`
with an open short, the float tolerance boundary, and multi-column sweeps. Seeded
rather than property-based, so a fixture file changes only when behaviour does
and the diff is the signal.

**Compare the record arrays field by field. Not aggregate metrics.** A Sharpe
ratio that matches to four decimals hides sign errors and off-by-one entry
indices; an order record array does not.

Comparison rules:

| Field type | Rule |
|---|---|
| Floats (`size`, `price`, `fees`, `pnl`) | Compared with `is_close` at the reference tolerances — which doubles as a check that the tolerance layer itself was ported correctly |
| Integers (`idx`, `col`, `id`, `side`, `status`, `parent_id`) | **Exact equality** |

Run the reference with `engine="numba"` — it is the authoritative path there, and
its own Rust backend has documented gaps (callbacks, RNG streams).

## Mutation testing keeps the harness honest

A differential test that passes trivially is worse than none, and "all fixtures
pass on the first run" is a claim to distrust, not celebrate. So before trusting
a harness, break the kernel on purpose and confirm it fails.

This is not hypothetical. Of five mutations applied to the order execution
kernel, four were caught immediately and one survived: replacing the tolerant
affordability check `is_close_or_less(total_req_cash, cash_limit)` with a plain
`<=`. The two spellings pick different branches, but the branches differ only in
the last few bits — below the 1e-9 comparison tolerance, and therefore invisible.

What makes it visible is `size_granularity`. The "insufficient cash" branch
floors its result to a lot boundary, turning a few-ULP difference into a whole
unit: 5.0 filled versus 4.0. Fixtures built on `math.nextafter` now pin that
boundary, and the mutation is caught by three of them.

The lesson generalizes: **when a comparison is tolerance-based, a fixture set
that only samples the interior cannot test it.** Aim at the boundary, and find
the discrete quantity — a lot size, a minimum, a status code — that amplifies a
sub-tolerance difference into an observable one.

Whenever a plausible mutation survives, add fixtures until it does not — **or,
when no fixture at that granularity can distinguish it, say why.**

The second case is real. The engine keeps balances on exact zeros through two
redundant mechanisms: an entry snap at the top of `execute_order`, and `add()`
in every running-balance update. Removing *either alone* is undetectable,
because the other covers for it. That is a property of the design, not a hole in
the tests. Removing *both* is caught — but only by the sequence harness, and
only because a discrete branch downstream amplifies the residue: a negative cash
balance turning a rejection into an error, or a dust-long position turning a
`ShortOnly` order into an invariant violation.

Finding those amplifiers took experiment, not reasoning. A first attempt at the
position case closed a position with the same number it had opened with, which
cancels exactly and amplifies nothing.

The full mutation matrix is in
[`tests/parity/README.md`](../tests/parity/README.md).

## Two harnesses, two granularities

| Harness | Unit | Catches |
|---|---|---|
| `parity.rs` | one `execute_order` call | Which stage of the size cascade is wrong |
| `parity_sequence.rs` | a chain of calls | State threading, residue accumulation, discrete branches that depend on an earlier step |

The sequence harness replays a chain feeding **its own** outputs forward,
reading only the initial cash from the fixture. Re-seeding from the expected
values each step would collapse it into the single-call test, because a drift
introduced at step 2 would be wiped before step 3 could amplify it.

It also fixes the chaining rule — mark to market, then execute — which is
exactly what stage 3's simulation loop must implement. So it is both the
regression test for stage 2 and the acceptance test for stage 3.

## Factor layer, separately

The factor engine is diffed against qlib on its own: same expression, same data,
compare the factor value matrices. Coverage focuses on the three known semantic
hazards — `Ref` across an irregular calendar, `NaN` propagation in partially
populated rolling windows, and per-symbol window boundaries in a panel. See
[06 Factor expressions](06-factor-expressions.md).

## Upstream test suites as oracles

The reference project's tests encode a large number of edge cases:
`test_portfolio.py` (~10.8k lines), `test_records.py` (~4.7k),
`test_engine.py` (~2.5k).

Extract inputs and expected values into language-neutral fixtures (JSON/Parquet)
and replay them from Rust. Reference the upstream *cases*; do not vendor their
source files — see [09 Licensing](09-licensing.md).

## Benchmarks

`benchmarks/` holds a harness that runs identical workloads through this engine
and through both of vectorbt's, and a committed baseline for catching our own
regressions. Two design points there generalize beyond benchmarking:

- **Both runners report a checksum of the input they generated, and the
  comparison aborts when they disagree.** A faster number that turns out to be
  less work is worse than no number.
- **The comparison is kernel against kernel.** vectorbt's dispatch wrapper
  costs about 2x, so measuring our raw Rust against their wrapped call inflates
  the result by exactly that and says nothing about the loops. The first
  version of the harness did this and reported 5.3x instead of 2.7x.

The headline result is in `benchmarks/README.md`. The one worth repeating here:
vectorbt's own Rust backend is **0.74x to 0.82x** its Numba one for this
function, independently reproducing their published figure — which is both a check that
the harness measures the right thing, and the reason "rewrite it in Rust" was
never the argument for this project.

## Measured, against the prediction

The design doc predicted **parity** with Numba on the simulation loop, reasoning
that Numba already emits LLVM machine code and a branchy scalar loop gives Rust
nothing to work with.

That reasoning was right and the conclusion was wrong. Measured at the kernel
tier: **roughly 2.6x to 3.3x** single-threaded, and 9.7x to 12.1x on four
cores. Those are ranges because three sittings that each measured both engines
together disagreed by 28% on the ratio — the reference's Numba loop is more
sensitive to a busy machine than ours is. See `benchmarks/README.md`.

The reasoning holds for the part it was about — vectorbt's own Rust backend is
0.74x to 0.82x its Numba one, so translating the loop into Rust really does buy nothing.
The gap therefore comes from design differences, not from the compiler.

**Which ones, precisely, is not yet measured.** The plausible candidates are the
`FlexArray` scalar path, column-major scans, stack-allocated state where Numba
builds named tuples, and — for the threaded figure — threads, which the
reference's architecture forecloses on. Attributing that gap among those would
need a variant-by-variant measurement against their kernel, which has not been
done. What *has* been measured is where our own remaining time goes; that table
is in `benchmarks/README.md`.

Saying more than that would repeat the mistake this section exists to record.

The honest reading is that "rewrite it in Rust" was never the argument, and
still is not. Rust made a different *design* possible; the design is what pays.

Still worth watching for:

- A result *below* the reference is a signal about layout or branching, and a
  useful regression detector.
- Parallel scaling, measured from 1 to N cores. On an independent workload,
  sub-linear scaling means false sharing, allocator contention, or memory
  bandwidth. Established so far: 90% efficiency at 4 cores on a sparse-fill
  workload, 41% on a dense one where record writing saturates memory. See
  [02 Parallelism](02-parallelism.md).
- Factor evaluation, on an Alpha158-scale library with heavy subexpression
  sharing. Run the Native/Polars comparison with
  `cargo bench -p barq-expr --features polars --bench backends`.

## CI

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Plus the differential harness at a fixed seed, and the performance gate:

```bash
cargo run --release --example bench -p barq-sim
python benchmarks/compare.py --check-baseline
```

All green is the gate. The performance check covers single-threaded rows only —
threaded timings on a shared machine vary by up to 47% between runs, which is
wider than any regression worth catching. The measurement behind that choice is
in `benchmarks/README.md`.

## Two harnesses, two granularities — and a third that spans everything

`parity_analysis.rs` is the widest harness in the project: one case runs
`from_orders`, derives the value curve from its records, and reduces that to a
dozen scalars. A single row therefore exercises the kernel, the loop, the
record layer, the trade views, the rebuild chain and the metrics at once.

That width is why it was built last. On its own it can only report that
something drifted; the narrow harnesses say what. Building them in the other
order would have produced a test that fails often and localizes nothing.

It compares the whole value and returns series, not only the scalars. A Sharpe
ratio that agrees over a curve that does not is a coincidence — the same
reasoning that puts record arrays rather than aggregates at the centre of this
document.

## End-to-end smoke test

`parity_analysis.rs` covers everything downstream of the orders.
`crates/barq/tests/factor_path.rs` covers the front of the pipeline: OHLCV → a
factor expression → a one-bar-delayed and cleaned signal → `Portfolio`.
`crates/barq-expr/tests/parity_qlib.rs` replays committed Qlib-generated panels
against all Alpha158 and Alpha360 expressions through both Native and Polars
backends.

## The bindings are differentially tested too, and more cheaply than anything else

Every harness above has the same shape: a Python generator writes fixtures, and
a Rust test replays them. That indirection exists because the two engines could
not otherwise be called from the same process.

`barq-py` removes it. With the binding installed, the reference and this engine
are two function calls in one interpreter, so a test can run both on the same
array and compare the records directly — no fixture file, no generator, no
schema to keep in step. `crates/barq-py/tests/test_bindings.py` does this for 13
signal cases, 14 order cases, 4 stop configurations, 10 log cases and all 28 rows
of the statistics table: integer fields exactly, floats to 1e-12, statistics to
1e-9.

**The log comparison is the widest check in the project, and it paid for itself
immediately.** A log row is the account on both sides of the order, the request
as it was resolved, and the result — 37 stored fields, of which the request half
appears in no other record. Comparing all of them found two defects that every
fixture harness had passed over: `min_size` defaulting to `0.0` where the
reference defaults to `1e-8` (observable only on dust orders, and every fixture
sets `min_size` explicitly), and a log's `order_id` not being re-based across
groups in the serial path (unobservable until a public entry point returned
logs). Both are in `docs/05-records.md`.

**This does not retire the fixture harnesses, and the difference is worth being
precise about.** A fixture is a committed artifact: it pins what the reference
answered at a known version, it runs in CI with no Python at all, and it can
cover the kernel and the signal pipeline directly rather than only through a
public entry point. The in-process tests cover less surface and cannot run
without the reference installed. What they add is *cost*: a new parameter
combination is one line in a list, so the cases that would never have justified
a generator get covered anyway.

They also cover something no fixture can, because it is not a numerical
question: that the binding hands the engine the array the caller meant. A
C-ordered matrix read as column-major is a transposed price series, and it
backtests. Those tests are in the same file, ungated, because they need no
reference — see `crates/barq-py/README.md`.

## Shared account acceptance

`cargo test -p barq-analysis --test shared_accounts` checks hand-calculated
rotation, isolated capital, target weights, first-bar fees and missing flat
symbols. `pytest crates/barq-py/tests/test_shared_accounts.py -q` exercises the
public grouping API and committed/live vectorbt parity, including serial versus
parallel records, logs and account statistics. Run `python examples/portfolio.py`
for the offline allocation and parameter-sweep smoke test. Fixture regeneration
is documented in `tests/parity/README.md`.
