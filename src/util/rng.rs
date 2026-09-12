//! A small deterministic PRNG.
//!
//! Deterministic on purpose: shuffle order and visualizer particle motion both
//! need to be reproducible in tests, and neither needs cryptographic quality.

#[derive(Debug, Clone)]
pub struct Lcg {
    state: u64,
}

/// splitmix64's finalising mix. Avalanches nearby seeds into unrelated states.
fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Lcg {
    pub fn new(seed: u64) -> Self {
        // Run the seed through a splitmix64 finaliser first. A plain LCG seeded
        // with two nearby values produces two nearby first outputs, and seeding
        // from the clock means consecutive seeds *are* nearby -- two shuffles a
        // millisecond apart would start the same way.
        Self {
            state: splitmix(seed ^ 0x9E37_79B9_7F4A_7C15),
        }
    }

    /// Seed from the environment, for a genuinely different sequence each time.
    ///
    /// The counter matters: two shuffles inside the same clock tick would
    /// otherwise get identical seeds. `RandomState` contributes OS entropy that
    /// is fixed per process, so it alone is not enough either.
    pub fn from_entropy() -> Self {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let process = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);

        Self::new(nanos ^ process.rotate_left(17) ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Return the high bits: the low bits of an LCG have short periods.
        self.state >> 11
    }

    /// Uniform in `[0, n)`. Rejection-sampled, so it has no modulo bias.
    pub fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let n = n as u64;
        let limit = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < limit {
                return (v % n) as usize;
            }
        }
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

impl Default for Lcg {
    fn default() -> Self {
        Self::new(0x5EED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_for_a_given_seed() {
        let a: Vec<u64> = (0..8).map(|_| Lcg::new(7).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]), "fresh instances agree");

        let mut x = Lcg::new(7);
        let mut y = Lcg::new(7);
        for _ in 0..100 {
            assert_eq!(x.next_u64(), y.next_u64());
        }
    }

    #[test]
    fn nearby_seeds_give_unrelated_streams() {
        // Seeding from the clock produces consecutive values, so this is the
        // property that actually matters for shuffle freshness.
        let a: Vec<u64> = (0..4).map(|_| Lcg::new(1_000).next_u64()).collect();
        let b: Vec<u64> = (0..4).map(|_| Lcg::new(1_001).next_u64()).collect();
        assert_ne!(a[0], b[0]);
        // Not merely different -- far apart, so `below(n)` does not collide.
        // `next_u64` returns the top 53 bits, so that is the range to compare
        // against; using u64::MAX here would be an unreachable threshold.
        const RANGE: u64 = u64::MAX >> 11;
        let spread = a[0].max(b[0]) - a[0].min(b[0]);
        assert!(spread > RANGE / 1000, "seeds barely diverged: {spread}");
    }

    #[test]
    fn from_entropy_differs_between_instances() {
        let a: Vec<u64> = (0..4).map(|_| Lcg::from_entropy().next_u64()).collect();
        // Four independent instances; all four agreeing would mean a fixed seed.
        assert!(
            a.windows(2).any(|w| w[0] != w[1]),
            "from_entropy produced the same value every time"
        );
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Lcg::new(1);
        let mut seen = [false; 10];
        for _ in 0..2000 {
            let v = r.below(10);
            assert!(v < 10);
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s), "every value should occur");
    }

    #[test]
    fn below_handles_degenerate_sizes() {
        let mut r = Lcg::new(1);
        assert_eq!(r.below(0), 0);
        assert_eq!(r.below(1), 0);
    }
}
