# Generators and golden fixtures

`tests/fixtures/` holds **golden fixtures**: expected outputs of the engine's
documented behaviour, committed as TSV (and one JSON) files. The Rust and
Python test suites replay them. They are committed, so `cargo test` needs no
Python.

A fixture is never regenerated to make a change pass. If a replay fails, the
engine's output changed; decide whether that is a bug or a deliberate change
of behaviour, and in the second case re-derive the expected value by hand
before editing the fixture. A changed line in a fixture diff is a finding,
not noise.

## What is here

| Fixtures | Replayed by | Notes |
|---|---|---|
| `execute_order.tsv`, `order_sequence.tsv` | `phasor-exec/tests/golden.rs`, `golden_sequence.rs` | One kernel call in isolation, then state threaded across chained calls |
| `from_orders.tsv`, `from_signals.tsv`, `signals.tsv`, `cash_sharing.tsv` | `phasor-sim/tests/golden_*.rs` | The simulation loops and the signal pipeline |
| `trades.tsv`, `drawdowns.tsv`, `mapped.tsv` | `phasor-records/tests/golden_*.rs` | Record views and reductions |
| `analysis.tsv`, `metrics.tsv`, `stats.tsv`, `margin.tsv` | `phasor-analysis/tests/golden_*.rs` | Simulation through to metrics, the statistics table, margin accounts |
| `generic.tsv`, `indicators.tsv`, `signals_gen.tsv` | `phasor-generic`, `phasor-indicators`, `phasor-signals` golden tests | Arrays, indicators, signal generation |
| `shared_accounts.json` | `crates/phasor-py/tests/test_shared_accounts.py` | Shared-account runs through the Python API |
| `qlib_alpha158.tsv`, `qlib_alpha360.tsv` | `parity_qlib.rs` in `phasor-expr`'s tests | Qlib's Alpha158 and Alpha360 outputs |
| `qlib_alpha158_configs.tsv` | `parity_qlib_ext.rs` in `phasor-expr`'s tests, `crates/phasor-py/tests/test_qlib_ext.py` | Names and expression strings of `Alpha158DL.get_feature_config(config)` for six configs |
| `qlib_highfreq.tsv` | `parity_qlib_ext.rs` in `phasor-expr`'s tests, `crates/phasor-py/tests/test_qlib_ext.py` | Qlib's high-frequency operators (`DayCumsum`, `DayLast`, `FFillNan`, `BFillNan`, `Date`, `Select`, `IsNull`, `IsInf`, `Cut`) and `ChangeInstrument` / `Mask` |
| `qlib_pit_records.tsv`, `qlib_pit.tsv` | `parity_qlib_ext.rs` in `phasor-expr`'s tests, `crates/phasor-py/tests/test_qlib_ext.py` | Point-in-time records and `P` / `PRef` read through Qlib's `LocalPITProvider` |
| `wq101.tsv` | `phasor-expr/tests/wq101.rs` | WorldQuant 101 alphas |

Fixture rows are field-by-field comparisons of records, never aggregates: an
equity curve that matches to four decimals hides sign errors and off-by-one
indices. Float fields compare with a tight tolerance; integers, enums and
`NaN`/infinity positions compare exactly. Cases are chosen to be awkward on
purpose: gaps, constant series, short windows, huge and tiny magnitudes,
every reachable rejection reason.

## The four generators

Only the fixtures that come from an external library or an independent
interpreter have a generator here.

### `generate_qlib_alpha158.py`, `generate_qlib_alpha360.py`

Alpha158 and Alpha360 use a local Qlib Conda environment:

```bash
conda run -n rdagent4qlib python tests/generators/generate_qlib_alpha158.py
conda run -n rdagent4qlib python tests/generators/generate_qlib_alpha360.py
cargo test -p phasor-expr --features polars --test parity_qlib
```

The generators run Qlib's own Alpha158 and Alpha360 expressions over
deterministic panels; they replace only Qlib's leaf feature provider, so no
market-data download is needed. The committed TSV files contain both f64
inputs and Qlib expression outputs; matching the Rust matrix dtype keeps the
tests focused on operator semantics rather than float32 cancellation.

The Alpha158 and Alpha360 expressions come from [Qlib](https://github.com/microsoft/qlib)
(MIT licence), and the header of each fixture records the Qlib version that
produced it.

### `generate_qlib_extensions.py`

The Qlib extensions beyond Alpha158 and Alpha360, from the same Conda
environment, in one command:

```bash
TMPDIR=/some/scratch conda run -n rdagent4qlib python tests/generators/generate_qlib_extensions.py
```

It writes four fixtures:

- `qlib_alpha158_configs.tsv`: `config, position, name, expression` rows for
  six `Alpha158DL.get_feature_config` configs; the header records each config
  as JSON.
- `qlib_highfreq.tsv`: two symbols, ten days of eight 30-minute bars, each
  expression column headed by its exact Qlib source. Only the bar provider
  and `qlib.contrib.ops.high_freq.get_calendar_day` are replaced. Booleans
  are 1/0, `Date()` is days since 1970-01-01, and rows that `Select` or `Cut`
  drop are `nan`.
- `qlib_pit_records.tsv` and `qlib_pit.tsv`: the generator writes real PIT
  binary files in the layout of Qlib's `scripts/dump_pit.py` (values through
  float32, as that script casts them) into a throwaway provider directory
  under `TMPDIR`, then evaluates `P` / `PRef` through Qlib's own
  `LocalPITProvider`. The records file lists the records in the order they
  were written; `P` is float32 in Qlib, and the outputs are the float64 of
  that float32.

Qlib's memory cache is keyed by an expression's string, and `DayCumsum` and
`Cut` print without their time or cut arguments (`DayCumsum($volume)`), so
two of them over the same feature would share one cached series. The
generator clears the cache before each expression so every column is that
operator's own output.

### `generate_wq101.py`

WorldQuant 101 has no library to run, so the generator is its own reference:
it parses the paper's formulas (copied verbatim into the script), prints
phasor's table from them, and evaluates them with a direct NumPy interpreter
that shares no code with phasor. It needs only NumPy:

```bash
uv run python tests/generators/generate_wq101.py fixture
python3 tests/generators/generate_wq101.py rust   # the table in phasor-expr/src/wq101.rs
cargo test -p phasor-expr --test wq101
```

Cells marked `~` may depend on how a tie in exact arithmetic rounds and are
not compared; the script's docstring says how they are found.

The alphas are from Kakushadze, "101 Formulaic Alphas" (arXiv:1601.00991).

## Keeping fixtures honest

A golden test that passes trivially is worse than none. Two guards:

1. Each suite has a coverage test that fails if the fixture set stops
   exercising a branch it is there for (a rejection path, a degenerate
   series), so a drifting fixture cannot silently pass.
2. Mutation testing. Before trusting a suite, break the code on purpose and
   confirm a test fails: flip a sign, drop a snap-to-zero, replace the
   tolerant comparison with a plain `<=`. Add a fixture whenever a plausible
   mutation survives and a case *can* distinguish it. Where a difference is
   smaller than the comparison tolerance, assert on bits in a unit test
   instead.

When a quantity depends on a random draw (`reject_prob`, random signals), it
is not in a fixture: draws are functions of `(seed, row, col)` and are tested
by their properties and against injected draws.
