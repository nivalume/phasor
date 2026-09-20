//! Benchmark runner for `from_orders`.
//!
//! Reads `benchmarks/workloads.tsv`, runs each workload, and writes results in
//! the format `benchmarks/compare.py` expects. The same workloads are run
//! against vectorbt by `benchmarks/bench_vectorbt.py`, so the numbers are
//! comparable — and every runner reports a checksum of the arrays it generated
//! so that claim can be checked rather than assumed.
//!
//! ```text
//! cargo run --release --example bench -p barq-sim -- benchmarks/results/barq.tsv
//! ```
//!
//! Release only. A debug build of the execution kernel is roughly fifty times
//! slower and the numbers mean nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use barq_core::flex::FlexArray;
use barq_model::{Direction, SizeType};
use barq_sim::groups::Groups;
use barq_sim::{FromOrders, simulate_from_orders, simulate_from_orders_parallel};

/// One row of `workloads.tsv`.
struct Workload {
    name: String,
    n_rows: usize,
    n_cols: usize,
    fill_every: usize,
    size_type: SizeType,
    direction: Direction,
    price_mode: String,
    fees: f64,
    fixed_fees: f64,
    slippage: f64,
}

/// Generates the input arrays.
///
/// **This formula is duplicated in `benchmarks/bench_vectorbt.py` and the two
/// must agree bit for bit.** All the arithmetic is IEEE double and the integer
/// term stays well inside 64 bits, so Python and Rust produce identical
/// results; the checksum in the output is what proves it on any given run.
fn generate(w: &Workload) -> (Vec<f64>, Vec<f64>) {
    let n = w.n_rows * w.n_cols;
    let mut close = Vec::with_capacity(n);
    let mut size = Vec::with_capacity(n);
    for col in 0..w.n_cols {
        let mut price = 10.0 + (col % 37) as f64;
        for i in 0..w.n_rows {
            let wiggle = ((i * 2_654_435_761 + col * 40_503) % 1000) as f64 / 1000.0 - 0.5;
            price = (price * (1.0 + wiggle * 0.02)).max(0.5);
            close.push(price);

            size.push(if (i + col) % w.fill_every != 0 {
                // Size zero is ignored by the kernel and writes no record.
                0.0
            } else if w.size_type == SizeType::TargetPercent {
                if (i / w.fill_every) % 2 == 0 {
                    0.5
                } else {
                    0.2
                }
            } else if (i / w.fill_every) % 2 == 0 {
                1.0
            } else {
                // Safe for a long-only column too: the kernel clamps a sell to
                // the position rather than letting it go short.
                -0.5
            });
        }
    }
    (close, size)
}

/// Order-independent-free checksum over the raw bit patterns.
///
/// Wrapping addition of `f64::to_bits`, which Python reproduces exactly with
/// `struct.unpack("<Q", struct.pack("<d", v))`. Cheap, and sensitive to any
/// difference that matters.
fn checksum(values: &[f64]) -> u64 {
    values
        .iter()
        .fold(0u64, |acc, v| acc.wrapping_add(v.to_bits()))
}

fn parse_workloads(path: &Path) -> Vec<Workload> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut lines = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty());
    let header: Vec<&str> = lines.next().expect("no header").split('\t').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("workloads.tsv has no column {name:?}"))
    };
    let (c_name, c_rows, c_cols, c_every) =
        (col("name"), col("n_rows"), col("n_cols"), col("fill_every"));
    let (c_st, c_dir, c_pm) = (col("size_type"), col("direction"), col("price_mode"));
    let (c_fees, c_fixed, c_slip) = (col("fees"), col("fixed_fees"), col("slippage"));

    lines
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            Workload {
                name: f[c_name].to_owned(),
                n_rows: f[c_rows].parse().unwrap(),
                n_cols: f[c_cols].parse().unwrap(),
                fill_every: f[c_every].parse().unwrap(),
                size_type: match f[c_st] {
                    "Amount" => SizeType::Amount,
                    "TargetPercent" => SizeType::TargetPercent,
                    other => panic!("unsupported size_type {other:?}"),
                },
                direction: match f[c_dir] {
                    "Both" => Direction::Both,
                    "LongOnly" => Direction::LongOnly,
                    "ShortOnly" => Direction::ShortOnly,
                    other => panic!("unsupported direction {other:?}"),
                },
                price_mode: f[c_pm].to_owned(),
                fees: f[c_fees].parse().unwrap(),
                fixed_fees: f[c_fixed].parse().unwrap(),
                slippage: f[c_slip].parse().unwrap(),
            }
        })
        .collect()
}

fn main() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out_path = std::env::args().nth(1).map_or_else(
        || repo_root.join("benchmarks/results/barq.tsv"),
        PathBuf::from,
    );
    let workloads = parse_workloads(&repo_root.join("benchmarks/workloads.tsv"));

    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let mut out = String::new();
    writeln!(out, "# barq (this engine), release build").unwrap();
    writeln!(out, "# cores: {cores}").unwrap();
    writeln!(
        out,
        "workload\timpl\tthreads\tn_rows\tn_cols\tfills\tchecksum\tbest_secs\telems_per_sec"
    )
    .unwrap();

    for w in &workloads {
        let (close, size) = generate(w);
        let sum = checksum(&close).wrapping_add(checksum(&size));
        let (price, val_price) = match w.price_mode.as_str() {
            "close" => (f64::INFINITY, f64::INFINITY),
            "prev" => (f64::NEG_INFINITY, f64::INFINITY),
            other => panic!("unsupported price_mode {other:?}"),
        };

        let args = FromOrders {
            n_rows: w.n_rows,
            n_cols: w.n_cols,
            groups: Groups::ungrouped(w.n_cols),
            init_cash: FlexArray::Scalar(100_000.0),
            close: FlexArray::full(&close, w.n_rows, w.n_cols).unwrap(),
            size: FlexArray::full(&size, w.n_rows, w.n_cols).unwrap(),
            price: FlexArray::Scalar(price),
            val_price: FlexArray::Scalar(val_price),
            size_type: FlexArray::Scalar(w.size_type),
            direction: FlexArray::Scalar(w.direction),
            fees: FlexArray::Scalar(w.fees),
            fixed_fees: FlexArray::Scalar(w.fixed_fees),
            slippage: FlexArray::Scalar(w.slippage),
            ..Default::default()
        };

        let elems = (w.n_rows * w.n_cols) as f64;
        let warm = simulate_from_orders(&args).expect("simulation failed");
        let fills = warm.len();

        // Serial first, so the numbers line up with the single-threaded
        // reference, then the thread counts this machine can actually use.
        let mut thread_counts = vec![1usize];
        thread_counts.extend((2..=cores).filter(|t| *t == 2 || *t == cores));

        for threads in thread_counts {
            // Best of seven. The shortest run is the least contaminated
            // estimate on a shared machine, and more samples tighten it:
            // at three, repeated runs of identical code moved by up to 10%,
            // which is wider than several of the optimizations worth
            // measuring. Costs about a minute for the whole suite.
            let mut best = f64::INFINITY;
            for _ in 0..7 {
                let start = Instant::now();
                let records = if threads == 1 {
                    simulate_from_orders(&args)
                } else {
                    simulate_from_orders_parallel(&args, threads)
                }
                .expect("simulation failed");
                best = best.min(start.elapsed().as_secs_f64());
                assert_eq!(records.len(), fills, "thread count changed the result");
            }
            writeln!(
                out,
                "{}\tvbt\t{threads}\t{}\t{}\t{fills}\t{sum}\t{best:.6}\t{:.0}",
                w.name,
                w.n_rows,
                w.n_cols,
                elems / best
            )
            .unwrap();
            println!(
                "{:<22} {threads:>2}t  {best:>8.4}s  {:>12.0} elem/s  ({fills} fills)",
                w.name,
                elems / best
            );
        }
    }

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("cannot create results directory");
    }
    std::fs::write(&out_path, out).expect("cannot write results");
    println!("\nwrote {}", out_path.display());
}
