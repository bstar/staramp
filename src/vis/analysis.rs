//! Spectrum analysis.
//!
//! Ported from cliamp's ui/visualizer.go (MIT, Copyright (c) Bjarne Øverli).
//!
//! The two-rate design is the part worth keeping from it: the FFT
//! runs at about 30 Hz while the smoothing pass runs at 60, so bars glide
//! between analyses instead of stepping. Running the FFT at frame rate looks no
//! better and costs twice as much.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// Winamp's band edges, in Hz. Log-spaced, weighted toward the low end where
/// music actually lives.
const ANCHORS: &[f32] = &[
    20.0, 100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0, 12800.0, 16000.0, 20000.0,
];

/// Below this peak amplitude the input is treated as silence and the display
/// decays, rather than the FFT amplifying dither into a light show.
const SILENCE: f32 = 1e-5;

/// Quietest level the display shows, in dBFS.
const FLOOR_DB: f32 = 60.0;
/// Loudest, above which bands pin to the top.
///
/// Zero rather than a few dB of slack: reserving the very top for genuinely
/// full-scale content means a loud master moves the bars instead of welding
/// them to the ceiling.
const CEIL_DB: f32 = 0.0;

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    size: usize,
    window: Vec<f32>,
    /// Sum of the window, for normalising the transform.
    ///
    /// `realfft`'s forward transform is unnormalised, so bin magnitudes scale
    /// with both the FFT size and the window. Without dividing this out, a
    /// 2048-point transform reports roughly +43 dB for ordinary music and every
    /// band pins to the top of the display -- which is exactly what it did.
    window_sum: f32,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    input: Vec<f32>,
    /// Band edges as FFT bin indices.
    edges: Vec<usize>,
    /// Shifts the whole window, for material that sits quieter or louder than
    /// the default range assumes.
    gain_db: f32,
    /// Smoothed output, one per band.
    bands: Vec<f32>,
    peaks: Vec<f32>,
    peak_age: Vec<f32>,
}

impl Analyzer {
    pub fn new(fft_size: usize, bands: usize, sample_rate: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let scratch = fft.make_output_vec();

        // Hann. Without a window, a tone that does not land exactly on a bin
        // smears across the whole spectrum and every bar lights up.
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let t = i as f32 / (fft_size - 1) as f32;
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * t).cos()
            })
            .collect();

        let window_sum: f32 = window.iter().sum();

        Self {
            fft,
            size: fft_size,
            window,
            window_sum,
            scratch,
            input: vec![0.0; fft_size],
            edges: band_edges(bands, fft_size, sample_rate),
            gain_db: 0.0,
            bands: vec![0.0; bands],
            peaks: vec![0.0; bands],
            peak_age: vec![0.0; bands],
        }
    }

    /// Shift the displayed range, in dB. Positive lifts quiet material.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = gain_db.clamp(-24.0, 24.0);
    }

    pub fn band_count(&self) -> usize {
        self.bands.len()
    }

    pub fn set_bands(&mut self, n: usize, sample_rate: f32) {
        if n == self.bands.len() || n == 0 {
            return;
        }
        self.edges = band_edges(n, self.size, sample_rate);
        self.bands = vec![0.0; n];
        self.peaks = vec![0.0; n];
        self.peak_age = vec![0.0; n];
    }

    /// Feed a fresh window of mono samples and advance the display by `dt`.
    pub fn analyze(&mut self, samples: &[f32], dt: f32) {
        let n = self.size.min(samples.len());
        let peak = samples[samples.len() - n..]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()));

        if peak < SILENCE {
            self.decay(dt);
            return;
        }

        self.input[..n].copy_from_slice(&samples[samples.len() - n..]);
        for (i, x) in self.input.iter_mut().enumerate() {
            *x *= self.window[i];
        }
        if self
            .fft
            .process(&mut self.input, &mut self.scratch)
            .is_err()
        {
            return;
        }

        // Normalise to full scale: a 0 dBFS sine reads 0 dB. The factor of two
        // accounts for the energy in the negative frequencies that a real
        // transform does not return.
        let norm = 2.0 / self.window_sum;

        let bands = self.bands.len();
        for b in 0..bands {
            let lo = self.edges[b];
            let hi = self.edges[b + 1].max(lo + 1).min(self.scratch.len());
            // Power, not magnitude: the log below absorbs the factor of two and
            // the sqrt is pure cost. Summed rather than averaged, so a tone in
            // a wide band is not diluted by the empty bins around it.
            let mut sum = 0.0f32;
            for c in &self.scratch[lo..hi] {
                let re = c.re * norm;
                let im = c.im * norm;
                sum += re * re + im * im;
            }
            let db = 10.0 * (sum + 1e-12).log10() + self.gain_db;
            // Music sits roughly between -60 and -5 dBFS per band, so that is
            // the window mapped onto the display.
            let target = ((db + FLOOR_DB) / (FLOOR_DB - CEIL_DB)).clamp(0.0, 1.0);

            // Fast attack, slow decay: a kick lights instantly and settles
            // visibly, which is what makes an analyzer read as responsive.
            let cur = self.bands[b];
            self.bands[b] = if target > cur {
                cur + (target - cur) * (1.0 - (-dt * 60.0).exp())
            } else {
                cur + (target - cur) * (1.0 - (-dt * 16.0).exp())
            };

            if self.bands[b] >= self.peaks[b] {
                self.peaks[b] = self.bands[b];
                self.peak_age[b] = 0.0;
            } else {
                self.peak_age[b] += dt;
                if self.peak_age[b] > 0.45 {
                    self.peaks[b] = (self.peaks[b] - 0.55 * dt).max(self.bands[b]);
                }
            }
        }
    }

    fn decay(&mut self, dt: f32) {
        let k = (-dt * 8.0).exp();
        for b in 0..self.bands.len() {
            self.bands[b] *= k;
            self.peaks[b] = (self.peaks[b] - 0.55 * dt).max(self.bands[b]);
        }
    }

    pub fn bands(&self) -> &[f32] {
        &self.bands
    }

    pub fn peaks(&self) -> &[f32] {
        &self.peaks
    }
}

/// Log-spaced band edges, interpolated along the Winamp anchor curve.
fn band_edges(bands: usize, fft_size: usize, sample_rate: f32) -> Vec<usize> {
    let bins = fft_size / 2 + 1;
    let nyquist = sample_rate / 2.0;
    (0..=bands)
        .map(|i| {
            let t = i as f32 / bands as f32 * (ANCHORS.len() - 1) as f32;
            let idx = (t.floor() as usize).min(ANCHORS.len() - 2);
            let frac = t - idx as f32;
            let hz = ANCHORS[idx] + (ANCHORS[idx + 1] - ANCHORS[idx]) * frac;
            ((hz / nyquist * bins as f32) as usize).min(bins - 1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin() * 0.5)
            .collect()
    }

    #[test]
    fn silence_produces_no_bars() {
        let mut a = Analyzer::new(2048, 20, 44_100.0);
        let quiet = vec![0.0f32; 2048];
        for _ in 0..30 {
            a.analyze(&quiet, 1.0 / 30.0);
        }
        assert!(
            a.bands().iter().all(|&b| b < 0.01),
            "silence should not light the analyzer: {:?}",
            a.bands()
        );
    }

    #[test]
    fn a_tone_lights_the_band_containing_it() {
        let mut a = Analyzer::new(4096, 20, 44_100.0);
        let s = sine(1000.0, 44_100.0, 4096);
        for _ in 0..60 {
            a.analyze(&s, 1.0 / 60.0);
        }
        let bands = a.bands();
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
            .unwrap()
            .0;

        // Which band covers 1kHz, by the same edge computation the analyzer uses.
        let edges = band_edges(20, 4096, 44_100.0);
        let bin = (1000.0 / 22_050.0 * (4096.0 / 2.0 + 1.0)) as usize;
        let expected = (0..20)
            .find(|&b| edges[b] <= bin && bin < edges[b + 1].max(edges[b] + 1))
            .unwrap();

        assert!(
            (loudest as i32 - expected as i32).abs() <= 1,
            "1kHz lit band {loudest}, expected around {expected}: {bands:?}"
        );
    }

    #[test]
    fn a_low_tone_lights_a_lower_band_than_a_high_one() {
        let mut lo = Analyzer::new(4096, 16, 44_100.0);
        let mut hi = Analyzer::new(4096, 16, 44_100.0);
        for _ in 0..60 {
            lo.analyze(&sine(100.0, 44_100.0, 4096), 1.0 / 60.0);
            hi.analyze(&sine(8000.0, 44_100.0, 4096), 1.0 / 60.0);
        }
        let peak = |a: &Analyzer| {
            a.bands()
                .iter()
                .enumerate()
                .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
                .unwrap()
                .0
        };
        assert!(
            peak(&lo) < peak(&hi),
            "{} should be below {}",
            peak(&lo),
            peak(&hi)
        );
    }

    #[test]
    fn quiet_and_loud_are_visibly_different() {
        // The bug this guards: without normalising the transform, every band
        // pinned to 1.0 and the display never moved. A -46 dBFS tone used to
        // read exactly the same as a -6 dBFS one.
        let level_for = |amp: f32| {
            let mut a = Analyzer::new(2048, 20, 44_100.0);
            let s: Vec<f32> = (0..2048)
                .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin() * amp)
                .collect();
            for _ in 0..120 {
                a.analyze(&s, 1.0 / 60.0);
            }
            a.bands().iter().cloned().fold(0.0f32, f32::max)
        };

        let loud = level_for(0.5);
        let mid = level_for(0.05);
        let quiet = level_for(0.005);

        assert!(loud > mid, "loud {loud:.2} should exceed mid {mid:.2}");
        assert!(mid > quiet, "mid {mid:.2} should exceed quiet {quiet:.2}");
        assert!(
            loud - quiet > 0.4,
            "not enough range: loud {loud:.2} vs quiet {quiet:.2}"
        );
    }

    #[test]
    fn ordinary_music_does_not_peg_the_display() {
        // Broadband noise at a realistic level should sit in the middle of the
        // range, not welded to the top.
        let mut a = Analyzer::new(2048, 20, 44_100.0);
        let mut seed = 12345u32;
        let noise: Vec<f32> = (0..2048)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 8) as f32 / 8388608.0 - 1.0) * 0.15
            })
            .collect();
        for _ in 0..120 {
            a.analyze(&noise, 1.0 / 60.0);
        }
        let pegged = a.bands().iter().filter(|b| **b > 0.97).count();
        assert!(
            pegged <= 2,
            "{pegged} of {} bands pinned to the top: {:?}",
            a.bands().len(),
            a.bands()
        );
    }

    #[test]
    fn gain_shifts_the_whole_range() {
        let level_at = |gain: f32| {
            let mut a = Analyzer::new(2048, 20, 44_100.0);
            a.set_gain_db(gain);
            let s: Vec<f32> = (0..2048)
                .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin() * 0.02)
                .collect();
            for _ in 0..120 {
                a.analyze(&s, 1.0 / 60.0);
            }
            a.bands().iter().cloned().fold(0.0f32, f32::max)
        };
        assert!(
            level_at(12.0) > level_at(0.0),
            "gain should lift quiet material"
        );
        assert!(level_at(-12.0) < level_at(0.0));
    }

    #[test]
    fn gain_is_clamped_to_something_sane() {
        let mut a = Analyzer::new(2048, 20, 44_100.0);
        a.set_gain_db(1000.0);
        assert_eq!(a.gain_db, 24.0);
        a.set_gain_db(-1000.0);
        assert_eq!(a.gain_db, -24.0);
    }

    #[test]
    fn a_full_scale_tone_does_reach_the_top() {
        // The other failure mode: normalising too hard so nothing ever fills.
        let mut a = Analyzer::new(2048, 20, 44_100.0);
        let s: Vec<f32> = (0..2048)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin())
            .collect();
        for _ in 0..120 {
            a.analyze(&s, 1.0 / 60.0);
        }
        let peak = a.bands().iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.9, "full scale only reached {peak:.2}");
    }

    #[test]
    fn values_stay_in_range() {
        let mut a = Analyzer::new(2048, 20, 44_100.0);
        // Full-scale noise: the loudest thing that can arrive.
        let loud: Vec<f32> = (0..2048)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        for _ in 0..60 {
            a.analyze(&loud, 1.0 / 60.0);
        }
        for &b in a.bands() {
            assert!((0.0..=1.0).contains(&b), "band out of range: {b}");
        }
        for &p in a.peaks() {
            assert!((0.0..=1.0).contains(&p));
        }
    }

    #[test]
    fn peaks_sit_at_or_above_the_bars_and_fall_back() {
        let mut a = Analyzer::new(2048, 16, 44_100.0);
        let s = sine(440.0, 44_100.0, 2048);
        for _ in 0..30 {
            a.analyze(&s, 1.0 / 30.0);
        }
        for (b, p) in a.bands().iter().zip(a.peaks()) {
            assert!(p >= b, "peak {p} below bar {b}");
        }
        let before: Vec<f32> = a.peaks().to_vec();
        let quiet = vec![0.0f32; 2048];
        for _ in 0..60 {
            a.analyze(&quiet, 1.0 / 30.0);
        }
        assert!(
            a.peaks().iter().zip(&before).all(|(now, then)| now <= then),
            "peaks should decay once the signal stops"
        );
    }

    #[test]
    fn band_edges_are_monotonic_and_bounded() {
        let e = band_edges(20, 4096, 44_100.0);
        assert_eq!(e.len(), 21);
        for w in e.windows(2) {
            assert!(w[1] >= w[0], "edges must not go backwards: {e:?}");
        }
        assert!(*e.last().unwrap() <= 4096 / 2);
    }
}

#[cfg(test)]
mod dynamics_probe {
    use super::*;

    /// Not an assertion -- run with `--nocapture` to see the response curve.
    #[test]
    fn print_response_curve() {
        println!("{:>12}  {:>8}  bar", "input", "peak");
        for (amp, label) in [
            (1.0, "0 dBFS"),
            (0.5, "-6 dBFS"),
            (0.25, "-12 dBFS"),
            (0.1, "-20 dBFS"),
            (0.05, "-26 dBFS"),
            (0.02, "-34 dBFS"),
            (0.005, "-46 dBFS"),
            (0.001, "-60 dBFS"),
        ] {
            let mut a = Analyzer::new(2048, 20, 44_100.0);
            let s: Vec<f32> = (0..2048)
                .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin() * amp)
                .collect();
            for _ in 0..120 {
                a.analyze(&s, 1.0 / 60.0);
            }
            let peak = a.bands().iter().cloned().fold(0.0f32, f32::max);
            let bar = "█".repeat((peak * 30.0).round() as usize);
            println!("{label:>12}  {peak:>8.2}  {bar}");
        }
    }
}
