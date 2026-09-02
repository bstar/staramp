//! Peak indicators, on broadcast meter ballistics.
//!
//! Original to staramp. A peak-programme meter is a well-specified instrument
//! -- IEC 60268-10 gives the numbers -- and the specification is what this
//! follows: catch the peak immediately, hold it long enough to be read, then
//! fall at a constant rate *in decibels per second*.
//!
//! Constant in dB, not in screen units, is the whole point and the thing that
//! makes it look like an instrument rather than an animation. The display is a
//! dB scale, so a fixed dB/s fall is a straight line down the screen at the
//! same speed wherever it starts. A cap that fell at a fixed fraction of its
//! height per second would race down from the top and crawl at the bottom,
//! which is what a naive implementation does and why it looks wrong.
//!
//! There is no spring and no gravity here: a falling body accelerates, and the
//! standard says a meter does not.

/// How long a peak is held before it starts to fall.
///
/// IEC 60268-10's Type II programme meter holds for around 1.5 s. Long enough
/// to read across a glance, short enough that the cap tracks the music.
const HOLD_S: f32 = 1.2;

/// How fast it falls once released, in dB per second.
///
/// The standard's return time is 20 dB in 1.7 s, near enough 12 dB/s.
const FALL_DB_S: f32 = 12.0;

/// The dB range the 0-to-1 display covers, so a fall in dB can be expressed
/// in the units everything downstream actually uses.
///
/// Taken from `spectrum` rather than copied from it. It was a hand-written
/// 62.0 with a comment claiming they matched, and they stopped matching the
/// moment the ceiling was recalibrated -- at which point the caps were
/// falling at a rate derived from a range that no longer existed, which is
/// the sort of drift a comment cannot prevent and an import can.
use crate::vis::spectrum::RANGE_DB;

/// The bars, and a peak indicator over each.
#[derive(Default)]
pub struct Meters {
    bar: Vec<f32>,
    peak: Vec<f32>,
    /// Seconds left to hold, per band.
    hold: Vec<f32>,
}

impl Meters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bars(&self) -> &[f32] {
        &self.bar
    }

    pub fn peaks(&self) -> &[f32] {
        &self.peak
    }

    /// Whether an indicator is far enough above its bar to be worth drawing
    /// is decided by the panel, not here: it resamples the bands to its own
    /// column count first, so the comparison has to happen after that on the
    /// values actually being drawn.
    ///
    /// Take a fresh set of levels and advance the indicators by `dt` seconds.
    ///
    /// The bars are passed through unchanged: they are already smoothed by
    /// `spectrum`'s envelope followers, and smoothing them twice would just
    /// make them late.
    pub fn update(&mut self, levels: &[f32], dt: f32) {
        if self.bar.len() != levels.len() {
            self.bar = vec![0.0; levels.len()];
            self.peak = vec![0.0; levels.len()];
            self.hold = vec![0.0; levels.len()];
        }
        let dt = dt.clamp(0.0, 0.25);
        // A constant rate down the dB scale, in display units.
        let fall = FALL_DB_S / RANGE_DB * dt;

        for (((bar, peak), hold), src) in self
            .bar
            .iter_mut()
            .zip(self.peak.iter_mut())
            .zip(self.hold.iter_mut())
            .zip(levels)
        {
            let level = src.clamp(0.0, 1.0);
            *bar = level;

            if level >= *peak {
                // A new peak: catch it exactly, and start the clock again.
                *peak = level;
                *hold = HOLD_S;
            } else if *hold > 0.0 {
                *hold -= dt;
            } else {
                *peak = (*peak - fall).max(level);
            }
        }
    }
}

/// Quarter-cell cap glyphs, top of the cell to the bottom.
///
/// Four box-drawing horizontals at different heights within the cell, which is
/// what gives the indicator four times the vertical resolution of the bar it
/// sits above.
pub const CAP_GLYPHS: [char; 4] = ['⎺', '⎻', '⎼', '⎽'];

/// Which row a cap sits on, and which quarter-cell glyph to use.
pub fn cap_position(level: f32, height: u16) -> (u16, char) {
    let dot_rows = (height.max(1) as usize) * 4;
    let dot_y = ((1.0 - level.clamp(0.0, 1.0)) * (dot_rows - 1) as f32).round() as usize;
    ((dot_y / 4) as u16, CAP_GLYPHS[dot_y % 4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peak_is_caught_exactly_and_held() {
        let mut m = Meters::new();
        m.update(&[0.9], 1.0 / 60.0);
        assert_eq!(m.peaks()[0], 0.9, "the indicator catches the peak at once");

        // Through the hold, with the bar gone, it does not move.
        for _ in 0..60 {
            m.update(&[0.0], 1.0 / 60.0);
        }
        assert!(m.peaks()[0] > 0.88, "still held: {}", m.peaks()[0]);
    }

    #[test]
    fn it_falls_at_a_constant_rate_in_decibels() {
        let mut m = Meters::new();
        m.update(&[1.0], 1.0 / 60.0);
        // Past the hold.
        for _ in 0..90 {
            m.update(&[0.0], 1.0 / 60.0);
        }
        let a = m.peaks()[0];
        for _ in 0..30 {
            m.update(&[0.0], 1.0 / 60.0);
        }
        let b = m.peaks()[0];
        for _ in 0..30 {
            m.update(&[0.0], 1.0 / 60.0);
        }
        let c = m.peaks()[0];
        // Equal time, equal distance -- which is what "constant rate" means,
        // and what a spring or a gravity model would not do.
        let (first, second) = (a - b, b - c);
        assert!(
            (first - second).abs() < 0.02,
            "fall is not linear: {first} then {second}"
        );
    }

    #[test]
    fn the_indicator_never_sits_below_its_bar() {
        let mut m = Meters::new();
        for _ in 0..200 {
            m.update(&[0.5], 1.0 / 60.0);
        }
        assert!(m.peaks()[0] >= m.bars()[0] - 1e-6);
    }

    #[test]
    fn the_fall_is_frame_rate_independent() {
        let step = |dt: f32, steps: usize| {
            let mut m = Meters::new();
            m.update(&[1.0], dt);
            for _ in 0..steps {
                m.update(&[0.0], dt);
            }
            m.peaks()[0]
        };
        // Two seconds of falling, at 30 and at 120 frames a second.
        let slow = step(1.0 / 30.0, 60);
        let fast = step(1.0 / 120.0, 240);
        assert!((slow - fast).abs() < 0.01, "{slow} vs {fast}");
    }

    #[test]
    fn cap_glyphs_span_the_cell_top_to_bottom() {
        let (row_top, g_top) = cap_position(1.0, 8);
        let (row_bot, _) = cap_position(0.0, 8);
        assert_eq!(row_top, 0, "full level is the top row");
        assert_eq!(g_top, CAP_GLYPHS[0]);
        assert_eq!(row_bot, 7, "silence is the bottom row");
    }
}
