---
name: phasor-feature
description: Add or change a feature in the phasor engine (Rust crates and the Python binding) without breaking its invariants — where code goes, the constraints every change must keep (bit-identical defaults, records as the only source, thread invariance, the 3% instruction budget), which tests and checks to run and when, and how to update docs/plan.md and the knowledge base. Use when implementing anything under crates/ or crates/phasor-py.
license: Apache-2.0 (the repository licence)
compatibility: Rust stable with clippy, Python via uv, valgrind for the instruction benchmark.
metadata:
  author: phasor maintainers
  version: "1.0"
  canonical-path: skills/phasor-feature
---

# Adding a feature to phasor

Read `AGENTS.md`, then `docs/plan.md` for where the feature sits and what is
done. The knowledge base (nivalume/phasor-docs) explains how to read each
crate.

## 1. Where code goes

| Kind of change | Crate |
|---|---|
| One order's arithmetic (price, size, fees, cash) | `phasor-exec` (the kernel) |
| The bar loop: signals, stops, limit orders, cash flows, margin | `phasor-sim` (`from_orders.rs`, `from_signals.rs`) |
| Records and their bytes | `phasor-records` |
| Anything computed after a run | `phasor-analysis` (from records only) |
| Factor expressions | `phasor-expr` (`parse.rs` arity, `graph.rs` node, `eval.rs` kernel) |
| Indicators | `phasor-indicators` (batch and `stream.rs`) |
| Python | `crates/phasor-py/src/*.rs` binding, `python/phasor/*.py` wrapper |

Library crates never download or write files; network code lives in
`examples/`.

## 2. Invariants

- **Defaults unchanged.** With the new parameter at its default, every output
  is bit-identical to before. `tests/fixtures/*.tsv` are reference outputs:
  never regenerate one to make a change pass.
- **Records are the only source.** Anything that moves cash or a position is
  a record; analysis rebuilds balances from records.
- **Threads do not change results.** Serial and parallel runs are
  bit-identical; random draws are functions of `(seed, row, col)`. Test 1, 3
  and N threads.
- **Floats** are compared through `phasor_core::math` (`is_close`, `add`);
  `clippy::float_cmp` is denied.
- **Cost when off ≤ 3%** in instructions:
  `python3 benchmarks/instructions.py --base <commit before the change>`.
  Put new branches in cold helpers; a feature that is off should cost one
  predictable branch.
- **Continuing runs.** If the feature keeps state across bars, it must be in
  `phasor_sim::snapshot::SimState`, and `crates/phasor-sim/tests/resume.rs`
  must still split anywhere bit for bit.

## 3. Tests, by what the feature is

| Source of truth | Test |
|---|---|
| vectorbt 1.1.0 has it | a fixture from `tests/parity/generate_*.py` (see skill `phasor-parity`) |
| PRO-only or new | hand-computed scenarios plus property tests: degenerate default, conservation (cash and value from records), linearity, split invariance, thread invariance |
| Any | a mutation check: remove the key line, the test must fail; then restore |

## 4. Order of work

Write the whole feature first; run the checks at the end of the module, not
after every few lines:

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
uv sync --reinstall-package phasor && uv run --with pytest python -m pytest crates/phasor-py/tests -q
python3 benchmarks/instructions.py --base <base>      # only if phasor-exec or phasor-sim changed
```

## 5. Finish

- Update `docs/plan.md`: tick the item, say what was built in one or two
  lines, add the instruction numbers to the performance table if measured.
- Commit with a message that says what changed and why; push to the working
  branch.
- Update the knowledge base pages that describe the changed code, pinned to
  the new commit with line numbers checked (see its README).
