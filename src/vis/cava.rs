//! cava's analysis core, ported.
//!
//! Ported from cava (MIT, Copyright (c) 2015 Karl Stavestrand), `cavacore.c`
//! for the analysis and the `monstercat_filter` from `cava.c`. The constants
//! are cava's own and are not re-derived here: they were tuned against the
//! renderer they feed, and changing one of them changes the feel of all of
//! them.
//!
//! Three things make this look different from [`super::analysis::Analyzer`],
//! and all three are worth having as a second option rather than as a
//! replacement:
//!
//! * **Two window sizes.** Bass gets a window twice as long as everything
//!   else, because that is where log-spaced bands are narrowest and a short
//!   window cannot resolve them. One FFT size has to choose between smearing
//!   the bass and lagging the treble.
//! * **Autosens.** The display scales itself to the material instead of
//!   sitting on a fixed dBFS window, so a quiet master and a loud one both
//!   fill the panel. There is no gain to configure.
//! * **Gravity and integral smoothing.** Bars fall under acceleration from
//!   their own peak rather than decaying exponentially, and rise through a
//!   leaky integrator. That is the fluid motion cava is recognised for; the
//!   asymmetric IIR in the other analyzer is crisper and less liquid.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// Frequency range the bars span, in Hz. cava's defaults.
const LOW_CUT_OFF: f64 = 50.0;
const HIGH_CUT_OFF: f64 = 8_000.0;

/// Bars below this frequency are taken from the long window.
const BASS_CUT_OFF: f64 = 100.0;

/// cava's `smoothing:noise_reduction`, already divided by 100.
///
/// Despite the name this is the main smoothing control: it is the integrator's
/// feedback coefficient and it also scales gravity. Higher is smoother and
/// slower.
///
/// cava ships 0.77 and runs at 60 fps. staramp analyses at about 30, where the
/// same figure measured 500 ms to fall and 233 ms to rise -- visibly behind
/// the music, and the reason this is not cava's number. At 0.45 the same
/// measurement gives 333 ms and 100 ms, which is fractionally quicker than
/// cava's own feel at 60 fps and still fluid. `[vis] smoothing` overrides it;
/// `latency_probe` prints the table these came from.
pub const DEFAULT_SMOOTHING: f64 = 0.45;

/// cava's `smoothing:monstercat`.
///
/// Off in cava's own config, on here. It spreads each bar's energy into its
/// neighbours, and with bars one cell wide there are enough of them that
/// without it the display is a picket fence rather than a waveform.
const MONSTERCAT: f64 = 1.5;

/// One FFT and its window.
struct Plan {
    fft: Arc<dyn RealToComplex<f64>>,
    size: usize,
    window: Vec<f64>,
    input: Vec<f64>,
    output: Vec<Complex<f64>>,
}

impl Plan {
    fn new(planner: &mut RealFftPlanner<f64>, size: usize) -> Self {
        let fft = planner.plan_fft_forward(size);
        let output = fft.make_output_vec();
        Self {
            fft,
            size,
            // Hann.
            window: (0..size)
                .map(|i| {
                    0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / (size - 1) as f64).cos())
                })
                .collect(),
            input: vec![0.0; size],
            output,
        }
    }

    /// Window `src` (newest sample first) and transform it.
    fn run(&mut self, src: &[f64]) {
        for (i, slot) in self.input.iter_mut().enumerate() {
            *slot = self.window[i] * src[i];
        }
        let _ = self.fft.process(&mut self.input, &mut self.output);
    }

    fn magnitude(&self, bin: usize) -> f64 {
        self.output.get(bin).map(|c| c.norm()).unwrap_or(0.0)
    }
}

pub struct Cava {
    bars: usize,
    rate: u32,

    bass: Plan,
    mid: Plan,

    /// Rolling history, newest sample at index 0, as cava keeps it.
    input: Vec<f64>,

    lower_cut: Vec<usize>,
    upper_cut: Vec<usize>,
    eq: Vec<f64>,
    /// The first bar at or above [`BASS_CUT_OFF`].
    bass_cut_off_bar: usize,

    // Smoothing state, one per bar.
    fall: Vec<f64>,
    mem: Vec<f64>,
    peak: Vec<f64>,
    prev: Vec<f64>,

    out: Vec<f32>,

    noise_reduction: f64,
    sens: f64,
    sens_init: bool,
    framerate: f64,
    frame_skip: u32,
}

/// cava's window size for a given rate: 512 points, doubled per octave of
/// sample rate, so the window covers a comparable span of time at any rate.
fn window_size(rate: u32) -> usize {
    let mut n = 512usize;
    if rate > 8_125 && rate <= 16_250 {
        n *= 2;
    } else if rate > 16_250 && rate <= 32_500 {
        n *= 4;
    } else if rate > 32_500 && rate <= 75_000 {
        n *= 8;
    } else if rate > 75_000 && rate <= 150_000 {
        n *= 16;
    } else if rate > 150_000 && rate <= 300_000 {
        n *= 32;
    } else if rate > 300_000 {
        n *= 64;
    }
    n
}

impl Cava {
    pub fn new(bars: usize, rate: u32) -> Self {
        Self::with_smoothing(bars, rate, DEFAULT_SMOOTHING)
    }

    /// `smoothing` is cava's `noise_reduction`, 0 to 1.
    pub fn with_smoothing(bars: usize, rate: u32, smoothing: f64) -> Self {
        let rate = rate.clamp(8_000, 384_000);
        let mid_size = window_size(rate);
        let bass_size = mid_size * 2;
        // cava refuses more bars than the transform has bins to give them.
        let bars = bars.clamp(1, mid_size / 2 + 1);

        let mut planner = RealFftPlanner::<f64>::new();
        let mut this = Self {
            bars,
            rate,
            bass: Plan::new(&mut planner, bass_size),
            mid: Plan::new(&mut planner, mid_size),
            input: vec![0.0; bass_size],
            lower_cut: vec![0; bars + 1],
            upper_cut: vec![0; bars + 1],
            eq: vec![0.0; bars + 1],
            bass_cut_off_bar: 0,
            fall: vec![0.0; bars],
            mem: vec![0.0; bars],
            peak: vec![0.0; bars],
            prev: vec![0.0; bars],
            out: vec![0.0; bars],
            noise_reduction: smoothing.clamp(0.05, 0.99),
            sens: 1.0,
            sens_init: true,
            framerate: 75.0,
            frame_skip: 1,
        };
        this.plan_bands();
        this
    }

    pub fn bars(&self) -> usize {
        self.bars
    }

    /// Rebuild for a new bar count or sample rate, keeping nothing.
    ///
    /// The band plan and every smoothing accumulator are indexed by bar, so
    /// there is nothing meaningful to carry across a change of either.
    pub fn reconfigure(&mut self, bars: usize, rate: u32) {
        if bars == self.bars && rate == self.rate {
            return;
        }
        *self = Self::with_smoothing(bars, rate, self.noise_reduction);
    }

    /// Distribute the bars across the frequency range and build the EQ.
    ///
    /// A direct port of the corresponding half of `cava_init`. The fiddly part
    /// is that log-spaced bars clump into the same FFT bin down in the bass, so
    /// cava walks upward pushing each bar past the previous one where there is
    /// a bin free -- which is why this cannot be written as a simple map over
    /// band edges.
    fn plan_bands(&mut self) {
        let bars = self.bars;
        let bass_half = (self.bass.size / 2) as f64;
        let mid_half = (self.mid.size / 2) as f64;
        let nyquist = self.rate as f64 / 2.0;
        let min_bandwidth = self.rate as f64 / self.bass.size as f64;

        let frequency_constant =
            (LOW_CUT_OFF / HIGH_CUT_OFF).log10() / (1.0 / (bars as f64 + 1.0) - 1.0);

        let mut cut_off_frequency = vec![0.0f64; bars + 1];
        let mut relative = vec![0.0f64; bars + 1];
        let mut first_bar = true;
        self.bass_cut_off_bar = 0;

        for n in 0..=bars {
            let coefficient =
                -frequency_constant + (n as f64 + 1.0) / (bars as f64 + 1.0) * frequency_constant;
            cut_off_frequency[n] = HIGH_CUT_OFF * 10f64.powf(coefficient);

            if n > 0 && cut_off_frequency[n - 1] >= cut_off_frequency[n] {
                cut_off_frequency[n] = cut_off_frequency[n - 1] + min_bandwidth;
            }
            relative[n] = cut_off_frequency[n] / nyquist;

            if cut_off_frequency[n] < BASS_CUT_OFF {
                self.lower_cut[n] = ((relative[n] * bass_half) as usize).min(self.bass.size / 2);
                self.bass_cut_off_bar += 1;
                if self.bass_cut_off_bar > 1 {
                    first_bar = false;
                }
            } else {
                self.lower_cut[n] =
                    ((relative[n] * mid_half).ceil() as usize).min(self.mid.size / 2);
                if n == self.bass_cut_off_bar {
                    first_bar = true;
                    if n > 0 {
                        self.upper_cut[n - 1] =
                            ((relative[n] * bass_half) as usize).saturating_sub(1);
                    }
                } else {
                    first_bar = false;
                }
            }

            if n > 0 {
                if !first_bar {
                    self.upper_cut[n - 1] = self.lower_cut[n].saturating_sub(1);

                    // Bars clumped into one bin: push this one up if the
                    // transform has a bin to spare.
                    if self.lower_cut[n] <= self.lower_cut[n - 1] {
                        let ceiling = if n < self.bass_cut_off_bar {
                            self.bass.size / 2 + 1
                        } else {
                            self.mid.size / 2 + 1
                        };
                        if self.lower_cut[n - 1] + 1 < ceiling {
                            self.lower_cut[n] = self.lower_cut[n - 1] + 1;
                            self.upper_cut[n - 1] = self.lower_cut[n] - 1;
                        }
                    }
                } else if self.upper_cut[n - 1] < self.lower_cut[n - 1] {
                    self.upper_cut[n - 1] = self.lower_cut[n - 1] + 1;
                }
            }

            // The cut-off the bar actually landed on, once rounded to a bin.
            relative[n] = if n < self.bass_cut_off_bar {
                self.lower_cut[n] as f64 / bass_half
            } else {
                self.lower_cut[n] as f64 / mid_half
            };
            cut_off_frequency[n] = relative[n] * nyquist;
        }

        // cava's hard-coded EQ. The FFT magnitudes are enormous, so most of
        // this is bringing them into a 0..1 range; the frequency term tilts the
        // response up so treble is not permanently dwarfed by bass.
        for n in 0..bars {
            let mut eq = 1.0 / 2f64.powi(28);
            eq *= cut_off_frequency[n + 1].powf(0.85);
            eq /= if n < self.bass_cut_off_bar {
                (self.bass.size as f64).log2()
            } else {
                (self.mid.size as f64).log2()
            };
            eq /= (self.upper_cut[n].saturating_sub(self.lower_cut[n]) + 1) as f64;
            self.eq[n] = eq;
        }
    }

    /// Feed new mono samples and advance one frame.
    ///
    /// `new` is the newest audio, oldest first. Passing an empty slice steps
    /// the smoothing without new input, which is what cava does when the
    /// display runs faster than audio arrives.
    pub fn execute(&mut self, new: &[f64]) {
        let mut silence = true;

        if !new.is_empty() {
            let n = new.len().min(self.input.len());

            // Approximate the call rate, so gravity and the integrator stay
            // put when the frame rate changes.
            self.framerate -= self.framerate / 64.0;
            self.framerate += (self.rate as f64 * self.frame_skip as f64) / n as f64 / 64.0;
            self.frame_skip = 1;

            self.input.rotate_right(n);
            for (i, &s) in new[new.len() - n..].iter().enumerate() {
                // Newest first, as cava stores it.
                self.input[n - i - 1] = s;
                if s != 0.0 {
                    silence = false;
                }
            }
        } else {
            self.frame_skip += 1;
        }

        let bass_len = self.bass.size;
        let mid_len = self.mid.size;
        self.bass.run(&self.input[..bass_len]);
        self.mid.run(&self.input[..mid_len]);

        // Sum the magnitudes in each bar's bins, then apply the EQ.
        let mut raw = vec![0.0f64; self.bars];
        // The index addresses four parallel arrays at once; iterating one of
        // them would only move the indexing somewhere less obvious.
        #[allow(clippy::needless_range_loop)]
        for n in 0..self.bars {
            let mut total = 0.0;
            for bin in self.lower_cut[n]..=self.upper_cut[n] {
                total += if n < self.bass_cut_off_bar {
                    self.bass.magnitude(bin)
                } else {
                    self.mid.magnitude(bin)
                };
            }
            raw[n] = total * self.eq[n] * self.sens;
        }

        let framerate_mod = 66.0 / self.framerate;
        let gravity_mod = framerate_mod.powf(2.5) * 2.0 / self.noise_reduction;
        let integral_mod = framerate_mod.powf(0.1);
        let mut overshoot = false;

        #[allow(clippy::needless_range_loop)]
        for n in 0..self.bars {
            // Falloff: a bar under its previous value falls away from its own
            // peak under acceleration, rather than decaying towards zero.
            if raw[n] < self.prev[n] {
                raw[n] = self.peak[n] * (1.0 - self.fall[n] * self.fall[n] * gravity_mod);
                if raw[n] < 0.0 {
                    raw[n] = 0.0;
                }
                self.fall[n] += 0.028;
            } else {
                self.peak[n] = raw[n];
                self.fall[n] = 0.0;
            }
            self.prev[n] = raw[n];

            // Integral: a leaky accumulator, which is what makes a rise glide
            // rather than jump.
            raw[n] += self.mem[n] * self.noise_reduction / integral_mod;
            self.mem[n] = raw[n];

            if raw[n] > 1.0 {
                overshoot = true;
                raw[n] = 1.0;
            }
        }

        monstercat_filter(&mut raw, MONSTERCAT);

        for (o, r) in self.out.iter_mut().zip(&raw) {
            *o = *r as f32;
        }

        // Autosens: back off hard when anything clips, creep up otherwise, so
        // the display fits the material without a gain setting.
        if overshoot {
            self.sens *= 1.0 - 0.02 * framerate_mod;
            self.sens_init = false;
        } else if !silence {
            self.sens *= 1.0 + 0.001 * framerate_mod;
            if self.sens_init {
                self.sens *= 1.0 + 0.1 * framerate_mod;
            }
        }
    }

    pub fn bands(&self) -> &[f32] {
        &self.out
    }
}

/// cava's monstercat filter: spread each bar's level into its neighbours,
/// falling off geometrically with distance.
///
/// Ported from `monstercat_filter` in cava's `cava.c`.
fn monstercat_filter(bars: &mut [f64], monstercat: f64) {
    if monstercat <= 0.0 || bars.len() < 2 {
        return;
    }
    let base = monstercat * 1.5;
    for z in 0..bars.len() {
        let here = bars[z];
        for m in (0..z).rev() {
            let de = (z - m) as i32;
            let spread = here / base.powi(de);
            if spread <= bars[m] {
                // Falls off with distance, so once it stops winning here it
                // cannot win further out either.
                break;
            }
            bars[m] = spread;
        }
        #[allow(clippy::needless_range_loop)]
        for m in z + 1..bars.len() {
            let de = (m - z) as i32;
            let spread = here / base.powi(de);
            if spread <= bars[m] {
                break;
            }
            bars[m] = spread;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin() * 0.5)
            .collect()
    }

    /// Run enough frames for autosens to settle.
    fn settle(c: &mut Cava, samples: &[f64], frames: usize) {
        let chunk = 1024;
        for f in 0..frames {
            let start = (f * chunk) % (samples.len() - chunk);
            c.execute(&samples[start..start + chunk]);
        }
    }

    #[test]
    fn silence_stays_dark() {
        let mut c = Cava::new(32, 44_100);
        settle(&mut c, &vec![0.0; 8192], 60);
        assert!(
            c.bands().iter().all(|b| *b < 0.02),
            "silence lit the display: {:?}",
            c.bands()
        );
    }

    #[test]
    fn a_tone_lights_the_bars_around_its_frequency() {
        let rate = 44_100;
        let mut c = Cava::new(32, rate);
        settle(&mut c, &tone(1000.0, rate, 44_100), 120);

        let bands = c.bands();
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        // 50 Hz to 8 kHz over 32 bars: 1 kHz sits a little past the middle.
        assert!(
            (14..=24).contains(&loudest),
            "1 kHz lit bar {loudest} of 32: {bands:?}"
        );
    }

    #[test]
    fn a_low_tone_lights_a_lower_bar_than_a_high_one() {
        let rate = 44_100;
        let peak = |freq: f64| {
            let mut c = Cava::new(32, rate);
            settle(&mut c, &tone(freq, rate, 44_100), 120);
            c.bands()
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0
        };
        assert!(peak(120.0) < peak(4000.0), "the spectrum runs backwards");
    }

    #[test]
    fn autosens_brings_a_quiet_signal_up() {
        // The point of autosens: level, not absolute amplitude, decides how
        // full the display is. A signal 20 dB down should still reach it.
        let rate = 44_100;
        let loud = tone(1000.0, rate, 44_100);
        let quiet: Vec<f64> = loud.iter().map(|s| s * 0.1).collect();

        let top = |src: &[f64]| {
            let mut c = Cava::new(32, rate);
            settle(&mut c, src, 400);
            c.bands().iter().cloned().fold(0.0f32, f32::max)
        };
        let q = top(&quiet);
        assert!(q > 0.3, "a quiet tone never rose: {q}");
    }

    #[test]
    fn values_stay_in_range() {
        let rate = 44_100;
        let mut c = Cava::new(48, rate);
        settle(&mut c, &tone(200.0, rate, 44_100), 200);
        for (i, b) in c.bands().iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(b) && b.is_finite(),
                "bar {i} is {b}, outside 0..1"
            );
        }
    }

    #[test]
    fn the_monstercat_filter_spreads_a_spike_into_its_neighbours() {
        let mut bars = vec![0.0; 9];
        bars[4] = 1.0;
        monstercat_filter(&mut bars, MONSTERCAT);

        assert_eq!(bars[4], 1.0, "the spike itself must not move");
        for d in 1..5 {
            assert!(
                bars[4 - d] > 0.0 && bars[4 + d] > 0.0,
                "nothing reached distance {d}: {bars:?}"
            );
            assert!(
                bars[4 - d] < bars[4 - d + 1],
                "the skirt should fall off with distance: {bars:?}"
            );
        }
        assert!(
            (bars[3] - bars[5]).abs() < 1e-12,
            "the skirt is lopsided: {bars:?}"
        );
    }

    #[test]
    fn the_filter_never_lowers_a_bar() {
        let mut bars = vec![0.9, 0.1, 0.8, 0.2, 0.7];
        let before = bars.clone();
        monstercat_filter(&mut bars, MONSTERCAT);
        for (i, (a, b)) in before.iter().zip(&bars).enumerate() {
            assert!(b >= a, "bar {i} fell from {a} to {b}");
        }
    }

    #[test]
    fn bar_count_is_capped_to_what_the_transform_can_resolve() {
        // 8 kHz uses a 512-point window, so it cannot give 4000 bars.
        let c = Cava::new(4000, 8_000);
        assert!(c.bars() <= c.mid.size / 2 + 1);
        assert!(c.bars() > 0);
    }

    #[test]
    fn reconfiguring_to_the_same_shape_is_a_no_op() {
        let mut c = Cava::new(32, 44_100);
        settle(&mut c, &tone(1000.0, 44_100, 44_100), 60);
        let before: Vec<f32> = c.bands().to_vec();
        c.reconfigure(32, 44_100);
        assert_eq!(c.bands(), &before[..], "state was thrown away needlessly");
        c.reconfigure(64, 44_100);
        assert_eq!(c.bars(), 64);
    }
}

#[cfg(test)]
mod latency_probe {
    use super::*;

    /// Not an assertion -- run with `--nocapture` to see the response time.
    #[test]
    fn print_step_response() {
        let rate = 44_100u32;
        // The app feeds one frame's worth of new audio per tick.
        for (fps, nr) in [(30.0f64, 0.77), (30.0, DEFAULT_SMOOTHING)] {
            let chunk = (rate as f64 / fps) as usize;
            let mut c = Cava::with_smoothing(64, rate, nr);
            let tone: Vec<f64> = (0..rate as usize)
                .map(|i| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate as f64).sin() * 0.5)
                .collect();

            // Settle on the tone so autosens is calibrated.
            for f in 0..400 {
                let s = (f * chunk) % (tone.len() - chunk);
                c.execute(&tone[s..s + chunk]);
            }
            let steady = c.bands().iter().cloned().fold(0.0f32, f32::max);

            // Silence, then measure how long the display takes to fall.
            let silence = vec![0.0; chunk];
            let mut fall = None;
            for f in 1..200 {
                c.execute(&silence);
                if c.bands().iter().cloned().fold(0.0f32, f32::max) < steady * 0.1 {
                    fall = Some(f);
                    break;
                }
            }

            // Then back to the tone, and measure the rise.
            let mut rise = None;
            for f in 1..200 {
                let s = (f * chunk) % (tone.len() - chunk);
                c.execute(&tone[s..s + chunk]);
                if c.bands().iter().cloned().fold(0.0f32, f32::max) > steady * 0.9 {
                    rise = Some(f);
                    break;
                }
            }
            let ms = |n: Option<usize>| match n {
                Some(f) => format!("{f} frames / {:.0} ms", f as f64 * 1000.0 / fps),
                None => "never".into(),
            };
            println!(
                "{fps:.0} fps  nr {nr:.2}  steady {steady:.2}  fall {}  rise {}",
                ms(fall),
                ms(rise)
            );
        }
    }
}
