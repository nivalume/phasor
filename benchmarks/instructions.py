#!/usr/bin/env python3
"""Instruction-count regression check: this tree against a base revision.

A new feature switched off may cost at most 3% (docs/plan.md, "关闭时不变慢"),
and interleaved wall-clock runs move by about 4% between rounds on a shared
machine, so timings cannot tell a 3.8% regression from a 2% one. Instruction
counts under callgrind are exact and repeat run to run, so they can.

They are not time. A cache miss costs no extra instructions and a divide costs
one, so a change that is all memory traffic can hide from a count. Use the
count to see a change and the interleaved timings to confirm it matters.

The check builds the ``bench`` example twice, from this tree and from a base
revision checked out in a scratch worktree, runs every workload once under
callgrind with collection switched on only inside ``run_once`` (so input
generation is not counted), and compares the counts per workload and per loop
as a geometric mean. A loop whose mean grows beyond the threshold fails.

Usage::

    python benchmarks/instructions.py                       # against main
    python benchmarks/instructions.py --base origin/main
    python benchmarks/instructions.py --base-dir ../checkout   # an existing tree
    python benchmarks/instructions.py --workloads signals_dense,single_long

Needs valgrind, so Linux. On macOS, run it in the container built from
``benchmarks/callgrind.Dockerfile``; see benchmarks/README.md.
"""

from __future__ import annotations

import argparse
import math
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
# Builds go under their own target directories: inside a container the tree is
# usually the host's, and a Linux build in target/release would overwrite the
# host's binaries.
TARGET = REPO_ROOT / "target/instructions"
LOOPS = {"from_orders": "workloads.tsv", "from_signals": "workloads_signals.tsv"}


def workload_names(path: Path) -> list[str]:
    lines = [line for line in path.read_text().splitlines() if line.strip() and not line.startswith("#")]
    column = lines[0].split("\t").index("name")
    return [line.split("\t")[column] for line in lines[1:]]


def build(tree: Path, target: Path) -> Path:
    # A bench example from before `--once` existed would take the flag for an
    # output path and run the whole timed suite under callgrind, for hours.
    sources = list(tree.glob("crates/*/examples/bench.rs"))
    if not sources or "--once" not in sources[0].read_text():
        sys.exit(f"{tree}: its bench example has no --once mode; pick a newer base")
    print(f"building {tree} ...", flush=True)
    subprocess.run(
        ["cargo", "build", "--release", "--quiet", "--example", "bench"],
        cwd=tree,
        env={**os.environ, "CARGO_TARGET_DIR": str(target)},
        check=True,
    )
    return target / "release/examples/bench"


def count(side: str, binary: Path, name: str, scratch: Path) -> tuple[int, str] | None:
    """Instructions executed inside `run_once`, and the run's fills/checksum.

    None when the binary does not know the workload: one added since the base.
    """
    out = scratch / f"{side}-{name}.out"
    proc = subprocess.run(
        [
            "valgrind",
            "--tool=callgrind",
            "--toggle-collect=*run_once*",
            f"--callgrind-out-file={out}",
            str(binary),
            "--once",
            name,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        if "no workload named" in proc.stderr:
            return None
        sys.exit(f"{binary} --once {name} failed:\n{proc.stderr}")
    totals = [line.split()[1] for line in out.read_text().splitlines() if line.startswith(("totals:", "summary:"))]
    work = proc.stdout.strip().split("\t", 1)[1]
    return int(totals[0]), work


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    base = parser.add_mutually_exclusive_group()
    base.add_argument("--base", default="main", help="revision to compare against (default main)")
    base.add_argument("--base-dir", type=Path, help="an existing checkout to compare against")
    parser.add_argument(
        "--threshold",
        type=float,
        default=0.03,
        help="fractional growth of a loop's mean count that fails (default 0.03)",
    )
    parser.add_argument("--workloads", help="comma-separated subset to run")
    args = parser.parse_args()

    if shutil.which("valgrind") is None:
        print(
            "valgrind not found. On macOS, run this in the container instead; see benchmarks/README.md.",
            file=sys.stderr,
        )
        return 2

    loop_of = {name: loop for loop, file in LOOPS.items() for name in workload_names(REPO_ROOT / "benchmarks" / file)}
    names = list(loop_of)
    if args.workloads:
        names = [n for n in names if n in args.workloads.split(",")]

    with tempfile.TemporaryDirectory() as tmp:
        scratch = Path(tmp)
        if args.base_dir:
            base_tree = args.base_dir.resolve()
        else:
            base_tree = scratch / "base"
            subprocess.run(
                ["git", "worktree", "add", "--quiet", "--detach", str(base_tree), args.base],
                cwd=REPO_ROOT,
                check=True,
            )
        # The base binary reads its workload files from its own tree at run
        # time, so the worktree has to outlive the counting.
        try:
            head_bin = build(REPO_ROOT, TARGET / "head")
            base_bin = build(base_tree, TARGET / "base")
            print(f"counting {len(names)} workloads x 2 under callgrind ...", flush=True)
            with ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
                jobs = {
                    (side, n): pool.submit(count, side, binary, n, scratch)
                    for side, binary in (("base", base_bin), ("head", head_bin))
                    for n in names
                }
                counts = {k: job.result() for k, job in jobs.items()}
        finally:
            if not args.base_dir:
                subprocess.run(
                    ["git", "worktree", "remove", "--force", str(base_tree)],
                    cwd=REPO_ROOT,
                    check=False,
                )

    label = args.base_dir or args.base
    print(f"\ninstructions inside run_once, {label} -> this tree\n")
    header = f"{'loop':<14}{'workload':<22}{'base':>16}{'head':>16}{'change':>9}"
    print(header)
    print("-" * len(header))
    failed = []
    for loop in LOOPS:
        ratios = []
        for n in (n for n in names if loop_of[n] == loop):
            base_count, head_count = counts[("base", n)], counts[("head", n)]
            if base_count is None:
                print(f"{loop:<14}{n:<22}{'-':>16}{head_count[0]:>16,}{'new':>9}")
                continue
            if base_count[1] != head_count[1]:
                print(
                    f"\nABORT: {n} did different work (fills, checksum): "
                    f"base {base_count[1]!r}, head {head_count[1]!r}",
                    file=sys.stderr,
                )
                return 2
            ratio = head_count[0] / base_count[0]
            ratios.append(ratio)
            print(f"{loop:<14}{n:<22}{base_count[0]:>16,}{head_count[0]:>16,}{ratio - 1:>+9.2%}")
        if ratios:
            mean = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
            print(f"{loop:<14}{'geometric mean':<22}{'':>32}{mean - 1:>+9.2%}\n")
            if mean - 1 > args.threshold:
                failed.append(f"{loop} {mean - 1:+.2%}")

    if failed:
        print(
            f"REGRESSION: beyond {args.threshold:.0%} of {label}: {', '.join(failed)}",
            file=sys.stderr,
        )
        return 1
    print(f"No loop grew beyond {args.threshold:.0%} of {label}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
