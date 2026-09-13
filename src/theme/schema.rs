//! The tables that mean something to a music player.
//!
//! The format itself is starkit's, and so are the tables every application
//! has: the palette, the chrome around a panel, the rows inside one, and the
//! status line. What is left is what only a player has -- a marquee, a time readout, a seek
//! bar, a volume slider, an equaliser, transport buttons and an analyzer --
//! and those are read out of the theme file's unknown-table bag on demand.
//! A theme file that carries them is still a perfectly good theme for anything
//! else that reads it.

use serde::{Deserialize, Serialize};

use super::color::Rgb;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeColors {
    pub digit_fg: Option<Rgb>,
    pub digit_dim_fg: Option<Rgb>,
    pub colon_fg: Option<Rgb>,
    pub remaining_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeekColors {
    pub track_fg: Option<Rgb>,
    pub filled_fg: Option<Rgb>,
    pub thumb_fg: Option<Rgb>,
    pub label_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VolumeColors {
    pub track_fg: Option<Rgb>,
    pub filled_fg: Option<Rgb>,
    pub thumb_fg: Option<Rgb>,
    pub mute_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EqColors {
    pub slider_track: Option<Rgb>,
    pub slider_thumb: Option<Rgb>,
    pub slider_fill_pos: Option<Rgb>,
    pub slider_fill_neg: Option<Rgb>,
    pub zero_line: Option<Rgb>,
    pub band_label: Option<Rgb>,
    pub band_value: Option<Rgb>,
    pub band_focused: Option<Rgb>,
    pub preamp_fg: Option<Rgb>,
    pub enabled_fg: Option<Rgb>,
    pub disabled_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransportColors {
    pub button_bg: Option<Rgb>,
    pub button_active_bg: Option<Rgb>,
    pub button_fg: Option<Rgb>,
    pub button_active_fg: Option<Rgb>,
    pub button_disabled_fg: Option<Rgb>,
    pub toggle_on_fg: Option<Rgb>,
    pub toggle_off_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarqueeColors {
    pub fg: Option<Rgb>,
    pub paused_fg: Option<Rgb>,
    pub stopped_fg: Option<Rgb>,
}

/// The visualizer palette. This is the VISCOLOR equivalent, and the reason the
/// analyzer can look like Winamp's rather than like a three-colour bar chart.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VisColors {
    pub bg: Option<Rgb>,
    pub grid_fg: Option<Rgb>,
    pub peak_fg: Option<Rgb>,
    /// Sixteen stops, bottom (quiet) to top (loud). Winamp's VISCOLOR.TXT
    /// indices 17 down to 2.
    #[serde(default)]
    pub ramp: Option<Vec<Rgb>>,
    /// Oscilloscope shades, brightest first.
    #[serde(default)]
    pub osc: Option<Vec<Rgb>>,
    #[serde(default)]
    pub grid: Option<String>,
}
