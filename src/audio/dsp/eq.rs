//! Ten-band equalizer.
//!
//! The centre frequencies are Winamp's classic ten: 60, 170, 310, 600 Hz and
//! 1, 3, 6, 12, 14, 16 kHz. That layout is the point -- this is a Winamp
//! tribute and the band labels are part of the look -- and a list of centre
//! frequencies is a specification rather than anyone's code. The filters
//! themselves are our own. Coefficients are
//! published to the output callback through an `ArcSwap`, so moving a slider is
//! heard immediately. Computing EQ on the decode thread instead would put a
//! whole ring buffer — about 200 ms — between the gesture and the sound, which
//! feels broken even though it measures fine.

use std::sync::Arc;

use arc_swap::ArcSwap;

use super::biquad::{Coeffs, State};

/// Winamp's ten bands.
pub const BANDS: [f64; 10] = [
    70.0, 180.0, 320.0, 600.0, 1_000.0, 3_000.0, 6_000.0, 12_000.0, 14_000.0, 16_000.0,
];

pub const BAND_LABELS: [&str; 10] = [
    "70", "180", "320", "600", "1k", "3k", "6k", "12k", "14k", "16k",
];

/// Shared by every band. 1.4 gives the broad, overlapping curve the original
/// had; a textbook 0.707 sounds noticeably narrower.
pub const Q: f64 = 1.4;

pub const MAX_GAIN_DB: f32 = 12.0;
pub const MIN_GAIN_DB: f32 = -12.0;

/// A named curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    pub gains: [f32; 10],
}

/// Winamp's bundled presets.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Flat",
        gains: [0.0; 10],
    },
    Preset {
        name: "Rock",
        gains: [4.0, 3.0, -2.0, -4.0, -2.0, 1.0, 4.0, 6.0, 6.0, 6.0],
    },
    Preset {
        name: "Pop",
        gains: [-1.0, 2.0, 4.0, 5.0, 3.0, 0.0, -1.0, -1.0, -1.0, -1.0],
    },
    Preset {
        name: "Jazz",
        gains: [3.0, 2.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0],
    },
    Preset {
        name: "Classical",
        gains: [4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0],
    },
    Preset {
        name: "Bass Boost",
        gains: [9.0, 7.0, 5.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Treble Boost",
        gains: [0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 5.0, 7.0, 8.0, 9.0],
    },
    Preset {
        name: "Vocal",
        gains: [-2.0, -1.0, 1.0, 4.0, 5.0, 4.0, 2.0, 0.0, -1.0, -2.0],
    },
    Preset {
        name: "Electronic",
        gains: [6.0, 5.0, 1.0, 0.0, -2.0, 1.0, 2.0, 4.0, 5.0, 6.0],
    },
    Preset {
        name: "Acoustic",
        gains: [4.0, 3.0, 2.0, 1.0, 1.0, 1.0, 2.0, 3.0, 3.0, 3.0],
    },
    Preset {
        name: "Dance",
        gains: [7.0, 6.0, 3.0, 0.0, 0.0, -3.0, -4.0, -4.0, 0.0, 0.0],
    },
    Preset {
        name: "Soft",
        gains: [3.0, 1.0, 0.0, -1.0, 0.0, 2.0, 4.0, 5.0, 6.0, 7.0],
    },
    Preset {
        name: "Party",
        gains: [5.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.0, 5.0],
    },
    Preset {
        name: "Club",
        gains: [0.0, 0.0, 3.0, 4.0, 4.0, 4.0, 3.0, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Reggae",
        gains: [0.0, 0.0, 0.0, -3.0, 0.0, 4.0, 4.0, 0.0, 0.0, 0.0],
    },
    Preset {
        name: "Techno",
        gains: [6.0, 5.0, 0.0, -3.0, -3.0, 0.0, 5.0, 6.0, 6.0, 5.0],
    },
];

/// What the audio callback reads. Immutable once published.
#[derive(Debug, Clone)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub coeffs: [Coeffs; 10],
    /// Precomputed so the callback does not test each band.
    pub all_flat: bool,
    pub preamp_linear: f32,
}

impl EqSettings {
    pub fn flat(sample_rate: u32) -> Self {
        Self::build(false, 0.0, &[0.0; 10], sample_rate)
    }

    pub fn build(enabled: bool, preamp_db: f32, gains: &[f32; 10], sample_rate: u32) -> Self {
        let mut coeffs = [Coeffs::IDENTITY; 10];
        let mut all_flat = true;
        for (i, &g) in gains.iter().enumerate() {
            let g = g.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
            if g.abs() >= 0.05 {
                all_flat = false;
            }
            // Every band gets real coefficients even at 0 dB. Bypassing a band
            // and re-enabling it later would resume filtering with stale state
            // and click; a 0 dB peaking filter is exactly transparent, so there
            // is nothing to gain by skipping it.
            coeffs[i] = Coeffs::peaking(BANDS[i], Q, g as f64, sample_rate as f64);
        }
        Self {
            enabled,
            preamp_db,
            coeffs,
            all_flat: all_flat && preamp_db.abs() < 0.05,
            preamp_linear: db_to_linear(preamp_db),
        }
    }

    /// True when this EQ would not alter a single sample.
    pub fn is_transparent(&self) -> bool {
        !self.enabled || self.all_flat
    }
}

pub fn db_to_linear(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Per-channel filter state, owned by the callback.
pub struct EqState {
    states: Vec<[State; 10]>,
}

impl EqState {
    pub fn new(channels: usize) -> Self {
        Self {
            states: vec![[State::default(); 10]; channels],
        }
    }

    pub fn reset(&mut self) {
        for ch in &mut self.states {
            for s in ch.iter_mut() {
                s.reset();
            }
        }
    }

    /// Apply in place to an interleaved buffer.
    #[inline]
    pub fn process(&mut self, settings: &EqSettings, buf: &mut [f32], channels: usize) {
        if settings.is_transparent() {
            return;
        }
        let nch = self.states.len();
        for frame in buf.chunks_mut(channels) {
            for (c, sample) in frame.iter_mut().enumerate() {
                let state = &mut self.states[c.min(nch - 1)];
                let mut x = *sample * settings.preamp_linear;
                for (filter, coeffs) in state.iter_mut().zip(&settings.coeffs) {
                    x = filter.process(coeffs, x);
                }
                *sample = x;
            }
        }
    }
}

/// The live, swappable EQ the callback reads without locking.
pub struct EqHandle {
    inner: ArcSwap<EqSettings>,
}

impl EqHandle {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            inner: ArcSwap::from_pointee(EqSettings::flat(sample_rate)),
        }
    }

    pub fn load(&self) -> Arc<EqSettings> {
        self.inner.load_full()
    }

    pub fn store(&self, settings: EqSettings) {
        self.inner.store(Arc::new(settings));
    }
}

pub fn preset_by_name(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winamp_band_layout() {
        assert_eq!(BANDS.len(), 10);
        assert_eq!(BANDS[0], 70.0);
        assert_eq!(BANDS[9], 16_000.0);
        assert_eq!(BAND_LABELS.len(), BANDS.len());
    }

    #[test]
    fn a_flat_eq_is_transparent_and_skipped() {
        let s = EqSettings::build(true, 0.0, &[0.0; 10], 44_100);
        assert!(s.is_transparent());

        let mut st = EqState::new(2);
        let mut buf = [0.5f32, -0.25, 0.75, -1.0];
        let before = buf;
        st.process(&s, &mut buf, 2);
        assert_eq!(buf, before, "a flat EQ must not touch the samples at all");
    }

    #[test]
    fn a_disabled_eq_is_transparent_even_with_gains_set() {
        let s = EqSettings::build(false, 0.0, &[12.0; 10], 44_100);
        assert!(s.is_transparent());
    }

    #[test]
    fn a_bass_boost_actually_boosts_bass() {
        let mut gains = [0.0f32; 10];
        gains[0] = 12.0; // 70 Hz
        let s = EqSettings::build(true, 0.0, &gains, 44_100);
        assert!(!s.is_transparent());

        let rms = |freq: f64| {
            let mut st = EqState::new(1);
            let n = 44_100;
            let mut buf: Vec<f32> = (0..n)
                .map(|i| {
                    (2.0 * std::f64::consts::PI * freq * i as f64 / 44_100.0).sin() as f32 * 0.25
                })
                .collect();
            st.process(&s, &mut buf, 1);
            // Skip the filter's settling transient.
            let tail = &buf[10_000..];
            (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
        };

        let bass = rms(70.0);
        let mid = rms(1_000.0);
        let boost_db = 20.0 * (bass / 0.1767).log10();
        assert!(
            boost_db > 8.0,
            "70Hz should be boosted well above unity, got {boost_db}dB"
        );
        assert!(
            (20.0 * (mid / 0.1767).log10()).abs() < 2.0,
            "1kHz should be near untouched"
        );
    }

    #[test]
    fn gains_are_clamped_to_the_advertised_range() {
        let s = EqSettings::build(true, 0.0, &[99.0; 10], 44_100);
        let g = 20.0 * s.coeffs[4].magnitude_at(1_000.0, 44_100.0).log10();
        assert!(g <= MAX_GAIN_DB as f64 + 0.1, "clamped to +12dB, got {g}");
    }

    #[test]
    fn every_preset_is_within_range_and_named() {
        for p in PRESETS {
            assert!(!p.name.is_empty());
            for g in p.gains {
                assert!(
                    (MIN_GAIN_DB..=MAX_GAIN_DB).contains(&g),
                    "{} has out-of-range gain {g}",
                    p.name
                );
            }
        }
        assert!(
            preset_by_name("rock").is_some(),
            "lookup is case-insensitive"
        );
        assert!(preset_by_name("nonesuch").is_none());
    }

    #[test]
    fn bands_above_nyquist_do_not_destabilise_a_low_rate_stream() {
        // 22.05kHz: the top three Winamp bands are all above Nyquist.
        let s = EqSettings::build(true, 0.0, &[6.0; 10], 22_050);
        let mut st = EqState::new(1);
        let mut buf: Vec<f32> = (0..20_000).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        st.process(&s, &mut buf, 1);
        assert!(buf.iter().all(|x| x.is_finite()), "filter blew up");
        assert!(buf.iter().all(|x| x.abs() < 10.0), "unbounded growth");
    }

    #[test]
    fn the_handle_publishes_new_settings() {
        let h = EqHandle::new(44_100);
        assert!(h.load().is_transparent());
        h.store(EqSettings::build(true, 0.0, &[6.0; 10], 44_100));
        assert!(!h.load().is_transparent());
    }
}
