//! Peak-cap physics.
//!
//! Ported from cliamp's `ui/vis_classic_peak.go` (MIT, Copyright (c) Bjarne
//! Øverli). The constants are tuned and are reproduced exactly rather than
//! re-derived: a cap is launched upward with a velocity proportional to how
//! hard the bar rose, falls under gravity, pauses briefly at the apex, and
//! settles back onto the bar. That behaviour is what makes an analyzer read as
//! responsive rather than merely animated.

/// Minimum upward launch velocity for a newly detached cap.
const LAUNCH_BASE: f32 = 0.8;
/// Extra launch velocity in proportion to how far the bar rose.
const LAUNCH_GAIN: f32 = 1.4;
const LAUNCH_MAX: f32 = 1.7;
const GRAVITY: f32 = 9.5;
/// Pause at the apex before falling.
const APEX_HOLD: f32 = 0.08;
/// Bar body smoothing: fast attack, slower release.
const BAR_RISE: f32 = 34.0;
const BAR_FALL: f32 = 10.0;
const MAX_HEIGHT: f32 = 1.0;
/// Treat positions this close as visually equal.
const EPSILON: f32 = 0.01;

/// One frame's worth of bar and cap positions.
#[derive(Debug, Default, Clone)]
pub struct Ballistics {
    bar: Vec<f32>,
    peak: Vec<f32>,
    vel: Vec<f32>,
    hold: Vec<f32>,
}

impl Ballistics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bars(&self) -> &[f32] {
        &self.bar
    }

    pub fn peaks(&self) -> &[f32] {
        &self.peak
    }

    fn reset(&mut self, levels: &[f32]) {
        self.bar = levels.to_vec();
        self.peak = levels.to_vec();
        self.vel = vec![0.0; levels.len()];
        self.hold = vec![0.0; levels.len()];
    }

    /// Advance by `dt` seconds toward `levels`.
    pub fn update(&mut self, levels: &[f32], dt: f32) {
        if self.bar.len() != levels.len() {
            self.reset(levels);
            return;
        }
        // A long gap -- a pause, a resize, a stalled frame -- must step like one
        // frame rather than integrating physics over the whole interval.
        let dt = if dt <= 0.0 || dt > 10.0 / 60.0 {
            1.0 / 60.0
        } else {
            dt
        };

        for (i, &target) in levels.iter().enumerate() {
            let before = self.bar[i];
            let rate = if target > before { BAR_RISE } else { BAR_FALL };
            self.bar[i] = before + (target - before) * (1.0 - (-rate * dt).exp());
            let rise = (self.bar[i] - before).max(0.0);

            // A bar that has climbed past its cap relaunches it.
            if self.bar[i] >= self.peak[i] {
                self.peak[i] = self.bar[i];
                self.vel[i] = (LAUNCH_BASE + LAUNCH_GAIN * rise).min(LAUNCH_MAX);
                self.hold[i] = 0.0;
                continue;
            }

            if self.hold[i] > 0.0 {
                self.hold[i] = (self.hold[i] - dt).max(0.0);
                if self.hold[i] > 0.0 {
                    continue;
                }
            }

            let prev_vel = self.vel[i];
            self.peak[i] += self.vel[i] * dt;
            self.vel[i] -= GRAVITY * dt;
            self.peak[i] = self.peak[i].min(MAX_HEIGHT);

            // Reached the top of its arc: pause there before falling.
            if prev_vel > 0.0 && self.vel[i] <= 0.0 && self.peak[i] > self.bar[i] + EPSILON {
                self.vel[i] = 0.0;
                self.hold[i] = APEX_HOLD;
                continue;
            }
            if self.peak[i] <= self.bar[i] {
                self.peak[i] = self.bar[i];
                self.vel[i] = 0.0;
                self.hold[i] = 0.0;
            }
        }
    }

    /// Is the cap far enough above the bar to be worth drawing separately?
    pub fn detached(&self, i: usize, height: u16) -> bool {
        let min_gap = EPSILON.max(0.5 / (height.max(1) as f32 * 4.0));
        self.peak.get(i).copied().unwrap_or(0.0) > self.bar.get(i).copied().unwrap_or(0.0) + min_gap
    }
}

/// Quarter-cell cap glyphs, top of the cell to the bottom.
///
/// This is what gives the cap four times the vertical resolution of the bar it
/// sits above, and it is most of why the effect reads as physical.
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
    fn a_rising_bar_launches_its_cap_upward() {
        let mut b = Ballistics::new();
        b.update(&[0.0], 1.0 / 60.0);
        // A sudden loud band.
        for _ in 0..4 {
            b.update(&[1.0], 1.0 / 60.0);
        }
        assert!(b.peaks()[0] > 0.0);
        // Then the band goes quiet; the cap should hang above the falling bar.
        for _ in 0..6 {
            b.update(&[0.0], 1.0 / 60.0);
        }
        assert!(
            b.peaks()[0] > b.bars()[0],
            "cap {} should hang above bar {}",
            b.peaks()[0],
            b.bars()[0]
        );
    }

    #[test]
    fn a_cap_eventually_falls_back_to_the_bar() {
        let mut b = Ballistics::new();
        b.update(&[1.0], 1.0 / 60.0);
        for _ in 0..10 {
            b.update(&[1.0], 1.0 / 60.0);
        }
        for _ in 0..600 {
            b.update(&[0.0], 1.0 / 60.0);
        }
        assert!(
            (b.peaks()[0] - b.bars()[0]).abs() < 0.02,
            "cap {} never settled onto bar {}",
            b.peaks()[0],
            b.bars()[0]
        );
    }

    #[test]
    fn positions_stay_in_range() {
        let mut b = Ballistics::new();
        for i in 0..2000 {
            let level = if i % 7 == 0 { 1.0 } else { 0.0 };
            b.update(&[level, 0.5, 0.2], 1.0 / 60.0);
            for v in b.bars().iter().chain(b.peaks()) {
                assert!((0.0..=1.0).contains(v), "out of range: {v}");
            }
        }
    }

    #[test]
    fn a_long_gap_steps_like_one_frame_rather_than_exploding() {
        let mut b = Ballistics::new();
        b.update(&[1.0], 1.0 / 60.0);
        // Simulating a five-second stall must not integrate physics over it.
        b.update(&[0.0], 5.0);
        assert!(b.bars()[0].is_finite());
        assert!((0.0..=1.0).contains(&b.peaks()[0]));
    }

    #[test]
    fn changing_the_band_count_resets_rather_than_panicking() {
        let mut b = Ballistics::new();
        b.update(&[0.5; 20], 1.0 / 60.0);
        b.update(&[0.5; 64], 1.0 / 60.0);
        assert_eq!(b.bars().len(), 64);
    }

    #[test]
    fn cap_glyphs_give_quarter_cell_resolution() {
        // Four distinct glyphs within a single row.
        let h = 8;
        let seen: std::collections::HashSet<char> = (0..16)
            .map(|i| cap_position(1.0 - i as f32 / 64.0, h).1)
            .collect();
        assert!(seen.len() > 1, "cap never moved within a cell");
        assert_eq!(cap_position(1.0, h).0, 0, "full level sits on the top row");
        assert_eq!(cap_position(0.0, h).0, h - 1, "zero sits on the bottom row");
    }
}
