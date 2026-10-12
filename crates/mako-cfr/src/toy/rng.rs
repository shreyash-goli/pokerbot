//! Test-local deterministic RNG for toy MCCFR.
//!
//! SplitMix64 (Steele, Lea, Flood, "Fast Splittable Pseudorandom Number
//! Generators", OOPSLA 2014). This generator is private to the toy module so
//! M-09 does not depend on `mako-core`; NLHE training will use the engine's
//! seeded RNG instead. Not cryptographically secure.

/// Minimal SplitMix64 generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Create a generator from a seed. Identical seeds yield identical streams.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64 uniformly distributed bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform float in `[0, 1)` using the top 53 bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in `[0, n)` (rejection sampling, unbiased). Panics if `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0) is undefined");
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let x = self.next_u64();
            if x < zone {
                return x % n;
            }
        }
    }

    /// Sample an index from a probability distribution (assumed normalized).
    /// Falls back to the last index with positive mass on rounding shortfall.
    pub fn sample_index(&mut self, probs: &[f64]) -> usize {
        assert!(!probs.is_empty(), "cannot sample from empty distribution");
        let u = self.next_f64();
        let mut acc = 0.0;
        let mut last_positive = 0;
        for (i, &p) in probs.iter().enumerate() {
            if p > 0.0 {
                last_positive = i;
            }
            acc += p;
            if u < acc {
                return i;
            }
        }
        last_positive
    }
}
