//! Spectrum analysis, on a perceptual scale.
//!
//! Original to staramp. The design is taken from published psychoacoustics and
//! ordinary audio engineering rather than from another player's analyser, and
//! every constant here is either a number out of a standard -- named where it
//! is used -- or a time in milliseconds chosen because that is how long the
//! eye wants the thing to take.
//!
//! Three ideas, in order:
//!
//! **Bands on the ERB scale.** Hearing does not resolve frequency linearly or
//! even in exact octaves: the ear's filters widen with frequency, and the
//! equivalent rectangular bandwidth of one of them is described by Glasberg
//! and Moore (1990) as `ERB(f) = 24.7 (0.00437 f + 1)` Hz. Integrating that
//! gives a scale on which equal distances are equally distinguishable, and
//! spacing the bands evenly along *it* is what makes a bass line and a hi-hat
//! occupy sensible amounts of the display. Octave-spaced bands spend half the
//! screen on the top octave, where music mostly is not.
//!
//! **A-weighting.** A 40 Hz tone and a 3 kHz tone of the same amplitude are
//! not the same loudness -- the ear is most sensitive around 2-5 kHz and falls
//! away steeply at the bottom. Weighting each band by the A curve (IEC 61672-1)
//! before display means a bar's height is roughly how loud that band *sounds*,
//! so a kick drum stops swamping everything and a cymbal stops disappearing.
//! This is the part that removes the need for the automatic gain that a
//! spectrum on a raw dB scale cannot do without.
//!
//! **Attack and release.** Every band is an envelope follower with two time
//! constants: fast up, slow down, the way a compressor or a level meter works.
//! One coefficient per direction, `1 - exp(-dt/tau)`, which is frame-rate
//! independent -- the display behaves the same at 30 Hz as at 120.

use std::f32::consts::PI;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

/// The band range, in Hz.
///
/// Not 20 Hz to 20 kHz: the bottom octave is mostly rumble and the top mostly
/// air, and giving them bands spends display on bars that never move.
const F_LOW: f32 = 40.0;
const F_HIGH: f32 = 16_000.0;

/// Quietest and loudest levels shown, in dB relative to full scale after
/// weighting. Everything below the floor is off the bottom of the display.
///
/// The ceiling is what *one band* reaches, not what the whole signal does,
/// and that distinction is the whole calibration. Energy is spread across
/// twenty ERB bands, so no single one holds anything near full scale: with
/// the ceiling at -6 dBFS a loud master drove the tallest bar to 0.63 and the
/// display simply never used its top third.
///
/// Measured rather than guessed, against pink noise at three mastering
/// levels -- see `preview_levels_for_realistic_material`, which is how these
/// numbers were arrived at and how to re-derive them if the weighting or the
/// band count changes:
///
/// | master   | tallest bar |
/// | -------- | ----------- |
/// | -20 dBFS | 0.78        |
/// | -14 dBFS | 0.95        |
/// |  -9 dBFS | 1.00        |
pub const FLOOR_DB: f32 = -68.0;
pub const CEIL_DB: f32 = -32.0;

/// The dB the display spans, for anything that has to convert a rate in
/// decibels into the 0-to-1 units everything downstream speaks.
pub const RANGE_DB: f32 = CEIL_DB - FLOOR_DB;

/// Below this peak sample amplitude the input counts as silence.
///
/// A digital-silence passage is not all zeroes -- there is dither, and there is
/// the noise floor of whatever made the file -- and an analyser with any gain
/// at all will happily draw it.
const SILENCE: f32 = 1e-4;

/// How quickly a band rises and falls, as time constants in milliseconds.
///
/// Attack is short enough that a transient reaches the top on the frame it
/// happens; release is long enough that the bar describes the note rather than
/// flickering at the sample rate. The ratio between them is most of what makes
/// an analyser look expensive rather than nervous.
const ATTACK_MS: f32 = 12.0;
const RELEASE_MS: f32 = 260.0;

/// The same, for the smoothest of the presets.
const SLOW_ATTACK_MS: f32 = 28.0;
const SLOW_RELEASE_MS: f32 = 420.0;

/// How much of a band's level is shared with its neighbours, 0 to 1.
///
/// A little spatial blur along the band axis. FFT bins are independent and the
/// ear's filters overlap, so without it a pure tone lights exactly one bar and
/// looks like a fault; with it the bars move as a surface.
const SPREAD: f32 = 0.28;

/// How the bars move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    /// Fast attack, moderate release. What the bar modes use.
    #[default]
    Quick,
    /// Slower both ways, with more neighbour spread: the bars behave like one
    /// surface rather than a row of independent meters.
    Fluid,
}

impl Motion {
    fn taus(self) -> (f32, f32) {
        match self {
            Motion::Quick => (ATTACK_MS, RELEASE_MS),
            Motion::Fluid => (SLOW_ATTACK_MS, SLOW_RELEASE_MS),
        }
    }

    fn spread(self) -> f32 {
        match self {
            Motion::Quick => SPREAD,
            Motion::Fluid => SPREAD * 1.8,
        }
    }
}

/// One band's extent in FFT bins, and what its level is worth.
#[derive(Debug, Clone, Copy)]
struct Band {
    /// Inclusive first bin, exclusive last.
    lo: usize,
    hi: usize,
    /// A-weighting at the band's centre, as a linear power factor.
    weight: f32,
}

pub struct Spectrum {
    fft: Arc<dyn RealToComplex<f32>>,
    size: usize,
    window: Vec<f32>,
    /// Sum of the window, for normalising the transform.
    ///
    /// `realfft`'s forward transform is unnormalised, so bin magnitudes scale
    /// with the FFT size and the window together. Without dividing it out the
    /// levels are meaningless and every band pins to the top.
    window_sum: f32,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    input: Vec<f32>,
    bands: Vec<Band>,
    motion: Motion,
    /// Scales the release time constant, 0 to 1. The `[vis] smoothing`
    /// setting: higher holds the bars up longer, lower snaps to the music.
    smoothing: f32,
    /// Shifts the whole window, for material quieter or louder than the range
    /// assumes.
    gain_db: f32,
    /// What the last transform said, before smoothing.
    raw: Vec<f32>,
    /// Scratch for the neighbour blur, kept so the display path allocates
    /// nothing per frame.
    blur: Vec<f32>,
    /// What is drawn.
    level: Vec<f32>,
}

impl Spectrum {
    pub fn new(fft_size: usize, bands: usize, sample_rate: f32) -> Self {
        Self::with_motion(fft_size, bands, sample_rate, Motion::Quick)
    }

    pub fn with_motion(fft_size: usize, bands: usize, sample_rate: f32, motion: Motion) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let scratch = fft.make_output_vec();

        // Hann. Without a window a tone that does not land exactly on a bin
        // smears across the whole spectrum and every bar lights up.
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let t = i as f32 / (fft_size - 1).max(1) as f32;
                0.5 - 0.5 * (2.0 * PI * t).cos()
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
            bands: plan_bands(bands, fft_size, sample_rate),
            motion,
            smoothing: 0.5,
            gain_db: 0.0,
            raw: vec![0.0; bands],
            blur: vec![0.0; bands],
            level: vec![0.0; bands],
        }
    }

    /// How long the bars take to fall, 0 to 1.
    ///
    /// Scales the release time constant either side of the motion's own: at
    /// 0.5 it is exactly that, at 0 the bars snap down about twice as fast,
    /// at 1 they take about twice as long. Attack is left alone -- a slow
    /// attack reads as lag rather than as smoothness.
    pub fn set_smoothing(&mut self, smoothing: f32) {
        self.smoothing = smoothing.clamp(0.0, 1.0);
    }

    /// Shift the displayed range, in dB. Positive lifts quiet material.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = gain_db.clamp(-24.0, 24.0);
    }

    pub fn set_bands(&mut self, n: usize, sample_rate: f32) {
        if n == self.level.len() || n == 0 {
            return;
        }
        self.bands = plan_bands(n, self.size, sample_rate);
        self.raw = vec![0.0; n];
        self.blur = vec![0.0; n];
        self.level = vec![0.0; n];
    }

    /// Re-plan for a new sample rate, keeping the band count.
    pub fn set_rate(&mut self, sample_rate: f32) {
        self.bands = plan_bands(self.level.len(), self.size, sample_rate);
    }

    /// Feed a window of mono samples and advance the display by `dt` seconds.
    ///
    /// `dt` is real elapsed time, not a frame count: the smoothing is defined
    /// by time constants, so a dropped frame slows the bars by exactly the
    /// time it cost rather than by one step of an unknown size.
    pub fn analyze(&mut self, samples: &[f32], dt: f32) {
        let n = self.size.min(samples.len());
        let loud = samples[..n].iter().fold(0.0f32, |m, s| m.max(s.abs()));

        if n == 0 || loud < SILENCE {
            self.raw.fill(0.0);
            self.follow(dt);
            return;
        }

        // Newest samples at the end, zero-padded at the front when short.
        self.input.fill(0.0);
        let start = self.size - n;
        for (dst, src) in self.input[start..].iter_mut().zip(&samples[..n]) {
            *dst = *src;
        }
        for (s, w) in self.input.iter_mut().zip(&self.window) {
            *s *= *w;
        }
        if self
            .fft
            .process(&mut self.input, &mut self.scratch)
            .is_err()
        {
            return;
        }

        // Power per bin, normalised so a full-scale sine reads 0 dBFS.
        let norm = 2.0 / self.window_sum;
        let gain = self.gain_db;
        for (i, band) in self.bands.iter().enumerate() {
            let mut power = 0.0f32;
            for c in &self.scratch[band.lo..band.hi] {
                let m = c.norm() * norm;
                power += m * m;
            }
            // Mean rather than sum: a band spanning more bins is not louder
            // for it, and summing makes the top of the spectrum -- where the
            // ERB bands are widest -- climb for no musical reason.
            let width = (band.hi - band.lo).max(1) as f32;
            let mean = power / width;
            let db = 10.0 * (mean * band.weight).max(1e-20).log10() + gain;
            self.raw[i] = ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0);
        }

        self.spread();
        self.follow(dt);
    }

    /// Blur each band into its neighbours a little.
    fn spread(&mut self) {
        let k = self.motion.spread();
        if k <= 0.0 || self.raw.len() < 3 {
            return;
        }
        let last = self.raw.len() - 1;
        self.blur.copy_from_slice(&self.raw);
        for (i, out) in self.raw.iter_mut().enumerate() {
            let l = self.blur[i.saturating_sub(1)];
            let r = self.blur[(i + 1).min(last)];
            // Normalised so a flat spectrum stays flat.
            *out = (self.blur[i] + k * (l + r)) / (1.0 + 2.0 * k);
        }
    }

    /// One envelope-follower step per band.
    fn follow(&mut self, dt: f32) {
        let (attack_ms, release_ms) = self.motion.taus();
        // 0.5 is the motion's own figure; the ends are half and double it.
        let release_ms = release_ms * (0.5 + self.smoothing);
        let dt = dt.clamp(0.0, 0.25);
        // `1 - exp(-dt/tau)`: the fraction of the remaining distance to cover
        // in this much time. Frame-rate independent by construction.
        let a = 1.0 - (-dt / (attack_ms / 1000.0)).exp();
        let r = 1.0 - (-dt / (release_ms / 1000.0)).exp();
        for (level, target) in self.level.iter_mut().zip(&self.raw) {
            let k = if *target > *level { a } else { r };
            *level += (*target - *level) * k;
        }
    }

    /// The bars, 0 to 1.
    pub fn bands(&self) -> &[f32] {
        &self.level
    }
}

/// Plan `n` bands spaced evenly on the ERB-rate scale.
///
/// Glasberg and Moore (1990): the number of ERBs below `f` is
/// `21.4 log10(0.00437 f + 1)`. Even steps along that, back to Hz, gives band
/// edges the ear would call evenly spaced -- narrow at the bottom, wide at the
/// top, and nothing like the geometric spacing an octave analyser uses.
fn plan_bands(n: usize, fft_size: usize, sample_rate: f32) -> Vec<Band> {
    let bins = fft_size / 2 + 1;
    let hz_per_bin = sample_rate / fft_size as f32;
    let top = (F_HIGH).min(sample_rate / 2.0 - hz_per_bin);

    let (e_lo, e_hi) = (erb_rate(F_LOW), erb_rate(top));
    let mut out = Vec::with_capacity(n);
    let mut prev = 0usize;
    for i in 0..n {
        let f0 = erb_hz(e_lo + (e_hi - e_lo) * i as f32 / n as f32);
        let f1 = erb_hz(e_lo + (e_hi - e_lo) * (i + 1) as f32 / n as f32);
        let lo = (f0 / hz_per_bin).floor() as usize;
        let hi = (f1 / hz_per_bin).ceil() as usize;
        // Every band gets at least one bin, and never one already spent: at
        // the bottom of the range several bands can land in the same bin, and
        // sharing it makes them move as one.
        let lo = lo.max(prev).min(bins.saturating_sub(1));
        let hi = hi.max(lo + 1).min(bins);
        prev = lo + 1;
        out.push(Band {
            lo,
            hi,
            weight: a_weight_power((f0 * f1).sqrt()),
        });
    }
    out
}

/// Number of ERBs below `f`, per Glasberg and Moore (1990).
fn erb_rate(f: f32) -> f32 {
    21.4 * (0.00437 * f + 1.0).log10()
}

/// The inverse: the frequency `e` ERBs up.
fn erb_hz(e: f32) -> f32 {
    (10f32.powf(e / 21.4) - 1.0) / 0.00437
}

/// A-weighting at `f`, as a linear *power* factor.
///
/// IEC 61672-1's curve, which approximates the 40-phon equal-loudness contour:
/// the ear's sensitivity peaks around 2.5 kHz and falls away steeply below a
/// few hundred Hz. Applied to power rather than amplitude, so it is the square
/// of the usual amplitude response.
fn a_weight_power(f: f32) -> f32 {
    let f2 = f * f;
    let num = 12194.0f32.powi(2) * f2 * f2;
    let den = (f2 + 20.6f32.powi(2))
        * ((f2 + 107.7f32.powi(2)) * (f2 + 737.9f32.powi(2))).sqrt()
        * (f2 + 12194.0f32.powi(2));
    if den == 0.0 {
        return 0.0;
    }
    // 1.2589 is +2.0 dB in amplitude, the curve's normalisation to unity at
    // 1 kHz. Squared here because this returns a power factor.
    let amplitude = 1.2589 * num / den;
    amplitude * amplitude
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / rate).sin())
            .collect()
    }

    /// Run enough frames for the envelope followers to settle.
    fn settle(s: &mut Spectrum, samples: &[f32]) {
        for _ in 0..120 {
            s.analyze(samples, 1.0 / 60.0);
        }
    }

    /// Pink-ish noise at a given RMS: equal energy per octave, which is
    /// roughly how music is distributed, unlike white noise or a tone.
    fn pink(rms: f32, n: usize) -> Vec<f32> {
        {
            let pink = |rms: f32, n: usize| -> Vec<f32> {
                let mut state = [0.0f32; 7];
                let mut seed = 0x2545_F491_4F6C_DD1Du64;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let w = (seed >> 40) as f32 / 8388608.0 - 1.0;
                    // Voss-McCartney-ish filter bank.
                    state[0] = 0.99886 * state[0] + w * 0.0555179;
                    state[1] = 0.99332 * state[1] + w * 0.0750759;
                    state[2] = 0.96900 * state[2] + w * 0.153852;
                    state[3] = 0.86650 * state[3] + w * 0.3104856;
                    state[4] = 0.55000 * state[4] + w * 0.5329522;
                    state[5] = -0.7616 * state[5] - w * 0.0168980;
                    let v = state.iter().take(6).sum::<f32>() + w * 0.5362;
                    state[6] = w * 0.115926;
                    out.push(v * 0.11);
                }
                let cur = (out.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt();
                let k = if cur > 0.0 { rms / cur } else { 0.0 };
                out.iter().map(|s| s * k).collect()
            };
            pink(rms, n)
        }
    }

    /// The tallest bar for a master at this RMS, once settled.
    fn peak_band(rms: f32) -> f32 {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        let sig = pink(rms, 2048);
        for _ in 0..120 {
            s.analyze(&sig, 1.0 / 60.0);
        }
        s.bands().iter().cloned().fold(0.0f32, f32::max)
    }

    /// The display has to use its whole height.
    ///
    /// The ceiling is what one *band* reaches, not what the signal does, and
    /// getting that wrong is invisible in every other test: with it set to
    /// -6 dBFS every bar was correct, smooth, in range -- and the top third
    /// of the panel was never drawn, because no single band of twenty ever
    /// holds that much of the energy.
    #[test]
    fn a_loud_master_fills_the_panel() {
        let loud = peak_band(0.355);
        assert!(loud >= 0.98, "a -9 dBFS master only reaches {loud:.2}");

        let ordinary = peak_band(0.2);
        assert!(
            ordinary > 0.85,
            "a -14 dBFS master only reaches {ordinary:.2}"
        );

        // And still climbs: a ceiling low enough to fill the panel must not
        // be so low that everything is pinned there and loudness stops
        // showing at all.
        let quiet = peak_band(0.1);
        assert!(
            quiet < ordinary,
            "{quiet:.2} then {ordinary:.2} is not a climb"
        );
        assert!(quiet > 0.5, "a -20 dBFS master is down at {quiet:.2}");
    }

    /// Not an assertion -- run with `--nocapture` to see the numbers the
    /// ceiling was calibrated against.
    #[test]
    fn preview_levels_for_realistic_material() {
        for (name, rms) in [("-20 dBFS", 0.1), ("-14 dBFS", 0.2), ("-9 dBFS", 0.355)] {
            let mut s = Spectrum::new(2048, 20, 44_100.0);
            let sig = pink(rms, 2048);
            for _ in 0..120 {
                s.analyze(&sig, 1.0 / 60.0);
            }
            let max = s.bands().iter().cloned().fold(0.0f32, f32::max);
            let mean = s.bands().iter().sum::<f32>() / s.bands().len() as f32;
            println!("  pink {name:10}  peak band {max:.2}  mean {mean:.2}");
        }
    }

    #[test]
    fn silence_produces_no_bars() {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        settle(&mut s, &vec![0.0; 2048]);
        assert!(s.bands().iter().all(|&b| b < 1e-3), "{:?}", s.bands());
    }

    #[test]
    fn dither_is_not_a_light_show() {
        // Well below SILENCE: an analyser with gain will draw this if it can.
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        let quiet: Vec<f32> = (0..2048)
            .map(|i| if i % 2 == 0 { 1e-6 } else { -1e-6 })
            .collect();
        settle(&mut s, &quiet);
        assert!(s.bands().iter().all(|&b| b < 1e-3));
    }

    #[test]
    fn a_tone_lights_the_band_it_belongs_to() {
        let mut s = Spectrum::new(4096, 24, 44_100.0);
        settle(&mut s, &sine(1000.0, 44_100.0, 4096));
        let loudest = s
            .bands()
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        // Which band 1 kHz falls in, computed the same way the planner does.
        let (e_lo, e_hi) = (erb_rate(F_LOW), erb_rate(F_HIGH));
        let want = ((erb_rate(1000.0) - e_lo) / (e_hi - e_lo) * 24.0) as usize;
        assert!(
            loudest.abs_diff(want) <= 1,
            "1 kHz lit band {loudest}, expected about {want}"
        );
    }

    #[test]
    fn a_low_tone_lights_a_lower_band_than_a_high_one() {
        let mut low = Spectrum::new(4096, 24, 44_100.0);
        let mut high = Spectrum::new(4096, 24, 44_100.0);
        settle(&mut low, &sine(120.0, 44_100.0, 4096));
        settle(&mut high, &sine(6000.0, 44_100.0, 4096));
        let peak = |s: &Spectrum| {
            s.bands()
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0
        };
        assert!(peak(&low) < peak(&high));
    }

    /// The point of A-weighting: equal amplitude is not equal loudness.
    #[test]
    fn the_ear_curve_favours_the_middle_over_the_bottom() {
        // Both at 2.5 kHz-ish sensitivity peak versus deep bass.
        assert!(a_weight_power(2500.0) > a_weight_power(60.0) * 100.0);
        // And rolls off again at the very top.
        assert!(a_weight_power(2500.0) > a_weight_power(16_000.0));
        // Unity at 1 kHz, by definition of the curve.
        let at_1k = a_weight_power(1000.0).sqrt();
        assert!((at_1k - 1.0).abs() < 0.02, "A(1kHz) = {at_1k}, want 1.0");
    }

    /// The ERB scale is the reason the bass end gets room.
    #[test]
    fn erb_bands_are_narrow_at_the_bottom_and_wide_at_the_top() {
        let bands = plan_bands(24, 4096, 44_100.0);
        let hz_per_bin = 44_100.0 / 4096.0;
        let width = |b: &Band| (b.hi - b.lo) as f32 * hz_per_bin;
        assert!(
            width(&bands[0]) < width(&bands[23]),
            "first {} Hz, last {} Hz",
            width(&bands[0]),
            width(&bands[23])
        );
    }

    #[test]
    fn erb_rate_and_hz_are_inverses() {
        for f in [50.0f32, 440.0, 1000.0, 5000.0, 15_000.0] {
            let back = erb_hz(erb_rate(f));
            assert!((back - f).abs() < f * 0.001, "{f} -> {back}");
        }
    }

    #[test]
    fn bands_are_ordered_and_never_empty() {
        for n in [8usize, 20, 24, 64, 128] {
            let bands = plan_bands(n, 2048, 44_100.0);
            assert_eq!(bands.len(), n);
            for b in &bands {
                assert!(b.hi > b.lo, "empty band in a {n}-band plan");
            }
            for w in bands.windows(2) {
                assert!(w[1].lo >= w[0].lo, "bands out of order in {n}");
            }
        }
    }

    /// Time constants, not per-frame steps: the same wall-clock time has to
    /// produce the same picture at any frame rate.
    #[test]
    fn smoothing_is_frame_rate_independent() {
        let run = |dt: f32, frames: usize| {
            let mut s = Spectrum::new(2048, 20, 44_100.0);
            let tone = sine(1000.0, 44_100.0, 2048);
            for _ in 0..frames {
                s.analyze(&tone, dt);
            }
            s.bands().iter().cloned().fold(0.0f32, f32::max)
        };
        // Half a second, at 30 and at 120 frames a second.
        let slow = run(1.0 / 30.0, 15);
        let fast = run(1.0 / 120.0, 60);
        assert!((slow - fast).abs() < 0.02, "{slow} vs {fast}");
    }

    #[test]
    fn a_band_rises_faster_than_it_falls() {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        let tone = sine(1000.0, 44_100.0, 2048);
        let silence = vec![0.0; 2048];
        // One frame of signal from cold.
        s.analyze(&tone, 1.0 / 60.0);
        let up = s.bands().iter().cloned().fold(0.0f32, f32::max);
        settle(&mut s, &tone);
        let held = s.bands().iter().cloned().fold(0.0f32, f32::max);
        // One frame of silence from settled.
        s.analyze(&silence, 1.0 / 60.0);
        let down = held - s.bands().iter().cloned().fold(0.0f32, f32::max);
        assert!(up > down, "attack {up} should outpace release {down}");
    }

    #[test]
    fn gain_lifts_quiet_material() {
        let quiet: Vec<f32> = sine(1000.0, 44_100.0, 2048)
            .iter()
            .map(|s| s * 0.01)
            .collect();
        let mut plain = Spectrum::new(2048, 20, 44_100.0);
        let mut lifted = Spectrum::new(2048, 20, 44_100.0);
        lifted.set_gain_db(12.0);
        settle(&mut plain, &quiet);
        settle(&mut lifted, &quiet);
        let peak = |s: &Spectrum| s.bands().iter().cloned().fold(0.0f32, f32::max);
        assert!(peak(&lifted) > peak(&plain));
    }

    #[test]
    fn levels_stay_in_range() {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        s.set_gain_db(24.0);
        // Full-scale square wave: the loudest thing there is.
        let loud: Vec<f32> = (0..2048)
            .map(|i| if (i / 8) % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        settle(&mut s, &loud);
        assert!(s.bands().iter().all(|&b| (0.0..=1.0).contains(&b)));
    }

    #[test]
    fn changing_the_band_count_resizes_everything() {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        settle(&mut s, &sine(1000.0, 44_100.0, 2048));
        s.set_bands(64, 44_100.0);
        assert_eq!(s.bands().len(), 64);
    }

    #[test]
    fn fluid_motion_settles_more_slowly_than_quick() {
        let tone = sine(1000.0, 44_100.0, 2048);
        let mut quick = Spectrum::with_motion(2048, 20, 44_100.0, Motion::Quick);
        let mut fluid = Spectrum::with_motion(2048, 20, 44_100.0, Motion::Fluid);
        for _ in 0..3 {
            quick.analyze(&tone, 1.0 / 60.0);
            fluid.analyze(&tone, 1.0 / 60.0);
        }
        let peak = |s: &Spectrum| s.bands().iter().cloned().fold(0.0f32, f32::max);
        assert!(peak(&quick) > peak(&fluid), "quick should lead fluid");
    }

    #[test]
    fn a_rate_change_replans_without_changing_the_band_count() {
        let mut s = Spectrum::new(2048, 20, 44_100.0);
        s.set_rate(48_000.0);
        assert_eq!(s.bands().len(), 20);
        settle(&mut s, &sine(1000.0, 48_000.0, 2048));
        assert!(s.bands().iter().any(|&b| b > 0.0));
    }

    /// A half-rate stream must not plan bands past Nyquist.
    #[test]
    fn bands_stay_below_nyquist_at_low_rates() {
        let bands = plan_bands(24, 2048, 8_000.0);
        let bins = 2048 / 2 + 1;
        assert!(bands.iter().all(|b| b.hi <= bins));
    }
}
