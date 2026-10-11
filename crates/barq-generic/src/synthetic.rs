//! Synthetic price paths with known properties.
//!
//! # What these are for
//!
//! A backtest on real prices cannot tell a strategy that found an edge from
//! one that fit the noise: the true process is unknown. A synthetic path
//! removes that ambiguity. A [`gbm`] with `mu = 0` has no edge by
//! construction, so a strategy that reports a healthy Sharpe on it is
//! measuring its own overfitting. A [`random_walk`] has independent steps, so
//! any signal that "predicts" one is reading noise. These functions exist to
//! build those controls, not to model a market.
//!
//! # Every draw is a function of `(seed, row, col)`
//!
//! There is no generator state. Each normal or uniform draw is derived from
//! the seed, the row and the column through SplitMix64, so the same arguments
//! always give the same path, bit for bit, and column `c` does not depend on
//! how many other columns were asked for. A result that came from a seed can
//! be reproduced from that seed alone.
//!
//! # Layout
//!
//! Every output is column-major: `n_rows * n_cols` values, with column `c` at
//! `[c * n_rows..(c + 1) * n_rows]`. Rows are bars, columns are independent
//! paths.

use barq_core::error::{Error, Result};

/// Salts that keep the draws of one `(seed, row, col)` independent of each
/// other. Each draw has its own, so changing one never shifts another.
const SALT_NORMAL_U1: u64 = 0x6E6F_726D_616C_5F31;
const SALT_NORMAL_U2: u64 = 0x6E6F_726D_616C_5F32;
const SALT_HIGH: u64 = 0x6F68_6C63_5F68_6967;
const SALT_LOW: u64 = 0x6F68_6C63_5F6C_6F77;

/// One SplitMix64 step: a bijective mix of 64 bits.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 64 random bits that depend only on `(seed, row, col, salt)`.
///
/// Chained rather than XOR-ed together, so that `(row, col)` and `(col, row)`
/// do not collide.
fn bits(seed: u64, row: usize, col: usize, salt: u64) -> u64 {
    let h = splitmix64(seed ^ salt);
    let h = splitmix64(h ^ row as u64);
    splitmix64(h ^ col as u64)
}

/// A uniform draw on the open interval `(0, 1)`.
///
/// The top 53 bits are centred in their bucket (`+ 0.5`), so the result is
/// never exactly `0` — which is what keeps `ln` in [`normal`] finite — and
/// never exactly `1`.
fn uniform(seed: u64, row: usize, col: usize, salt: u64) -> f64 {
    const SCALE: f64 = 1.0 / (1_u64 << 53) as f64;
    ((bits(seed, row, col, salt) >> 11) as f64 + 0.5) * SCALE
}

/// A standard normal draw by Box–Muller, from two independent uniforms.
fn normal(seed: u64, row: usize, col: usize) -> f64 {
    // `uniform` cannot return 0, but the floor costs nothing and keeps the
    // logarithm finite even if that ever changes.
    let u1 = uniform(seed, row, col, SALT_NORMAL_U1).max(f64::MIN_POSITIVE);
    let u2 = uniform(seed, row, col, SALT_NORMAL_U2);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Rejects a shape whose element count does not fit in memory's index type.
fn checked_len(n_rows: usize, n_cols: usize) -> Result<usize> {
    n_rows.checked_mul(n_cols).ok_or_else(|| {
        Error::Invariant(format!(
            "n_rows ({n_rows}) * n_cols ({n_cols}) overflows usize"
        ))
    })
}

/// Fills a column-major matrix whose first row is `start` and whose later
/// rows are `step(previous, row, col)`.
fn build_paths(
    n_rows: usize,
    n_cols: usize,
    start: f64,
    mut step: impl FnMut(f64, usize, usize) -> f64,
) -> Result<Vec<f64>> {
    let mut out = vec![0.0; checked_len(n_rows, n_cols)?];
    if n_rows == 0 {
        return Ok(out);
    }
    for (col, path) in out.chunks_exact_mut(n_rows).enumerate() {
        path[0] = start;
        for row in 1..n_rows {
            path[row] = step(path[row - 1], row, col);
        }
    }
    Ok(out)
}

/// Gaussian random walks: `x_{t+1} = x_t + step_std * Z`.
///
/// The first row of every column is `start`. Steps are additive, so a path
/// can go negative; use [`gbm`] for something that stays positive.
///
/// # Errors
///
/// Returns [`Error::Invariant`] if `start` is not finite, `step_std` is
/// negative or not finite, or `n_rows * n_cols` overflows.
pub fn random_walk(
    n_rows: usize,
    n_cols: usize,
    start: f64,
    step_std: f64,
    seed: u64,
) -> Result<Vec<f64>> {
    if !start.is_finite() {
        return Err(Error::Invariant(format!(
            "start must be finite, got {start}"
        )));
    }
    if !step_std.is_finite() || step_std < 0.0 {
        return Err(Error::Invariant(format!(
            "step_std must be finite and non-negative, got {step_std}"
        )));
    }
    build_paths(n_rows, n_cols, start, |prev, row, col| {
        prev + step_std * normal(seed, row, col)
    })
}

/// Geometric Brownian motion, discretized exactly.
///
/// `S_{t+1} = S_t * exp((mu - sigma²/2) dt + sigma sqrt(dt) Z)`. This is the
/// closed-form solution over one step, not an Euler approximation, so the log
/// returns are exactly normal with mean `(mu - sigma²/2) dt` and standard
/// deviation `sigma sqrt(dt)` whatever `dt` is. With `mu = 0` the expected
/// price is flat: there is no edge to find.
///
/// The first row of every column is `start`.
///
/// # Errors
///
/// Returns [`Error::Invariant`] if `start` is not positive and finite, `mu` is
/// not finite, `sigma` is negative or not finite, `dt` is not positive and
/// finite, or `n_rows * n_cols` overflows.
pub fn gbm(
    n_rows: usize,
    n_cols: usize,
    start: f64,
    mu: f64,
    sigma: f64,
    dt: f64,
    seed: u64,
) -> Result<Vec<f64>> {
    if !start.is_finite() || start <= 0.0 {
        return Err(Error::Invariant(format!(
            "start must be positive and finite, got {start}"
        )));
    }
    if !mu.is_finite() {
        return Err(Error::Invariant(format!("mu must be finite, got {mu}")));
    }
    if !sigma.is_finite() || sigma < 0.0 {
        return Err(Error::Invariant(format!(
            "sigma must be finite and non-negative, got {sigma}"
        )));
    }
    if !dt.is_finite() || dt <= 0.0 {
        return Err(Error::Invariant(format!(
            "dt must be positive and finite, got {dt}"
        )));
    }
    let drift = (mu - 0.5 * sigma * sigma) * dt;
    let vol = sigma * dt.sqrt();
    build_paths(n_rows, n_cols, start, |prev, row, col| {
        prev * (drift + vol * normal(seed, row, col)).exp()
    })
}

/// Opens, highs and lows consistent with a close matrix.
///
/// Returns `(open, high, low)` in the same column-major layout as `close`:
///
/// - **open** is the previous bar's close in the same column; the first bar
///   opens at its own close.
/// - **high** is `max(open, close) * (1 + spread * U)`.
/// - **low** is `min(open, close) * (1 - spread * U')`.
///
/// `U` and `U'` are independent uniforms on `(0, 1)`. The extension is applied
/// to the magnitude (`hi + |hi| * spread * U`), which is the formula above for
/// positive prices and still widens the bar for negative ones, so
/// `low <= min(open, close) <= max(open, close) <= high` holds on every bar of
/// non-`NaN` input — a random walk that crosses zero included.
///
/// # Errors
///
/// Returns [`Error::Invariant`] if `close.len()` is not `n_rows * n_cols`, or
/// `spread` is not in `[0, 1)`.
pub fn ohlc_from_close(
    close: &[f64],
    n_rows: usize,
    n_cols: usize,
    spread: f64,
    seed: u64,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let len = checked_len(n_rows, n_cols)?;
    if close.len() != len {
        return Err(Error::Invariant(format!(
            "close has {} values, expected n_rows ({n_rows}) * n_cols ({n_cols}) = {len}",
            close.len()
        )));
    }
    // A spread of 1 would let a positive low reach zero.
    if !(0.0..1.0).contains(&spread) {
        return Err(Error::Invariant(format!(
            "spread must be in [0, 1), got {spread}"
        )));
    }
    let mut open = vec![0.0; len];
    let mut high = vec![0.0; len];
    let mut low = vec![0.0; len];
    if n_rows == 0 {
        return Ok((open, high, low));
    }
    for col in 0..n_cols {
        let base = col * n_rows;
        for row in 0..n_rows {
            let i = base + row;
            let c = close[i];
            let o = if row == 0 { c } else { close[i - 1] };
            let hi = o.max(c);
            let lo = o.min(c);
            open[i] = o;
            high[i] = hi + hi.abs() * spread * uniform(seed, row, col, SALT_HIGH);
            low[i] = lo - lo.abs() * spread * uniform(seed, row, col, SALT_LOW);
        }
    }
    Ok((open, high, low))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_of(a: &[f64]) -> Vec<u64> {
        a.iter().map(|x| x.to_bits()).collect()
    }

    fn mean_std(a: &[f64]) -> (f64, f64) {
        let n = a.len() as f64;
        let mean = a.iter().sum::<f64>() / n;
        let var = a.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        (mean, var.sqrt())
    }

    /// Per-bar changes within each column, `f(prev, next)`, pooled.
    fn steps(a: &[f64], n_rows: usize, f: impl Fn(f64, f64) -> f64) -> Vec<f64> {
        a.chunks_exact(n_rows)
            .flat_map(|col| col.windows(2).map(|w| f(w[0], w[1])).collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn the_same_seed_gives_the_same_paths() {
        let a = gbm(50, 4, 100.0, 0.05, 0.2, 1.0 / 252.0, 7).unwrap();
        let b = gbm(50, 4, 100.0, 0.05, 0.2, 1.0 / 252.0, 7).unwrap();
        assert_eq!(bits_of(&a), bits_of(&b));
        let a = random_walk(50, 4, 0.0, 1.0, 7).unwrap();
        let b = random_walk(50, 4, 0.0, 1.0, 7).unwrap();
        assert_eq!(bits_of(&a), bits_of(&b));
    }

    #[test]
    fn a_different_seed_gives_different_paths() {
        let a = gbm(50, 4, 100.0, 0.05, 0.2, 1.0 / 252.0, 7).unwrap();
        let b = gbm(50, 4, 100.0, 0.05, 0.2, 1.0 / 252.0, 8).unwrap();
        assert_ne!(bits_of(&a), bits_of(&b));
    }

    #[test]
    fn a_column_does_not_depend_on_how_many_columns_were_asked_for() {
        let narrow = random_walk(30, 2, 0.0, 1.0, 3).unwrap();
        let wide = random_walk(30, 5, 0.0, 1.0, 3).unwrap();
        assert_eq!(bits_of(&narrow), bits_of(&wide[..60]));
    }

    #[test]
    fn columns_are_different_paths() {
        let n_rows = 100;
        let a = gbm(n_rows, 3, 100.0, 0.0, 0.2, 1.0, 11).unwrap();
        let cols: Vec<_> = a.chunks_exact(n_rows).map(bits_of).collect();
        assert_ne!(cols[0], cols[1]);
        assert_ne!(cols[1], cols[2]);
        assert_ne!(cols[0], cols[2]);
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "the first row is copied, not computed")]
    fn the_first_row_is_start() {
        let n_rows = 10;
        for path in gbm(n_rows, 3, 42.0, 0.1, 0.3, 0.5, 1)
            .unwrap()
            .chunks_exact(n_rows)
        {
            assert_eq!(path[0], 42.0);
        }
        for path in random_walk(n_rows, 3, -5.0, 2.0, 1)
            .unwrap()
            .chunks_exact(n_rows)
        {
            assert_eq!(path[0], -5.0);
        }
    }

    #[test]
    fn gbm_log_returns_have_the_stated_mean_and_std() {
        // 200 steps x 500 paths = 100k log returns.
        let (n_rows, n_cols) = (201, 500);
        let (mu, sigma, dt) = (0.1, 0.3, 0.5);
        let s = gbm(n_rows, n_cols, 100.0, mu, sigma, dt, 2024).unwrap();
        let r = steps(&s, n_rows, |a, b| (b / a).ln());
        assert_eq!(r.len(), 100_000);
        let (mean, std) = mean_std(&r);
        let want_mean = (mu - 0.5 * sigma * sigma) * dt;
        let want_std = sigma * dt.sqrt();
        // Standard error of the mean is ~6.7e-4; allow five of them.
        assert!(
            (mean - want_mean).abs() < 5.0 * want_std / (r.len() as f64).sqrt(),
            "mean {mean} vs {want_mean}"
        );
        assert!(
            (std / want_std - 1.0).abs() < 0.015,
            "std {std} vs {want_std}"
        );
    }

    #[test]
    fn a_driftless_gbm_has_no_edge() {
        // mu = 0: the log drift is exactly -sigma²/2 dt, and the mean simple
        // return is zero.
        let (n_rows, n_cols) = (201, 500);
        let s = gbm(n_rows, n_cols, 100.0, 0.0, 0.2, 1.0, 5).unwrap();
        let r = steps(&s, n_rows, |a, b| b / a - 1.0);
        let (mean, std) = mean_std(&r);
        assert!(mean.abs() < 5.0 * std / (r.len() as f64).sqrt(), "{mean}");
    }

    #[test]
    fn random_walk_steps_have_the_stated_std() {
        let (n_rows, n_cols) = (1001, 100);
        let step_std = 2.5;
        let x = random_walk(n_rows, n_cols, 10.0, step_std, 99).unwrap();
        let d = steps(&x, n_rows, |a, b| b - a);
        assert_eq!(d.len(), 100_000);
        let (mean, std) = mean_std(&d);
        assert!(
            mean.abs() < 5.0 * step_std / (d.len() as f64).sqrt(),
            "{mean}"
        );
        assert!((std / step_std - 1.0).abs() < 0.015, "{std}");
    }

    #[test]
    fn uniforms_stay_strictly_inside_the_unit_interval() {
        for row in 0..10_000 {
            let u = uniform(1, row, 0, SALT_NORMAL_U1);
            assert!(u > 0.0 && u < 1.0);
            assert!(normal(1, row, 0).is_finite());
        }
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "open is copied from close, not computed")]
    fn ohlc_brackets_every_bar() {
        let n_rows = 500;
        let n_cols = 4;
        // A random walk from zero crosses it, so negative closes are covered.
        for close in [
            gbm(n_rows, n_cols, 100.0, 0.0, 0.3, 1.0 / 252.0, 13).unwrap(),
            random_walk(n_rows, n_cols, 0.0, 1.0, 13).unwrap(),
        ] {
            let (open, high, low) = ohlc_from_close(&close, n_rows, n_cols, 0.01, 17).unwrap();
            for col in 0..n_cols {
                for row in 0..n_rows {
                    let i = col * n_rows + row;
                    let want_open = if row == 0 { close[i] } else { close[i - 1] };
                    assert_eq!(open[i], want_open);
                    let (o, c) = (open[i], close[i]);
                    assert!(low[i] <= o.min(c), "bar {i}");
                    assert!(o.min(c) <= o.max(c));
                    assert!(o.max(c) <= high[i], "bar {i}");
                }
            }
        }
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "a zero spread adds exactly zero")]
    fn a_zero_spread_collapses_to_open_and_close() {
        let close = [1.0, 2.0, 1.5, 3.0];
        let (open, high, low) = ohlc_from_close(&close, 4, 1, 0.0, 0).unwrap();
        assert_eq!(open, vec![1.0, 1.0, 2.0, 1.5]);
        assert_eq!(high, vec![1.0, 2.0, 2.0, 3.0]);
        assert_eq!(low, vec![1.0, 1.0, 1.5, 1.5]);
    }

    #[test]
    fn ohlc_is_reproducible_and_seed_sensitive() {
        let close = gbm(100, 2, 50.0, 0.0, 0.2, 1.0, 1).unwrap();
        let a = ohlc_from_close(&close, 100, 2, 0.02, 4).unwrap();
        let b = ohlc_from_close(&close, 100, 2, 0.02, 4).unwrap();
        let c = ohlc_from_close(&close, 100, 2, 0.02, 5).unwrap();
        assert_eq!(bits_of(&a.1), bits_of(&b.1));
        assert_eq!(bits_of(&a.2), bits_of(&b.2));
        assert_ne!(bits_of(&a.1), bits_of(&c.1));
        assert_ne!(bits_of(&a.2), bits_of(&c.2));
    }

    #[test]
    fn empty_shapes_give_empty_output() {
        assert!(random_walk(0, 3, 0.0, 1.0, 0).unwrap().is_empty());
        assert!(gbm(5, 0, 1.0, 0.0, 0.2, 1.0, 0).unwrap().is_empty());
        let (o, h, l) = ohlc_from_close(&[], 0, 3, 0.1, 0).unwrap();
        assert!(o.is_empty() && h.is_empty() && l.is_empty());
    }

    #[test]
    fn bad_arguments_are_rejected() {
        let invariant = |r: Result<Vec<f64>>| matches!(r, Err(Error::Invariant(_)));
        assert!(invariant(random_walk(5, 1, f64::NAN, 1.0, 0)));
        assert!(invariant(random_walk(5, 1, 0.0, -1.0, 0)));
        assert!(invariant(random_walk(5, 1, 0.0, f64::INFINITY, 0)));
        assert!(invariant(random_walk(usize::MAX, 2, 0.0, 1.0, 0)));
        assert!(invariant(gbm(5, 1, 0.0, 0.0, 0.2, 1.0, 0)));
        assert!(invariant(gbm(5, 1, -1.0, 0.0, 0.2, 1.0, 0)));
        assert!(invariant(gbm(5, 1, 1.0, f64::NAN, 0.2, 1.0, 0)));
        assert!(invariant(gbm(5, 1, 1.0, 0.0, -0.2, 1.0, 0)));
        assert!(invariant(gbm(5, 1, 1.0, 0.0, 0.2, 0.0, 0)));
        assert!(invariant(gbm(5, 1, 1.0, 0.0, 0.2, -1.0, 0)));

        let close = [1.0, 2.0, 3.0];
        let ohlc_invariant = |n_rows, n_cols, spread| {
            matches!(
                ohlc_from_close(&close, n_rows, n_cols, spread, 0),
                Err(Error::Invariant(_))
            )
        };
        assert!(ohlc_invariant(2, 1, 0.1));
        assert!(ohlc_invariant(3, 1, -0.1));
        assert!(ohlc_invariant(3, 1, 1.0));
        assert!(ohlc_invariant(3, 1, f64::NAN));
        assert!(!ohlc_invariant(3, 1, 0.1));
    }
}
