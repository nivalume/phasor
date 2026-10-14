# Parity harness

Generates differential-test fixtures by running the reference implementation
(vectorbt) and recording what it produces. The Rust test suite replays them.

Fixtures are **committed**, so `cargo test` needs neither Python nor vectorbt.
You only need this directory when changing what is covered, or when checking
against a new reference version.

## Setup

```bash
python -m venv .venv-parity
.venv-parity/bin/pip install "vectorbt==1.1.0" "plotly<6"
```

The `plotly<6` pin is load-bearing: vectorbt 1.1.0 references the
`scattermapbox` trace, which plotly 6 removed, and importing vectorbt fails
without it.

## The three harnesses

| Generator | Fixture | Rust test | Covers |
|---|---|---|---|
| `generate_execute_order.py` | `execute_order.tsv` | `phasor-exec/tests/parity.rs` | One kernel call in isolation |
| `generate_order_sequence.py` | `order_sequence.tsv` | `phasor-exec/tests/parity_sequence.rs` | State threaded across chained calls |
| `generate_from_orders.py` | `from_orders.tsv` | `phasor-sim/tests/parity_from_orders.rs` | The simulation loop driving both |

They are layered on purpose. Because the kernel is already pinned by the first
two, a failure in the third is almost certainly the loop rather than the
arithmetic — the search space is already narrowed before you start reading
code.

The loop fixtures encode array parameters with a prefix, which also exercises
all four broadcasting paths: `s:` scalar, `c:` per column, `r:` per row, `f:`
full matrix (column-major, matching the engine's layout — NumPy is row-major,
so the generator reshapes with `order="F"`).

The sequence harness replays a chain feeding **its own** outputs forward,
reading only the initial cash from the fixture. Re-seeding from the expected
column each step would collapse it back into the single-call test, since a
drift introduced at step 2 would be wiped before step 3 could amplify it.

Its chaining rule is fixed on both sides, because `val_price` and `value` are
not kernel outputs — a simulation loop derives them from the bar:

```text
state = flat(init_cash)
for each step i:
    val_price = price path
    value     = state.cash + state.position * val_price
    state, result = execute_order(state, order[i])
```

That rule is also what stage 3's simulation loop has to implement, so this
harness doubles as its acceptance test.

## Regenerating

Each `generate_*.py` writes the one fixture its harness replays, so regenerate
the one you touched rather than all of them:

```bash
.venv-parity/bin/python tests/parity/generate_execute_order.py
cargo test -p phasor-exec

# or, to rebuild every fixture at once
for g in tests/parity/generate_*.py; do .venv-parity/bin/python "$g"; done
cargo test --workspace
```

Alpha158 and Alpha360 use the already initialized local Qlib Conda environment:

```bash
conda run -n rdagent4qlib python tests/parity/generate_qlib_alpha158.py
conda run -n rdagent4qlib python tests/parity/generate_qlib_alpha360.py
cargo test -p phasor-expr --features polars --test parity_qlib
```

The generators run Qlib's own Alpha158 and Alpha360 expressions over
deterministic panels; they replace only Qlib's leaf feature provider, so no
market-data download is needed. The committed TSV files contain both f64
inputs and Qlib expression outputs; matching the Rust matrix dtype keeps the
tests focused on operator semantics rather than float32 cancellation.

Then **read the diff**. Fixtures are seeded, so a changed line means the two
implementations now disagree somewhere — either you changed the reference
version, or you changed what a case covers. An unexplained change is a finding,
not noise.

## Granularity

One fixture is one `execute_order` call, not one simulation. Diffing whole
simulations tells you *that* something drifted; diffing single calls tells you
*which* stage of the size cascade did it.

## What is deliberately not covered

`reject_prob > 0` consumes NumPy's global RNG, so its outcome is not
reproducible across implementations. Our kernel takes the random draw as an
argument instead of owning a generator, and random rejection is tested on the
Rust side against injected draws.

## Keeping the fixtures honest

A differential test that passes trivially is worse than none. Two guards:

1. `fixtures_cover_every_reachable_status_info` fails if the set stops
   exercising a rejection path.
2. Mutation testing. Before trusting a harness, break the kernel on purpose
   and confirm it fails.

   | Mutation | single-call | sequence |
   |---|---:|---:|
   | Short reservation factor `2` → `1` | 72 | — |
   | Sell slippage sign flipped | 42 | — |
   | Fees added on top of the cash cap instead of inside it | 11 | — |
   | Skip granularity after the cash back-solve | 9 | — |
   | `is_close_or_less` → plain `<=` | 3 | — |
   | Ignore-checks moved after validation | 2 | — |
   | Entry zero-snap removed **alone** | 0 | 0 |
   | `add()` → `+` for cash **alone** | 0 | 0 |
   | `add()` → `+` for position **alone** | 0 | 0 |
   | Entry snap **and** `add()` for cash | 0 | 2 |
   | Entry snap **and** `add()` for position | 0 | 2 |
   | All snapping removed | 0 | 4 |

   `is_close_or_less` → `<=` originally survived, which is why the
   `tolerance_boundary_*` cases exist. Add fixtures whenever a plausible
   mutation survives and a fixture *can* distinguish it.

## Why the zero-snapping rows look like that

Two mechanisms keep balances on exact zeros: the entry snap at the top of
`execute_order`, and `add()` in every running-balance update. **Removing either
one alone is undetectable, because the other covers for it.** That is a real
property of the design — they are deliberately redundant — not a gap in the
fixtures.

Within one call, nothing can see the difference: the residues are ~1e-17 against
a `abs_tol = 1e-12` comparison floor, five orders of magnitude below the
threshold at which the two implementations are defined to agree.

Across a sequence it becomes visible, but only when both guards are gone,
because the amplifier is a *discrete* branch downstream:

- **Cash path.** The tolerant affordability check lets a buy through when the
  required cash sits a few ULPs above the cash on hand. Snapped, the balance
  lands on exactly `0.0` and the next order is rejected with `NoCashLong`
  (which tests `cash_limit == 0.0`, exactly). Unsnapped it lands at about
  `-1.8e-15` — a negative balance, which is an invariant violation, so the next
  step turns from a rejection into an error. `snap_amplifier_negative_cash*`.
- **Position path.** Buying twice by value performs two divisions and selling
  the combined notional performs a third; they do not cancel, leaving the
  position about `7.1e-15` long instead of flat. Snapped, a following
  `ShortOnly` order opens a short normally. Unsnapped, the column is nominally
  long, so `ShortOnly` is an invariant violation. `snap_amplifier_position_*`.

The two amplifiers had to be found, not guessed. An earlier attempt at the
position case closed the position with the same number it had opened with,
which cancels exactly and amplifies nothing.

**The general lesson: when a comparison is tolerance-based, fixtures that only
sample the interior cannot test it. Aim at the boundary, then find the discrete
quantity — a lot size, an exact-zero branch, a status code — that turns a
sub-tolerance difference into an observable one.**

## Mutations for the simulation loop

All caught, against `from_orders.tsv`:

| Mutation | Cases |
|---|---:|
| Record row index off by one | 239 |
| Order price `+inf` resolves to the previous close, not this one | 150 |
| Portfolio value omits the position leg | 57 |
| `debt` not threaded between bars | 48 |
| `free_cash` not threaded between bars | 37 |
| `free_cash` not seeded from `init_cash` | 67 |
| `init_cash` read from column 0 for every column | 35 |
| Valuation `-inf` resolves to this close, not the previous | 26 |
| Valuation `+inf` resolves to the close, not the order price | 25 |
| Forward-fill of the valuation price ignored | 3 |
| `update_value` applied unconditionally | 2 |
| Post-order valuation write-back deleted | 1 |

Three of these needed new fixtures, and the reasons generalize:

- **Flags interact.** `update_value` re-marks the valuation price to the fill
  price. Covering it and `ffill_val_price` separately proved nothing: the write
  is overwritten by the next bar unless *that* bar's valuation is unusable and
  forward-fill is on. Only a fixture at the intersection can see it — and the
  order price has to stay valid on the gap bars, or nothing fills there and the
  difference never reaches a record.
- **A default that is always the same tests nothing.** Every fixture passed a
  scalar `init_cash`, so reading column 0 for every column was invisible.
  Per-column starting capital is part of the sweep shape and now appears in the
  fixtures.
- **Some code is provably dead.** The reference guards the post-order valuation
  write with a NaN test that, in this loop, can only ever assign NaN over NaN.
  Rather than carry a branch no fixture can reach, the guard was removed and the
  reasoning recorded at the call site. Same for the value half of the reference's
  `update_value`, which this loop recomputes from scratch each bar. Both become
  live when cash sharing arrives.

## Mutations for the trade views

All 19 caught, against `trades.tsv` plus the exactness tests described below:

| Mutation | Result |
|---|---|
| Entry view charges every entry the full exit fees, not its share | caught |
| Entry view uses the first entry's price as the position exit price | caught |
| Entry view forgets the first entry's carried size and fees | caught |
| Entry view flushes exits instead of entries | caught |
| Entry view treats an exact close as a partial one | caught |
| Entry view does not prorate a reversing fill's fees | caught |
| Exit view uses the last entry price, not the weighted average | caught |
| Exit view does not scale the remainder down after a partial exit | caught |
| Exit view gives the reversing fill's whole fee to the closing trade | caught |
| Exit view does not flip direction on a reversal | caught |
| Exit view does not snap an almost-full exit to the position size | caught\* |
| Positions take the first trade's status, or the first exit index | caught |
| Positions sum prices instead of size-weighting them | caught |
| Positions re-derive a one-trade position instead of copying it | caught\* |
| `pnl` does not negate a short's value difference | caught |
| `add()` → `-` in the trade statistics | caught\* |
| Parent ids restart per column | caught |

\* **Not by the differential harness.** Three of these change values by less
than the tolerance that record comparison uses, so no fixture at that
granularity can see them — the same structural situation as the kernel's
zero-snapping pair, and confirmed by measurement rather than assumed: with the
tolerant `add` removed, the number of positions with exactly zero PnL across
the whole fixture set did not move.

What these three guard is *exactness*, so the right instrument is an exact
assertion, not a tolerant comparison. Each has a unit test:

- **A break-even round trip returns exactly zero.** Entering in three lots and
  exiting at the same price is mathematically break-even, but the averaged
  entry price does not divide out: the case in the fixtures leaves 5.7e-14.
  `add` snaps it. Plain subtraction leaves it, and a downstream `pnl > 0` then
  counts the trade as a winner — which is where this would first do damage, so
  the analysis layer should re-run this mutation when win rate lands.
- **An almost-full exit records the whole position size**, not the order's.
- **A one-trade position equals its trade bit for bit**, asserted on
  `to_bits()`.

The third took two searches to make bite, and both failures are instructive.
Re-deriving a position computes `sum(size * price) / size`; the *exit* view
already stores a divided price, so re-deriving it is idempotent and the
mutation survives — the test has to use the entry view, which stores the raw
order price. And most size/price pairs round-trip through that division
anyway: `5.128 * 58.78 / 5.128` is `58.779999999999994`, but the first pair
tried was not like that. **A mutation that survives an exactness test may be
telling you the test picked cooperative numbers.**

## Mutations for the analysis layer

22 mutations across the rebuild chain, the metrics and the trade statistics.
The end-to-end harness alone caught 17; the full suite catches 21. The gap is
itself the finding, and each part of it says something different:

- **Two needed new fixtures, and both were reachable all along.** Nothing in
  the set drove a portfolio's value negative or to zero, so the two non-obvious
  branches of `period_return` — flip the sign of a return measured from a
  negative base, and give a signed infinity rather than `NaN` from a zero base
  — were untested. A short into a rising market and a portfolio with no capital
  cover them. **A branch that exists for a rare case needs a fixture that is
  actually rare**, not a random sweep that never wanders there.
- **Two are unreachable through `from_orders` and belong in unit tests.**
  Several fills landing on one bar cannot happen in this loop — it issues one
  order per element — but will in `from_signals` with stops and in the
  event-driven tier, so the accumulation is written and unit-tested now.
  Likewise the per-column position reset in `asset_flow`: only the directional
  flows read that position, and the end-to-end chain only ever asks for
  `Direction::Both`.
- **One is genuinely undetectable, and that is worth knowing.** `sharpe_ratio`
  and `annualized_volatility` guard against a series shorter than two bars, and
  removing either guard changes no output: both route through `nanstd`, whose
  `ddof` arithmetic already returns `NaN` in that case. `sortino_ratio`'s
  identical-looking guard *is* load-bearing, because it divides by
  `downside_risk`, which has no `ddof` and returns a finite number from one bar.
  Same three lines, two of them redundant and one not. All three are kept —
  they are all in the reference — and the asymmetry is recorded in the module
  docs, since nothing in the code shows it.

## Mutations for the signal pipeline

All 17 caught, against `signals.tsv`.

The fixture set there is **exhaustive** rather than sampled, which is unusual
in this project and worth saying why: the pipeline's inputs are a position
sign, a few booleans and some small enums, so the whole space is about five
thousand rows. When a space is that small, sampling it is choosing to test
less for no reason — and the cases a sample misses are exactly the ones that
matter. `Adjacent` with a flat position drops *both* signals rather than
picking one; `CloseReduce` differs from `Close` in one field out of five;
`Percent` is rejected only when a position is actually being reversed.

Being exhaustive over the enums does not make it exhaustive, and the mutations
were still worth running: `position` takes three values and `size` five, so a
mutation that only misbehaves at some other magnitude would survive. None of
the seventeen did.

One mutation is caught by a unit test rather than the fixtures. The reference
folds the direction-mode mapping (one `entries` array plus a `direction`) into
its main loop rather than exposing it, so there is nothing to generate
fixtures against. It is the mapping where an "exit" in a two-sided column
means a *short entry* rather than a close, which is easy to get wrong and easy
to state as a test.

## Mutations for the `from_signals` loop

24 mutations; 20 caught, and the four survivors are provably redundant rather
than untested.

The first pass caught 16. Everything the gap branch touches was caught
immediately — a stop that always fills at its level, one that fills outside
the bar's range, a trailing stop updated before it is tested rather than after
— which is the reassuring part, because those are the ones that would quietly
delete the largest losses from a backtest.

**Four needed fixtures, and every amplifier had to be constructed.** The
pattern from the trade views held: it is not enough for a mutation to change a
number, it has to change *whether or where a stop fires*.

- Reconstructing a missing high and low from the open and close is invisible
  when only the close is given — the reconstructed range collapses to a point
  and the gap branch fires first anyway. The case needs an open *and* a close
  but no high or low, with the stop level between them.
- Re-anchoring a stop on a position *increase* only when the update mode says
  so needed a genuine partial exit. The first attempt used one size array for
  both the entry and the exit, so the exit closed the position outright and
  took the clearing branch instead. Entry of 5, exit of 2.
- `Override` with a `NaN` stop must keep the existing stop rather than
  overwrite it with `NaN` and disarm the stop entirely, so the stop distance
  has to vary down the column.
- The valuation price is resolved from the *pre-stop* order price. The
  existing `StopEntryPrice::ValPrice` case had an order price of 10.5 against
  a close of 10.0, and both anchors happened to gap through the next bar's
  open at the same fill. Widening that to 12.0 against 10.0 moves the level
  from 10.8 to 9.0, and only one of them fires.

**Four survivors, redundant by construction** — verified against the code, not
assumed:

| Guard | Why it changes nothing |
|---|---|
| Clearing stops when a position closes | `stop_price` returns `NaN` while flat, and opening always re-anchors |
| Resolving conflicts only when an entry fired | all three resolvers already guard on an entry |
| Testing `use_stops` before acting on a trigger | the trigger is only assigned inside a block that tested it |
| Adding the position leg to the portfolio value | `value` is read only by the kernel's `TargetPercent` branch, which this loop cannot produce |

All four are kept: they are in the reference, and three become load-bearing
the moment cash sharing or a callback tier arrives. The last is the
interesting one — computing the value *wrongly* now would cost nothing today
and be a trap set for whoever enables `TargetPercent`.

## Mutations for the array layer

25 mutations against `generic.tsv`; 24 caught, and the survivor is provably
equivalent rather than untested.

**One mutation was a real bug, found before the fixtures existed.** The
reference computes the rolling variance as `window_len * mean**2`, and the
first port wrote `len * mean * mean` — which associates as `(len * mean) *
mean`. On a constant series that is the difference between exactly zero and
1.07e-9. A unit test caught it; the fixtures would have too.

**Two are exactness properties no fixture can reach**, the same structural
situation as the trade views, and both needed their case searched for:

- `a.mul_add(b, c)` rounds once where `a * b + c` rounds twice. It is faster
  *and* more accurate, and therefore wrong: the reference rounds twice. It
  diverges bitwise on every fixture series, by about one unit in the last
  place — far below the comparison tolerance. The test recomputes the fused
  form alongside and requires the two to differ.
- The exponential mean skips its own update when the value has not changed,
  which the reference comments as "avoid numerical errors on constant
  series". Most constants survive the round trip unchanged, so the obvious
  test proves nothing: `0.1` at span 5 is stable either way. Searching found
  `8.19478` at span 10, which drifts to `8.194779999999998`. **A guard that
  exists for numerical reasons needs a case that is numerically awkward, and
  the first value you reach for will not be.**

**The survivor.** `ffill` seeds its carry from element zero, and seeding from
`NaN` instead is identical: the seed is only ever read when element zero is
itself `NaN`. The reference's spelling is kept because it says what it means.

### What the fixture series are for

Each one exists because a plausible implementation gets it wrong and a
well-behaved series would not notice: leading and trailing gaps, interior
gaps, all-`NaN` (where a maximum must stay `NaN` rather than becoming
`-inf`), constant, shorter than its own window, and `wide_scale` — values
spanning sixteen orders of magnitude, which is what separates the
accumulating rolling mean from a re-summing one. A second test asserts those
series are still present and that `wide_scale` still spans the range, because
a fixture set can lose its teeth by being tidied.

## Mutations for the indicators

24 mutations against `indicators.tsv`, all caught — but only after two rounds,
and both gaps were the same mistake in different clothes: **a grid that varies
everything together varies nothing.**

- The MACD grid had `macd_ewm` and `signal_ewm` set to the same value in every
  row, so swapping one for the other changed no output. Two rows where they
  differ fixed it.
- No fixture had a missing *volume*, only missing prices. On-balance volume
  compares `close` against the previous close, and a `NaN` close makes that
  comparison false rather than producing a `NaN` — so the only way to reach
  the accumulator's `NaN` guard is through the volume series.

The sweep layout gets its own test rather than a fixture. Each fixture pins
one parameter combination, which says nothing about how a sweep arranges
several: `a_sweep_equals_its_parts` runs four windows at once and requires
each one's columns to be bit-identical to running it alone, in the position
`column_of` predicts. It includes a repeated window, so the cache is exercised
and still has to produce its own columns.

## Mutations for cash sharing

24 mutations against `cash_sharing.tsv`; 21 caught. The three survivors are
the interesting part, and two of them are provably unobservable rather than
untested.

This is the feature phase 1 deliberately left out, and the one place columns
stop being independent. Everything else in the engine is a self-contained
per-column simulation; in a shared group each fill changes what the next
column can afford, so the order the columns execute in is part of the result.

**Every case is generated both ways** — layout order and sorted by order value
— and a test asserts that at least one case's fills actually differ between
them. Without that a port ignoring the sort would pass the whole harness by
producing the same records twice.

Two bugs came out of it, both predicted by a comment written months earlier.

The first: **the group's value is threaded through the bar, not re-derived
after each fill.** Re-summing the group after every fill is the obvious
implementation and it is what this port did first. It is wrong — it makes a
rebalance's later legs see a book the earlier legs already changed, where the
reference sizes every order in a bar against the value the bar opened with.
The difference only shows up with costs, which is why it took a
`TargetPercent`-with-fees fixture to find.

The second: **`update_value` was dead code until this feature existed.** The
comment where the arithmetic should have gone said exactly that — with one
column per group the value is re-derived at the top of every bar, so anything
written there was overwritten before it could be read, and mutating it changed
no output. It said "re-add it there, with a fixture that can see it", and that
is what happened.

| Mutation | Result |
|---|---|
| The group value is re-summed after each fill | caught |
| The group value is not threaded at all | caught\* |
| The group value excludes the positions | caught |
| The call order is ignored | caught |
| Cash is split per column instead of shared | caught |
| Free cash is not threaded across columns | caught |
| `update_value` drops the cash flow, or the asset-value change | caught |
| `update_value` re-derives instead of carrying | caught |
| The auto sort is unstable, or puts `NaN` first | caught |
| The order estimate ignores the position | caught |
| The order estimate leaves `Amount` / `TargetAmount` unpriced | caught\* |
| The order estimate prices `Value` a second time | caught\* |
| `TargetPercent` estimated against cash; `TargetValue` not net of the position | caught |
| A short-only size is not negated | **survived** |
| The `2 ×` in the short reserve is dropped | **survived** |
| `Percent` sells take the general branch under long-only | **survived** |

\* Needed a fixture or an amplifier that did not exist yet.

### Why the three survived, measured rather than guessed

An order-value mutation can only reach the output through two conditions
holding at once: it has to **change the ranking**, and cash has to be **binding
at that moment** so the ranking decides who gets filled. Guessing at amplifiers
was going nowhere, so a probe computed the two orderings directly for every
generated case and reported which condition failed.

- **The short-only negation** changes the ranking in five cases and changes no
  fills in any of them. Opening a short *raises* cash, so a group of them is
  never starved. Cases were added where one column buys its short back while
  another opens one — the only shape where the order can bite — and it still
  did not bind. That one is a genuine hole, and saying so is better than
  claiming an amplifier that does not exist.
- **The other two never change the ranking at all, and cannot.** Free cash is
  shared across a group, so both mutations are monotone transforms of the same
  quantity: `a ↦ k·a + f` preserves order for any `k > 0` and any shared `f`.
  Mixed signs do not rescue it either, since a buy's estimate is positive and a
  sell's is negative, so the sell sorts first whatever the constants. **These
  are not gaps in the fixtures — there is nothing for a fixture to see.**

That distinction is the point of running the sweep: "no test can reach this"
and "no test exists for this yet" look identical in a survivor list and mean
opposite things.

## The statistics table, and why it needs the user-facing API

`stats.tsv` is the only fixture set generated through the reference's
`Portfolio.stats()` rather than its kernels, and that is the whole point.
**A port can agree with every kernel and still print a table nobody can line
up against the reference's**, because the row titles, the row order, the
percent scalings and the open-trade default are not visible from the kernel
tier at all.

It found three real bugs in code the kernel-tier harnesses had already passed:

- **Trade and drawdown durations were off by one, sometimes.** The reference
  measures a *closed* range exclusively and an *open* one inclusively:
  `end - start` against `end - start + 1`. That is not arbitrary — a closed
  range's end bar belongs to whatever came next, while an open one's last bar
  is one it still covered. Using a single convention shifts every average
  holding period by up to a bar, in a direction that depends on how many
  trades happened to be open. Fixed in `TradeRecords`, `DrawdownRecords` and
  `RangeRecords` alike, since all three are the same kind of range.
- **Gross exposure divided by the wrong thing.** It is
  `|asset value| / (|asset value| + free cash)`, not over portfolio value —
  and *free* cash, which an open short holds down by the margin it reserves.
  The two denominators agree exactly on a long-only column, so only the
  shorting fixtures could see it.
- **Free cash was not derived at all**, and a comment claimed it could not be
  without carrying the kernel's `debt` state out of the simulation. It can:
  `debt` is a function of the fill sequence alone, which is the same reason
  the whole rebuild chain works.

One semantic was wrong before a fixture existed to catch it, and is worth
stating on its own: **the trade-quality rows count closed trades only.** Win
rate, best and worst trade, the winning and losing averages and durations,
profit factor and expectancy all exclude trades still open, and the default is
the right one — an open trade's PnL is a mark to market, not a realised
result, so counting it makes the win rate part measurement and part
projection, and the projection moves every time the last bar does. The three
trade *counts* are not filtered, because they report the split. `incl_open` is
how a caller opts in, and the fixtures generate both settings; a test asserts
that at least one case's table actually differs between them, so a port that
ignored the flag could not pass.

The other thing the generator had to discover: without a frequency set, the
reference silently **drops** the four annualized ratio rows from the table
rather than computing them. It emits a warning, not an error, and a generator
that ignored warnings would have produced a 24-row table and a harness that
cheerfully agreed with it.

## Mutations for the drawdown records

12 mutations against `drawdowns.tsv`; 11 caught, and the survivor is
unreachable.

The fixture set enumerates every path of up to five bars over a three-value
alphabet, plus every path of up to four bars over two values and a gap. Three
levels is the smallest alphabet that can express a peak, a valley and a
partial recovery, and equal neighbours cover the flat stretches. That makes
the four boundary cases certain rather than likely: a flat stretch at the top
must extend the peak rather than open an episode; regaining *exactly* the old
peak counts as recovery; a V-shaped rebound straight into another decline is
two records and not one; and a gap on the *last* bar cannot close an episode,
so a column ending on one has no active record at all.

| Mutation | Result |
|---|---|
| An equal bar opens an episode instead of extending the peak | caught |
| Recovery requires exceeding the old peak rather than matching it | caught |
| `start_idx` equals `peak_idx` instead of following it | caught |
| The valley is not updated on a new low | caught |
| Missing bars are not skipped | caught |
| No active record is stored at the last bar | caught |
| State is not reset after storing a record | caught |
| Decline duration not inclusive; recovery duration inclusive | caught |
| The drawdown's sign is flipped | caught |
| Recovery return measured against the peak | caught |
| The nested recovery test inside the running branch | **survived** |

The survivor is worth reading, because the argument that it is *not* redundant
is convincing and wrong. The reference tests `cur >= peak_val` again inside
the branch that already tested `peak_val.is_nan() || cur >= peak_val`, and the
obvious reading is that the nested test catches the `NaN` case the outer one
lets through. But the running flag can only have been set in the branch
reached when the outer test *failed*, which requires `peak_val` to be a
number. So it is never `NaN` there, and the outer test has already established
the inequality. The code comment asserting otherwise was mine, and was wrong
until the mutation sweep contradicted it.

## Mutations for the return metrics

25 mutations against `metrics.tsv`; 22 caught, and all three survivors are
provably equivalent rather than untested.

`generate_analysis.py` already ran a whole simulation and checked the metrics
at the end of it. This fixture set exists because **a simulation will not
readily produce the inputs the metrics disagree on**: a column that is entirely
missing, one that never moved, one whose benchmark never rose, one whose equity
went to infinity. Those are hand-built here, and crossed with the parameters
that change a *branch* rather than only a number — an annualization factor of
exactly 1 (which skips omega's threshold conversion) and one at -1 (which has
no real root), cutoffs at 0 and 1, `min_periods` below the window.

That paid for itself immediately. `max_drawdown` was wrong, and had been since
it was written: the reference takes a *plain* minimum of the drawdown series,
not a `NaN`-skipping one, and this port skipped. The two agree on every series
a simulation is likely to produce, because a missing return never reaches the
drawdown series — the level carries through a gap. They diverge on an infinite
return, which drives level and peak both to infinity and leaves their ratio
undefined. Skipping answers `0.0` there, which reads as *"this column never drew
down"* — the most flattering possible reading of a column whose equity blew up,
and one that would have sorted it to the top of a sweep.

A second finding is about how the reference is compiled rather than what it
says. `np.percentile` under Numba is not `np.percentile` under NumPy: Numba
interpolates as `lower * (1 - m) + upper * m` where NumPy switches formula at
`m >= 0.5`, and the textbook `lower + m * (upper - lower)` is a third spelling
again. All three are algebraically identical and all three round differently,
and three metrics here are built on a percentile. The reference's kernels are
Numba-compiled, so Numba's spelling is the one to match.

A third is a hazard this port introduces rather than inherits. Ordering by
`f64::total_cmp` reads the sign bit, so a *negative* `NaN` sorts below negative
infinity — where NumPy's partition puts every `NaN` last whatever its sign.
Nothing constructs one deliberately, but `0.0 * -inf` and a negated `NaN` both
produce one, and that would move a missing bar from the top of the ordering to
the bottom: from a value the tail metrics ignore to the worst one they saw.

| Mutation | Result |
|---|---|
| `max_drawdown` skips `NaN` instead of propagating | caught\* |
| Percentile interpolates the textbook way, or NumPy's way | caught |
| Percentile rank drops the `+1`, or divides by `n` not `n - 1` | caught |
| `NaN` not sorted last | caught |
| Omega's threshold not de-annualized | caught |
| Omega's `ann_factor <= -1` guard dropped | caught |
| Tail ratio uses 90/10, or drops the `abs` on the left tail | caught |
| Value at risk stops filtering `NaN` | caught |
| Conditional VaR filters `NaN` like its neighbours | caught |
| Conditional VaR rounds the cutoff index, or uses `len` not `len - 1` | caught |
| Beta does not mask the benchmark where the column is missing | caught |
| Beta's 1e-30 blanking removed | caught |
| Alpha does not subtract beta's share, or annualizes by multiplying | caught |
| Capture's up/down filter swapped, or keeps flat benchmark bars | caught |
| Information ratio ignores `ddof` | caught |
| Rolling benchmark window left-anchored, or off by one | caught |
| Rolling window filtered before the reduction is called | caught |
| Rolling `min_periods` compared against window length, not valid count | caught |
| Rolling window not right-anchored | caught |
| Percentile's `q == 0` / `q == 100` bypass removed | **survived** |
| Omega's `< 0` on the losing side weakened to `<= 0` | **survived** |
| Beta centres the returns as well as the benchmark | **survived** |

\* Found by the fixtures, not by a mutation — the mutation is the regression
test written afterwards.

All three survivors are equivalences, and each for a different reason:

- **The percentile endpoint bypass** is redundant because at either endpoint
  the interpolation weight is exactly zero, so the general path reduces to the
  same order statistic — and it reproduces the reference's strange-looking
  infinity rules *by accident*, since `inf * 0.0` is `NaN` exactly where those
  rules say `NaN`. Checked over every arrangement of infinities and finite
  values up to four elements.
- **Omega's strictness** does not matter because a bar exactly at the threshold
  contributes zero to whichever sum it joins.
- **Beta's centring** is algebraically a no-op: the correction term is
  `mean(residual)` times a constant, and that mean is zero by construction. It
  changes only the last bits — below the tolerance this layer is compared at,
  and distinguishing it would require summing in exactly NumPy's pairwise
  order, which this port does not claim to do. That one is worth being explicit
  about: it is the difference between "the fixtures cannot see this" and "there
  is nothing here to see".

## Mutations for the mapped-array layer

22 mutations against `mapped.tsv`; 21 caught, and the survivor is a shortcut
that provably changes nothing.

**The fixture set is exhaustive at these sizes.** The input is a column
assignment plus a value vector, so every assignment of up to four records to up
to three columns is enumerated — 157 of them — crossed with value vectors
chosen for the branch each one reaches. Two cases a hand-picked set would drift
away from are therefore guaranteed: the column that gets *no* records, and the
assignment that interleaves two columns and so forces the gathering path.

This layer produced the clearest example yet of **a generator that confirms the
wrong thing**. The first version reached for `np.sum`, `np.min` and `np.mean`
as its reduce functions — the obvious choices — and every fixture passed
against a port that propagated `NaN`. But `MappedArray.sum()` does not dispatch
to `np.sum`; it dispatches to `generic_nb.sum_reduce_nb`, which is `np.nansum`,
and the same is true of `mean`, `min`, `max` and `std`. Taking the reference's
own kernels rather than what they look like they do turned 2,700 green rows red
and found four reductions that were wrong.

Two consequences of that fix are pinned by name, because neither is the obvious
reading:

- **`sum` of an all-`NaN` column is `0.0`, but its `mean` is `NaN`.** A sum of
  no values is zero however the values went missing; a mean of none is not.
- **`count` is the one reduction that counts `NaN`s**, because it reads the
  segment lengths straight off the map rather than reducing anything. Paired
  with any other reduction it is what says how much of a column was measurable.

A third divergence is **deliberate and documented**: `reduce_to_idx` returns the
fill value for a column with nothing comparable, where the reference raises
`ValueError("All-NaN slice encountered")`. A sweep has too many unmeasurable
columns for that to be exceptional. The index fixtures therefore skip all-`NaN`
columns and a unit test pins the port's answer instead.

| Mutation | Result |
|---|---|
| Contiguity test `<` → `<=`, so sorted records take the gathering path | caught |
| Counting sort fills each column back to front | caught |
| Gathering path reverses a column | caught |
| Reductions visit empty columns instead of using the fill | caught |
| `sum`/`mean`/`min` propagate `NaN` instead of skipping it | caught |
| `mean` divides by the record count, not the measurable count | caught |
| `count` counts measurable values instead of records | caught |
| `std` simplifies `(ss / n) * n / rcount` to `ss / rcount` | caught |
| `std` guard `cnt <= ddof` → `cnt < ddof` | caught\* |
| `arg_extreme` substitutes the wrong infinity for `NaN` | caught |
| `arg_extreme` skips `NaN` instead of substituting | caught |
| `arg_extreme` breaks ties toward the last | caught |
| `reduce_to_idx` returns the position instead of the `idx_arr` entry | caught |
| Top-N and bottom-N break ties from the same end | caught |
| Top-N pivot off by one | caught |
| Top-N takes values equal to the pivot as strictly beyond it | caught |
| `stack_expand` bottom-aligns instead of top-aligning | caught |
| `value_counts` assigns instead of accumulating | caught |
| Top-N `n >= col_len` shortcut weakened to `n > col_len` | **survived** |

\* Two needed a fixture or test that did not exist yet.

The `std` guard looked redundant — at `cnt == ddof` the division by zero
already yields `NaN`. It is not: at `ddof > cnt` the unsigned subtraction
underflows and panics, and at `cnt == 2, ddof == 2` the unguarded expression
answers `+inf` rather than `NaN`. The fixtures could not see any of it, because
the reference's kernels are only ever called with `ddof` 0 or 1. A unit test
covers it now — **an unreachable-by-fixture branch is a coverage gap, not a
redundant guard, and the two are worth telling apart.**

The genuine survivor is the top-N shortcut. At `n == col_len` the general path
below it reaches the same all-true answer by three different routes, depending
on how many of the column's values are measurable. Kept because the reference
has it and because it saves a selection over a column that was going to be
taken whole.

## Mutations for the signal layer

25 mutations against `signals_gen.tsv`; 24 caught, and the survivor is
redundant by construction.

**The fixture set is exhaustive at these sizes**, like the signal pipeline's
and for the same reason: the inputs are boolean arrays, so the awkward cases
are about *shape* rather than magnitude, and every array of up to six bars is
126 of them. `clean_enex` takes two arrays, so it enumerates every pair of up
to five bars — 2731 cases. Nothing is left to chance about leading runs,
trailing runs, alternating bars or empty columns, and none of those had to be
thought of individually.

Two behaviours are asserted because they read the opposite way at a glance,
and both were *my* mistake before they were tests — the first draft of each
had the expectation backwards and the reference settled it:

- **A reset inside a run renumbers from that bar.** It does not wait for the
  run to end. The guard that looks like it prevents that (`reset > previous
  run end`) is doing something else: stopping a *stale* reset, one from before
  the previous run finished, from firing again in this one.
- **`clean_enex` on a bar carrying both an entry and an exit produces
  neither.** An array that says both has not said anything, and picking one
  would pre-empt `from_signals`' five conflict modes — which belong there,
  where a position exists to resolve against.

The survivor: `normalized_average_index` guards against a one-row column, and
with one row the rescale divides by zero and already yields `NaN`. Kept
because the reference has it.

## Running mutations

There is no mutation framework here — dependencies cannot be fetched in this
environment. A textual patch, a test run, and a restore is enough:

```python
# scratch mutator: fails loudly if the pattern does not match, so a
# "SURVIVED" can never be a silent no-op
import sys, pathlib
p = pathlib.Path(sys.argv[1]); t = p.read_text()
old, new = sys.argv[2], sys.argv[3]
if old not in t:
    sys.exit("MUTATION NOT APPLIED")
p.write_text(t.replace(old, new, 1))
```

That check matters. A first pass at the loop mutations used `sed` with
multi-line patterns, which silently matched nothing, and reported four
"survivors" that were never actually applied.

## Shared-account Python boundary

Install the local reference into the binding test environment and generate the
full account oracle from the phasor repository root:

```sh
python -m pip install -e ../vectorbt
python tests/parity/generate_shared_accounts.py
pytest crates/phasor-py/tests/test_shared_accounts.py -q
```

`shared_accounts.json` records 48 cases with deterministic inputs, orders, all stored log
fields, symbol-level trades/positions, account cash/value/returns, and statistics
for both order and signal paths. It covers default/auto execution, multiple
accounts, short positions, reversals, costs, stops, missing prices and valuation
updates. Committed-oracle tests run without vectorbt; the live test repeats the
reference calculation when vectorbt is installed. Shared-account benchmarks are
intentionally absent in phasor, so that statistics row is checked as `None`.
Missing valuation bars keep the existing per-bar return semantics. Shared-account
total return uses final equity against initial capital, so intermediate missing
prices cannot erase part of the account's PnL.
