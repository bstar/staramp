//! Visualizer modes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VisMode {
    /// Narrow bars, one per column, at eighth-block resolution, with slower
    /// time constants and more neighbour spread than the others: the bars
    /// move as one surface rather than as a row of separate meters.
    Fluid,
    /// Winamp's LED analyzer: two LED rows per terminal row, coloured by row
    /// rather than by level, over a dot grid.
    Leds,
    /// Smooth fractional-block bars, coloured by how loud each band is.
    #[default]
    Bars,
    /// Bars with ballistic peak caps that hold and then fall.
    Peaks,
    /// Braille stipple, for a dotted texture.
    Dots,
    /// Braille oscilloscope: the waveform itself as a continuous trace.
    Wave,
    /// The waveform as scattered dots, shaded by distance from the centre.
    Scope,
    Off,
}

impl VisMode {
    pub fn name(self) -> &'static str {
        match self {
            VisMode::Fluid => "fluid",
            VisMode::Leds => "leds",
            VisMode::Bars => "bars",
            VisMode::Peaks => "peaks",
            VisMode::Dots => "dots",
            VisMode::Wave => "wave",
            VisMode::Scope => "scope",
            VisMode::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "fluid" => VisMode::Fluid,
            // What this mode used to be called, so an existing config loads.
            "cava" => VisMode::Fluid,
            "leds" | "led" | "classic" | "viscolor" => VisMode::Leds,
            "bars" | "bar" => VisMode::Bars,
            "peaks" | "peak" => VisMode::Peaks,
            "dots" | "dot" | "braille" => VisMode::Dots,
            "wave" | "osc" | "oscilloscope" => VisMode::Wave,
            "scope" => VisMode::Scope,
            "off" | "none" => VisMode::Off,
            _ => return None,
        })
    }

    /// Cycle order, `off` last so `w` reaches every mode before disabling.
    pub fn all() -> &'static [VisMode] {
        &[
            VisMode::Fluid,
            VisMode::Leds,
            VisMode::Bars,
            VisMode::Peaks,
            VisMode::Dots,
            VisMode::Wave,
            VisMode::Scope,
            VisMode::Off,
        ]
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|&m| m == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    /// Does this mode draw the raw waveform rather than a spectrum?
    pub fn needs_waveform(self) -> bool {
        matches!(self, VisMode::Wave | VisMode::Scope)
    }

    /// Does this mode read the slow-smoothed analysis rather than the shared
    /// one? Only the fluid mode does; it also sets its own band count from
    /// the panel width.
    ///
    /// The two pipelines differ all the way down -- window sizes, band
    /// distribution, smoothing -- so only one of them runs per frame.
    pub fn uses_fluid(self) -> bool {
        matches!(self, VisMode::Fluid)
    }

    pub fn prev(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|&m| m == self).unwrap_or(0);
        all[(i + all.len() - 1) % all.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for m in VisMode::all() {
            assert_eq!(VisMode::parse(m.name()), Some(*m));
        }
        assert_eq!(VisMode::parse("nonsense"), None);
    }

    #[test]
    fn the_default_is_bars() {
        assert_eq!(VisMode::default(), VisMode::Bars);
    }

    #[test]
    fn the_trace_modes_are_distinct_from_the_spectrum_ones() {
        for m in [VisMode::Wave, VisMode::Scope] {
            assert!(m.needs_waveform(), "{} needs the raw samples", m.name());
        }
        for m in VisMode::all()
            .iter()
            .filter(|m| !matches!(m, VisMode::Wave | VisMode::Scope))
        {
            assert!(!m.needs_waveform(), "{} reads the spectrum", m.name());
        }
    }

    #[test]
    fn aliases_resolve() {
        assert_eq!(VisMode::parse("viscolor"), Some(VisMode::Leds));
        assert_eq!(VisMode::parse("none"), Some(VisMode::Off));
    }

    #[test]
    fn the_cycle_is_the_kept_modes_plus_off() {
        let names: Vec<&str> = VisMode::all().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            ["fluid", "leds", "bars", "peaks", "dots", "wave", "scope", "off"]
        );
    }

    #[test]
    fn the_fluid_mode_has_its_own_analysis_and_the_others_share_one() {
        assert!(VisMode::Fluid.uses_fluid());
        for m in VisMode::all().iter().filter(|m| **m != VisMode::Fluid) {
            assert!(
                !m.uses_fluid(),
                "{} should use the shared analyzer",
                m.name()
            );
        }
    }

    #[test]
    fn cycling_visits_every_mode_and_returns() {
        let mut m = VisMode::Fluid;
        let mut seen = vec![m];
        for _ in 0..VisMode::all().len() - 1 {
            m = m.next();
            seen.push(m);
        }
        seen.sort_by_key(|m| m.name());
        seen.dedup();
        assert_eq!(seen.len(), VisMode::all().len());
        assert_eq!(m.next(), VisMode::Fluid, "wraps back to the start");
    }

    #[test]
    fn prev_is_the_inverse_of_next() {
        for m in VisMode::all() {
            assert_eq!(m.next().prev(), *m);
        }
    }

    #[test]
    fn off_is_last_so_cycling_reaches_every_mode_first() {
        assert_eq!(*VisMode::all().last().unwrap(), VisMode::Off);
    }
}
