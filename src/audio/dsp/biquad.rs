//! Biquad filters, transposed direct form II.
//!
//! State is `f64` deliberately. With `f32` state, high-Q filters at low centre
//! frequencies accumulate audible quantisation noise — and that is exactly where
//! a bass EQ band lives, so the one place it matters is the one place people
//! reach for first.

/// Coefficients, normalised so `a0 == 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    /// Pass-through.
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Peaking EQ, from the RBJ Audio EQ Cookbook.
    ///
    /// `f0` is clamped below Nyquist. Without that clamp a 16 kHz band at a
    /// 22.05 kHz sample rate gives `w0 > pi`, and the filter aliases into
    /// something that is not an EQ at all — a real defect in the reference
    /// implementation this design is derived from.
    pub fn peaking(f0: f64, q: f64, gain_db: f64, sample_rate: f64) -> Coeffs {
        if sample_rate <= 0.0 {
            return Coeffs::IDENTITY;
        }
        let nyquist = sample_rate * 0.5;
        // Stay clear of Nyquist itself, where the response degenerates.
        let f0 = f0.min(nyquist * 0.95).max(1.0);
        let q = q.max(1e-4);

        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / sample_rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Equalizer APO's RBJ biquad construction, including its bandwidth,
    /// shelf-slope, and corner-frequency conventions.
    pub fn apo(
        kind: super::apo::BiquadKind,
        frequency: f64,
        gain_db: f64,
        width: super::apo::Width,
        corner_frequency: bool,
        sample_rate: f64,
    ) -> Coeffs {
        use super::apo::{BiquadKind as K, Width};
        if sample_rate <= 0.0 || frequency <= 0.0 {
            return Coeffs::IDENTITY;
        }
        let a = if matches!(kind, K::Peaking | K::LowShelf | K::HighShelf) {
            10f64.powf(gain_db / 40.0)
        } else {
            10f64.powf(gain_db / 20.0)
        };
        let mut f0 = frequency.min(sample_rate * 0.5 * (1.0 - f64::EPSILON));
        let (mut value, is_bandwidth_or_slope) = match width {
            Width::Q(q) => (q, false),
            Width::Bandwidth(bw) => (bw, true),
            Width::Slope(db_per_octave) => (db_per_octave / 12.0, true),
        };
        if corner_frequency && matches!(kind, K::LowShelf | K::HighShelf) {
            let slope = if is_bandwidth_or_slope {
                value
            } else {
                1.0 / ((1.0 / (value * value) - 2.0) / (a + 1.0 / a) + 1.0)
            };
            let factor = 10f64.powf(gain_db.abs() / 80.0 / slope);
            if kind == K::LowShelf {
                f0 *= factor;
            } else {
                f0 /= factor;
            }
            f0 = f0.min(sample_rate * 0.5 * (1.0 - f64::EPSILON));
        }
        value = value.max(f64::MIN_POSITIVE);
        let omega = 2.0 * std::f64::consts::PI * f0 / sample_rate;
        let (sn, cs) = omega.sin_cos();
        let alpha = if !is_bandwidth_or_slope {
            sn / (2.0 * value)
        } else if matches!(kind, K::LowShelf | K::HighShelf) {
            sn / 2.0 * ((a + 1.0 / a) * (1.0 / value - 1.0) + 2.0).sqrt()
        } else {
            sn * (std::f64::consts::LN_2 / 2.0 * value * omega / sn).sinh()
        };
        let beta = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match kind {
            K::LowPass => (
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            K::HighPass => (
                (1.0 + cs) / 2.0,
                -(1.0 + cs),
                (1.0 + cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            K::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            K::Notch => (1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            K::AllPass => (
                1.0 - alpha,
                -2.0 * cs,
                1.0 + alpha,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            K::Peaking => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            K::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cs + beta),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                a * ((a + 1.0) - (a - 1.0) * cs - beta),
                (a + 1.0) + (a - 1.0) * cs + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                (a + 1.0) + (a - 1.0) * cs - beta,
            ),
            K::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cs + beta),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                a * ((a + 1.0) + (a - 1.0) * cs - beta),
                (a + 1.0) - (a - 1.0) * cs + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * cs),
                (a + 1.0) - (a - 1.0) * cs - beta,
            ),
        };
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Magnitude response at a frequency, for tests and for drawing a curve.
    pub fn magnitude_at(&self, f: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im)).sqrt()
    }
}

/// One filter's per-channel state.
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    s1: f64,
    s2: f64,
}

impl State {
    #[inline]
    pub fn process_f64(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        self.process_f64(c, x as f64) as f32
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passes_audio_through_unchanged() {
        let mut s = State::default();
        for x in [0.0f32, 0.5, -0.25, 1.0, -1.0] {
            assert_eq!(s.process(&Coeffs::IDENTITY, x), x);
        }
    }

    #[test]
    fn a_boost_raises_its_own_band_by_the_requested_amount() {
        let c = Coeffs::peaking(1000.0, 1.4, 6.0, 44_100.0);
        let gain_db = 20.0 * c.magnitude_at(1000.0, 44_100.0).log10();
        assert!(
            (gain_db - 6.0).abs() < 0.01,
            "expected +6dB at centre, got {gain_db}"
        );
    }

    #[test]
    fn a_cut_lowers_its_own_band() {
        let c = Coeffs::peaking(1000.0, 1.4, -9.0, 44_100.0);
        let gain_db = 20.0 * c.magnitude_at(1000.0, 44_100.0).log10();
        assert!((gain_db + 9.0).abs() < 0.01, "got {gain_db}");
    }

    #[test]
    fn a_band_barely_affects_distant_frequencies() {
        let c = Coeffs::peaking(1000.0, 1.4, 12.0, 44_100.0);
        for f in [50.0, 15_000.0] {
            let g = 20.0 * c.magnitude_at(f, 44_100.0).log10();
            assert!(g.abs() < 1.0, "{f}Hz moved {g}dB, expected near 0");
        }
    }

    #[test]
    fn a_band_above_nyquist_is_clamped_rather_than_aliasing() {
        // 16kHz at a 22.05kHz sample rate: Nyquist is 11.025kHz. Left unclamped
        // this produces w0 > pi and a filter that is not an EQ.
        let c = Coeffs::peaking(16_000.0, 1.4, 12.0, 22_050.0);
        for f in [100.0, 1000.0, 5000.0, 10_000.0] {
            let g = 20.0 * c.magnitude_at(f, 22_050.0).log10();
            assert!(
                g.is_finite() && g > -30.0 && g < 30.0,
                "{f}Hz gave {g}dB — filter has gone unstable"
            );
        }
    }

    #[test]
    fn zero_gain_is_transparent() {
        let c = Coeffs::peaking(1000.0, 1.4, 0.0, 44_100.0);
        for f in [100.0, 1000.0, 10_000.0] {
            let g = 20.0 * c.magnitude_at(f, 44_100.0).log10();
            assert!(g.abs() < 1e-9, "{f}Hz moved {g}dB");
        }
    }

    #[test]
    fn stays_stable_over_a_long_signal() {
        let c = Coeffs::peaking(70.0, 1.4, 12.0, 44_100.0);
        let mut s = State::default();
        let mut peak = 0f32;
        for i in 0..200_000 {
            let x = ((i as f64 * 0.01).sin() * 0.5) as f32;
            let y = s.process(&c, x);
            assert!(y.is_finite(), "went non-finite at sample {i}");
            peak = peak.max(y.abs());
        }
        assert!(peak < 4.0, "unreasonable growth: peak {peak}");
    }
}
