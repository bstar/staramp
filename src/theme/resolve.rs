//! Turning a theme file into concrete colours.
//!
//! Every role a theme omits is derived from one it supplied, following a fixed
//! chain. That is what lets an eight-line theme be usable and a two-hundred-line
//! one be exact, without two code paths.

use super::color::{ramp, Rgb};
use super::schema::{ThemeFile, Variant};

/// Every colour the UI can ask for, all concrete.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub id: String,
    pub variant: Variant,

    pub bg: Rgb,
    pub fg: Rgb,
    pub dim: Rgb,
    pub accent: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub error: Rgb,

    pub titlebar_active_fg: Rgb,
    pub titlebar_active_bg: Rgb,
    pub titlebar_inactive_fg: Rgb,
    pub titlebar_inactive_bg: Rgb,
    pub border: Rgb,
    pub border_focused: Rgb,
    pub divider: Rgb,

    pub panel_bg: Rgb,
    pub panel_fg: Rgb,
    pub header_fg: Rgb,
    pub header_bg: Rgb,
    pub empty_fg: Rgb,

    pub row_fg: Rgb,
    pub row_bg: Rgb,
    pub row_index_fg: Rgb,
    pub row_duration_fg: Rgb,
    pub row_meta_fg: Rgb,
    pub row_selected_fg: Rgb,
    pub row_selected_bg: Rgb,
    pub row_cursor_fg: Rgb,
    pub row_cursor_bg: Rgb,
    pub row_playing_fg: Rgb,
    pub row_playing_bg: Option<Rgb>,
    pub row_marked_fg: Rgb,
    pub row_missing_fg: Rgb,
    pub row_virtual_fg: Rgb,

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
    /// Sixteen stops, quiet to loud. The Winamp analyzer's whole character.
    pub vis_ramp: [Rgb; 16],
    pub vis_osc: [Rgb; 5],

    pub status_fg: Rgb,
    pub status_bg: Rgb,
    pub hint_key_fg: Rgb,
    pub hint_key_bg: Rgb,
    pub hint_desc_fg: Rgb,
}

/// How far the derived selection bar is pulled back toward the background.
///
/// The contrast that matters on a selected row is between its text and its
/// bar, not between its bar and the panel; a quieter bar keeps the row legible
/// and stops the list looking like it has a hole burnt in it.
/// How far a focused panel's border is tinted toward the accent.
///
/// Low, and with a contrast floor under it, because this colour is drawn on
/// the seam between two docked panels as well as around the focused one --
/// the focused panel's bottom edge is the top of whatever is below it. Loud
/// enough to find, quiet enough that the shared edge does not read as a fault.
const FOCUS_BORDER_TINT: f64 = 0.38;

const SELECTION_DARKEN: f64 = 0.68;

const WHITE: Rgb = Rgb::new(255, 255, 255);
const BLACK: Rgb = Rgb::new(0, 0, 0);

impl Theme {
    pub fn resolve(f: &ThemeFile) -> Self {
        // A base16 block fills in anything the theme did not state explicitly.
        //
        // No lightness flip for light schemes. base00 is *always* the default
        // background in the base16 spec -- a light scheme simply has a light
        // base00 already, as Catppuccin Latte's #eff1f5 does. Swapping the ends
        // produced lavender text on grey at 2.06:1, which the contrast test
        // caught.
        let b16 = f.base16;

        let pick = |explicit: Option<Rgb>, from16: Option<Rgb>, fallback: Rgb| -> Rgb {
            explicit.or(from16).unwrap_or(fallback)
        };

        let variant = f.meta.variant;
        let default_bg = if variant == Variant::Dark {
            BLACK
        } else {
            WHITE
        };
        let default_fg = if variant == Variant::Dark {
            Rgb::new(0x96, 0x96, 0x96)
        } else {
            Rgb::new(0x30, 0x30, 0x30)
        };

        let bg = pick(f.app.bg, b16.map(|b| b.base00), default_bg);
        let fg = pick(f.app.fg, b16.map(|b| b.base05), default_fg);
        let accent = pick(f.app.accent, b16.map(|b| b.base0D), Rgb::new(0, 255, 0));
        // Halfway between fg and bg reads as "muted" rather than "faded".
        //
        // Lifted to stay readable: base16 specifies base03 as a comment colour
        // and many real schemes put it far too dark for text -- COSMIC's
        // #5A5A5A is 2.50:1 on its own background. staramp uses `dim` for
        // hints, track numbers and durations, so it has to clear AA. An
        // explicitly stated `dim` is taken as given; only the derived one is
        // adjusted.
        let dim = match f.app.dim {
            Some(c) => c,
            None => b16
                .map(|b| b.base03)
                .unwrap_or_else(|| fg.mix(bg, 0.45))
                .ensure_contrast(bg, 4.5),
        };
        let ok = pick(f.app.ok, b16.map(|b| b.base0B), Rgb::new(0x29, 0xce, 0x10));
        let warn = pick(
            f.app.warn,
            b16.map(|b| b.base0A),
            Rgb::new(0xd6, 0xb5, 0x21),
        );
        let error = pick(
            f.app.error,
            b16.map(|b| b.base08),
            Rgb::new(0xef, 0x31, 0x10),
        );

        let track_bg = bg.mix(fg, 0.18);
        // A theme that states its own selection colour gets exactly that.
        // Otherwise the derived one is pulled back toward the background:
        // base02 is a *surface* colour, meant for a panel behind text rather
        // than a bar under one line of it, and used raw the selection reads as
        // a lit block rather than as a row that happens to be current.
        let sel_bg = f.row.selected_bg.unwrap_or_else(|| {
            let derived = b16
                .map(|b| b.base02)
                .unwrap_or_else(|| bg.mix(accent, 0.30));
            derived.mix(bg, SELECTION_DARKEN)
        });
        // Whichever of the obvious candidates actually reads on that background.
        //
        // base06 is *not* used blindly: it is only a light foreground in dark
        // schemes. On a light scheme like Catppuccin Latte it is a salmon
        // (#dc8a78) that sits at 1.71:1 on base02, which the contrast test
        // caught. Take it only when it genuinely reads, otherwise pick the
        // candidate that does.
        let sel_fg = f.row.selected_fg.unwrap_or_else(|| {
            let preferred = b16.map(|b| b.base06);
            match preferred {
                Some(c) if sel_bg.contrast(c) >= 4.5 => c,
                _ => sel_bg.best_contrast_against(&[fg, WHITE, BLACK]),
            }
        });

        // Chrome should frame the content, not compete with it.
        //
        // The focused border previously derived from base07, which is #FFFFFF
        // in most schemes -- so focused panels were outlined in white. Falling
        // back to the raw accent is not much better: on a scheme whose accent is
        // saturated green on black it reads as a highlight rather than a frame.
        //
        // So: a muted line with a floor so it stays visible on pure black, and a
        // focused variant that is only a gentle tint toward the accent with a
        // ceiling on how loud it can get. The step between them is deliberately
        // small -- focus should be noticeable, not shouted.
        let border = f
            .chrome
            .border
            .unwrap_or_else(|| bg.mix(fg, 0.22).ensure_contrast(bg, 1.45));
        // A tint toward the accent, and a ceiling on how far it can go.
        //
        // This was the same grey as an unfocused border for a while, because
        // two panels meet along a shared edge -- the player's floor is the
        // playlist's ceiling -- and lighting one panel's frame lights half of
        // its neighbour's, so the seam becomes a step between two greys. That
        // is still true and is the cost of this: a focused panel's bottom
        // edge is also the top of whatever sits under it.
        //
        // It is drawn anyway because a focus mark that only touches the four
        // corners is too quiet to find, which is the complaint that brought
        // this back. The tint is kept low and capped so the frame still
        // frames rather than competing with the content.
        let border_focused = f.chrome.border_focused.unwrap_or_else(|| {
            border
                .mix(accent, FOCUS_BORDER_TINT)
                // Enough of a step from the unfocused grey to read as a
                // change, whatever the theme's accent happens to be.
                .ensure_contrast(bg, 1.9)
        });

        let vis_ramp: [Rgb; 16] = {
            let stops = f
                .vis
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

        let vis_peak = pick(f.vis.peak_fg, None, bg.best_contrast_against(&[fg, WHITE]));
        let vis_osc: [Rgb; 5] = {
            let v = f
                .vis
                .osc
                .clone()
                .filter(|o| o.len() >= 2)
                .unwrap_or_else(|| ramp(&[vis_peak, dim], 5));
            std::array::from_fn(|i| v[i.min(v.len() - 1)])
        };

        Theme {
            name: if f.meta.name.is_empty() {
                "Unnamed".into()
            } else {
                f.meta.name.clone()
            },
            id: if f.meta.id.is_empty() {
                f.meta.name.to_lowercase().replace(' ', "-")
            } else {
                f.meta.id.clone()
            },
            variant,

            bg,
            fg,
            dim,
            accent,
            ok,
            warn,
            error,

            titlebar_active_fg: pick(f.chrome.titlebar_active_fg, None, accent),
            titlebar_active_bg: pick(
                f.chrome.titlebar_active_bg,
                b16.map(|b| b.base01),
                bg.mix(fg, 0.06),
            ),
            titlebar_inactive_fg: pick(f.chrome.titlebar_inactive_fg, None, dim),
            titlebar_inactive_bg: pick(f.chrome.titlebar_inactive_bg, None, bg),
            border,
            border_focused,
            divider: pick(f.chrome.divider, None, bg.mix(fg, 0.12)),

            panel_bg: pick(f.panel.bg, None, bg),
            panel_fg: pick(f.panel.fg, None, fg),
            header_fg: pick(f.panel.header_fg, None, accent),
            header_bg: pick(f.panel.header_bg, b16.map(|b| b.base01), bg.mix(fg, 0.06)),
            empty_fg: pick(f.panel.empty_fg, None, dim.mix(bg, 0.4)),

            row_fg: pick(f.row.fg, None, fg),
            row_bg: pick(f.row.bg, None, bg),
            // Full weight, not dim: the number and the length are the two
            // things scanned down a playlist, and dimming them made the
            // column of titles the only legible thing on the panel.
            row_index_fg: pick(f.row.index_fg, None, fg),
            row_duration_fg: pick(f.row.duration_fg, None, fg),
            row_meta_fg: pick(f.row.meta_fg, b16.map(|b| b.base04), dim),
            row_selected_fg: sel_fg,
            row_selected_bg: sel_bg,
            row_cursor_fg: pick(f.row.cursor_fg, None, sel_fg),
            row_cursor_bg: pick(f.row.cursor_bg, None, sel_bg.mix(bg, 0.35)),
            row_playing_fg: pick(f.row.playing_fg, None, accent),
            row_playing_bg: f.row.playing_bg,
            row_marked_fg: pick(f.row.marked_fg, None, warn),
            row_missing_fg: pick(f.row.missing_fg, None, error),
            row_virtual_fg: pick(f.row.virtual_fg, b16.map(|b| b.base0F), fg.mix(ok, 0.4)),

            marquee_fg: pick(f.marquee.fg, b16.map(|b| b.base0C), accent),
            marquee_paused_fg: pick(f.marquee.paused_fg, None, dim),
            marquee_stopped_fg: pick(f.marquee.stopped_fg, None, dim.mix(bg, 0.4)),

            time_digit_fg: pick(f.time.digit_fg, None, accent),
            time_digit_dim_fg: f.time.digit_dim_fg,
            time_colon_fg: pick(f.time.colon_fg, None, accent),
            time_remaining_fg: pick(f.time.remaining_fg, None, warn),

            seek_track_fg: pick(f.seek.track_fg, None, track_bg),
            seek_filled_fg: pick(f.seek.filled_fg, None, accent),
            seek_thumb_fg: pick(
                f.seek.thumb_fg,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            seek_label_fg: pick(f.seek.label_fg, None, dim),

            volume_track_fg: pick(f.volume.track_fg, None, track_bg),
            // The accent, not the "ok" green. Volume is not a status; green
            // would be reporting something it never reports. It followed the
            // border for a while, which stopped working once the borders went
            // grey: a control has to be readable, and the frame does not.
            volume_filled_fg: pick(f.volume.filled_fg, None, accent),
            volume_thumb_fg: pick(
                f.volume.thumb_fg,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            volume_mute_fg: pick(f.volume.mute_fg, None, error),

            eq_slider_track: pick(f.eq.slider_track, None, track_bg),
            eq_slider_thumb: pick(
                f.eq.slider_thumb,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            eq_slider_fill_pos: pick(f.eq.slider_fill_pos, None, ok),
            eq_slider_fill_neg: pick(f.eq.slider_fill_neg, b16.map(|b| b.base09), warn),
            eq_zero_line: pick(f.eq.zero_line, None, bg.mix(fg, 0.30)),
            eq_band_label: pick(f.eq.band_label, None, dim),
            eq_band_value: pick(f.eq.band_value, None, accent),
            eq_band_focused: pick(
                f.eq.band_focused,
                None,
                bg.best_contrast_against(&[fg, WHITE]),
            ),
            eq_preamp_fg: pick(f.eq.preamp_fg, None, warn),
            eq_enabled_fg: pick(f.eq.enabled_fg, None, ok),
            eq_disabled_fg: pick(f.eq.disabled_fg, None, dim.mix(bg, 0.4)),

            // A face the font draws small cannot be made larger, so the
            // *button* is what carries the size: a padded plate behind the
            // glyph, which reads as a control however the glyph is drawn.
            transport_button_bg: pick(f.transport.button_bg, None, bg.mix(fg, 0.14)),
            transport_button_active_bg: pick(
                f.transport.button_active_bg,
                None,
                bg.mix(accent, 0.30),
            ),
            transport_button_fg: pick(f.transport.button_fg, None, dim),
            transport_button_active_fg: pick(f.transport.button_active_fg, None, accent),
            transport_button_disabled_fg: pick(
                f.transport.button_disabled_fg,
                None,
                dim.mix(bg, 0.6),
            ),
            transport_toggle_on_fg: pick(f.transport.toggle_on_fg, None, accent),
            transport_toggle_off_fg: pick(f.transport.toggle_off_fg, None, dim.mix(bg, 0.4)),

            vis_bg: pick(f.vis.bg, None, bg),
            vis_grid_fg: pick(f.vis.grid_fg, b16.map(|b| b.base01), bg.mix(fg, 0.12)),
            vis_peak_fg: vis_peak,
            vis_ramp,
            vis_osc,

            status_fg: pick(f.status.fg, None, dim),
            status_bg: pick(f.status.bg, None, bg.mix(fg, 0.04)),
            hint_key_fg: pick(
                f.status.hint_key_fg,
                None,
                accent.best_contrast_against(&[BLACK, WHITE]),
            ),
            hint_key_bg: pick(f.status.hint_key_bg, None, accent),
            hint_desc_fg: pick(f.status.hint_desc_fg, None, dim),
        }
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
        assert_eq!(t.bg, Rgb::new(0, 0, 0));
        assert_eq!(t.accent, Rgb::new(0, 255, 0));
        // Derived, not defaulted to something arbitrary.
        assert_ne!(t.row_selected_bg, t.bg);
        assert_eq!(t.vis_ramp.len(), 16);
        assert_eq!(t.vis_osc.len(), 5);
    }

    #[test]
    fn borders_are_muted_and_all_one_grey() {
        for src in [
            include_str!("../../themes/cosmic.toml"),
            include_str!("../../themes/nord.toml"),
            include_str!("../../themes/gruvbox-dark.toml"),
            include_str!("../../themes/catppuccin-latte.toml"),
        ] {
            let t = Theme::resolve(&ThemeFile::parse(src).unwrap());
            let idle = t.bg.contrast(t.border);
            let focused = t.bg.contrast(t.border_focused);

            assert!(
                idle <= 2.5,
                "{}: idle border is {idle:.2}:1, too loud",
                t.id
            );
            assert!(
                idle >= 1.2,
                "{}: idle border is {idle:.2}:1, invisible",
                t.id
            );
            assert!(
                focused <= 3.6,
                "{}: focused border is {focused:.2}:1, too loud",
                t.id
            );
            // Different from the unfocused one, and visibly so: a focus
            // mark confined to four corners is too quiet to find, which is
            // what put the tint back. The cost is real and accepted -- two
            // panels share an edge, so a focused panel's bottom border is
            // also the top of whatever sits under it.
            assert_ne!(
                t.border, t.border_focused,
                "{}: the focused border is the same as the idle one",
                t.id
            );
        }
    }

    /// Not an assertion -- run with `--nocapture` to see the two borders.
    #[test]
    fn preview_the_focus_borders() {
        for name in [
            "cosmic",
            "catppuccin-mocha",
            "nord",
            "gruvbox-dark",
            "tokyo-night",
        ] {
            let t = crate::theme::builtin::load(name).unwrap();
            println!(
                "{name:18} bg {}  idle {}  focused {}  accent {}",
                t.bg.to_hex(),
                t.border.to_hex(),
                t.border_focused.to_hex(),
                t.accent.to_hex()
            );
        }
    }

    #[test]
    fn an_idle_border_is_grey_and_a_focused_one_is_tinted() {
        // The idle frame is quiet by design. The focused one carries the
        // theme's own hue, which is what makes it findable at a glance --
        // a difference in weight alone reads as a rendering artefact rather
        // than as a state.
        for name in ["cosmic", "catppuccin-mocha", "nord", "tokyo-night"] {
            let t = crate::theme::builtin::load(name).unwrap();
            let spread = |c: Rgb| c.r.max(c.g).max(c.b) as i32 - c.r.min(c.g).min(c.b) as i32;
            assert!(
                spread(t.border) <= 28,
                "{name}: the idle border is tinted, not grey: {:?}",
                t.border
            );
            assert!(
                spread(t.border_focused) > spread(t.border),
                "{name}: the focused border carries no more hue than the idle one"
            );
        }
    }

    #[test]
    fn a_focused_border_is_never_plain_white() {
        // base07 is #FFFFFF in most schemes, and deriving from it outlined
        // every focused panel in white.
        let t =
            Theme::resolve(&ThemeFile::parse(include_str!("../../themes/cosmic.toml")).unwrap());
        assert_ne!(t.border_focused, Rgb::new(255, 255, 255));
    }

    #[test]
    fn selected_text_is_readable_on_the_selected_background() {
        let t = Theme::resolve(&minimal());
        let c = t.row_selected_bg.contrast(t.row_selected_fg);
        assert!(c >= 4.5, "selected row contrast only {c:.2}:1");
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
    fn base16_fills_in_the_whole_palette() {
        let src = r##"
            [meta]
            name = "Stylix"
            variant = "dark"
            [base16]
            base00 = "#1e1e2e"
            base01 = "#181825"
            base02 = "#313244"
            base03 = "#45475a"
            base04 = "#585b70"
            base05 = "#cdd6f4"
            base06 = "#f5e0dc"
            base07 = "#b4befe"
            base08 = "#f38ba8"
            base09 = "#fab387"
            base0A = "#f9e2af"
            base0B = "#a6e3a1"
            base0C = "#94e2d5"
            base0D = "#89b4fa"
            base0E = "#cba6f7"
            base0F = "#f2cdcd"
        "##;
        let t = Theme::resolve(&ThemeFile::parse(src).unwrap());
        assert_eq!(t.bg, Rgb::parse_hex("#1e1e2e").unwrap());
        assert_eq!(t.fg, Rgb::parse_hex("#cdd6f4").unwrap());
        assert_eq!(t.accent, Rgb::parse_hex("#89b4fa").unwrap());
        assert_eq!(t.ok, Rgb::parse_hex("#a6e3a1").unwrap());
        assert_eq!(t.error, Rgb::parse_hex("#f38ba8").unwrap());
        // The analyzer ramp comes out of the scheme, so it matches the
        // desktop: the scheme's accent through it, its foreground on top.
        assert_eq!(t.vis_ramp[15], t.fg);
        assert_ne!(t.vis_ramp[0], t.ok, "the old green-to-red VU");
    }

    #[test]
    fn explicit_roles_beat_base16() {
        let src = r##"
            [meta]
            name = "X"
            [base16]
            base00 = "#1e1e2e"
            base01 = "#181825"
            base02 = "#313244"
            base03 = "#45475a"
            base04 = "#585b70"
            base05 = "#cdd6f4"
            base06 = "#f5e0dc"
            base07 = "#b4befe"
            base08 = "#f38ba8"
            base09 = "#fab387"
            base0A = "#f9e2af"
            base0B = "#a6e3a1"
            base0C = "#94e2d5"
            base0D = "#89b4fa"
            base0E = "#cba6f7"
            base0F = "#f2cdcd"
            [app]
            accent = "#00ff00"
        "##;
        let t = Theme::resolve(&ThemeFile::parse(src).unwrap());
        assert_eq!(t.accent, Rgb::new(0, 255, 0));
        assert_eq!(
            t.bg,
            Rgb::parse_hex("#1e1e2e").unwrap(),
            "base16 still fills the rest"
        );
    }

    #[test]
    fn a_light_scheme_keeps_base00_as_the_background() {
        // A genuinely light palette, so the assertion means something.
        let src = r##"
            [meta]
            name = "Latte"
            variant = "light"
            [base16]
            base00 = "#eff1f5"
            base01 = "#e6e9ef"
            base02 = "#ccd0da"
            base03 = "#bcc0cc"
            base04 = "#acb0be"
            base05 = "#4c4f69"
            base06 = "#dc8a78"
            base07 = "#7287fd"
            base08 = "#d20f39"
            base09 = "#fe640b"
            base0A = "#df8e1d"
            base0B = "#40a02b"
            base0C = "#179299"
            base0D = "#1e66f5"
            base0E = "#8839ef"
            base0F = "#dd7878"
        "##;
        let t = Theme::resolve(&ThemeFile::parse(src).unwrap());

        // base00 is the background in every scheme, light or dark. Swapping the
        // lightness ends -- which an earlier version did -- left light themes
        // at 2:1 and unreadable.
        assert_eq!(t.bg, Rgb::parse_hex("#eff1f5").unwrap());
        assert_eq!(t.fg, Rgb::parse_hex("#4c4f69").unwrap());
        assert!(
            t.bg.contrast(t.fg) >= 4.5,
            "light theme body text is only {:.2}:1",
            t.bg.contrast(t.fg)
        );
        // And the selected row has to read on a light background too.
        assert!(
            t.row_selected_bg.contrast(t.row_selected_fg) >= 4.5,
            "light theme selected row is only {:.2}:1",
            t.row_selected_bg.contrast(t.row_selected_fg)
        );
    }
}
