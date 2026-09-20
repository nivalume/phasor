# Column layout

## One axis for parameters and symbols

Every sweep collapses into a flat column axis:

```
col = param_idx * n_symbols + symbol_idx
```

With 3 windows over 2 symbols:

| col | 0 | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| `window` | 10 | 10 | 20 | 20 | 50 | 50 |
| `symbol` | BTC | ETH | BTC | ETH | BTC | ETH |

Parameter labels are **repeated**, symbol labels are **tiled**
(`Level::repeat` and `Level::tile` in `barq_core::columns`). That asymmetry is the
whole trick, and it buys two things:

**Contiguity.** All columns for one parameter combination sit next to each other.
A cash-sharing group is a contiguous slice: `group_lens=[2, 2, 2]` makes
the table above three independent accounts, each trading both symbols.

**Uniformity.** The simulator takes the shape and account group lengths. It does not know
what a column means, so adding a parameter dimension, a symbol or a factor
requires no change to it at all.

The invariant is established by the constructors and relied on downstream;
nothing enforces it at runtime. `columns.rs` has a test asserting that every
parameter block comes out contiguous.

## Labels are how results become answers

An ungrouped sweep of 2000 columns produces 2000 Sharpe ratios. A grouped
sweep produces one Sharpe per account from its combined equity curve. Unlabelled, that is not an
answer to anything. `Columns` carries named levels — parameters, symbols,
factors — and supports `group_indices(level)`, which is what every aggregation is
built on: "mean Sharpe per factor", "best quantile per symbol", `idxmax` over the
whole sweep reported as `(quantile=0.9, symbol=BTCUSDT)`.

This is why the label machinery lives in the foundation crate rather than in the
analysis layer. It is load-bearing for factor screening, which is one of the
three things the project is for.

## Memory layout: column-major

2-D data — OHLCV panels, signal matrices, `FlexArray::Full` — is **column-major**:

```
data[col * n_rows + row]
```

For ungrouped runs each column is scanned start to finish. Shared-account
runs advance the group one bar at a time; the input stays column-major, as do
factor outputs and symbol-level analysis.

The same reasoning makes OHLCV a struct-of-arrays (`OhlcvPanel`) rather than an
array of `Bar` structs: the stop engine touches `open`, `high` and `low` in
separate passes, and an array-of-structs layout would pull whole rows through
cache for each pass.

## Flexible broadcasting

Simulation parameters — price, fees, slippage, stop distance, size — can each be
a scalar, a value per symbol, a value per bar, or a full matrix. Materializing
every one to `(n_rows, n_cols)` would dominate memory: a 100k-bar × 2000-column
`f64` matrix is 1.6 GB, and there are a dozen such parameters.

`FlexArray` keeps the original shape and indexes into it instead:

```rust
pub enum FlexArray<'a, T> {
    Scalar(T),                                   // constant everywhere
    PerColumn(&'a [T]),                          // len == n_cols
    PerRow(&'a [T]),                             // len == n_rows
    Full { data: &'a [T], n_rows: usize },       // column-major
}
```

All four read through one `get(row, col)`. The scalar case — overwhelmingly the
most common — touches no memory. Checked constructors catch length mismatches at
setup rather than as an out-of-bounds read deep inside a simulation.

`varies_by_row()` lets a caller hoist a per-column constant out of its inner
loop.
