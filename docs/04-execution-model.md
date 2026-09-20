# Execution model

`barq-exec` is the semantic core. Both tiers route through it, so it must stay
callable for a single bar in isolation — no dependence on the rest of the column
being known. That constraint is what lets phase 2 reuse it.

## Rejection is not an error

Two distinct failure modes, deliberately different types:

| | Meaning | Carried as |
|---|---|---|
| **Rejected / ignored** | The order cannot execute — no cash, size below minimum, price is `NaN` | `OrderResult` with a `StatusInfo` |
| **Invariant violation** | The caller handed us something impossible — negative cash, non-finite position, a long-only column holding a short | `Err` |

Conflating them is how a simulation silently produces a plausible, wrong equity
curve. There are fourteen rejection reasons and they are kept distinct because
*"why did this backtest not trade"* is the first question anyone asks of a sweep.

## Float tolerances are load-bearing

`barq_core::math` is ~50 lines and is the single most important piece of the port.
The kernel snaps `cash`, `position`, `debt`, `free_cash`, `val_price` and `value`
to exactly zero via `is_close` before doing anything else, and decides
affordability with `is_close_or_less` rather than `<=`.

Reproduce the arithmetic but not the tolerances and fills diverge in ways that
surface only as cent-level drift thousands of bars later. A workspace lint denies
`==` on floats so this cannot be bypassed by accident.

Defaults: `rel_tol = 1e-9`, `abs_tol = 1e-12`.

One deliberate deviation from [PEP 485](https://peps.python.org/pep-0485/):
**infinities are never close, not even to themselves.** The `is_infinite` check
precedes the `a == b` short circuit. Infinity is a live sentinel here — size
`inf` means "as much as possible", price `inf` means "use the close" — so
treating two infinities as equal would conflate distinct sentinels.

`add()` snaps a cancelling sum to exactly `0.0`, comparing by magnitude when the
signs differ (summing two nearly-cancelling large values destroys the significant
digits, making the sum's distance from zero meaningless at a relative tolerance).
Every running-balance update goes through it; plain `+` leaves residue that keeps
closed positions nominally open.

## `debt` and `free_cash`

The least obvious part of the state model, and the easiest to get wrong.

```rust
pub struct ExecuteOrderState {
    pub cash: f64,
    pub position: f64,      // signed; negative is short
    pub debt: f64,          // proceeds of open shorts, at average entry price
    pub free_cash: f64,     // cash net of short margin reservation
}
```

- `debt` is what an open short owes back, held at its average entry price.
- `free_cash` is cash minus the margin those shorts tie up. Opening or extending
  a short moves it by `proceeds - 2 * short_value` — once for the position's own
  value, once for the round-trip reservation. Closing unwinds symmetrically via
  `avg_entry_price = debt / |position|`.

Carrying only `cash` and `position` and deriving the rest does not work: the
reservation is path-dependent.

## The size cascade

Order-sensitive. Each stage rewrites `size_type` until only `Amount` remains:

1. `Direction::ShortOnly` → flip the sign.
2. `TargetPercent` × `value` → `TargetValue`. (Ignored if `value` is `NaN`;
   rejected `ValueZeroNeg` if `value <= 0`.)
3. `Value` / `TargetValue` ÷ `val_price` → `Amount` / `TargetAmount`. (Error if
   `val_price` is infinite or `<= 0`; ignored `ValPriceNaN` if `NaN`.)
4. `TargetAmount` − `position` → `Amount`.
5. `Amount` of `-inf` under Short/Both → special-cased to `Percent` of `-1.0`
   ("100% short").
6. `Percent` → `percent = |size|`, `size = sign * inf`, type becomes `Amount`.
7. Sign of the result dispatches to the buy or the sell path.

## Fees, slippage, lots

**Slippage is directional**: buys fill at `price * (1 + s)`, sells at
`price * (1 - s)`.

**Fees are inside the cash cap, not added on top.** When cash is short, the
affordable notional is back-solved:

```
max_req_cash = (cash_limit - fixed_fees) / (1 + fees)
```

**Granularity is applied twice.** `size_granularity` floors the size, and is
re-applied *after* the cash-constrained resize, so lot discipline survives a
partial fill.

**`lock_cash`** on the buy path has a three-branch closed form covering how much
can be spent while still covering an existing short.

## Stops

`sl_stop`, `sl_trail`, `tp_stop` broadcast per element; each column keeps its own
trailing state.

The hit test must reproduce the gap branch:

```
if open <= target      { return open }      // gapped through — fill at the open
if low <= target <= high { return target }
otherwise NaN                                // not hit this bar
```

Missing the first branch silently assumes you always get filled at your stop
price, which flatters every result that includes an overnight gap.

Stop behaviour is further parameterized by `StopEntryPrice` (what the stop is
anchored to), `StopExitPrice` (fill at the stop price or add slippage),
`StopExitMode` (close / reduce / reverse) and `StopUpdateMode` (whether a new
value overrides a live stop, including with `NaN`).
