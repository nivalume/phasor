#!/usr/bin/env python3
"""Run the benchmark workloads through vectorbt, for a head-to-head comparison.

Reads the same ``benchmarks/workloads.tsv`` the Rust runner reads and emits the
same result format, so ``compare.py`` can join them.

Two tiers are measured, and keeping them apart matters more than it looks:

``kernel``
    The simulation loop alone. For Numba that is ``simulate_from_orders_nb``;
    for their Rust backend it is ``simulate_from_orders_rs`` with the flexible
    array preparation hoisted out of the timing loop. **This is the honest
    engineering comparison** — loop against loop, with nobody's language
    bindings in the measurement.

``api``
    What a Python caller actually pays: the engine-neutral dispatch wrapper,
    which validates and prepares roughly eighteen array parameters on every
    call before the loop starts.

The gap between them is large — the wrapper costs 2-3x — so comparing our raw
Rust against their wrapped call would flatter us by exactly that factor and
say nothing about the loops. Our Python layer pays its own version of that
cost; `bench_python.py` times it on its own rather than beside these rows.

Neither engine is threaded: vectorbt's loop is single-threaded, which is why
our results are reported per thread count and the parallel rows have no
counterpart here.

Usage::

    .venv-parity/bin/python benchmarks/bench_vectorbt.py

Setup is the same as the parity harness; see tests/parity/README.md. The Rust
engine additionally needs ``pip install vectorbt-rust==1.1.0``.
"""

from __future__ import annotations

import argparse
import math
import time
from pathlib import Path

import numpy as np
from numba import njit

REPO_ROOT = Path(__file__).resolve().parents[1]


@njit
def _generate(n_rows, n_cols, fill_every, target_percent):
    """Generates the input arrays, column-major.

    **This formula is duplicated in `crates/phasor-sim/examples/bench.rs` and the
    two must agree bit for bit.** The price walk is sequential and cannot be
    vectorized without reordering the multiplications, which would change the
    last bits — hence the explicit loop, JIT-compiled so generating the data is
    not the slowest part of the benchmark.

    All the arithmetic is IEEE double and the integer term stays well inside
    64 bits, so both languages produce identical results. The checksum in the
    output proves it on any given run rather than assuming it.
    """
    close = np.empty(n_rows * n_cols, dtype=np.float64)
    size = np.empty(n_rows * n_cols, dtype=np.float64)
    k = 0
    for col in range(n_cols):
        price = 10.0 + float(col % 37)
        for i in range(n_rows):
            wiggle = ((i * 2654435761 + col * 40503) % 1000) / 1000.0 - 0.5
            price = max(price * (1.0 + wiggle * 0.02), 0.5)
            close[k] = price

            if (i + col) % fill_every != 0:
                # Size zero is ignored by the kernel and writes no record.
                size[k] = 0.0
            elif target_percent:
                size[k] = 0.5 if (i // fill_every) % 2 == 0 else 0.2
            else:
                size[k] = 1.0 if (i // fill_every) % 2 == 0 else -0.5
            k += 1
    return close, size


def checksum(values: np.ndarray) -> int:
    """Wrapping sum of the raw bit patterns, matching the Rust runner.

    Viewing the buffer as ``uint64`` and summing with a ``uint64`` accumulator
    wraps modulo 2**64, which is what ``wrapping_add`` does.
    """
    return int(values.view(np.uint64).sum(dtype=np.uint64))


def read_workloads(path: Path) -> list[dict[str, str]]:
    lines = [
        line
        for line in path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    header = lines[0].split("\t")
    return [dict(zip(header, line.split("\t"), strict=True)) for line in lines[1:]]


def best_of(fn, repeat: int) -> float:
    """Shortest of `repeat` runs: the least polluted by scheduling noise."""
    best = math.inf
    for _ in range(repeat):
        start = time.perf_counter()
        fn()
        best = min(best, time.perf_counter() - start)
    return best


def build_rust_kernel_call(kwargs: dict, target_shape, flex_2d: bool):
    """Binds their Rust kernel with its array preparation already done.

    The dispatch wrapper prepares every flexible parameter on each call. Doing
    that once, outside the timing loop, is what isolates the loop itself.
    Returns ``None`` if the backend is not installed.
    """
    try:
        from vectorbt._engine import prepare_array_for_rust, prepare_flex_array_for_rust
        from vectorbt_rust.portfolio import simulate_from_orders_rs
    except ImportError:
        return None

    def flex(name, value, dtype):
        return prepare_flex_array_for_rust(
            np.asarray(value), target_shape, dtype, flex_2d, name=name
        )

    f64, i64, b = np.float64, np.int64, np.bool_
    prepared = (
        target_shape,
        prepare_array_for_rust(kwargs["group_lens"], dtype=i64),
        prepare_array_for_rust(kwargs["init_cash"], dtype=f64),
        kwargs["call_seq"],
        flex("size", kwargs["size"], f64),
        flex("price", kwargs["price"], f64),
        flex("size_type", kwargs["size_type"], i64),
        flex("direction", kwargs["direction"], i64),
        flex("fees", kwargs["fees"], f64),
        flex("fixed_fees", kwargs["fixed_fees"], f64),
        flex("slippage", kwargs["slippage"], f64),
        flex("min_size", 0.0, f64),
        flex("max_size", np.inf, f64),
        flex("size_granularity", np.nan, f64),
        flex("reject_prob", 0.0, f64),
        flex("lock_cash", False, b),
        flex("allow_partial", True, b),
        flex("raise_reject", False, b),
        flex("log", False, b),
        flex("val_price", np.inf, f64),
        flex("close", kwargs["close"], f64),
    )
    return lambda: simulate_from_orders_rs(*prepared, flex_2d=flex_2d)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=REPO_ROOT / "benchmarks/results/vectorbt.tsv")
    parser.add_argument("--repeat", type=int, default=3)
    args = parser.parse_args()

    import vectorbt
    from vectorbt._engine import is_rust_available
    from vectorbt.portfolio.dispatch import simulate_from_orders as dispatch
    from vectorbt.portfolio.enums import Direction, SizeType
    from vectorbt.portfolio.nb import simulate_from_orders_nb

    have_rust = is_rust_available()
    if not have_rust:
        print("vectorbt-rust not installed; measuring numba only\n")

    size_types = {"Amount": SizeType.Amount, "TargetPercent": SizeType.TargetPercent}
    directions = {
        "Both": Direction.Both,
        "LongOnly": Direction.LongOnly,
        "ShortOnly": Direction.ShortOnly,
    }

    rows: list[str] = []
    for w in read_workloads(REPO_ROOT / "benchmarks/workloads.tsv"):
        n_rows, n_cols = int(w["n_rows"]), int(w["n_cols"])
        close_flat, size_flat = _generate(
            n_rows, n_cols, int(w["fill_every"]), w["size_type"] == "TargetPercent"
        )
        total = (checksum(close_flat) + checksum(size_flat)) % (1 << 64)

        # The shared layout is column-major; NumPy defaults to row-major.
        target_shape = (n_rows, n_cols)
        kwargs = dict(
            target_shape=target_shape,
            # All ones: every column is its own group, so no cash sharing —
            # matching what our engine does.
            group_lens=np.full(n_cols, 1, dtype=np.int64),
            init_cash=np.full(n_cols, 100_000.0, dtype=np.float64),
            call_seq=np.zeros(target_shape, dtype=np.int64),
            close=close_flat.reshape(target_shape, order="F"),
            size=size_flat.reshape(target_shape, order="F"),
            price=np.asarray(np.inf if w["price_mode"] == "close" else -np.inf),
            size_type=np.asarray(size_types[w["size_type"]]),
            direction=np.asarray(directions[w["direction"]]),
            fees=np.asarray(float(w["fees"])),
            fixed_fees=np.asarray(float(w["fixed_fees"])),
            slippage=np.asarray(float(w["slippage"])),
            flex_2d=True,
        )

        elems = float(n_rows * n_cols)
        measurements = [
            ("vectorbt-numba-kernel", lambda: simulate_from_orders_nb(**kwargs)),
            ("vectorbt-numba-api", lambda: dispatch(engine="numba", **kwargs)),
        ]
        if have_rust:
            rust_kernel = build_rust_kernel_call(kwargs, target_shape, True)
            if rust_kernel is not None:
                measurements.append(("vectorbt-rust-kernel", rust_kernel))
            measurements.append(("vectorbt-rust-api", lambda: dispatch(engine="rust", **kwargs)))

        for impl, call in measurements:
            # Warm up. The first Numba call pays JIT compilation; timing that
            # instead of the loop would be meaningless.
            order_records, _ = call()
            fills = len(order_records)
            best = best_of(call, args.repeat)
            rows.append(
                f"{w['name']}\t{impl}\t1\t{n_rows}\t{n_cols}\t{fills}\t"
                f"{total}\t{best:.6f}\t{elems / best:.0f}"
            )
            print(
                f"{w['name']:<22} {impl:<22} {best:>8.4f}s  "
                f"{elems / best / 1e6:>7.1f}M elem/s  ({fills} fills)"
            )

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as fh:
        fh.write(f"# vectorbt {vectorbt.__version__}, single-threaded\n")
        fh.write("# -kernel: the simulation loop alone, array preparation hoisted out\n")
        fh.write("# -api:    through the engine-neutral dispatch wrapper\n")
        fh.write(
            "workload\timpl\tthreads\tn_rows\tn_cols\tfills\tchecksum\tbest_secs\telems_per_sec\n"
        )
        fh.write("\n".join(rows) + "\n")
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
