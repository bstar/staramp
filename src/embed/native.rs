//! STAR/AMP's native player. Hosts only compose this surface and relay input.
use super::{
    render::{HitTarget, PlayerRenderState},
    GraphicsConfig, Palette,
};
use crate::audio::player::PlayState;
use crate::ui::panels::player::SeekStyle;
use crate::vis::mode::VisMode;
use starkit::native_surface::{HitRegion, Metrics, PixelRect as R, Primitive, Surface};

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
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
    use starkit::native_surface::classic::{self, Colors};
    let m = Metrics::from_cell(graphics.cell_width, graphics.cell_height);
    let c = Colors::new(
        palette.bg,
        palette.fg,
        palette.muted,
        palette.accent,
        palette.border,
    );
    let accent = c.accent.clone();
    let fg = c.ink.clone();
    let w = w.min((8_000_000 / u32::from(h.max(1))) as u16);
    let mut s = Surface::new(w, h, hex(palette.bg));
    if w < 240 || h < 150 {
        classic::label(&mut s, R::new(0, 0, w, h), &state.title, &fg, m.font, true);
        return s;
    }
    let pad = 12;
    classic::frame(&mut s, R::new(0, 0, w, h), &c, radius, false);
    let header = m.font + 12;
    classic::label(
        &mut s,
        R::new(pad, 4, w - pad * 2, header),
        "S T A R / A M P · PLAYER",
        &c.ink,
        m.font,
        true,
    );
    let button = (m.font + 16).min(40);
    let controls_y = h - button - pad;
    let seek_y = controls_y - header - 4;
    let display_y = header + 8;
    let display_h = seek_y.saturating_sub(display_y + 8);
    classic::frame(
        &mut s,
        R::new(pad, display_y, w - pad * 2, display_h),
        &c,
        0,
        true,
    );
    let clock_w = (m.font * 10).min(w / 3);
    let clock_h = (m.font * 3).min(display_h.saturating_sub(30));
    classic::clock(
        &mut s,
        R::new(pad + 12, display_y + 12, clock_w - 24, clock_h),
        &clock(state.position),
        &accent,
    );
    classic::label(
        &mut s,
        R::new(pad + 12, display_y + display_h - 24, clock_w - 16, 20),
        match state.state {
            PlayState::Playing => "PLAY",
            PlayState::Paused => "PAUSE",
            PlayState::Stopped => "STOP",
        },
        &accent,
        m.font.saturating_sub(2).max(1),
        true,
    );
    let x = pad + clock_w;
    let width = w.saturating_sub(pad * 2 + x);
    let analyzer_top = display_y + 8;
    let analyzer_height = display_h.saturating_sub(header * 2 + 16);
    let pad = pad * 2;
    if analyzer_height > 2 {
        if state.vis_mode.needs_waveform() {
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
            let bands = state
                .bars
                .count(width / graphics.cell_width.max(1))
                .clamp(1, 128);
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
        &fg,
        m.font,
        true,
    );
    classic::label(
        &mut s,
        R::new(x, display_y + display_h - header, width, header),
        if state.bit_perfect {
            format!("{} · BIT PERFECT", state.tech)
        } else {
            state.tech.clone()
        },
        &c.dim,
        m.font.saturating_sub(2).max(1),
        false,
    );
    let pad = 12;
    let time_w = (m.font * 5).min(w / 5);
    classic::label(
        &mut s,
        R::new(pad, seek_y, time_w, header),
        clock(state.position),
        &c.dim,
        m.font,
        false,
    );
    classic::label(
        &mut s,
        R::new(w - pad - time_w, seek_y, time_w, header),
        clock(state.duration),
        &c.dim,
        m.font,
        false,
    );
    let seek = R::new(pad + time_w, seek_y, w - pad * 2 - time_w * 2, header);
    let fraction = if state.duration > 0. {
        (state.position / state.duration).clamp(0., 1.)
    } else {
        0.
    };
    let played = (f64::from(seek.width) * fraction) as u16;
    let thickness = if state.seek_style == SeekStyle::THIN {
        2
    } else if state.seek_style == SeekStyle::BLOCKS {
        header.saturating_sub(4).max(1)
    } else {
        4
    }
    .min(header);
    if state.seek_style == SeekStyle::BAR && header >= 6 {
        for dy in [0, 4] {
            let y = seek.y + (header - 6) / 2 + dy;
            s.fill(R::new(seek.x, y, seek.width, 2), &c.shadow, 0);
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
            &c.shadow,
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
        let active = (*name == "play" && state.state == PlayState::Playing)
            || (*name == "pause" && state.state == PlayState::Paused);
        classic::frame(&mut s, rect, &c, 0, active);
        let icon = button.saturating_sub(8);
        s.nodes.push(Primitive::Icon {
            rect: R::new(rect.x + 4, rect.y + 4, icon, icon),
            name: match *name {
                "previous" | "next" => format!("media-{name}"),
                _ => format!("media-{name}"),
            },
            color: if active { accent.clone() } else { fg.clone() },
        });
        s.hits.push(HitRegion {
            rect,
            action: (*name).into(),
        });
    }
    let toggle_x = pad + 5 * (button + 6) + 24;
    if w > 800 {
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
            classic::button(
                &mut s,
                R::new(toggle_x + i as u16 * 108, controls_y, 100, button),
                &c,
                text,
                action,
                m.font.saturating_sub(2),
                active,
            );
        }
    }
    let volume_w = 120.min(w / 5);
    let volume = R::new(w - pad - volume_w, controls_y, volume_w, button);
    if volume.x > pad + 5 * (button + 6) + 24 {
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
    let close = R::new(width.saturating_sub(row), 0, row, row);
    surface.text(close, "×", &hex(palette.fg), font, true);
    surface.hits.push(HitRegion {
        rect: close,
        action: "picker_close".into(),
    });
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
    let border = hex(palette.border);
    let mut s = Surface::new(w, h, hex(palette.bg));
    if w < 128 || h < 64 {
        s.text(R::new(0, 0, w, h), &state.title, &fg, m.font, false);
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
        &state.title,
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
        if state.vis_mode.needs_waveform() {
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
        2
    } else if state.seek_style == SeekStyle::BLOCKS {
        line.saturating_sub(4).max(1)
    } else {
        4
    }
    .min(line);
    let played = (f64::from(seek.width) * fraction) as u16;
    if state.seek_style == SeekStyle::BAR && line >= 6 {
        for offset in [0, 4] {
            let y = seek.y + (line - 6) / 2 + offset;
            s.fill(R::new(seek.x, y, seek.width, 2), &border, 1);
            s.fill(R::new(seek.x, y, played, 2), &accent, 1);
        }
    } else {
        let y = seek.y + (line - thickness) / 2;
        let radius = if state.seek_style == SeekStyle::BLOCKS {
            0
        } else {
            2
        };
        s.fill(R::new(seek.x, y, seek.width, thickness), &border, radius);
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
        state.state == PlayState::Playing,
        state.state == PlayState::Paused,
        state.volume,
        false,
    );
    s
}
