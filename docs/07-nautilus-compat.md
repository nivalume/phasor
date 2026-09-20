# Nautilus compatibility

## What "compatible" means here

**Source-level portability, not a code dependency.** A strategy written against
NautilusTrader should port to this engine by changing imports, not by being
rewritten. We implement the same method names and signatures; we do not link
their crates. See [09 Licensing](09-licensing.md).

This is a phase 2 deliverable. Phase 1 carries two constraints on its behalf,
both cheap now and expensive later:

- `execute_order` must be callable for a single bar in isolation.
- Record writes go through `RecordSink`, not directly into column storage.

## Target v2, not v1

NautilusTrader v2 removed Cython entirely — the checkout surveyed for this design
had **zero** `.pyx`/`.pxd` files and ~2M lines of Rust across 25 crates. Almost
every tutorial and blog post online describes v1, and its API differs
substantially. Copying from v1 documentation yields an API nobody can use.

| v1 | v2 |
|---|---|
| `on_quote_tick` | `on_quote` |
| `on_trade_tick` | `on_trade` |
| `on_order_book*` | `on_book*` |
| `cache.quote_tick` | `cache.quote` |
| `cache.quote_ticks` | `cache.quotes` |
| `on_event` | **removed** — use `on_time_event`, `on_order_event`, `on_position_event` |
| `self.msgbus` | **removed** — use `publish_message`, `subscribe_topic`, `unsubscribe_topic` |
| `modify_order(order, ..)` | takes a `ClientOrderId`, not the order object |
| `cancel_all_orders(..)` | defaults to `strategy_only = true` |
| `is_running` (property) | `is_running()` (method), same for `state()` |
| `NO_ORDER_SIDE` etc. | evaluate to `None`, not an enum member |
| `BarType` (struct) | an enum: `Standard` / `Composite` |

Exception — these kept their v1 names: `register_indicator_for_quote_ticks`,
`register_indicator_for_trade_ticks`, `register_indicator_for_bars`.

## The shape to match

Native Rust strategies in v2 look like this:

```rust
pub struct EmaCross {
    core: StrategyCore,
    // user state
}

barq_strategy!(EmaCross);

impl DataActor for EmaCross {
    fn on_start(&mut self) -> Result<()> {
        self.subscribe_bars(self.bar_type);
        Ok(())
    }

    fn on_bar(&mut self, bar: &Bar) -> Result<()> {
        // ...
        self.submit_order(order, None, None, None)
    }
}
```

A struct holding a `StrategyCore` plus user state, a macro generating the
runtime wiring, and trait impls for the handlers.

## Minimum viable surface

Small enough to be a first cut that already runs the canonical tutorial
strategy:

- `Strategy::new(config)`, `self.config`
- `self.cache.instrument()`
- `self.order_factory.market()` (or `self.order().market()`)
- `self.submit_order()`, `self.close_all_positions()`
- `self.portfolio.is_net_flat()` / `is_net_long()` / `is_net_short()`
- `self.subscribe_bars()`, `self.register_indicator_for_bars()`,
  `self.indicators_initialized()`
- `on_start`, `on_bar`, `on_stop`

Everything else is progressive enhancement.

## Value types

`barq-model`'s value and identifier types follow Nautilus's shape — fixed-point
raw integers with per-instance precision, interned identifiers — so that phase 2
does not convert on every call. Standard precision is 9 decimals; the raw type
is an alias so the 16-decimal `i128` variant is a type change rather than a
rewrite.

The *execution* enums follow vectorbt instead, because reproducing its fill
semantics is the acceptance test. The two lineages are deliberate; see
`barq-model`'s crate docs.

## The parallel sweep API

```rust
let grid = ParamGrid::new()
    .sweep("fast_period", [5, 10, 20])
    .sweep("slow_period", [50, 100, 200])
    .per_instrument(["BTCUSDT.BINANCE", "ETHUSDT.BINANCE", "SOLUSDT.BINANCE"]);
    //  per_instrument = each symbol backtested separately (symbols join the parallel axis)
    //  instruments    = one strategy watching all symbols (symbols live inside one run)

let pf = Portfolio::from_strategy::<EmaCross>(&data, grid).run()?;   // threaded
```

`from_strategy` returns **the same `Portfolio` type** as `from_signals`. Same
columns, same records, same `stats()`. That is the payoff of unifying at the
record layer: prototype at vectorized speed, validate at event-driven fidelity,
compare column by column.

This is also the thing the reference engine cannot do — its sweep is a
sequential loop for structural reasons. See [02 Parallelism](02-parallelism.md).

## Bar matching semantics

If phase 2 aims to match Nautilus's bar execution exactly:

- Bar execution only applies on an L1 book.
- One bar expands to four synthetic ticks: open → high → low → close. With
  `bar_adaptive_high_low_ordering` enabled, whichever of high/low is nearer the
  open goes first.
- Volume is split across the four legs.
- The open leg carries `fill_at_market` so gaps from the previous bar fill at
  market.
- When several bar types are subscribed, the finest timedelta wins.
