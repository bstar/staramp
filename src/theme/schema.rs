//! Theme file format.
//!
//! Every colour role is optional. A minimal theme is a handful of lines and the
//! rest is derived; a maximal one specifies everything. That is the opposite of
//! the reference implementation, whose seven colours cannot express "selected
//! row" and "playing row" as different things, and where one `accent` value
//! drives the title, the selection, the seek bar and the key pills at once.

use serde::{Deserialize, Serialize};

use super::color::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub variant: Variant,
    /// Inherit from another theme, then override.
    #[serde(default)]
    pub extends: Option<String>,
    /// Where an imported theme came from, e.g. a `.wsz` filename.
    #[serde(default)]
    pub source: Option<String>,
}

/// A base16 scheme, which can stand in for the whole palette.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct Base16 {
    pub base00: Rgb,
    pub base01: Rgb,
    pub base02: Rgb,
    pub base03: Rgb,
    pub base04: Rgb,
    pub base05: Rgb,
    pub base06: Rgb,
    pub base07: Rgb,
    pub base08: Rgb,
    pub base09: Rgb,
    pub base0A: Rgb,
    pub base0B: Rgb,
    pub base0C: Rgb,
    pub base0D: Rgb,
    pub base0E: Rgb,
    pub base0F: Rgb,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppColors {
    pub bg: Option<Rgb>,
    pub fg: Option<Rgb>,
    pub dim: Option<Rgb>,
    pub accent: Option<Rgb>,
    pub ok: Option<Rgb>,
    pub warn: Option<Rgb>,
    pub error: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChromeColors {
    pub titlebar_active_fg: Option<Rgb>,
    pub titlebar_active_bg: Option<Rgb>,
    pub titlebar_inactive_fg: Option<Rgb>,
    pub titlebar_inactive_bg: Option<Rgb>,
    pub border: Option<Rgb>,
    pub border_focused: Option<Rgb>,
    pub divider: Option<Rgb>,
    #[serde(default)]
    pub border_style: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RowColors {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub index_fg: Option<Rgb>,
    pub duration_fg: Option<Rgb>,
    pub meta_fg: Option<Rgb>,
    pub selected_fg: Option<Rgb>,
    pub selected_bg: Option<Rgb>,
    pub cursor_fg: Option<Rgb>,
    pub cursor_bg: Option<Rgb>,
    pub playing_fg: Option<Rgb>,
    pub playing_bg: Option<Rgb>,
    pub marked_fg: Option<Rgb>,
    pub missing_fg: Option<Rgb>,
    pub virtual_fg: Option<Rgb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PanelColors {
    pub bg: Option<Rgb>,
    pub fg: Option<Rgb>,
    pub header_fg: Option<Rgb>,
    pub header_bg: Option<Rgb>,
    pub empty_fg: Option<Rgb>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusColors {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub hint_key_fg: Option<Rgb>,
    pub hint_key_bg: Option<Rgb>,
    pub hint_desc_fg: Option<Rgb>,
}

/// A theme file, before derivation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeFile {
    pub meta: Meta,
    #[serde(default)]
    pub base16: Option<Base16>,
    #[serde(default)]
    pub app: AppColors,
    #[serde(default)]
    pub chrome: ChromeColors,
    #[serde(default)]
    pub panel: PanelColors,
    #[serde(default)]
    pub row: RowColors,
    #[serde(default)]
    pub marquee: MarqueeColors,
    #[serde(default)]
    pub time: TimeColors,
    #[serde(default)]
    pub seek: SeekColors,
    #[serde(default)]
    pub volume: VolumeColors,
    #[serde(default)]
    pub eq: EqColors,
    #[serde(default)]
    pub transport: TransportColors,
    #[serde(default)]
    pub vis: VisColors,
    #[serde(default)]
    pub status: StatusColors,
}

impl ThemeFile {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}
