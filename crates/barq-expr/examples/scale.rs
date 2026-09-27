//! Wall-clock timing of the native backend at a realistic panel size.
//!
//! ```bash
//! cargo run --release --example scale -p barq-expr -- 2520 500          # everything
//! cargo run --release --example scale -p barq-expr -- 2520 500 sets     # only the sets
//! cargo run --release --example scale -p barq-expr -- 20000 16 ops      # only operators
//! ```
//!
//! Criterion (`benches/backends.rs`) covers small panels with statistics; this
//! covers the size where an `O(window)` kernel or an unbounded cache shows up.
//! Peak RSS (`VmHWM`) is printed after each set on Linux; it is a high-water
//! mark for the process, so the first set's number includes the inputs.

use std::time::{Duration, Instant};

use barq_expr::{FactorInputs, FactorSet, alpha158, alpha360};
use barq_model::OhlcvPanel;

fn main() {
    let mut args = std::env::args().skip(1);
    let n_rows = args.next().map_or(2_520, |value| value.parse().unwrap());
    let n_cols = args.next().map_or(500, |value| value.parse().unwrap());
    let which = args.next().unwrap_or_else(|| "all".to_owned());
    let (ops, sets) = match which.as_str() {
        "all" => (true, true),
        "ops" => (true, false),
        "sets" => (false, true),
        other => panic!("unknown selection {other:?}; use all, ops or sets"),
    };
    let series = |base: f64, trend: f64, wave: f64| {
        (0..n_cols)
            .flat_map(|column| {
                (0..n_rows).map(move |row| {
                    base + column as f64 * 7.0
                        + row as f64 * trend
                        + ((row * 17 + column * 11) % 23) as f64 * wave
                })
            })
            .collect::<Vec<_>>()
    };
    let panel = OhlcvPanel {
        open: series(100.0, 0.010, 0.03),
        high: series(101.0, 0.012, 0.04),
        low: series(99.0, 0.008, 0.02),
        close: series(100.5, 0.011, 0.05),
        volume: series(1_000.0, 0.30, 0.7),
        n_rows,
        n_cols,
    };
    let vwap = series(100.4, 0.0105, 0.025);
    let inputs = FactorInputs::from_ohlcv(&panel)
        .unwrap()
        .with_field("vwap", &vwap)
        .unwrap();

    println!("panel {n_rows}x{n_cols}");
    let single = [
        "Mean($close,60)",
        "Sum($volume,60)",
        "Std($close,60)",
        "Var($close,60)",
        "Max($high,60)",
        "Min($low,60)",
        "WMA($close,60)",
        "Slope($close,60)",
        "Rsquare($close,60)",
        "Resi($close,60)",
        "Corr($close,Log($volume+1),60)",
        "Cov($close,$volume,60)",
        "Quantile($close,60,0.8)",
        "Med($close,60)",
        "Rank($close,60)",
        "Mad($close,60)",
        "Skew($close,60)",
        "Kurt($close,60)",
        "IdxMax($high,60)",
        "IdxMin($low,60)",
        "Count($close,60)",
        "($close-$open)/($high-$low+1e-12)",
    ];
    if ops {
        for source in single {
            let factors = FactorSet::compile([("f", source)]).unwrap();
            println!("{:>40}  {:>9.1} ms", source, time(&factors, &inputs));
        }
    }
    if sets {
        for (name, set) in [
            ("alpha158", alpha158().unwrap()),
            ("alpha360", alpha360().unwrap()),
        ] {
            let ms = time(&set, &inputs);
            match peak_rss_mb() {
                Some(mb) => println!("{name:>40}  {ms:>9.1} ms   peak RSS {mb:.0} MB"),
                None => println!("{name:>40}  {ms:>9.1} ms"),
            }
        }
    }
}

/// The process high-water mark from `/proc/self/status`, Linux only.
fn peak_rss_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn time(factors: &FactorSet, inputs: &FactorInputs<'_>) -> f64 {
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        std::hint::black_box(factors.evaluate(inputs).unwrap());
        best = best.min(start.elapsed());
    }
    best.as_secs_f64() * 1e3
}
