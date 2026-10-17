---
name: phasor-parity
description: Run and extend phasor's comparison tests against reference implementations — vectorbt 1.1.0 TSV fixtures, Qlib Alpha158/360, NautilusTrader margin, the WorldQuant 101 interpreter — and the property and mutation checks used where no reference exists. Use when a change touches numbers a fixture pins, when adding coverage for a vectorbt-compatible feature, or when a parity test fails and you need to tell a real divergence from a fixture problem.
license: Apache-2.0 (the repository licence)
compatibility: cargo test replays committed fixtures with no Python. Regenerating needs the reference environment (vectorbt 1.1.0 with plotly<6, a Qlib conda env, NumPy for WQ101).
metadata:
  author: phasor maintainers
  version: "1.0"
  canonical-path: skills/phasor-parity
---

# Comparison tests

Full detail is in `tests/parity/README.md`; this is the working order.

## 1. Replaying is free

`cargo test --workspace` replays every committed fixture; nothing to install.
A parity failure means phasor and the reference now disagree. Find out which
changed before touching anything:

1. `git log -- tests/fixtures/<file>`: did the fixture change? It should not
   have, unless the reference version did.
2. Run the one failing case and print both sides; most fixture rows name the
   case.
3. Decide: a bug in the change (fix the code), a deliberate divergence from
   the reference (document it in the test and in `docs/plan.md`, keep the
   fixture), or a reference bug (keep the fixture, skip the row with a
   comment saying why).

Never regenerate a fixture to make a test pass.

## 2. References

| Area | Generator | Fixture | Test |
|---|---|---|---|
| Kernel, sequences, loops, analysis, signals, arrays | `tests/parity/generate_*.py` (vectorbt 1.1.0) | `tests/fixtures/*.tsv` | `crates/*/tests/parity*.rs` |
| Alpha158 / Alpha360 | `generate_qlib_alpha158.py`, `..._alpha360.py` (Qlib env) | `qlib_alpha*.tsv` | `phasor-expr/tests/parity_qlib.rs` |
| WorldQuant 101 | `generate_wq101.py fixture` (NumPy only) | `wq101.tsv` (`~` marks tie-dependent cells) | `phasor-expr/tests/wq101.rs` |
| Margin, liquidation | NautilusTrader scenarios | `nautilus_margin.tsv` | `phasor-analysis/tests/parity_nautilus.rs` |

Regenerate only when changing what a case covers or the reference version,
then read the diff line by line: a changed value is a finding.

## 3. Adding coverage

Add a case to the generator, chosen to be awkward (gaps, a constant series, a
short window, huge and tiny magnitudes), regenerate, and add the replay. A
fixture of well-behaved numbers hides exactly the differences worth finding.

## 4. Without a reference

Hand-compute a small scenario, then property tests: default parameters
bit-identical, cash and value rebuilt from records, `multiplier = k` equals
size × k, a run split anywhere (`resume`) equals one run, 1/3/N threads
identical. Then a mutation check: delete the line the feature hinges on and
confirm a test fails.
