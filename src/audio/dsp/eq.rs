//! Ordered parametric and Equalizer APO-compatible equalization.
//!
//! The old Winamp ten-band curves remain as built-in profiles and as the
//! legacy config representation. Imported and edited profiles compile to one
//! ordered chain. Coefficients, filter state, custom IIR processing and
//! GraphicEQ convolution are all `f64`; only the surrounding player buffer is
//! `f32`. An `ArcSwap` publishes a complete immutable chain without exposing a
//! partly edited profile to the playback thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::apo::{ChannelMask, Filter, Profile};
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
    /// An empty chain means the classic ten-band fields above are in use.
    pub stages: Vec<CompiledStage>,
}

#[derive(Debug, Clone)]
pub struct CompiledStage {
    pub channels: ChannelMask,
    pub filter: CompiledFilter,
}

#[derive(Debug, Clone)]
pub enum CompiledFilter {
    Gain(f64),
    Biquad(Coeffs),
    Iir {
        numerator: Vec<f64>,
        feedback: Vec<f64>,
    },
    Fir(Arc<[f64]>),
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
            stages: Vec::new(),
        }
    }

    /// The chain's magnitude response at `f`, in dB.
    ///
    /// Computed from the *compiled* filters -- the same coefficients the
    /// audio path runs -- so a curve drawn from this cannot drift away from
    /// what is actually being heard. Reimplementing the response from the
    /// profile would be a second copy of the maths, and the two would agree
    /// only until one of them was edited.
    ///
    /// Channel masks are ignored: the display shows one curve, and a filter
    /// applied to one channel still belongs on it.
    pub fn magnitude_db_at(&self, f: f64, sample_rate: u32) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        let sr = sample_rate.max(1) as f64;
        let mut mag = self.preamp_linear as f64;

        if self.stages.is_empty() {
            // The classic ten-band chain, which is held as coefficients
            // rather than as stages.
            for c in &self.coeffs {
                mag *= c.magnitude_at(f, sr);
            }
        } else {
            for stage in &self.stages {
                mag *= match &stage.filter {
                    CompiledFilter::Gain(v) => *v,
                    CompiledFilter::Biquad(c) => c.magnitude_at(f, sr),
                    CompiledFilter::Iir {
                        numerator,
                        feedback,
                    } => {
                        // The same evaluation a biquad does, for an arbitrary
                        // order: sum each side around the unit circle at w.
                        let w = std::f64::consts::TAU * f / sr;
                        let poly = |taps: &[f64], first: f64| {
                            let (mut re, mut im) = (first, 0.0);
                            for (n, a) in taps.iter().enumerate() {
                                let ang = -(n as f64 + 1.0) * w;
                                re += a * ang.cos();
                                im += a * ang.sin();
                            }
                            (re * re + im * im).sqrt()
                        };
                        let num = poly(&numerator[1..], numerator[0]);
                        // `feedback` is already negated, so the denominator is
                        // one *minus* it.
                        let den = poly(&feedback.iter().map(|v| -v).collect::<Vec<_>>(), 1.0);
                        if den > 0.0 {
                            num / den
                        } else {
                            1.0
                        }
                    }
                    CompiledFilter::Fir(impulse) => {
                        let w = std::f64::consts::TAU * f / sr;
                        let (mut re, mut im) = (0.0, 0.0);
                        for (n, h) in impulse.iter().enumerate() {
                            let ang = -(n as f64) * w;
                            re += h * ang.cos();
                            im += h * ang.sin();
                        }
                        (re * re + im * im).sqrt()
                    }
                };
            }
        }
        20.0 * mag.max(1e-9).log10()
    }

    pub fn from_profile(enabled: bool, profile: &Profile, sample_rate: u32) -> Self {
        let stages = profile
            .stages
            .iter()
            .filter(|stage| stage.enabled)
            .map(|stage| {
                let filter = match &stage.filter {
                    Filter::Preamp { gain_db } => CompiledFilter::Gain(10f64.powf(gain_db / 20.0)),
                    Filter::Biquad {
                        kind,
                        frequency,
                        gain_db,
                        width,
                        corner_frequency,
                    } => CompiledFilter::Biquad(Coeffs::apo(
                        *kind,
                        *frequency,
                        *gain_db,
                        *width,
                        *corner_frequency,
                        sample_rate as f64,
                    )),
                    Filter::Iir {
                        numerator,
                        denominator,
                    } => {
                        let a0 = denominator[0];
                        CompiledFilter::Iir {
                            numerator: numerator.iter().map(|v| v / a0).collect(),
                            feedback: denominator[1..].iter().map(|v| -v / a0).collect(),
                        }
                    }
                    Filter::GraphicEq { points } => {
                        CompiledFilter::Fir(graphic_impulse(points, sample_rate))
                    }
                };
                CompiledStage {
                    channels: stage.channels,
                    filter,
                }
            })
            .collect::<Vec<_>>();
        let all_flat = stages.is_empty()
            || stages.iter().all(|s| match &s.filter {
                CompiledFilter::Gain(v) => (*v - 1.0).abs() < f64::EPSILON,
                CompiledFilter::Biquad(c) => *c == Coeffs::IDENTITY,
                _ => false,
            });
        Self {
            enabled,
            preamp_db: 0.0,
            coeffs: [Coeffs::IDENTITY; 10],
            all_flat,
            preamp_linear: 1.0,
            stages,
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
    dynamic: Vec<Vec<StageState>>,
    work: Vec<f64>,
}

enum StageState {
    Gain,
    Biquad(State),
    Iir { x: Vec<f64>, y: Vec<f64> },
    Fir(Convolver),
}

struct Convolver {
    history: Vec<f64>,
    forward: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
    data: Vec<Complex<f64>>,
    filter: Vec<Complex<f64>>,
    filter_impulse: Option<Arc<[f64]>>,
    scratch: Vec<Complex<f64>>,
}

impl EqState {
    pub fn new(channels: usize) -> Self {
        Self {
            states: vec![[State::default(); 10]; channels],
            dynamic: Vec::new(),
            work: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        for ch in &mut self.states {
            for s in ch.iter_mut() {
                s.reset();
            }
        }
        self.dynamic.clear();
        self.work.clear();
    }

    /// Apply in place to an interleaved buffer.
    #[inline]
    pub fn process(&mut self, settings: &EqSettings, buf: &mut [f32], channels: usize) {
        if settings.is_transparent() {
            return;
        }
        if !settings.stages.is_empty() {
            return self.process_dynamic(settings, buf, channels);
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

    fn process_dynamic(&mut self, settings: &EqSettings, buf: &mut [f32], channels: usize) {
        self.ensure_dynamic(settings, channels);
        self.work.resize(buf.len(), 0.0);
        for (dst, src) in self.work.iter_mut().zip(buf.iter()) {
            *dst = *src as f64;
        }
        for (stage_index, stage) in settings.stages.iter().enumerate() {
            match &stage.filter {
                CompiledFilter::Gain(gain) => {
                    for frame in self.work.chunks_mut(channels) {
                        for (channel, sample) in frame.iter_mut().enumerate() {
                            if stage.channels.contains(channel) {
                                *sample *= gain;
                            }
                        }
                    }
                }
                CompiledFilter::Biquad(coeffs) => {
                    for frame in self.work.chunks_mut(channels) {
                        for (channel, sample) in frame.iter_mut().enumerate() {
                            if stage.channels.contains(channel) {
                                let StageState::Biquad(state) =
                                    &mut self.dynamic[channel][stage_index]
                                else {
                                    unreachable!()
                                };
                                *sample = state.process_f64(coeffs, *sample);
                            }
                        }
                    }
                }
                CompiledFilter::Iir {
                    numerator,
                    feedback,
                } => {
                    for frame in self.work.chunks_mut(channels) {
                        for (channel, sample) in frame.iter_mut().enumerate() {
                            if !stage.channels.contains(channel) {
                                continue;
                            }
                            let StageState::Iir { x, y } = &mut self.dynamic[channel][stage_index]
                            else {
                                unreachable!()
                            };
                            let input = *sample;
                            let mut output = numerator[0] * input;
                            for i in 0..x.len() {
                                output += numerator[i + 1] * x[i] + feedback[i] * y[i];
                            }
                            x.rotate_right(1);
                            y.rotate_right(1);
                            x[0] = input;
                            y[0] = output;
                            *sample = output;
                        }
                    }
                }
                CompiledFilter::Fir(impulse) => {
                    let frames = self.work.len() / channels;
                    for channel in 0..channels {
                        if !stage.channels.contains(channel) {
                            continue;
                        }
                        let StageState::Fir(convolver) = &mut self.dynamic[channel][stage_index]
                        else {
                            unreachable!()
                        };
                        convolver.process_interleaved(
                            &mut self.work,
                            channels,
                            channel,
                            frames,
                            impulse,
                        );
                    }
                }
            }
        }
        for (dst, src) in buf.iter_mut().zip(self.work.iter()) {
            *dst = *src as f32;
        }
    }

    fn ensure_dynamic(&mut self, settings: &EqSettings, channels: usize) {
        let shape_matches = self.dynamic.len() == channels
            && self
                .dynamic
                .first()
                .map_or(settings.stages.is_empty(), |states| {
                    states.len() == settings.stages.len()
                        && states.iter().zip(&settings.stages).all(|(state, stage)| {
                            match (state, &stage.filter) {
                                (StageState::Gain, CompiledFilter::Gain(_))
                                | (StageState::Biquad(_), CompiledFilter::Biquad(_))
                                | (StageState::Fir(_), CompiledFilter::Fir(_)) => true,
                                (
                                    StageState::Iir { x, y },
                                    CompiledFilter::Iir {
                                        numerator,
                                        feedback,
                                    },
                                ) => x.len() + 1 == numerator.len() && y.len() == feedback.len(),
                                _ => false,
                            }
                        })
                });
        if shape_matches {
            return;
        }
        self.dynamic = (0..channels)
            .map(|_| {
                settings
                    .stages
                    .iter()
                    .map(|stage| match &stage.filter {
                        CompiledFilter::Gain(_) => StageState::Gain,
                        CompiledFilter::Biquad(_) => StageState::Biquad(State::default()),
                        CompiledFilter::Iir { numerator, .. } => StageState::Iir {
                            x: vec![0.0; numerator.len() - 1],
                            y: vec![0.0; numerator.len() - 1],
                        },
                        CompiledFilter::Fir(impulse) => {
                            StageState::Fir(Convolver::new(impulse.len()))
                        }
                    })
                    .collect()
            })
            .collect();
    }
}

impl Convolver {
    fn new(filter_len: usize) -> Self {
        let n = (filter_len + 4096 - 1).next_power_of_two();
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(n);
        let inverse = planner.plan_fft_inverse(n);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        Self {
            history: vec![0.0; filter_len.saturating_sub(1)],
            forward,
            inverse,
            data: vec![Complex::default(); n],
            filter: vec![Complex::default(); n],
            filter_impulse: None,
            scratch: vec![Complex::default(); scratch_len],
        }
    }

    fn process_interleaved(
        &mut self,
        data: &mut [f64],
        channels: usize,
        channel: usize,
        frames: usize,
        impulse: &Arc<[f64]>,
    ) {
        let history_len = self.history.len();
        self.data.fill(Complex::default());
        for (dst, &src) in self.data.iter_mut().zip(&self.history) {
            dst.re = src;
        }
        for frame in 0..frames {
            self.data[history_len + frame].re = data[frame * channels + channel];
        }
        if frames >= history_len {
            for (i, dst) in self.history.iter_mut().enumerate() {
                *dst = data[(frames - history_len + i) * channels + channel];
            }
        } else {
            self.history.rotate_left(frames);
            for frame in 0..frames {
                self.history[history_len - frames + frame] = data[frame * channels + channel];
            }
        }
        self.forward
            .process_with_scratch(&mut self.data, &mut self.scratch);
        let changed = self
            .filter_impulse
            .as_ref()
            .is_none_or(|cached| !Arc::ptr_eq(cached, impulse));
        if changed {
            self.filter.fill(Complex::default());
            for (dst, &tap) in self.filter.iter_mut().zip(impulse.iter()) {
                dst.re = tap;
            }
            self.forward
                .process_with_scratch(&mut self.filter, &mut self.scratch);
            self.filter_impulse = Some(Arc::clone(impulse));
        }
        for (sample, h) in self.data.iter_mut().zip(&self.filter) {
            *sample *= *h;
        }
        self.inverse
            .process_with_scratch(&mut self.data, &mut self.scratch);
        let scale = 1.0 / self.data.len() as f64;
        for frame in 0..frames {
            data[frame * channels + channel] = self.data[history_len + frame].re * scale;
        }
    }
}

const GRAPHIC_EQ_TAPS: usize = 16_384;

fn graphic_impulse(points: &[(f64, f64)], sample_rate: u32) -> Arc<[f64]> {
    let n = GRAPHIC_EQ_TAPS * 2;
    let mut planner = FftPlanner::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    let scratch_len = forward
        .get_inplace_scratch_len()
        .max(inverse.get_inplace_scratch_len());
    let mut scratch = vec![Complex::default(); scratch_len];
    let mut spectrum = vec![Complex::default(); n];
    for i in 0..GRAPHIC_EQ_TAPS {
        let frequency = i as f64 * sample_rate as f64 / n as f64;
        let gain = 10f64.powf(graphic_gain(points, frequency) / 20.0);
        spectrum[i].re = gain;
        spectrum[n - i - 1].re = gain;
    }
    for bin in &mut spectrum {
        bin.re = bin.re.max(1e-5).ln();
        bin.im = 0.0;
    }
    inverse.process_with_scratch(&mut spectrum, &mut scratch);
    let scale = 1.0 / n as f64;
    for bin in &mut spectrum {
        *bin *= scale;
    }
    for i in 1..GRAPHIC_EQ_TAPS {
        spectrum[i].re += spectrum[n - i].re;
        spectrum[i].im -= spectrum[n - i].im;
        spectrum[n - i] = Complex::default();
    }
    spectrum[GRAPHIC_EQ_TAPS].im *= -1.0;
    forward.process_with_scratch(&mut spectrum, &mut scratch);
    for bin in &mut spectrum {
        *bin = bin.exp();
    }
    inverse.process_with_scratch(&mut spectrum, &mut scratch);
    let impulse = (0..GRAPHIC_EQ_TAPS)
        .map(|i| {
            let window = 0.5 * (1.0 + (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos());
            spectrum[i].re * scale * window
        })
        .collect::<Vec<_>>();
    impulse.into()
}

fn graphic_gain(points: &[(f64, f64)], frequency: f64) -> f64 {
    if frequency <= points[0].0 {
        return points[0].1;
    }
    if frequency >= points[points.len() - 1].0 {
        return points[points.len() - 1].1;
    }
    let right = points.partition_point(|p| p.0 < frequency);
    let (lf, lg) = points[right - 1];
    let (rf, rg) = points[right];
    let t = (frequency.ln() - lf.ln()) / (rf.ln() - lf.ln());
    lg + t * (rg - lg)
}

/// The live, swappable EQ the callback reads without locking.
pub struct EqHandle {
    inner: ArcSwap<EqSettings>,
    profile: ArcSwap<Profile>,
    enabled: AtomicBool,
}

impl EqHandle {
    pub fn new(sample_rate: u32) -> Self {
        let flat = Profile::legacy("Flat", 0.0, &[0.0; 10]);
        Self {
            inner: ArcSwap::from_pointee(EqSettings::flat(sample_rate)),
            profile: ArcSwap::from_pointee(flat),
            enabled: AtomicBool::new(false),
        }
    }

    pub fn load(&self) -> Arc<EqSettings> {
        self.inner.load_full()
    }

    pub fn store(&self, settings: EqSettings) {
        self.inner.store(Arc::new(settings));
    }

    pub fn store_profile(&self, enabled: bool, profile: Profile, sample_rate: u32) {
        let compiled = EqSettings::from_profile(enabled, &profile, sample_rate);
        self.profile.store(Arc::new(profile));
        self.enabled.store(enabled, Ordering::Release);
        self.inner.store(Arc::new(compiled));
    }

    /// Recompile the semantic profile when the output device's rate changes.
    pub fn rebuild(&self, sample_rate: u32) {
        let profile = self.profile.load_full();
        let enabled = self.enabled.load(Ordering::Acquire);
        self.inner.store(Arc::new(EqSettings::from_profile(
            enabled,
            &profile,
            sample_rate,
        )));
    }
}

pub fn preset_by_name(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {

    /// The curve has to agree with the filters it is drawn from.
    #[test]
    fn the_response_matches_what_the_filters_do() {
        use crate::audio::dsp::apo::{BiquadKind, Filter, Profile, Stage, Width};

        let sr = 44_100u32;
        let stage = |filter| Stage {
            enabled: true,
            channels: ChannelMask::ALL,
            filter,
        };
        let profile = Profile {
            stages: vec![stage(Filter::Biquad {
                kind: BiquadKind::Peaking,
                frequency: 1_000.0,
                gain_db: 6.0,
                width: Width::Q(1.4),
                corner_frequency: false,
            })],
            name: "test".into(),
        };
        let s = EqSettings::from_profile(true, &profile, sr);

        // At the centre, a +6 dB peaking filter is +6 dB.
        let at_centre = s.magnitude_db_at(1_000.0, sr);
        assert!(
            (at_centre - 6.0).abs() < 0.1,
            "1 kHz reads {at_centre:.2} dB, wanted +6"
        );
        // Far either side it does nothing.
        assert!(s.magnitude_db_at(50.0, sr).abs() < 0.5);
        assert!(s.magnitude_db_at(15_000.0, sr).abs() < 0.5);
    }

    /// A preamp moves the whole curve, and a disabled chain is flat.
    #[test]
    fn the_response_accounts_for_preamp_and_bypass() {
        use crate::audio::dsp::apo::{Filter, Profile, Stage};

        let sr = 44_100u32;
        let profile = Profile {
            stages: vec![Stage {
                enabled: true,
                channels: ChannelMask::ALL,
                filter: Filter::Preamp { gain_db: -6.0 },
            }],
            name: "test".into(),
        };
        let on = EqSettings::from_profile(true, &profile, sr);
        for f in [100.0, 1_000.0, 10_000.0] {
            let db = on.magnitude_db_at(f, sr);
            assert!((db + 6.0).abs() < 0.1, "{f} Hz reads {db:.2}, wanted -6");
        }

        // Bypassed is flat, whatever the profile says.
        let off = EqSettings::from_profile(false, &profile, sr);
        assert_eq!(off.magnitude_db_at(1_000.0, sr), 0.0);
    }

    /// The ten-band chain is held as coefficients rather than stages, and has
    /// to report a response too.
    #[test]
    fn the_ten_band_chain_reports_its_own_response() {
        let sr = 44_100u32;
        let mut gains = [0.0f32; 10];
        gains[4] = 8.0;
        let s = EqSettings::build(true, 0.0, &gains, sr);
        let at_band = s.magnitude_db_at(BANDS[4], sr);
        assert!(
            at_band > 6.0,
            "the lifted band reads {at_band:.2} dB, wanted most of +8"
        );
    }

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

    #[test]
    fn a_profile_is_applied_in_order_and_only_to_selected_channels() {
        use super::super::apo::{ChannelMask, Filter, Stage};
        let profile = Profile {
            name: "left only".into(),
            stages: vec![Stage {
                enabled: true,
                channels: ChannelMask(1),
                filter: Filter::Preamp { gain_db: 6.0 },
            }],
        };
        let settings = EqSettings::from_profile(true, &profile, 48_000);
        let mut state = EqState::new(2);
        let mut audio = [0.25, 0.25, -0.25, -0.25];
        state.process(&settings, &mut audio, 2);
        let gain = 10f32.powf(6.0 / 20.0);
        assert!((audio[0] - 0.25 * gain).abs() < 1e-6);
        assert_eq!(audio[1], 0.25);
        assert!((audio[2] + 0.25 * gain).abs() < 1e-6);
        assert_eq!(audio[3], -0.25);
    }

    #[test]
    fn custom_iir_uses_the_apo_difference_equation() {
        use super::super::apo::{ChannelMask, Filter, Stage};
        let profile = Profile {
            name: "one pole".into(),
            stages: vec![Stage {
                enabled: true,
                channels: ChannelMask::ALL,
                filter: Filter::Iir {
                    numerator: vec![0.5, 0.0],
                    denominator: vec![1.0, -0.5],
                },
            }],
        };
        let settings = EqSettings::from_profile(true, &profile, 48_000);
        let mut state = EqState::new(1);
        let mut impulse = [1.0, 0.0, 0.0, 0.0];
        state.process(&settings, &mut impulse, 1);
        assert_eq!(impulse, [0.5, 0.25, 0.125, 0.0625]);
    }

    #[test]
    fn flat_graphic_eq_is_numerically_transparent() {
        use super::super::apo::{ChannelMask, Filter, Stage};
        let profile = Profile {
            name: "graphic flat".into(),
            stages: vec![Stage {
                enabled: true,
                channels: ChannelMask::ALL,
                filter: Filter::GraphicEq {
                    points: vec![(20.0, 0.0), (20_000.0, 0.0)],
                },
            }],
        };
        let settings = EqSettings::from_profile(true, &profile, 48_000);
        let mut state = EqState::new(1);
        let mut impulse = vec![0.0; 4096];
        impulse[0] = 1.0;
        state.process(&settings, &mut impulse, 1);
        assert!((impulse[0] - 1.0).abs() < 1e-5, "first tap {}", impulse[0]);
        assert!(impulse[1..].iter().all(|sample| sample.abs() < 1e-5));
    }

    #[test]
    fn rebuilding_for_a_new_rate_changes_the_coefficients() {
        let profile = Profile::legacy("rate", 0.0, &[6.0; 10]);
        let handle = EqHandle::new(44_100);
        handle.store_profile(true, profile, 44_100);
        let first = match &handle.load().stages[1].filter {
            CompiledFilter::Biquad(coeffs) => *coeffs,
            _ => unreachable!(),
        };
        handle.rebuild(96_000);
        let second = match &handle.load().stages[1].filter {
            CompiledFilter::Biquad(coeffs) => *coeffs,
            _ => unreachable!(),
        };
        assert_ne!(first, second);
    }
}
