# Licensing

Both reference projects are studied as specifications and used as test oracles.
**No code is copied from either.** Every line in this repository is an
independent implementation written against publicly documented semantics.

## The two constraints

**NautilusTrader — LGPL-3.0-only.** Linking its crates would pull LGPL
obligations into this project. Rust links statically by default, so satisfying
LGPL §4 would mean shipping relinkable object files or a dynamic-library path.
Not linking it avoids the question entirely.

API *names and signatures* are not what a copyright licence protects. A
clean-room implementation that presents the same method names, so that a user's
strategy ports with an import change, is a different thing from copying the
implementation — and it is what [07 Nautilus compatibility](07-nautilus-compat.md)
describes.

**vectorbt — Apache-2.0 with Commons Clause.** The Commons Clause forbids
selling the software, including hosting or consulting whose value derives
substantially from it. Copying its code into a product with any commercial
intent is off the table. Reimplementing the documented behaviour is not
restricted.

## The plug-in route is closed

Worth recording, because it looks viable and is not.

`nautilus-plugin` is a *guest* SDK only. It supplies a manifest, the
`nautilus_plugin_init` entry symbol, and `HostVTable` / `HostContext` types that
are **zero-sized opaque tokens** — the crate's own tests assert
`size_of::<HostVTable>() == 0`. There are no callable host functions in it.

Its `manifest.rs` states the boundary directly: *"Public OSS metadata stops here.
Strategy, actor, controller, and model extension registration belongs to the
host/sys layer that generates and validates the private bridge contract."*

And the open-source host refuses to load anything:
`live/src/node/mod.rs` bails with *"requires host-side plug-in support;
nautilus-plugin is the guest SDK only."*

Even if it opened up, the experimental component contract covers
`DataActor | Strategy | ExecutionAlgorithm` — it is a way to ship a *strategy
into* Nautilus, not a way to substitute a different backtest engine.

## Practical rules

1. Read the reference implementations to understand semantics. Do not copy code.
2. Test fixtures may be *derived* from upstream test cases — inputs and expected
   values extracted to language-neutral JSON/Parquet. Upstream source files are
   not vendored into this repository.
3. `docs/` cites file paths and quotes short passages for attribution, which is
   ordinary technical citation.
4. This project is Apache-2.0, with no additional clause.
