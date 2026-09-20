# Benchmarks

Two jobs, and they need different things from the same measurements:

1. **Head-to-head against vectorbt** — is our loop actually faster, and where?
2. **Our own baseline** — did a change cost us throughput?

## Running

```bash
# Our engine. Release only: a debug build of the execution kernel is ~50x
# slower and the numbers mean nothing.
cargo run --release --example bench -p barq-sim

# vectorbt, both engines, both tiers. Needs the parity venv.
.venv-parity/bin/python benchmarks/bench_vectorbt.py

python benchmarks/compare.py                    # head-to-head
python benchmarks/compare.py --check-baseline   # regression gate
python benchmarks/compare.py --write-baseline   # accept current numbers
```

## Same work, or no comparison

Both runners read `workloads.tsv` and generate their inputs from a formula
each implements independently. That is two chances to diverge, so every runner
reports a checksum of the arrays it produced and the fill count its simulation
returned, and `compare.py` **aborts** if they disagree.

A "faster" number that turns out to be less work is worse than no number. The
gate is tested by corrupting a result file and confirming it fires, rather than
trusted because it exists.

## Kernel and API are different measurements

vectorbt's dispatch wrapper validates and prepares roughly eighteen array
parameters before the loop starts, and costs **about 2x**. So the benchmark
measures two tiers:

| Tier | What it is |
|---|---|
| `kernel` | The simulation loop alone. `simulate_from_orders_nb` direct; `simulate_from_orders_rs` with array preparation hoisted out of the timing loop. |
| `api` | Through the engine-neutral dispatch wrapper — what a Python caller pays today. |

**The kernel tier is the comparison that means something**: loop against loop,
with nobody's language bindings in the measurement. Our engine has no Python
layer yet, so comparing our raw Rust against their wrapped call would flatter
us by exactly the wrapper's cost and say nothing about the loops. When we grow
a Python shell it will pay its own version of that cost and belong in the `api`
tier.

This distinction was not obvious up front. The first version of this harness
compared our kernel against their API and reported a 5.3x median, which was
mostly wrapper overhead.

## Results on the reference machine

4-core Xeon @ 2.10GHz, vectorbt 1.1.0, vectorbt-rust 1.1.0. Millions of
elements per second, kernel tier:

| Ratio | Across three sittings | Range within one |
|---|---:|---:|
| ours vs their numba | 2.61x, 2.79x, 3.34x | 2.0x – 3.6x |
| ours vs their rust | 3.81x, 3.53x, 4.86x | 1.4x – 5.5x |
| **their rust vs their numba** | **0.78x, 0.82x, 0.74x** | 0.60x – 1.70x |
| ours x4 vs their numba | 9.65x, 9.76x, 12.13x | 1.9x – 13.4x |

**The ratio itself moves with the machine, which it should not.** Each column
above is one sitting that measured both engines together — the rule this file
already insists on — and the first row still spans 2.61x to 3.34x, a 28%
spread. The explanation is that the two engines are not equally sensitive to a
busy machine: the reference's Numba loop loses more of it than ours does, so a
quiet box flatters us.

So the headline is a range, not a figure. Earlier revisions of this file
quoted 2.61x and then 2.79x as though they were measurements of the same
quantity; they were measurements of the same quantity *on differently busy
machines*, and the difference between them is not a change in the engine.
Three sittings is enough to see the spread and not enough to characterize it.

That third row is worth its own note. **vectorbt's own Rust backend is slower
than its Numba one for this function**, and independently reproducing their
published figure of 0.73–1.15x is the strongest evidence the harness is
measuring the right thing. It has come out at 0.78x, 0.82x and 0.74x across
three sittings, which is the steadiest number in this file — and a check on
the harness itself, since it is a ratio between two things measured the same
way.

**Both sides are re-measured together or not at all.** Our loop got 22–30%
faster in one sitting and the median ratio moved 2.61x → 2.79x, not to 3.2x:
vectorbt's numbers moved too. Recomputing a ratio from a fresh run of one
engine against a stored run of the other would have reported the machine.

That rule is necessary and, as the table above shows, not sufficient.

It is also the answer to "why bother writing this in Rust": *not* because Rust
beats Numba on a branchy scalar loop — it does not, by their own measurements
and ours. The gap is design, not compiler.

**Which design difference contributes what is not measured.** The candidates
are the `FlexArray` scalar path, column-major scans, stack-allocated state
where Numba builds named tuples, and — for the threaded column — threads, which
vectorbt's single-threaded loop has no counterpart to. Attributing that gap
among them needs a variant-by-variant measurement against their kernel, which
has not been done. What *is* measured is where our own remaining time goes; see
the next section.

## Where the remaining time goes

Measured by building variants as separate binaries and interleaving their runs,
five rounds each, comparing medians. Cross-session A/B does **not** work here:
the baseline drifted 11% between two sittings and made a 12% "win" evaporate.

| Change | Dense workloads | Sparse workloads |
|---|---:|---:|
| Preallocate the serial record buffer | **+6% to +19%** | ~0% |
| Hoist the 7 column-constant parameter checks out of validation | +10% | **+20%** |
| Build the order request once per column, not once per bar | +11% | +8% |
| Remove record writing entirely (ceiling, not a change) | +36% to +66% | ~0% |
| Array-of-structs record buffer, transposed at the end | −5% to −27% | −22% |

Landed: the preallocation, and the two hoists, which ship together as
`ValidatedOrder` — one order request built per column, carrying its own
parameter check, with the four bar-varying fields rewritten each bar. Together
**+30% sparse and +22% dense**, measured twice on separate sittings (29.6% and
29.5% sparse) before being believed.

Validation is the larger half on sparse workloads and the smaller half on
dense ones, which is the shape to expect: a bar that does not fill used to pay
all fourteen checks and almost nothing else, so removing seven of them is most
of that bar's work. A bar that does fill has real arithmetic to amortize them
against.

**One row here reverses an earlier result.** Building the order once per column
was previously recorded at −5% to −36% — *worse*. That measurement held the
hoisted order in a `thread_local` `RefCell` and spent more on borrow
bookkeeping per bar than it saved; it measured the container, not the hoist.
A plain local rebuilt only when `params_vary_by_row()` says to is +8% to +11%.
The negative result was real for what it tested and wrong about the question
it was taken to answer.

Not landed, and why:

- **Record writing** has the largest ceiling and nothing to reach it with. The
  obvious approach — reserve exactly, then write through raw pointers — is
  blocked by the workspace's `unsafe_code = "deny"`. That deny costs nothing
  here: a *safe* stand-in that removes the same bookkeeping (pre-sized
  vectors written by index) measured 8–12% **slower** on every workload, since
  `Vec::push`'s capacity branch is already free and the pre-fill is an extra
  pass over memory the dense workloads are already saturating. The 36–66%
  figure is the ceiling for not writing records at all, which is not an
  optimization; it should not be read as available headroom.
- **Array-of-structs records** measured worse: six bounds checks in the hot
  loop traded for one, plus a full extra transpose pass.

## What the baseline gate can and cannot tell you

Running the suite unchanged, repeatedly, on the reference machine:

| Threads | Median spread | Worst |
|---:|---:|---:|
| 1, best-of-3 | 3.4% | 9.5% |
| 1, best-of-7 | **1.3%** | **3.7%** |
| 2 | 4.1% | 21.9% |
| 4 | 5.9% | **47.3%** |

The runner samples best-of-7 for this reason: at best-of-3 the noise floor was
wide enough to hide a 10% regression, and wide enough that three separate
optimization experiments produced numbers that did not survive a paired
re-measurement.

The gate therefore runs at a 10% threshold on **single-threaded rows only**. A gate whose
noise floor is wider than the regressions it is meant to catch does not gate
anything; it teaches you to ignore it. The threaded numbers stay in the results
for the head-to-head story, where a 2x difference survives the noise, but they
are not evidence of a regression on a shared machine. `--include-threaded`
exists for a quieter one.

Even at one thread, a result near the threshold deserves a re-run before it
deserves an investigation.

**A baseline is only as good as the quiet it was measured in.** One was taken
minutes after a package install, while the machine was still busy. Every
subsequent run then reported +35% to +55% across all ten workloads on a hot
path the diff had not touched. A uniform *improvement* is the same signal as a
uniform regression and a more dangerous one, because nobody investigates good
news — they refresh the baseline, and the busy machine's numbers get baked in
so the next honest run looks like a regression. The gate now says so in both
directions.

**A uniform slowdown is about the machine, not the code.** The gate says so
itself: if every workload regressed by close to the same factor, it prints a
note pointing at the load average. A code regression is selective — it hits the
workloads that exercise the changed path and leaves the rest alone. Observed
during this project: an unchanged build reported −40% to −43% across all ten
workloads, with a load average of 1.37 on four cores. The gate still exits
non-zero; the note explains, it does not excuse.

The threaded figures were found by trying to inject a 30% slowdown and watching
the same code report "+63%" between runs instead.

## Adding a workload

Add a row to `workloads.tsv`. Both runners pick it up; nothing else needs
changing as long as the parameters stay within what both support. Then
regenerate the baseline, since the gate reports unknown entries rather than
silently skipping them.

Workloads are shaped to separate things that behave differently: fill rate
(dense runs are memory-bound on record writing, sparse ones are compute-bound),
aspect ratio (one long column versus many short ones), and per-bar work
(`TargetPercent` revalues every bar; `Amount` does not).
