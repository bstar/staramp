//! ReplayGain, master volume, and clip protection.
//!
//! ReplayGain is applied on the **decode thread**: it is constant for a track
//! and changes only at boundaries, which is where the decode thread already has
//! control, so the change lands on an exact sample. Master volume is applied in
//! the **callback**, where a slider must be heard immediately.

/// Where a track's ReplayGain came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgSource {
    None = 0,
    Tags = 1,
    Scanned = 2,
}

/// Which gain to prefer when a track has both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RgMode {
    Off,
    /// Each track normalised individually. Right for shuffle.
    Track,
    /// Album-relative, preserving the loudness the artist intended between
    /// tracks. Right for listening to an album end to end.
    #[default]
    Album,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReplayGain {
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGain {
    /// The linear scalar to apply, honouring `preamp` and clipping headroom.
    ///
    /// When a peak is known, the gain is reduced so the loudest sample lands at
    /// or below full scale. Without that, ReplayGain plus a positive preamp
    /// clips audibly on already-loud masters.
    pub fn scalar(&self, mode: RgMode, preamp_db: f32, prevent_clipping: bool) -> f32 {
        let gain_db = match mode {
            RgMode::Off => return 1.0,
            RgMode::Track => self.track_gain_db.or(self.album_gain_db),
            RgMode::Album => self.album_gain_db.or(self.track_gain_db),
        };
        let Some(gain_db) = gain_db else { return 1.0 };

        let mut scalar = super::eq::db_to_linear(gain_db + preamp_db);

        if prevent_clipping {
            let peak = match mode {
                RgMode::Album => self.album_peak.or(self.track_peak),
                _ => self.track_peak.or(self.album_peak),
            };
            if let Some(peak) = peak {
                if peak > 0.0 && scalar * peak > 1.0 {
                    scalar = 1.0 / peak;
                }
            }
        }
        scalar
    }

    pub fn is_known(&self) -> bool {
        self.track_gain_db.is_some() || self.album_gain_db.is_some()
    }
}

/// Soft clipper.
///
/// A hard clamp on a 24-bit vinyl rip sounds worse than the overshoot it
/// prevents. This leaves everything below the knee untouched — so it is exactly
/// transparent in the bit-perfect case — and compresses only what would have
/// clipped.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    if x.abs() <= KNEE {
        return x;
    }
    let sign = x.signum();
    let over = x.abs() - KNEE;
    let compressed = KNEE + (1.0 - KNEE) * (over / (1.0 - KNEE)).tanh();
    sign * compressed
}

/// A volume that ramps rather than jumping.
///
/// An instant coefficient change is a step discontinuity, which is a click. One
/// ring-buffer's worth of ramp makes it inaudible.
#[derive(Debug, Clone, Copy)]
pub struct VolumeRamp {
    current: f32,
    target: f32,
    step: f32,
}

impl VolumeRamp {
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            step: 0.0,
        }
    }

    /// Retarget, reaching the new value over `frames`.
    pub fn set_target(&mut self, target: f32, frames: usize) {
        self.target = target;
        self.step = if frames == 0 {
            0.0
        } else {
            (target - self.current) / frames as f32
        };
        if self.step == 0.0 {
            self.current = target;
        }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.step != 0.0 {
            self.current += self.step;
            let done = if self.step > 0.0 {
                self.current >= self.target
            } else {
                self.current <= self.target
            };
            if done {
                self.current = self.target;
                self.step = 0.0;
            }
        }
        self.current
    }

    pub fn value(&self) -> f32 {
        self.current
    }

    pub fn is_settled(&self) -> bool {
        self.step == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_replaygain_information_means_no_change() {
        let rg = ReplayGain::default();
        assert_eq!(rg.scalar(RgMode::Album, 0.0, true), 1.0);
        assert!(!rg.is_known());
    }

    #[test]
    fn off_ignores_available_gain() {
        let rg = ReplayGain {
            track_gain_db: Some(-6.0),
            ..Default::default()
        };
        assert_eq!(rg.scalar(RgMode::Off, 0.0, true), 1.0);
    }

    #[test]
    fn album_mode_prefers_album_gain_and_track_mode_prefers_track() {
        let rg = ReplayGain {
            track_gain_db: Some(-6.0),
            album_gain_db: Some(-3.0),
            ..Default::default()
        };
        let album = rg.scalar(RgMode::Album, 0.0, false);
        let track = rg.scalar(RgMode::Track, 0.0, false);
        assert!((album - super::super::eq::db_to_linear(-3.0)).abs() < 1e-6);
        assert!((track - super::super::eq::db_to_linear(-6.0)).abs() < 1e-6);
    }

    #[test]
    fn falls_back_to_whichever_gain_exists() {
        let only_album = ReplayGain {
            album_gain_db: Some(-3.0),
            ..Default::default()
        };
        assert!(only_album.scalar(RgMode::Track, 0.0, false) < 1.0);
    }

    #[test]
    fn clipping_prevention_pulls_positive_gain_back() {
        // +6dB on a track that already peaks at 0.95 would clip hard.
        let rg = ReplayGain {
            track_gain_db: Some(6.0),
            track_peak: Some(0.95),
            ..Default::default()
        };
        let unguarded = rg.scalar(RgMode::Track, 0.0, false);
        let guarded = rg.scalar(RgMode::Track, 0.0, true);
        assert!(unguarded * 0.95 > 1.0, "the test case must actually clip");
        assert!(
            (guarded * 0.95 - 1.0).abs() < 1e-6,
            "guarded gain should land the peak exactly at full scale"
        );
    }

    #[test]
    fn soft_clip_is_exactly_transparent_below_the_knee() {
        // Matters for the bit-perfect claim: quiet audio must be untouched.
        for x in [0.0f32, 0.1, -0.5, 0.89, -0.9, 0.9] {
            assert_eq!(soft_clip(x), x, "{x} should pass through unchanged");
        }
    }

    #[test]
    fn soft_clip_bounds_overshoot_without_a_hard_edge() {
        // The curve asymptotes to full scale, so at large inputs it reaches
        // exactly 1.0 in f32 rather than approaching it. What matters is that it
        // never goes past.
        for x in [1.5f32, 2.0, 10.0, 1000.0] {
            assert!(soft_clip(x) <= 1.0, "{x} produced {}", soft_clip(x));
            assert!(soft_clip(-x) >= -1.0, "-{x} produced {}", soft_clip(-x));
        }
        assert!(
            soft_clip(1.0) > 0.9,
            "just over the knee stays close to input"
        );
        // Monotonic: no folding back, which would sound far worse than clipping.
        let mut prev = soft_clip(0.9);
        for i in 1..100 {
            let v = soft_clip(0.9 + i as f32 * 0.05);
            assert!(v >= prev, "must be monotonic");
            prev = v;
        }
    }

    #[test]
    fn volume_ramps_rather_than_stepping() {
        let mut v = VolumeRamp::new(1.0);
        v.set_target(0.0, 100);
        assert!(!v.is_settled());
        let first = v.next();
        assert!(first < 1.0 && first > 0.9, "gradual, not a jump: {first}");
        for _ in 0..200 {
            v.next();
        }
        assert!(v.is_settled());
        assert_eq!(v.value(), 0.0);
    }

    #[test]
    fn a_zero_frame_ramp_applies_immediately() {
        let mut v = VolumeRamp::new(1.0);
        v.set_target(0.5, 0);
        assert_eq!(v.next(), 0.5);
        assert!(v.is_settled());
    }
}
