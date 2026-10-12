//! STAR/AMP's native player. Hosts only compose this surface and relay input.
use super::{
    render::{HitTarget, PlayerRenderState},
    GraphicsConfig, Palette,
};
use crate::audio::player::PlayState;
use crate::ui::panels::player::SeekStyle;
use crate::vis::mode::VisMode;
use starkit::native_surface::{HitRegion, Metrics, PixelRect as R, Primitive, Surface};

/// Muted silver separates received audio from the accent and charcoal groove.
fn buffer_color() -> String {
    "#9ba5b3".into()
}

fn seek_track_color(palette: &Palette) -> String {
    hex(std::array::from_fn(|i| {
        ((u16::from(palette.bg[i]) * 3 + u16::from(palette.border[i])) / 4) as u8
    }))
}

fn buffered_width(state: &PlayerRenderState, width: u16) -> u16 {
    if state.duration <= 0. || !state.buffered.is_finite() {
        return 0;
    }
    if state.buffered <= state.position {
        return ((state.position / state.duration).clamp(0., 1.) * f64::from(width)) as u16;
    }
    ((state.buffered.max(state.position) / state.duration).clamp(0., 1.) * f64::from(width)).ceil()
        as u16
}

pub(crate) fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
/// Common title strip for every rack module, including the embedded player.
pub(crate) fn panel_header(
    surface: &mut Surface,
    width: u16,
    colors: &starkit::native_surface::classic::Colors,
) {
    let rect = R::new(7, 7, width.saturating_sub(14), 24);
    surface.fill(rect, &colors.title, 2);
    heading_depth(surface, rect, &colors.title, &colors.ink);
}
/// A restrained raised edge, independent of the heading's text metrics.
pub(crate) fn heading_depth(surface: &mut Surface, rect: R, background: &str, ink: &str) {
    use starkit::theme::color::Rgb;
    let bg = Rgb::parse_hex(background).unwrap();
    let light = bg.mix(Rgb::parse_hex(ink).unwrap(), 0.06).to_hex();
    let dark = bg.mix(Rgb::new(0, 0, 0), 0.18).to_hex();
    surface.fill(
        R::new(rect.x + 2, rect.y, rect.width.saturating_sub(4), 1),
        &light,
        0,
    );
    surface.fill(
        R::new(
            rect.x + 2,
            rect.y + rect.height - 1,
            rect.width.saturating_sub(4),
            1,
        ),
        &dark,
        0,
    );
}

/// A single accent stroke follows the unskinned pane frame's corner geometry.
pub(crate) fn focus_outline(surface: &mut Surface, accent: &str, radius: u16) {
    surface.nodes.push(Primitive::Border {
        rect: R::new(0, 0, surface.width, surface.height),
        color: accent.into(),
        radius,
    });
}
/// Draw the same centered close mark in player pickers and rack headers.
/// The hit target is larger than the mark and independent of font metrics.
pub(crate) fn close_button(
    surface: &mut Surface,
    rect: R,
    colors: &starkit::native_surface::classic::Colors,
    action: &str,
    hovered: bool,
) {
    if hovered {
        surface.fill(rect, &colors.raised, 3);
    }
    let size = rect.width.min(rect.height).min(28).saturating_sub(2);
    surface.nodes.push(Primitive::Icon {
        rect: R::new(
            rect.x + (rect.width - size) / 2,
            rect.y + (rect.height - size) / 2,
            size,
            size,
        ),
        name: "close".into(),
        color: if hovered { &colors.accent } else { &colors.ink }.clone(),
    });
    surface.hits.push(HitRegion {
        rect,
        action: action.into(),
    });
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Indeterminate progress belongs in the display, without covering transport controls.
fn loading_indicator(s: &mut Surface, rect: R, ink: &str, accent: &str, phase: f32, font: u16) {
    let line = (font + 8).min(rect.height);
    s.text(
        R::new(rect.x, rect.y, rect.width, line),
        "LOADING TRACK…",
        ink,
        font,
        true,
    );
    if rect.height > line + 4 && rect.width > 4 {
        let width = (rect.width / 6).max(4);
        let travel = rect.width - width;
        let progress = 1. - (phase.clamp(0., 1.) * 2. - 1.).abs();
        s.fill(
            R::new(
                rect.x + (f32::from(travel) * progress) as u16,
                rect.y + line + 2,
                width,
                3,
            ),
            accent,
            0,
        );
    }
}

pub fn surface(
    width: u16,
    height: u16,
    graphics: GraphicsConfig,
    palette: &Palette,
    state: &PlayerRenderState,
) -> Surface {
    surface_with_radius(width, height, graphics, palette, state, 8)
}

pub fn surface_with_radius(
    width: u16,
    height: u16,
    graphics: GraphicsConfig,
    palette: &Palette,
    state: &PlayerRenderState,
    radius: u16,
) -> Surface {
    let minimum = Metrics::from_cell(graphics.cell_width, graphics.cell_height)
        .font
        .saturating_mul(3)
        .saturating_add(132);
    if height.saturating_mul(graphics.cell_height) < minimum {
        return compact_surface(width, height, graphics, palette, state);
    }
    classic_surface(
        width.saturating_mul(graphics.cell_width).min(8192),
        height.saturating_mul(graphics.cell_height).min(2048),
        graphics,
        palette,
        state,
        radius,
    )
}

/// Shared production player for standalone Classic Rack and embedded hosts.
/// Its geometry remains local; hosts never reimplement player controls.
pub fn classic_surface(
    w: u16,
    h: u16,
    graphics: GraphicsConfig,
    palette: &Palette,
    state: &PlayerRenderState,
    radius: u16,
) -> Surface {
    player_surface(w, h, graphics, palette, state, radius, None)
}

fn player_frame(
    art: Option<&dyn PlayerArtwork>,
    s: &mut Surface,
    r: R,
    c: &starkit::native_surface::classic::Colors,
    radius: u16,
    inset: bool,
) {
    if let Some(art) = art {
        art.frame(s, r, inset);
    } else {
        starkit::native_surface::classic::frame(s, r, c, radius, inset);
    }
}
#[allow(clippy::too_many_arguments)]
fn player_button(
    art: Option<&dyn PlayerArtwork>,
    s: &mut Surface,
    r: R,
    c: &starkit::native_surface::classic::Colors,
    text: &str,
    action: &str,
    size: u16,
    active: bool,
) {
    if let Some(art) = art {
        art.button(s, r, action, active);
        let face = super::skin_palette::button_rect(r);
        if art.fixed_label(s, face, text, &art.button_ink(action, active), true) {
            s.hits.push(HitRegion {
                rect: r,
                action: action.into(),
            });
            return;
        }
        let text_width = (text.chars().count() as u16)
            .saturating_mul(7)
            .saturating_add(6);
        let pad = face
            .width
            .saturating_sub(text_width)
            .saturating_div(2)
            .max(3)
            .min(face.width / 4);
        starkit::native_surface::classic::label(
            s,
            R::new(
                face.x + pad,
                face.y,
                face.width.saturating_sub(pad * 2),
                face.height,
            ),
            text,
            &art.button_ink(action, active),
            11,
            true,
        );
        s.hits.push(HitRegion {
            rect: r,
            action: action.into(),
        });
    } else {
        starkit::native_surface::classic::button(s, r, c, text, action, size, active);
    }
}

pub(super) trait PlayerArtwork {
    fn readout_advance(&self, size: u16) -> Option<u16>;
    fn readout_width(&self, text: &str, size: u16) -> Option<u16>;
    fn readout(&self, s: &mut Surface, rect: R, text: &str, color: &str, size: u16) -> bool;
    fn fixed_label(
        &self,
        s: &mut Surface,
        rect: R,
        text: &str,
        color: &str,
        centered: bool,
    ) -> bool;
    fn frame(&self, s: &mut Surface, rect: R, inset: bool);
    fn button(&self, s: &mut Surface, rect: R, action: &str, active: bool);
    fn button_ink(&self, action: &str, active: bool) -> String;
    fn clock(&self, s: &mut Surface, origin: [u16; 2], value: &str, color: &str);
    fn transport_icon(&self, s: &mut Surface, rect: R, action: &str, active: bool);
}

fn player_readout(
    artwork: Option<&dyn PlayerArtwork>,
    s: &mut Surface,
    rect: R,
    text: &str,
    color: &str,
    size: u16,
) {
    if !artwork.is_some_and(|art| art.readout(s, rect, text, color, size)) {
        starkit::native_surface::classic::label(s, rect, text, color, size, false);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn player_surface(
    w: u16,
    h: u16,
    graphics: GraphicsConfig,
    palette: &Palette,
    state: &PlayerRenderState,
    radius: u16,
    artwork: Option<&dyn PlayerArtwork>,
) -> Surface {
    use starkit::native_surface::classic::{self, Colors};
    let m = Metrics::from_cell(graphics.cell_width, graphics.cell_height);
    let mut c = if artwork.is_some() {
        super::skin_palette::colors(palette)
    } else {
        Colors::new(
            palette.bg,
            palette.fg,
            palette.muted,
            palette.accent,
            palette.border,
        )
    };
    if let Some(inset) = palette.inset_bg {
        c.inset = hex(inset);
    }
    let accent = c.accent.clone();
    let header_ink = super::skin_palette::tone(palette, "#bbc6d5", true);
    let led_off = super::skin_palette::inactive_led(palette);
    let fg = c.ink.clone();
    let w = w.min((8_000_000 / u32::from(h.max(1))) as u16);
    let mut s = Surface::new(w, h, hex(palette.bg));
    if w < 240 || h < 150 {
        classic::label(&mut s, R::new(0, 0, w, h), &state.title, &fg, m.font, true);
        return s;
    }
    let pad = if artwork.is_some() { 14 } else { 12 };
    player_frame(artwork, &mut s, R::new(0, 0, w, h), &c, radius, false);
    panel_header(&mut s, w, &c);
    let header = 24;
    if !artwork.is_some_and(|art| {
        art.fixed_label(
            &mut s,
            R::new(18, 7, w - 36, 24),
            "S T A R / A M P",
            &header_ink,
            false,
        )
    }) {
        classic::label(
            &mut s,
            R::new(18, 7, w - 36, header),
            "S T A R / A M P",
            &c.ink,
            m.font.saturating_sub(1),
            true,
        );
    }
    if artwork.is_some() && state.bit_perfect {
        classic::label(
            &mut s,
            R::new(w.saturating_sub(218), 7, 108, 24),
            "bit-perfect",
            &c.dim,
            10,
            false,
        );
    }
    let button = 29;
    let controls_y = h - button - pad;
    let seek_y = controls_y - header - 4;
    let display_y = 43;
    let display_h = seek_y.saturating_sub(display_y + if artwork.is_some() { 6 } else { 8 });
    player_frame(
        artwork,
        &mut s,
        R::new(pad, display_y, w - pad * 2, display_h),
        &c,
        0,
        true,
    );
    let clock_w = 174.min(w / 3);
    let clock_h = 54.min(display_h.saturating_sub(30));
    if let Some(art) = artwork {
        art.clock(&mut s, [28, 58], &clock(state.position), &accent);
    } else {
        classic::clock(
            &mut s,
            R::new(
                pad + if artwork.is_some() { 14 } else { 12 },
                display_y + if artwork.is_some() { 15 } else { 12 },
                clock_w - 24,
                clock_h,
            ),
            &clock(state.position),
            &accent,
        );
    }
    classic::label(
        &mut s,
        R::new(pad + 12, display_y + display_h - 24, clock_w - 16, 20),
        if state.loading {
            "LOADING"
        } else {
            match state.state {
                PlayState::Playing => "PLAY · STEREO",
                PlayState::Paused => "PAUSE · STEREO",
                PlayState::Stopped => "STOP",
            }
        },
        &accent,
        m.font.saturating_sub(2).max(1),
        true,
    );
    let x = pad + clock_w + if artwork.is_some() { 2 } else { 0 };
    let width = w.saturating_sub(pad * 2 + x);
    let analyzer_top = display_y + if artwork.is_some() { 9 } else { 8 };
    let analyzer_height = if artwork.is_some() {
        48
    } else {
        display_h.saturating_sub(header * 2 + 16)
    };
    let pad = pad * 2;
    if analyzer_height > 2 {
        if state.loading {
            loading_indicator(
                &mut s,
                R::new(x, analyzer_top, width, analyzer_height),
                &fg,
                &accent,
                state.seek_phase,
                m.font,
            );
        } else if state.vis_mode.needs_waveform() {
            let count = width.clamp(1, 256);
            let step = (width / count).max(1);
            let mut previous = analyzer_height / 2;
            for i in 0..count {
                let sample = state
                    .wave
                    .get(usize::from(i) * state.wave.len() / usize::from(count))
                    .copied()
                    .unwrap_or(0.)
                    .clamp(-1., 1.);
                let level =
                    ((sample + 1.) * 0.5 * f32::from(analyzer_height.saturating_sub(2))) as u16;
                let top = if state.vis_mode == VisMode::Wave {
                    level.min(previous)
                } else {
                    level
                };
                let height = if state.vis_mode == VisMode::Wave {
                    level.abs_diff(previous).max(2)
                } else {
                    2
                };
                s.fill(
                    R::new(
                        x + i * step,
                        analyzer_top + top,
                        step,
                        height.min(analyzer_height - top),
                    ),
                    &accent,
                    0,
                );
                previous = level;
            }
        } else if state.vis_mode != VisMode::Off {
            let bands = if artwork.is_some() {
                56.min((width / 4) as usize).max(1)
            } else {
                state
                    .bars
                    .count(width / graphics.cell_width.max(1))
                    .clamp(1, 128)
            };
            let step = (width / bands as u16).max(1);
            for i in 0..bands {
                let level = state
                    .bands
                    .get(i * state.bands.len() / bands)
                    .copied()
                    .unwrap_or(0.)
                    .clamp(0., 1.);
                let bar_h = ((f32::from(analyzer_height) * level) as u16)
                    .max(2)
                    .min(analyzer_height);
                let bx = x + i as u16 * step;
                if bx + step > w - pad {
                    break;
                }
                if matches!(state.vis_mode, VisMode::Leds | VisMode::Dots) {
                    let size = 2;
                    let pitch = 3.max(usize::from(analyzer_height) / 32);
                    for dy in (0..analyzer_height).step_by(pitch) {
                        s.fill(
                            R::new(
                                bx,
                                analyzer_top + analyzer_height
                                    - dy
                                    - size.min(analyzer_height - dy),
                                if state.vis_mode == VisMode::Dots {
                                    2.min(step)
                                } else {
                                    step.saturating_sub(3).max(1)
                                },
                                size.min(analyzer_height - dy),
                            ),
                            if dy < (f32::from(analyzer_height) * level) as u16 {
                                &accent
                            } else if artwork.is_some() {
                                &led_off
                            } else {
                                &c.title
                            },
                            0,
                        );
                    }
                } else {
                    s.fill(
                        R::new(
                            bx,
                            analyzer_top + analyzer_height - bar_h,
                            step.saturating_sub(2).max(1),
                            bar_h,
                        ),
                        &accent,
                        1,
                    );
                }
                if state.vis_mode == VisMode::Peaks {
                    let peak = (f32::from(analyzer_height.saturating_sub(2))
                        * state
                            .peaks
                            .get(i * state.peaks.len() / bands)
                            .copied()
                            .unwrap_or(level)
                            .clamp(0., 1.)) as u16;
                    s.fill(
                        R::new(
                            bx,
                            analyzer_top + analyzer_height - 2 - peak,
                            step.saturating_sub(2).max(1),
                            2,
                        ),
                        &fg,
                        0,
                    );
                }
            }
        }

        s.hits.push(HitRegion {
            rect: R::new(x, analyzer_top, width, analyzer_height),
            action: "visualizer".into(),
        });
    }
    classic::label(
        &mut s,
        R::new(x, display_y + display_h - header * 2, width, header),
        &state.title,
        &accent,
        if artwork.is_some() { 13 } else { m.font },
        false,
    );
    classic::label(
        &mut s,
        R::new(x, display_y + display_h - header, width, header),
        if state.loading {
            "Loading audio · waiting for media".into()
        } else if state.bit_perfect && artwork.is_none() {
            format!("{} · BIT PERFECT", state.tech)
        } else {
            state.tech.clone()
        },
        &c.dim,
        if artwork.is_some() {
            11
        } else {
            m.font.saturating_sub(2).max(1)
        },
        false,
    );
    let pad = if artwork.is_some() { 16 } else { 12 };
    // Skin readouts use their actual bitmap ink bounds, independent of the
    // terminal's font metrics. Both ends of each track meet the same ink gap.
    let text_width = |text: &str, size: u16| {
        artwork
            .and_then(|art| art.readout_width(text, size))
            .unwrap_or_else(|| (text.chars().count() as u16 * size * 3).div_ceil(4) + 2)
    };
    let track_gap = 24;
    let time_font = if artwork.is_some() { 13 } else { m.font };
    let elapsed = clock(state.position);
    let total = clock(state.duration);
    let elapsed_ink = text_width(&elapsed, time_font).min(w / 5);
    let elapsed_width = artwork
        .and_then(|art| art.readout_advance(time_font))
        .map(|advance| advance * elapsed.chars().count() as u16)
        .unwrap_or(elapsed_ink)
        .max(elapsed_ink)
        .min(w / 5);
    let total_width = text_width(&total, time_font).min(w / 5);
    player_readout(
        artwork,
        &mut s,
        R::new(
            pad + elapsed_width - elapsed_ink,
            seek_y,
            elapsed_ink,
            header,
        ),
        &elapsed,
        &c.dim,
        time_font,
    );
    let total_x = w - pad - total_width;
    player_readout(
        artwork,
        &mut s,
        R::new(total_x, seek_y, total_width, header),
        &total,
        &c.dim,
        time_font,
    );
    let seek_x = pad + elapsed_width + track_gap;
    let seek = R::new(seek_x, seek_y, total_x - track_gap - seek_x, header);
    let fraction = if state.duration > 0. {
        (state.position / state.duration).clamp(0., 1.)
    } else {
        0.
    };
    let played = (f64::from(seek.width) * fraction) as u16;
    let buffered = buffered_width(state, seek.width);
    let buffer_ink = buffer_color();
    let groove = seek_track_color(palette);
    let thickness = if artwork.is_some() {
        6
    } else if state.seek_style == SeekStyle::THIN {
        4
    } else if state.seek_style == SeekStyle::BLOCKS {
        header.saturating_sub(4).max(1)
    } else {
        6
    }
    .min(header);
    if artwork.is_none() && state.seek_style == SeekStyle::BAR && header >= 6 {
        for dy in [0, 4] {
            let y = seek.y + (header - 6) / 2 + dy;
            s.fill(R::new(seek.x, y, seek.width, 2), &groove, 0);
            s.fill(R::new(seek.x, y, buffered, 2), &buffer_ink, 0);
            s.fill(R::new(seek.x, y, played, 2), &accent, 0);
        }
    } else {
        let radius = if state.seek_style == SeekStyle::BLOCKS {
            0
        } else {
            2
        };
        s.fill(
            R::new(
                seek.x,
                seek.y + (header - thickness) / 2,
                seek.width,
                thickness,
            ),
            &groove,
            radius,
        );
        s.fill(
            R::new(
                seek.x,
                seek.y + (header - thickness) / 2,
                buffered,
                thickness,
            ),
            &buffer_ink,
            radius,
        );
        s.fill(
            R::new(seek.x, seek.y + (header - thickness) / 2, played, thickness),
            &accent,
            radius,
        );
    }
    s.hits.push(HitRegion {
        rect: seek,
        action: "seek".into(),
    });
    for (i, name) in ["previous", "play", "pause", "stop", "next"]
        .iter()
        .enumerate()
    {
        let rect = R::new(pad + i as u16 * (button + 6), controls_y, button, button);
        let active = !state.loading
            && ((*name == "play" && state.state == PlayState::Playing)
                || (*name == "pause" && state.state == PlayState::Paused));
        player_button(artwork, &mut s, rect, &c, "", name, m.font, active);
        if let Some(art) = artwork {
            art.transport_icon(&mut s, rect, name, active);
        } else {
            let icon = button.saturating_sub(8);
            s.nodes.push(Primitive::Icon {
                rect: R::new(rect.x + 4, rect.y + 4, icon, icon),
                name: match *name {
                    "previous" | "next" => format!("media-{name}"),
                    _ => format!("media-{name}"),
                },
                color: artwork
                    .map(|art| art.button_ink(name, active))
                    .unwrap_or_else(|| {
                        if !state.control_enabled(name) {
                            c.dim.clone()
                        } else if active {
                            accent.clone()
                        } else {
                            fg.clone()
                        }
                    }),
            });
        }
        s.hits.push(HitRegion {
            rect,
            action: (*name).into(),
        });
    }
    let toggle_x = pad + 5 * (button + 6) + if artwork.is_some() { 46 } else { 24 };
    if w > if artwork.is_some() { 719 } else { 800 } {
        for (i, (text, action, active)) in [
            ("SHUFFLE", "shuffle", state.shuffled),
            (
                match state.repeat {
                    crate::playlist::queue::RepeatMode::Off => "REP OFF",
                    crate::playlist::queue::RepeatMode::One => "REP ONE",
                    crate::playlist::queue::RepeatMode::All => "REP ALL",
                },
                "repeat",
                state.repeat != crate::playlist::queue::RepeatMode::Off,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            player_button(
                artwork,
                &mut s,
                R::new(
                    toggle_x + i as u16 * if artwork.is_some() { 109 } else { 108 },
                    controls_y,
                    100,
                    button,
                ),
                &c,
                text,
                action,
                m.font.saturating_sub(2),
                active,
            );
        }
    }
    let volume_w = 130.min(w / 5);
    let track_gap = 18;
    let volume_font = if artwork.is_some() {
        11
    } else {
        m.font.saturating_sub(2)
    };
    let value_width = text_width("100", volume_font);
    let volume = R::new(
        w - pad - value_width - track_gap - volume_w,
        controls_y,
        volume_w,
        button,
    );
    if volume.x > pad + 5 * (button + 6) + 24 {
        let caption_width = text_width("VOL", volume_font);
        player_readout(
            artwork,
            &mut s,
            R::new(
                volume.x - track_gap - caption_width,
                controls_y,
                caption_width,
                button,
            ),
            "VOL",
            &c.dim,
            volume_font,
        );
        player_readout(
            artwork,
            &mut s,
            R::new(
                volume.x + volume.width + track_gap,
                controls_y,
                value_width,
                button,
            ),
            &format!("{:.0}", state.volume * 100.),
            &c.ink,
            volume_font,
        );
        s.fill(
            R::new(volume.x, volume.y + button / 2, volume.width, 3),
            &c.shadow,
            0,
        );
        s.fill(
            R::new(
                volume.x,
                volume.y + button / 2,
                (f32::from(volume.width) * state.volume.clamp(0., 1.)) as u16,
                3,
            ),
            &accent,
            0,
        );
        s.hits.push(HitRegion {
            rect: volume,
            action: "volume".into(),
        });
    }
    s.hits.retain(|hit| state.control_enabled(&hit.action));
    s
}

pub fn hit_test(surface: &Surface, x: u16, y: u16, duration: f64) -> Option<HitTarget> {
    let hit = surface.hit(x, y)?;
    let fraction = f64::from(x.saturating_sub(hit.rect.x)) / f64::from(hit.rect.width.max(1));
    Some(match hit.action.as_str() {
        "previous" => HitTarget::Previous,
        "play" => HitTarget::Play,
        "pause" => HitTarget::Pause,
        "stop" => HitTarget::Stop,
        "next" => HitTarget::Next,
        "repeat" => HitTarget::Repeat,
        "shuffle" => HitTarget::Shuffle,
        "seek" => HitTarget::Seek(fraction * duration),
        "volume" => HitTarget::Volume(fraction as f32),
        "visualizer" => HitTarget::Visualizer,
        _ => return None,
    })
}

// Both the embedded player and video hosts use AMP's transport geometry/artwork.
fn control_glyph(s: &mut Surface, rect: R, name: &str, color: &str) {
    if rect.width.min(rect.height) < 8 {
        return;
    }
    s.nodes.push(Primitive::Icon {
        rect,
        name: format!("media-{name}"),
        color: color.into(),
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_transport(
    s: &mut Surface,
    pad: u16,
    control: u16,
    controls_y: u16,
    inner: u16,
    m: Metrics,
    palette: &Palette,
    playing: bool,
    paused: bool,
    volume_level: f32,
    movie: bool,
) {
    let w = s.width;
    let border = hex(palette.border);
    let selected = hex(palette.selected);
    let accent = hex(palette.accent);
    let fg = hex(palette.fg);
    let bg = hex(palette.bg);
    let button = control.min(44).min(inner / 6).max(1);
    let toggle = if playing && !paused { "pause" } else { "play" };
    let movie_actions = ["previous", toggle, "next", "stop"];
    let audio_actions = ["previous", "play", "pause", "stop", "next"];
    let actions: &[&str] = if movie {
        &movie_actions
    } else {
        &audio_actions
    };
    for (i, name) in actions.iter().enumerate() {
        let x = pad + i as u16 * (button + m.small);
        if x + button > w - pad {
            break;
        }
        let rect = R::new(x, controls_y, button, control);
        let active = if movie {
            i == 1
        } else {
            (*name == "play" && playing) || (*name == "pause" && paused)
        };
        s.fill(
            rect,
            if active { &accent } else { &selected },
            (control / 4).min(10),
        );
        // Use the full 24-unit face: AMP's artwork already includes optical padding.
        let size = button.min(control).min(40);
        s.nodes.push(Primitive::Icon {
            rect: R::new(
                x + (button - size) / 2,
                controls_y + (control - size) / 2,
                size,
                size,
            ),
            name: match *name {
                "previous" if movie => "media-rewind".into(),
                "next" if movie => "media-forward".into(),
                "play" | "pause" | "stop" => format!("media-{name}"),
                other => other.into(),
            },
            color: if active { bg.clone() } else { fg.clone() },
        });
        s.hits.push(HitRegion {
            rect,
            action: (*name).into(),
        });
    }
    let volume_width = 120.min(inner / 4);
    let volume = R::new(w - pad - volume_width, controls_y, volume_width, control);
    if volume.x > pad + actions.len() as u16 * (button + m.small) + 32 {
        let size = 22.min(control);
        control_glyph(
            s,
            R::new(volume.x - 30, controls_y + (control - size) / 2, size, size),
            "speaker",
            &fg,
        );
        let track_height = 4.min(control);
        let track_y = controls_y + (control - track_height) / 2;
        s.fill(
            R::new(volume.x, track_y, volume_width, track_height),
            &border,
            2,
        );
        s.fill(
            R::new(
                volume.x,
                track_y,
                (f32::from(volume_width) * volume_level.clamp(0., 1.)) as u16,
                track_height,
            ),
            &accent,
            2,
        );
        s.hits.push(HitRegion {
            rect: volume,
            action: "volume".into(),
        });
    }
}

pub fn transport_surface(
    width: u16,
    height: u16,
    palette: &Palette,
    playing: bool,
    paused: bool,
    volume: f32,
) -> Surface {
    let mut s = Surface::new(width, height, hex(palette.bg));
    let m = Metrics::from_cell(12, 24);
    let pad = 4.min(height / 4).min(width / 12);
    let control = height.saturating_sub(pad * 2).max(1);
    let inner = width.saturating_sub(pad * 2);
    draw_transport(
        &mut s, pad, control, pad, inner, m, palette, playing, paused, volume, false,
    );
    s
}

/// Movie controls share AMP's transport artwork and add native track pickers.
#[allow(clippy::too_many_arguments)]
pub fn movie_transport_surface(
    width: u16,
    height: u16,
    palette: &Palette,
    playing: bool,
    paused: bool,
    volume: f32,
    font: u16,
) -> Surface {
    let reserve = (width / 2).min(408);
    let separation = 24.min(width / 24);
    let transport_width = width.saturating_sub(reserve + separation).max(1);
    let mut surface = Surface::new(transport_width, height, hex(palette.bg));
    let m = Metrics::from_cell(12, 24);
    let pad = 4.min(height / 4).min(transport_width / 12);
    let control = height.saturating_sub(pad * 2).max(1);
    draw_transport(
        &mut surface,
        pad,
        control,
        pad,
        transport_width.saturating_sub(pad * 2),
        m,
        palette,
        playing,
        paused,
        volume,
        true,
    );
    surface.width = width;
    let button = reserve / 3;
    let pad = 4.min(height / 4);
    for (i, (label, action, glyph)) in [
        ("Audio", "audio_tracks", "headphones"),
        ("Subtitles", "subtitle_tracks", "subtitles"),
        ("Full screen", "fullscreen", "fullscreen"),
    ]
    .iter()
    .enumerate()
    {
        let rect = R::new(
            width - reserve + i as u16 * button,
            pad,
            button.saturating_sub(4),
            height.saturating_sub(pad * 2),
        );
        surface.fill(rect, &hex(palette.selected), (rect.height / 4).min(10));
        let icon = 22
            .min(rect.height.saturating_sub(8))
            .min(rect.width.saturating_sub(12));
        if rect.width >= 16 {
            control_glyph(
                &mut surface,
                R::new(rect.x + 6, rect.y + (rect.height - icon) / 2, icon, icon),
                glyph,
                &hex(palette.fg),
            );
        }
        if rect.width >= 88 {
            surface.text(
                R::new(
                    rect.x + icon + 12,
                    rect.y,
                    rect.width.saturating_sub(icon + 18),
                    rect.height,
                ),
                *label,
                &hex(palette.fg),
                font.clamp(10, 16),
                false,
            );
        }
        surface.hits.push(HitRegion {
            rect,
            action: (*action).into(),
        });
    }
    surface
}

pub fn track_surface(
    width: u16,
    height: u16,
    palette: &Palette,
    title: &str,
    entries: &[String],
    selected: usize,
    font: u16,
) -> Surface {
    let mut surface = Surface::new(width, height, hex(palette.bg));
    let font = font.max(10);
    let row = font + 12;
    let pad = 12.min(width / 4);
    surface.text(
        R::new(pad, 4, width.saturating_sub(pad * 2 + row), row),
        title,
        &hex(palette.accent),
        font,
        true,
    );
    let close_width = row.max(30).min(width);
    close_button(
        &mut surface,
        R::new(width.saturating_sub(close_width + 6), 4, close_width, row),
        &super::skin_palette::colors(palette),
        "picker_close",
        false,
    );
    let visible = height.saturating_sub(row + 8) / row;
    let offset = selected.saturating_sub(visible.saturating_sub(1) as usize);
    for (index, label) in entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible as usize)
    {
        let rect = R::new(
            pad,
            row + 4 + (index - offset) as u16 * row,
            width.saturating_sub(pad * 2),
            row,
        );
        if index == selected {
            surface.fill(rect, &hex(palette.selected), 4);
        }
        surface.text(
            R::new(
                rect.x + 8,
                rect.y,
                rect.width.saturating_sub(16),
                rect.height,
            ),
            label,
            &hex(palette.fg),
            font,
            index == selected,
        );
        surface.hits.push(HitRegion {
            rect,
            action: format!("track:{index}"),
        });
    }
    surface
}

fn compact_surface(
    width: u16,
    height: u16,
    graphics: GraphicsConfig,
    palette: &Palette,
    state: &PlayerRenderState,
) -> Surface {
    let w = width.saturating_mul(graphics.cell_width).min(8192);
    let h = height.saturating_mul(graphics.cell_height).min(2048);
    let m = Metrics::from_cell(graphics.cell_width, graphics.cell_height);
    let fg = hex(palette.fg);
    let accent = hex(palette.accent);
    let muted = hex(palette.muted);
    let mut s = Surface::new(w, h, hex(palette.bg));
    if w < 128 || h < 64 {
        let title = if state.loading {
            format!("LOADING… {}", state.title)
        } else {
            state.title.clone()
        };
        s.text(R::new(0, 0, w, h), title, &fg, m.font, false);
        return s;
    }
    // Compact sizes keep title, seeking and transport; the analyzer takes the slack.
    let pad = m.small.min(w / 10).min(h / 10);
    let inner = w.saturating_sub(2 * pad);
    let line = graphics.cell_height.min(h / 4).max(1);
    let control = m.control.min(h / 3).max(1);
    let controls_y = h.saturating_sub(control + pad);
    let seek_y = controls_y.saturating_sub(line + m.small);
    s.text(
        R::new(pad, pad, inner, line),
        if state.loading {
            format!("LOADING… {}", state.title)
        } else {
            state.title.clone()
        },
        &fg,
        m.font,
        true,
    );
    if h >= line * 6 {
        s.text(
            R::new(pad, pad + line, inner, line),
            &state.subtitle,
            &muted,
            m.font,
            false,
        );
    }
    let analyzer_top = (pad + line * 2 + m.small).min(seek_y);
    let analyzer_height = seek_y.saturating_sub(analyzer_top + m.small);
    let clock_w = (m.font * 7).min(inner / 3);
    if analyzer_height >= line {
        s.text(
            R::new(pad, analyzer_top, clock_w, analyzer_height),
            clock(state.position),
            &accent,
            (m.font * 2).min(128),
            false,
        );
        let x = pad + clock_w + m.gap.min(inner / 10);
        let width = w.saturating_sub(pad + x);
        if state.loading {
            loading_indicator(
                &mut s,
                R::new(x, analyzer_top, width, analyzer_height),
                &fg,
                &accent,
                state.seek_phase,
                m.font,
            );
        } else if state.vis_mode.needs_waveform() {
            let count = width.clamp(1, 256);
            let step = (width / count).max(1);
            let mut previous = analyzer_height / 2;
            for i in 0..count {
                let sample = state
                    .wave
                    .get(usize::from(i) * state.wave.len() / usize::from(count))
                    .copied()
                    .unwrap_or(0.)
                    .clamp(-1., 1.);
                let level =
                    ((sample + 1.) * 0.5 * f32::from(analyzer_height.saturating_sub(2))) as u16;
                let top = if state.vis_mode == VisMode::Wave {
                    level.min(previous)
                } else {
                    level
                };
                let height = if state.vis_mode == VisMode::Wave {
                    level.abs_diff(previous).max(2)
                } else {
                    2
                };
                s.fill(
                    R::new(
                        x + i * step,
                        analyzer_top + top,
                        step,
                        height.min(analyzer_height - top),
                    ),
                    &accent,
                    0,
                );
                previous = level;
            }
        } else if state.vis_mode != VisMode::Off {
            let bands = state.bands.len().clamp(1, 64);
            let step = (width / bands as u16).max(1);
            for i in 0..bands {
                let level = state.bands.get(i).copied().unwrap_or(0.).clamp(0., 1.);
                let bar_h = ((f32::from(analyzer_height) * level) as u16)
                    .max(2)
                    .min(analyzer_height);
                let bx = x + i as u16 * step;
                if bx + step > w - pad {
                    break;
                }
                if matches!(state.vis_mode, VisMode::Leds | VisMode::Dots) {
                    let size = if state.vis_mode == VisMode::Dots {
                        2
                    } else {
                        4
                    };
                    for dy in (0..bar_h).step_by(6.max(usize::from(bar_h) / 32)) {
                        s.fill(
                            R::new(
                                bx,
                                analyzer_top + analyzer_height - bar_h + dy,
                                if state.vis_mode == VisMode::Dots {
                                    2.min(step)
                                } else {
                                    step.saturating_sub(2).max(1)
                                },
                                size.min(bar_h - dy),
                            ),
                            &accent,
                            1,
                        );
                    }
                } else {
                    s.fill(
                        R::new(
                            bx,
                            analyzer_top + analyzer_height - bar_h,
                            step.saturating_sub(2).max(1),
                            bar_h,
                        ),
                        &accent,
                        1,
                    );
                }
                if state.vis_mode == VisMode::Peaks {
                    let peak = (f32::from(analyzer_height.saturating_sub(2))
                        * state.peaks.get(i).copied().unwrap_or(level).clamp(0., 1.))
                        as u16;
                    s.fill(
                        R::new(
                            bx,
                            analyzer_top + analyzer_height - 2 - peak,
                            step.saturating_sub(2).max(1),
                            2,
                        ),
                        &fg,
                        0,
                    );
                }
            }
        }
        s.hits.push(HitRegion {
            rect: R::new(x, analyzer_top, width, analyzer_height),
            action: "visualizer".into(),
        });
    }
    let time_width = (m.font * 4).min(inner / 4);
    s.text(
        R::new(pad, seek_y, time_width, line),
        clock(state.position),
        &muted,
        m.font,
        false,
    );
    s.text(
        R::new(w - pad - time_width, seek_y, time_width, line),
        clock(state.duration),
        &muted,
        m.font,
        false,
    );
    let seek = R::new(
        pad + time_width,
        seek_y,
        inner.saturating_sub(2 * time_width),
        line,
    );
    let fraction = if state.duration > 0. {
        (state.position / state.duration).clamp(0., 1.)
    } else {
        0.
    };
    // Keep the same seek hit region for every presentation. Styles change
    // native stroke weight, never the position or size of the pointer target.
    let thickness = if state.seek_style == SeekStyle::THIN {
        4
    } else if state.seek_style == SeekStyle::BLOCKS {
        line.saturating_sub(4).max(1)
    } else {
        6
    }
    .min(line);
    let played = (f64::from(seek.width) * fraction) as u16;
    let buffered = buffered_width(state, seek.width);
    let buffer_ink = buffer_color();
    let groove = seek_track_color(palette);
    if state.seek_style == SeekStyle::BAR && line >= 6 {
        for offset in [0, 4] {
            let y = seek.y + (line - 6) / 2 + offset;
            s.fill(R::new(seek.x, y, seek.width, 2), &groove, 1);
            s.fill(R::new(seek.x, y, buffered, 2), &buffer_ink, 1);
            s.fill(R::new(seek.x, y, played, 2), &accent, 1);
        }
    } else {
        let y = seek.y + (line - thickness) / 2;
        let radius = if state.seek_style == SeekStyle::BLOCKS {
            0
        } else {
            2
        };
        s.fill(R::new(seek.x, y, seek.width, thickness), &groove, radius);
        s.fill(R::new(seek.x, y, buffered, thickness), &buffer_ink, radius);
        s.fill(R::new(seek.x, y, played, thickness), &accent, radius);
    }
    s.hits.push(HitRegion {
        rect: seek,
        action: "seek".into(),
    });
    draw_transport(
        &mut s,
        pad,
        control,
        controls_y,
        inner,
        m,
        palette,
        !state.loading && state.state == PlayState::Playing,
        !state.loading && state.state == PlayState::Paused,
        state.volume,
        false,
    );
    s.hits.retain(|hit| state.control_enabled(&hit.action));
    s
}
