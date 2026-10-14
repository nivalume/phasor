//! Reproducible random draws for `reject_prob`.
//!
//! The execution kernel never draws a random number itself: the caller passes
//! the draw in (`docs/ARCHITECTURE.md §7.6`). A shared generator advanced by
//! the loop would make each draw depend on how many came before it, which
//! depends on the grouping and the thread count. A draw computed from
//! `(seed, row, col)` alone depends on neither, so a run is reproducible and
//! its parallel and serial results stay identical.

/// One SplitMix64 step: a bijective mix of 64 bits.
const fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A draw from `U(0, 1)` for one order: column `col`, bar `row`.
#[inline(never)]
pub(crate) fn uniform(seed: u64, row: usize, col: usize) -> f64 {
    let bits = mix(mix(mix(seed) ^ row as u64) ^ col as u64);
    // The top 53 bits, as a fraction of 2^53: every value is exact and in [0, 1).
    (bits >> 11) as f64 / (1_u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_reproducible_and_spread_over_the_unit_interval() {
        assert_eq!(uniform(7, 3, 4).to_bits(), uniform(7, 3, 4).to_bits());
        assert_ne!(uniform(7, 3, 4).to_bits(), uniform(8, 3, 4).to_bits());
        assert_ne!(uniform(7, 3, 4).to_bits(), uniform(7, 4, 3).to_bits());
        let n = 100_000;
        let draws: Vec<f64> = (0..n).map(|i| uniform(1, i, i % 13)).collect();
        assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
        let mean = draws.iter().sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
        let below = draws.iter().filter(|d| **d < 0.1).count() as f64 / n as f64;
        assert!((below - 0.1).abs() < 0.01, "P(d < 0.1) = {below}");
    }
}
