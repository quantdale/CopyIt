//! Persona definitions: named, configurable behavioral timing profiles.
//!
//! Each persona decides how a simulated user behaves: how long it "thinks"
//! between actions, how fast it types per character, and how often it makes a
//! typo-and-correct. All randomness draws from the run's seeded PRNG only, so a
//! persona is fully deterministic per seed. Personas are plain data and can be
//! parameterized in code without touching the harness.

use rand::rngs::StdRng;
use rand::Rng;

/// A named behavioral timing profile.
#[derive(Clone, Copy, Debug)]
pub struct Persona {
    /// Machine-readable name; also used in report paths.
    pub name: &'static str,
    /// Think time range in milliseconds between discrete actions.
    pub think_ms: (u32, u32),
    /// Per-character typing cadence range in milliseconds.
    pub type_ms: (u32, u32),
    /// Probability (0.0..=1.0) that a character is typed as a typo-and-correct.
    pub typo_prob: f32,
}

impl Persona {
    /// A think time drawn from this persona's range.
    pub fn think_ms(&self, rng: &mut StdRng) -> u64 {
        rng.gen_range(self.think_ms.0..=self.think_ms.1) as u64
    }

    /// A per-character typing delay drawn from this persona's range.
    pub fn type_ms(&self, rng: &mut StdRng) -> u64 {
        rng.gen_range(self.type_ms.0..=self.type_ms.1) as u64
    }

    /// Whether this character should be typed as a typo-and-correct sequence.
    pub fn should_typo(&self, rng: &mut StdRng) -> bool {
        rng.gen::<f32>() < self.typo_prob
    }
}

/// A new user who explores the seeded library: slow, deliberate, few typos.
pub const FIRST_RUN_EXPLORER: Persona = Persona {
    name: "FirstRunExplorer",
    think_ms: (150, 400),
    type_ms: (45, 90),
    typo_prob: 0.02,
};

/// A power user who organizes a large library: quick, decisive, occasional typos.
pub const POWER_ORGANIZER: Persona = Persona {
    name: "PowerOrganizer",
    think_ms: (100, 300),
    type_ms: (50, 120),
    typo_prob: 0.05,
};

/// A user exercising failure surfaces: careful, steady, rare typos.
pub const ERROR_HANDLER: Persona = Persona {
    name: "ErrorHandler",
    think_ms: (200, 500),
    type_ms: (60, 110),
    typo_prob: 0.03,
};

/// A user who switches visual themes: fast, minimal typing.
pub const THEME_HOPPER: Persona = Persona {
    name: "ThemeHopper",
    think_ms: (150, 350),
    type_ms: (40, 80),
    typo_prob: 0.01,
};

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn persona_timing_is_deterministic_per_seed() {
        let mut rng_a = StdRng::seed_from_u64(1);
        let mut rng_b = StdRng::seed_from_u64(1);
        for _ in 0..100 {
            let (a, b) = (
                FIRST_RUN_EXPLORER.think_ms(&mut rng_a),
                FIRST_RUN_EXPLORER.think_ms(&mut rng_b),
            );
            assert_eq!(a, b);
            let (a, b) = (
                FIRST_RUN_EXPLORER.type_ms(&mut rng_a),
                FIRST_RUN_EXPLORER.type_ms(&mut rng_b),
            );
            assert_eq!(a, b);
        }
    }

    #[test]
    fn persona_timing_respects_its_range() {
        let mut rng = StdRng::seed_from_u64(2);
        for _ in 0..200 {
            let t = POWER_ORGANIZER.think_ms(&mut rng);
            assert!(
                (POWER_ORGANIZER.think_ms.0 as u64) <= t
                    && t <= (POWER_ORGANIZER.think_ms.1 as u64)
            );
            let t = POWER_ORGANIZER.type_ms(&mut rng);
            assert!(
                (POWER_ORGANIZER.type_ms.0 as u64) <= t && t <= (POWER_ORGANIZER.type_ms.1 as u64)
            );
        }
    }

    #[test]
    fn typo_probability_is_bounded() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut typos = 0u32;
        let total = 500;
        for _ in 0..total {
            if POWER_ORGANIZER.should_typo(&mut rng) {
                typos += 1;
            }
        }
        // ~5% typo rate; allow a wide band to avoid flakiness.
        let rate = typos as f32 / total as f32;
        assert!((0.01..=0.12).contains(&rate), "typo rate {rate}");
    }
}
