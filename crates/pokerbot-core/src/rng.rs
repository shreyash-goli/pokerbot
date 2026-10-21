//! Portable, dependency-free deterministic random number generation.
//!
//! [`PokerbotRng`] is the xoshiro256** generator (D. Blackman and S. Vigna,
//! "Scrambled Linear Pseudorandom Number Generators", ACM TOMS 2021;
//! reference description at <https://prng.di.unimi.it/>). Seeding from a
//! single `u64` expands the seed with SplitMix64 (G. Steele, D. Lea,
//! C. Flood, "Fast Splittable Pseudorandom Number Generators", OOPSLA 2014),
//! as recommended by the xoshiro authors.
//!
//! Bounded integers use Lemire's multiply-and-reject method (D. Lemire,
//! "Fast Random Integer Generation in an Interval", ACM TOMACS 2019), which
//! is exactly unbiased. Floats use the top 53 bits of one output.
//!
//! Everything is pure 64-bit integer arithmetic with wrapping semantics, so
//! the output sequence is identical on every platform (including
//! `wasm32-unknown-unknown`) for a given seed. The generator is **not**
//! cryptographically secure; it is for simulation and training only.

/// One step of the SplitMix64 generator: advances `state` and returns the
/// next output.
#[inline]
pub fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic xoshiro256** generator with exportable state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PokerbotRng {
    s: [u64; 4],
}

impl PokerbotRng {
    /// Seeds the generator by expanding `seed` through SplitMix64.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut sm = seed;
        let s = [
            splitmix64_next(&mut sm),
            splitmix64_next(&mut sm),
            splitmix64_next(&mut sm),
            splitmix64_next(&mut sm),
        ];
        // SplitMix64 is a bijection of a full-period counter, so four
        // consecutive outputs can never all be zero; keep a guard anyway.
        Self::from_state(s)
    }

    /// Restores a generator from a state previously returned by
    /// [`PokerbotRng::state`]. The all-zero state is a fixed point of xoshiro
    /// and is replaced by the state for seed 0.
    pub fn from_state(s: [u64; 4]) -> Self {
        if s == [0; 4] {
            return Self::seed_from_u64(0);
        }
        Self { s }
    }

    /// Exports the full internal state (for checkpoints).
    pub fn state(&self) -> [u64; 4] {
        self.s
    }

    /// Returns the next 64 uniformly distributed bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Returns a uniformly distributed integer in `0..n`, without modulo
    /// bias (Lemire's method).
    ///
    /// # Panics
    /// Panics if `n == 0` (the range would be empty).
    pub fn gen_range(&mut self, n: u64) -> u64 {
        assert!(n > 0, "PokerbotRng::gen_range called with empty range");
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        let mut low = m as u64;
        if low < n {
            // threshold = 2^64 mod n
            let threshold = n.wrapping_neg() % n;
            while low < threshold {
                m = u128::from(self.next_u64()) * u128::from(n);
                low = m as u64;
            }
        }
        (m >> 64) as u64
    }

    /// Returns a uniformly distributed `f64` in `[0, 1)` built from the top
    /// 53 bits of one output.
    #[inline]
    pub fn gen_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "empty range")]
    fn gen_range_zero_panics() {
        PokerbotRng::seed_from_u64(1).gen_range(0);
    }

    #[test]
    fn zero_state_is_replaced() {
        let r = PokerbotRng::from_state([0; 4]);
        assert_ne!(r.state(), [0; 4]);
    }
}
