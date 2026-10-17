//! Summary statistics and confidence intervals.
//!
//! For a sample `x_1..x_n`:
//! * mean `m = Σx / n`
//! * sample variance `s² = Σ(x - m)² / (n - 1)` (Bessel-corrected)
//! * standard error `se = s / √n`
//! * normal-approximation 95% CI `m ± z·se`, `z = 1.959964`
//!
//! The CI needs `n ≥ 2`; with fewer samples it is reported as unbounded.
//! It relies on the central limit theorem over i.i.d. pair results, which is
//! reasonable for hundreds of pairs but is a statistical statement, not a
//! guarantee.
//!
//! The seeded percentile bootstrap resamples the `n` values with replacement
//! `B` times and takes the 2.5% and 97.5% order statistics of the resampled
//! means (nearest-rank on the sorted means). It is a cross-check, not a
//! replacement.

use mako_core::rng::MakoRng;

/// Two-sided 95% standard-normal quantile.
pub const Z95: f64 = 1.959_963_984_540_054;

/// Mean, spread and 95% CI of a sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub n: u64,
    pub mean: f64,
    /// Sample standard deviation (`NaN` when `n < 2`).
    pub sd: f64,
    /// Standard error of the mean (`+inf` when `n < 2`).
    pub se: f64,
    pub ci_low: f64,
    pub ci_high: f64,
}

impl Summary {
    /// Multiply every location/scale quantity by `k > 0`.
    pub fn scaled(&self, k: f64) -> Summary {
        Summary {
            n: self.n,
            mean: self.mean * k,
            sd: self.sd * k,
            se: self.se * k,
            ci_low: self.ci_low * k,
            ci_high: self.ci_high * k,
        }
    }

    /// Half-width of the CI.
    pub fn half_width(&self) -> f64 {
        (self.ci_high - self.ci_low) / 2.0
    }

    /// Whether the CI excludes zero from below.
    pub fn significantly_positive(&self) -> bool {
        self.n >= 2 && self.ci_low > 0.0
    }
}

/// Summarize `xs` with a normal-approximation CI at quantile `z`.
pub fn summarize(xs: &[f64], z: f64) -> Summary {
    let n = xs.len();
    if n == 0 {
        return Summary {
            n: 0,
            mean: f64::NAN,
            sd: f64::NAN,
            se: f64::INFINITY,
            ci_low: f64::NEG_INFINITY,
            ci_high: f64::INFINITY,
        };
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    if n < 2 {
        return Summary {
            n: 1,
            mean,
            sd: f64::NAN,
            se: f64::INFINITY,
            ci_low: f64::NEG_INFINITY,
            ci_high: f64::INFINITY,
        };
    }
    let ss: f64 = xs.iter().map(|x| (x - mean) * (x - mean)).sum();
    let sd = (ss / (n as f64 - 1.0)).sqrt();
    let se = sd / (n as f64).sqrt();
    Summary {
        n: n as u64,
        mean,
        sd,
        se,
        ci_low: mean - z * se,
        ci_high: mean + z * se,
    }
}

/// Chips per hand to big blinds per 100 hands.
pub fn bb_per_100(chips_per_hand: f64, big_blind: u32) -> f64 {
    chips_per_hand / f64::from(big_blind) * 100.0
}

/// Seeded percentile-bootstrap 95% CI of the mean. `None` if `xs` is empty
/// or `resamples == 0`.
pub fn bootstrap_ci(xs: &[f64], resamples: u32, seed: u64) -> Option<(f64, f64)> {
    if xs.is_empty() || resamples == 0 {
        return None;
    }
    let n = xs.len();
    let mut rng = MakoRng::seed_from_u64(seed);
    let mut means: Vec<f64> = (0..resamples)
        .map(|_| {
            let s: f64 = (0..n).map(|_| xs[rng.gen_range(n as u64) as usize]).sum();
            s / n as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    let pick = |q: f64| {
        // Nearest rank: ceil(q * B), 1-based.
        let r = ((q * resamples as f64).ceil() as usize).clamp(1, means.len());
        means[r - 1]
    };
    Some((pick(0.025), pick(0.975)))
}
