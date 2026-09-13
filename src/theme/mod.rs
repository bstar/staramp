//! The player's own colours, derived from the same file as everything else.
//!
//! The format, the derivation chain for the shared roles, the registry and the
//! importers are starkit's. What is here is the sixty-odd roles only a music
//! player has -- the marquee, the time readout, the seek bar, the volume
//! slider, the equaliser, the transport buttons and the analyzer -- derived
//! from the palette the core already resolved, so a theme dresses the analyzer
//! and the playlist in colours that agree with each other.
//!
//! [`Theme`] derefs to the core one, so `theme.bg` and `theme.vis_ramp` are
//! both ordinary field reads and none of the seventy-odd call sites had to
//! learn where the boundary is.

pub mod builtin;
pub mod schema;

use std::ops::Deref;

use starkit::theme::color::{ramp, Rgb};
use starkit::theme::{pick, Resolve, ThemeFile, WHITE};

// `crate::theme::color::Rgb` is spelled that way in forty-odd places, and
// `safe_id`, the base16 reader and the desktop detection are called from
// `main`. Re-exported rather than renamed: where they live is not something
// the call sites should have to know.
pub use starkit::theme::{base16, color, safe_id, system};
// Winamp skin import is behind a feature there, and this is the application
// that turns it on.
pub use starkit::theme::wsz;

use schema::{
    EqColors, MarqueeColors, SeekColors, TimeColors, TransportColors, VisColors, VolumeColors,
};

/// Every colour the player can ask for, all concrete.
#[derive(Debug, Clone)]
pub struct Theme {
    core: starkit::theme::Theme,

    pub marquee_fg: Rgb,
    pub marquee_paused_fg: Rgb,
    pub marquee_stopped_fg: Rgb,

    pub time_digit_fg: Rgb,
    pub time_digit_dim_fg: Option<Rgb>,
    pub time_colon_fg: Rgb,
    pub time_remaining_fg: Rgb,

    pub seek_track_fg: Rgb,
    pub seek_filled_fg: Rgb,
    pub seek_thumb_fg: Rgb,
    pub seek_label_fg: Rgb,

    pub volume_track_fg: Rgb,
    pub volume_filled_fg: Rgb,
    pub volume_thumb_fg: Rgb,
    pub volume_mute_fg: Rgb,

    pub eq_slider_track: Rgb,
    pub eq_slider_thumb: Rgb,
    pub eq_slider_fill_pos: Rgb,
    pub eq_slider_fill_neg: Rgb,
    pub eq_zero_line: Rgb,
    pub eq_band_label: Rgb,
    pub eq_band_value: Rgb,
    pub eq_band_focused: Rgb,
    pub eq_preamp_fg: Rgb,
    pub eq_enabled_fg: Rgb,
    pub eq_disabled_fg: Rgb,

    pub transport_button_bg: Rgb,
    pub transport_button_active_bg: Rgb,
    pub transport_button_fg: Rgb,
    pub transport_button_active_fg: Rgb,
    pub transport_button_disabled_fg: Rgb,
    pub transport_toggle_on_fg: Rgb,
    pub transport_toggle_off_fg: Rgb,

    pub vis_bg: Rgb,
    pub vis_grid_fg: Rgb,
    pub vis_peak_fg: Rgb,
    pub vis_ramp: [Rgb; 16],
    pub vis_osc: [Rgb; 5],
}

impl Deref for Theme {
    type Target = starkit::theme::Theme;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl Resolve for Theme {
    fn resolve(f: &ThemeFile) -> Self {
        let core = starkit::theme::Theme::resolve(f);

        // The player's own tables, out of the bag of tables the core does not
        // read. A malformed one is treated as absent rather than fatal: the
        // rest of the theme is fine, and refusing to start over a mistyped
        // analyzer colour is not a trade anyone would make.
        let marquee: MarqueeColors = f.table("marquee").unwrap_or_default();
        let time: TimeColors = f.table("time").unwrap_or_default();
        let seek: SeekColors = f.table("seek").unwrap_or_default();
        let volume: VolumeColors = f.table("volume").unwrap_or_default();
        let eq: EqColors = f.table("eq").unwrap_or_default();
        let transport: TransportColors = f.table("transport").unwrap_or_default();
        let vis: VisColors = f.table("vis").unwrap_or_default();

        let b16 = f.base16;
        let bg = core.bg;
        let fg = core.fg;
        let dim = core.dim;
        let accent = core.accent;
        let ok = core.ok;
        let warn = core.warn;
        let error = core.error;
        let track_bg = core.track_bg();

        let vis_ramp: [Rgb; 16] = {
            let stops = vis
                .ramp
                .clone()
                .filter(|r| r.len() >= 2)
                // The theme's own accent, not a green-to-red VU. A derived
                // theme -- `system`, or any base16 scheme -- had a spectrum
                // borrowed from Winamp regardless of what the rest of it
                // looked like, which is the one panel most visible from across
                // a room. Dark in the accent's hue at the bottom, the accent
                // through the middle, the foreground at the top.
                //
                // A theme that wants the classic VU says so: winamp-classic
                // carries its sixteen steps from VISCOLOR.TXT.
                .unwrap_or_else(|| vec![bg.mix(accent, 0.45), accent, accent.mix(fg, 0.5), fg]);
            let v = if stops.len() == 16 {
                stops
            } else {
                ramp(&stops, 16)
            };
            std::array::from_fn(|i| v[i.min(v.len() - 1)])
        };

        let vis_peak = pick(vis.peak_fg, None, bg.best_contrast_against(&[fg, WHITE]));
        let vis_osc: [Rgb; 5] = {
            let v = vis
                .osc
                .clone()
                .filter(|o| o.len() >= 2)
                .unwrap_or_else(|| ramp(&[vis_peak, dim], 5));
            std::array::from_fn(|i| v[i.min(v.len() - 1)])
        };

        Theme {
            marquee_fg: pick(marquee.fg, b16.map(|b| b.base0C), accent),
            marquee_paused_fg: pick(marquee.paused_fg, None, dim),
            marquee_stopped_fg: pick(marquee.stopped_fg, None, dim.mix(bg, 0.4)),

            time_digit_fg: pick(time.digit_fg, None, accent),
            time_digit_dim_fg: time.digit_dim_fg,
            time_colon_fg: pick(time.colon_fg, None, accent),
            time_remaining_fg: pick(time.remaining_fg, None, warn),

            seek_track_fg: pick(seek.track_fg, None, track_bg),
            seek_filled_fg: pick(seek.filled_fg, None, accent),
            seek_thumb_fg: pick(seek.thumb_fg, None, bg.best_contrast_against(&[fg, WHITE])),
            seek_label_fg: pick(seek.label_fg, None, dim),

            volume_track_fg: pick(volume.track_fg, None, track_bg),
            // The accent, not the "ok" green. Volume is not a status; green
            // would be reporting something it never reports. It followed the
            // border for a while, which stopped working once the borders went
            // grey: a control has to be readable, and the frame does not.
            volume_filled_fg: pick(volume.filled_fg, None, accent),
            volume_thumb_fg: pick(
                volume.thumb_fg,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            volume_mute_fg: pick(volume.mute_fg, None, error),

            eq_slider_track: pick(eq.slider_track, None, track_bg),
            eq_slider_thumb: pick(
                eq.slider_thumb,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            eq_slider_fill_pos: pick(eq.slider_fill_pos, None, ok),
            eq_slider_fill_neg: pick(eq.slider_fill_neg, b16.map(|b| b.base09), warn),
            eq_zero_line: pick(eq.zero_line, None, bg.mix(fg, 0.30)),
            eq_band_label: pick(eq.band_label, None, dim),
            eq_band_value: pick(eq.band_value, None, accent),
            eq_band_focused: pick(
                eq.band_focused,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            eq_preamp_fg: pick(eq.preamp_fg, None, warn),
            eq_enabled_fg: pick(eq.enabled_fg, None, ok),
            eq_disabled_fg: pick(eq.disabled_fg, None, dim.mix(bg, 0.4)),

            // A face the font draws small cannot be made larger, so the
            // *button* is what carries the size: a padded plate behind the
            // glyph, which reads as a control however the glyph is drawn.
            transport_button_bg: pick(transport.button_bg, None, bg.mix(fg, 0.14)),
            transport_button_active_bg: pick(
                transport.button_active_bg,
                None,
                bg.mix(accent, 0.30),
            ),
            transport_button_fg: pick(transport.button_fg, None, dim),
            transport_button_active_fg: pick(transport.button_active_fg, None, accent),
            transport_button_disabled_fg: pick(
                transport.button_disabled_fg,
                None,
                dim.mix(bg, 0.6),
            ),
            transport_toggle_on_fg: pick(transport.toggle_on_fg, None, accent),
            transport_toggle_off_fg: pick(transport.toggle_off_fg, None, dim.mix(bg, 0.4)),

            vis_bg: pick(vis.bg, None, bg),
            vis_grid_fg: pick(vis.grid_fg, b16.map(|b| b.base01), bg.mix(fg, 0.12)),
            vis_peak_fg: vis_peak,
            vis_ramp,
            vis_osc,

            core,
        }
    }

    fn core(&self) -> &starkit::theme::Theme {
        &self.core
    }
}

impl Theme {
    /// Every resolved colour, one `field = #rrggbb` line in declaration order.
    ///
    /// This is only here for `every_builtin_resolves_as_recorded`, which pins
    /// the whole derivation chain against `testdata/theme-golden/`. Rewriting
    /// where resolution happens is only correct if all sixteen of those files
    /// still match byte for byte, and diffing two dumps says which role moved
    /// when they do not.
    ///
    /// The order is the order the roles were declared in when they all lived
    /// in one struct, so the shared ones and the player's own interleave. That
    /// is deliberate: the files are the record of what the derivation produced
    /// before it was split in two, and reordering them would throw away the
    /// only evidence that it still produces the same thing.
    pub fn dump(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        macro_rules! line {
            ($f:ident) => {
                let _ = writeln!(s, "{} = {}", stringify!($f), self.$f);
            };
        }
        // An unset optional role is not the same as one resolved to black, so
        // it gets a word rather than a colour.
        macro_rules! maybe {
            ($f:ident) => {
                let _ = match self.$f {
                    Some(c) => writeln!(s, "{} = {}", stringify!($f), c),
                    None => writeln!(s, "{} = none", stringify!($f)),
                };
            };
        }
        macro_rules! indexed {
            ($f:ident) => {
                for (i, c) in self.$f.iter().enumerate() {
                    let _ = writeln!(s, "{}[{}] = {}", stringify!($f), i, c);
                }
            };
        }

        line!(bg);
        line!(fg);
        line!(dim);
        line!(accent);
        line!(ok);
        line!(warn);
        line!(error);
        line!(titlebar_active_fg);
        line!(titlebar_active_bg);
        line!(titlebar_inactive_fg);
        line!(titlebar_inactive_bg);
        line!(border);
        line!(border_focused);
        line!(divider);
        line!(panel_bg);
        line!(panel_fg);
        line!(header_fg);
        line!(header_bg);
        line!(empty_fg);
        line!(row_fg);
        line!(row_bg);
        line!(row_index_fg);
        line!(row_duration_fg);
        line!(row_meta_fg);
        line!(row_selected_fg);
        line!(row_selected_bg);
        line!(row_cursor_fg);
        line!(row_cursor_bg);
        line!(row_playing_fg);
        maybe!(row_playing_bg);
        line!(row_marked_fg);
        line!(row_missing_fg);
        line!(row_virtual_fg);
        line!(marquee_fg);
        line!(marquee_paused_fg);
        line!(marquee_stopped_fg);
        line!(time_digit_fg);
        maybe!(time_digit_dim_fg);
        line!(time_colon_fg);
        line!(time_remaining_fg);
        line!(seek_track_fg);
        line!(seek_filled_fg);
        line!(seek_thumb_fg);
        line!(seek_label_fg);
        line!(volume_track_fg);
        line!(volume_filled_fg);
        line!(volume_thumb_fg);
        line!(volume_mute_fg);
        line!(eq_slider_track);
        line!(eq_slider_thumb);
        line!(eq_slider_fill_pos);
        line!(eq_slider_fill_neg);
        line!(eq_zero_line);
        line!(eq_band_label);
        line!(eq_band_value);
        line!(eq_band_focused);
        line!(eq_preamp_fg);
        line!(eq_enabled_fg);
        line!(eq_disabled_fg);
        line!(transport_button_bg);
        line!(transport_button_active_bg);
        line!(transport_button_fg);
        line!(transport_button_active_fg);
        line!(transport_button_disabled_fg);
        line!(transport_toggle_on_fg);
        line!(transport_toggle_off_fg);
        line!(vis_bg);
        line!(vis_grid_fg);
        line!(vis_peak_fg);
        indexed!(vis_ramp);
        indexed!(vis_osc);
        line!(status_fg);
        line!(status_bg);
        line!(hint_key_fg);
        line!(hint_key_bg);
        line!(hint_desc_fg);

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> ThemeFile {
        ThemeFile::parse(
            r##"
            [meta]
            name = "Minimal"
            [app]
            bg = "#000000"
            fg = "#969696"
            accent = "#00FF00"
            "##,
        )
        .unwrap()
    }

    #[test]
    fn a_minimal_theme_resolves_every_role() {
        let t = Theme::resolve(&minimal());
        // Through the deref, which is what the rest of the player does.
        assert_eq!(t.bg, Rgb::new(0, 0, 0));
        assert_eq!(t.accent, Rgb::new(0, 255, 0));
        assert_eq!(t.vis_ramp.len(), 16);
        assert_eq!(t.vis_osc.len(), 5);
    }

    #[test]
    fn every_groove_is_the_same_grey() {
        // The seek bar, the volume slider and the equaliser bands share one
        // track colour, computed once in the core. Three copies of the
        // arithmetic would be three chances for one of them to drift.
        let t = Theme::resolve(&minimal());
        assert_eq!(t.seek_track_fg, t.track_bg());
        assert_eq!(t.volume_track_fg, t.track_bg());
        assert_eq!(t.eq_slider_track, t.track_bg());
    }

    #[test]
    fn a_derived_ramp_runs_quiet_to_loud_in_the_theme_s_own_colour() {
        let t = Theme::resolve(&minimal());
        // Dark at the bottom, the theme's foreground at the top, and the
        // accent through the middle -- not a green-to-red VU borrowed from
        // Winamp regardless of what the theme looks like.
        assert_eq!(t.vis_ramp[15], t.fg);
        let lift = |c: Rgb| c.r as u32 + c.g as u32 + c.b as u32;
        assert!(
            lift(t.vis_ramp[0]) < lift(t.vis_ramp[15]),
            "the ramp does not brighten"
        );
        assert_ne!(t.vis_ramp[0], t.ok, "still the old VU");
        assert_ne!(t.vis_ramp[15], t.error, "still the old VU");
    }

    #[test]
    fn an_explicit_sixteen_stop_ramp_is_used_verbatim() {
        // A Winamp skin's VISCOLOR must survive untouched, or the import is
        // pointless.
        let stops: Vec<String> = (0..16)
            .map(|i| format!("\"#{:02x}0000\"", i * 16))
            .collect();
        let src = format!(
            "[meta]\nname=\"X\"\n[app]\nbg=\"#000000\"\n[vis]\nramp=[{}]\n",
            stops.join(",")
        );
        let t = Theme::resolve(&ThemeFile::parse(&src).unwrap());
        assert_eq!(t.vis_ramp[0], Rgb::new(0x00, 0, 0));
        assert_eq!(t.vis_ramp[15], Rgb::new(0xf0, 0, 0));
    }

    #[test]
    fn a_base16_scheme_reaches_the_analyzer_too() {
        // The analyzer is the panel most visible from across a room, so it
        // follows the scheme rather than keeping Winamp's spectrum whatever
        // the rest of the theme looks like.
        let t = builtin::load("catppuccin-mocha").unwrap();
        assert_eq!(t.vis_ramp[15], t.fg);
        assert_ne!(t.vis_ramp[0], t.ok, "the old green-to-red VU");
    }

    #[test]
    fn a_malformed_player_table_does_not_take_the_rest_of_the_theme_with_it() {
        // The core roles are the ones that decide whether the player is usable
        // at all. A typo in `[vis]` costs the analyzer its colours and nothing
        // else; refusing to start over it would be the wrong trade.
        let src = "[meta]\nname=\"X\"\n[app]\nbg=\"#000000\"\naccent=\"#00ff00\"\n\
                   [vis]\nramp = \"not a list\"\n";
        let t = Theme::resolve(&ThemeFile::parse(src).unwrap());
        assert_eq!(t.bg, Rgb::new(0, 0, 0));
        assert_eq!(t.vis_ramp.len(), 16);
    }

    #[test]
    fn an_imported_skin_keeps_its_viscolor_ramp() {
        // The whole point of importing a skin: VISCOLOR.TXT survives the trip
        // through the generated theme file untouched. Our ramp is
        // quiet-to-loud and VISCOLOR is loud-to-quiet, so index 0 here is
        // VISCOLOR 17 and index 15 is VISCOLOR 2.
        let viscolor = "\
0,0,0
24,33,41
239,49,16
206,41,16
214,90,0
214,102,0
214,115,0
198,123,8
222,165,24
214,181,33
189,222,41
148,222,33
41,206,16
50,190,16
57,181,16
49,156,8
41,148,0
33,140,0
255,255,255
214,214,222
181,189,189
160,170,175
148,156,165
150,150,150
";
        let (colors, _) = wsz::parse_viscolor(viscolor);
        let skin = wsz::SkinColors {
            viscolor: colors,
            pledit_normal: Some(Rgb::new(0, 255, 0)),
            pledit_current: Some(Rgb::new(255, 255, 255)),
            pledit_normal_bg: Some(Rgb::new(0, 0, 0)),
            pledit_selected_bg: Some(Rgb::new(0, 0, 0xC6)),
            ..Default::default()
        };
        let toml = wsz::to_theme_toml(&skin, "Base", "base.wsz");
        let t = Theme::resolve(&ThemeFile::parse(&toml).unwrap());

        assert_eq!(t.vis_ramp[0], Rgb::new(33, 140, 0));
        assert_eq!(t.vis_ramp[15], Rgb::new(239, 49, 16));
        assert_eq!(t.vis_peak_fg, Rgb::new(255, 255, 255));
        assert_eq!(t.vis_grid_fg, Rgb::new(24, 33, 41));
        assert_eq!(t.row_selected_bg, Rgb::new(0, 0, 0xC6));
        assert_eq!(t.row_playing_fg, Rgb::new(255, 255, 255));
    }

    #[test]
    fn a_skin_with_no_viscolor_still_gets_a_full_ramp() {
        let toml = wsz::to_theme_toml(&wsz::SkinColors::default(), "Bare", "bare.wsz");
        let t = Theme::resolve(&ThemeFile::parse(&toml).unwrap());
        assert_eq!(t.vis_ramp.len(), 16, "derivation fills the gap");
    }

    #[test]
    fn a_system_scheme_reaches_the_analyzer() {
        if let Some((f, _)) = system::theme() {
            let t = Theme::resolve(&f);
            assert_eq!(t.vis_ramp.len(), 16);
        }
    }
}
