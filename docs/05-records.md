# Records

Records are the contract between the two tiers. Both emit the same schemas, so
the analysis layer is written once and either tier's output can be fed to it —
or compared against the other's. See [01 Architecture](01-architecture.md).

## Schemas

| Schema | Fields | Notes |
|---|---|---|
| `order` | 7 | `id, col, idx, size, price, fees, side` |
| `trade` | 14 | adds entry/exit index, price and fees, `pnl`, `return`, `direction`, `status`, `parent_id` |
| `log` | 38 | complete before/after snapshot: pre-state, the full request, post-state, result. Opt-in per order |
| `range`, `drawdown` | 5 / 10 | derived, for duration and drawdown statistics |

Every schema carries `id` and `col`. That is the invariant the whole layer rests
on: records are a flat, column-tagged stream, and every aggregation is a
segmented reduction over `col`.

**Struct-of-arrays, not array-of-structs.** Records hand off to Arrow without a
transpose, and a query touching one field does not pull the other thirteen
through cache.

## The three trade views

One schema, three different semantics. This is the most commonly mis-ported part
of the design:

| View | One record per | Attribution |
|---|---|---|
| `EntryTrades` | entry fill | Exits allocated back to entries, FIFO within the position |
| `ExitTrades` | exit fill | Entries averaged into it |
| `Positions` | position | Trades sharing a `parent_id` aggregated |

They produce different trade counts and different per-trade PnL from identical
order records. A win rate computed over `EntryTrades` is not the same number as
one over `ExitTrades`, and neither is wrong — they answer different questions.
`parent_id` is the link from trades to their position.

## `MappedArray`

The reduction primitive: a flat value array plus a parallel `col_arr`. Not a
matrix — a ragged, column-grouped vector. Every aggregation is a segmented
reduction over `col_arr`, with a `col_map` (indices grouped by column) built
once and shared across calls, since the grouping depends on `col_arr` alone.

**The grouping has a fast path this engine always takes.** Records come out in
column-major order by construction — that is the ordering contract above, and
what makes a parallel run reproducible — so each column's records are a
*contiguous run* and a reduction reads a subslice with nothing to gather and
nothing to allocate. `ColMap::build` detects that and `for_each_column` acts on
it. The general counting-sort path still exists, because records from a merge,
a filter, or phase 2's tier need not arrive sorted, and both paths produce
identical results.

Trade derivation is the one caller that *cannot* fall back: it walks a column's
fills in sequence to build positions, so unsorted records are an error there
rather than a slower path.

Reductions skip missing values rather than propagating them, which matches the
reference and is not the obvious choice. A record field can be missing for
reasons that say nothing about the other records — an open trade has no exit
return — and one such record should not erase a column's statistic. `count` is
the exception: it counts records, so pairing it with any other reduction says
how much of the column that other number came from.

## Drawdowns are records too

`max_drawdown` is one number out of a series that usually contained several
separate episodes. `DrawdownRecords` gives one record per episode — peak,
valley, end, and whether it recovered — so a column's drawdown history reduces
through the same `MappedArray` machinery as anything else.

**Ranges share a duration convention, and it is not the obvious one.** A
closed range is measured exclusively (`end - start`) and an open one
inclusively (`end - start + 1`). A closed range's end bar belongs to whatever
came next; an open one has no such bar and its last index is one it still
covered. Trades, drawdowns and signal ranges all follow it. Picking a single
convention instead shifts every average by up to a bar, in a direction that
depends on how many happened to still be running.

## Logs answer the question order records cannot

An order record says what happened. A log says what was *asked for* and what
the account looked like on both sides of the answer — 38 fields, one row per
order **considered** rather than per order filled.

That difference is the whole value. Without logs, a bar that did not trade
leaves no evidence at all, and "no order record" is consistent with a dozen
causes: the size resolved to zero, the cash ran out, the granularity floored
it away, the direction forbade it, the price was `NaN`. With a log the reason
is a field, and `reason_counts` tallies them per column — a strategy that
traded far less than expected almost always has one reason dominating.

It is off by default and per order, through the request's own `log` flag,
because a log is five times the width of an order record written twenty times
as often. The useful shapes are one column under investigation, or one row
across a sweep.

**"Considered" means the order reached the execution kernel, and the two loops
differ in how often that happens.** `from_orders` submits one order per element,
so it logs every element. `from_signals` reaches the kernel only when the
signals resolved to a nonzero size, so a bar with no signal gets no row — there
is nothing to explain about a bar the strategy had no opinion on, and a row per
bar would bury the interesting ones. That is the reference's rule, and it is
pinned by comparing row for row against it.

`order_id` joins a log to the fill it produced, or is `-1` when nothing
filled. **It is an index into the buffer it was written to, and is re-based when
buffers merge** — the same rule `id` follows, and for the same reason: a global
counter would make the numbering depend on thread interleaving. Getting the
re-basing wrong is quiet, because every index stays in range and simply names a
fill from another column.

Logging is additional, never alternative: a test asserts the same workload
produces identical order records with logging on and off, because an
investigation that changed the run would be investigating something else.

**Two logs are rarely `==`.** A rejected order fills nothing, so its `res_size`,
`res_price` and `res_fees` are `NaN` — and `NaN != NaN`. Any run that refused
anything compares unequal to itself, which makes the derived `PartialEq` a trap
rather than a tool.

## What exposing the logs found

Nothing in the record layer, and two things elsewhere. Both were invisible to
every fixture the project had, and both surfaced the moment all 37 fields were
compared against the reference row by row:

- **`min_size` defaulted to `0.0`, where the reference defaults to `1e-8`.** A
  `5e-9` order is rejected there and filled here. No fixture could catch it,
  because a randomly generated size is never dust and every fixture sets
  `min_size` explicitly. `req_min_size` is a log field, so the comparison failed
  on the first run.
- **`order_id` was not re-based across groups in the serial path.** The counter
  reset per group while every group wrote into one buffer, so group 1's logs
  pointed at group 0's fills. Nothing could observe it until a public entry point
  returned logs.

That is the argument for exposing a thing rather than merely implementing it:
the widest record either engine emits is also the one that checks the most.

## Open-position state must not be scanned

NautilusTrader's v2 engine beats its predecessor by 4–13x across almost every
benchmark scenario, and loses by **1.64x** on one: a single netting position
accumulating thousands of fills. The crossover sits between 1,000 and 4,000
fills. That is an O(n) scan over position history.

A factor sweep on minute bars lands squarely in that range, so open-position
lookup here is indexed, never linear.

## `RecordSink`

Writes go through a trait rather than directly into the vectorized tier's
preallocated column storage, so phase 2's event-driven tier can reuse the record
layer without inheriting that storage model. It costs nothing now; retrofitting
it would mean touching every write site.
