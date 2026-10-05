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
    );
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
        "seek" => HitTarget::Seek(fraction * duration),
        "volume" => HitTarget::Volume(fraction as f32),
        "visualizer" => HitTarget::Visualizer,
        _ => return None,
    })
}

// Both the embedded player and video hosts use AMP's transport geometry/artwork.
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
) {
    let w = s.width;
    let border = hex(palette.border);
    let selected = hex(palette.selected);
    let accent = hex(palette.accent);
    let button = control.min(inner / 6).max(1);
    for (i, name) in ["previous", "play", "pause", "stop", "next"]
        .iter()
        .enumerate()
    {
        let x = pad + i as u16 * (button + m.small);
        if x + button > w - pad {
            break;
        }
        let rect = R::new(x, controls_y, button, control);
        let active = (*name == "play" && playing) || (*name == "pause" && paused);
        s.fill(rect, if active { &border } else { &selected }, 4);
        let size = m.font.min(button.saturating_sub(4)).min(control);
        s.nodes.push(Primitive::Icon {
            rect: R::new(
                x + (button - size) / 2,
                controls_y + (control - size) / 2,
                size,
                size,
            ),
            name: (*name).into(),
            color: accent.clone(),
        });
        s.hits.push(HitRegion {
            rect,
            action: (*name).into(),
        });
    }
    let volume_width = 120.min(inner / 4);
    let volume = R::new(w - pad - volume_width, controls_y, volume_width, control);
    if volume.x > pad + 5 * (button + m.small) {
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
        &mut s, pad, control, pad, inner, m, palette, playing, paused, volume,
    );
    s
}
