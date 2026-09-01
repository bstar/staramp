//! Driving effects from the audio.
//!
//! This is the part that does not exist elsewhere. TerminalTextEffects and its
//! ports animate on a fixed timeline; here the spectrum decides how fast an
//! effect resolves and how hard it reacts, so a transition lands with the music
//! rather than alongside it.

/// A simple onset detector over the analyzer's low bands.
///
/// Spectral flux restricted to the bass region: a kick is a large positive
/// change in low-frequency energy, and ignoring the rest avoids firing on
/// cymbals and vocal sibilance.
pub struct OnsetDetector {
    previous: Vec<f32>,
    /// Rolling average of recent flux, so the threshold adapts to the material
    /// instead of needing tuning per album.
    average: f32,
    cooldown: f32,
}

impl Default for OnsetDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl OnsetDetector {
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
            average: 0.0,
            cooldown: 0.0,
        }
    }

    /// Feed a frame of bands. Returns true on a detected onset.
    pub fn feed(&mut self, bands: &[f32], dt: f32) -> bool {
        self.cooldown = (self.cooldown - dt).max(0.0);

        if self.previous.len() != bands.len() {
            self.previous = bands.to_vec();
            return false;
        }

        // Bottom third of the spectrum.
        let n = (bands.len() / 3).max(1);
        let flux: f32 = bands[..n]
            .iter()
            .zip(&self.previous[..n])
            .map(|(now, then)| (now - then).max(0.0))
            .sum::<f32>()
            / n as f32;

        self.previous.copy_from_slice(bands);
        self.average = self.average * 0.9 + flux * 0.1;

        // A fixed threshold would fire constantly on loud material and never on
        // quiet material.
        let fired = flux > self.average * 2.0 + 0.02 && self.cooldown <= 0.0;
        if fired {
            self.cooldown = 0.12;
        }
        fired
    }

    /// How energetic things are right now, 0..1. Effects use it to scale speed.
    pub fn energy(&self) -> f32 {
        (self.average * 8.0).clamp(0.0, 1.0)
    }
}

/// Scale an effect's step by the current energy.
///
/// Clamped so a quiet passage still completes the transition and a loud one
/// does not skip it entirely.
pub fn reactive_dt(dt: f32, energy: f32) -> f32 {
    dt * (0.7 + energy * 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_produces_no_onsets() {
        let mut d = OnsetDetector::new();
        let quiet = vec![0.0f32; 20];
        for _ in 0..100 {
            assert!(!d.feed(&quiet, 1.0 / 30.0));
        }
    }

    #[test]
    fn a_bass_transient_fires_an_onset() {
        let mut d = OnsetDetector::new();
        let quiet = vec![0.01f32; 20];
        for _ in 0..30 {
            d.feed(&quiet, 1.0 / 30.0);
        }
        let mut kick = quiet.clone();
        for b in kick.iter_mut().take(6) {
            *b = 0.9;
        }
        assert!(d.feed(&kick, 1.0 / 30.0), "a kick should register");
    }

    #[test]
    fn high_frequency_content_alone_does_not_fire() {
        // Cymbals and sibilance should not be mistaken for a beat.
        let mut d = OnsetDetector::new();
        let quiet = vec![0.01f32; 21];
        for _ in 0..30 {
            d.feed(&quiet, 1.0 / 30.0);
        }
        let mut hats = quiet.clone();
        for b in hats.iter_mut().skip(14) {
            *b = 0.9;
        }
        assert!(!d.feed(&hats, 1.0 / 30.0));
    }

    #[test]
    fn a_cooldown_prevents_one_hit_firing_repeatedly() {
        let mut d = OnsetDetector::new();
        let quiet = vec![0.01f32; 20];
        for _ in 0..30 {
            d.feed(&quiet, 1.0 / 30.0);
        }
        let mut kick = quiet.clone();
        for b in kick.iter_mut().take(6) {
            *b = 0.9;
        }
        assert!(d.feed(&kick, 1.0 / 30.0));
        // The same loud frame held should not re-trigger immediately.
        assert!(!d.feed(&kick, 1.0 / 30.0));
    }

    #[test]
    fn reactive_dt_stays_within_sane_bounds() {
        for e in [0.0f32, 0.5, 1.0] {
            let d = reactive_dt(1.0 / 30.0, e);
            assert!(d > 0.0);
            assert!(d < 1.0 / 30.0 * 2.0, "energy {e} scaled too far: {d}");
        }
    }
}
