//! Classic Rack composition. This adapter shares the ordinary application's
//! playback owner, state, actions, services and modal key dispatch.
use super::*;
use crate::audio::dsp::apo::{Filter, Width};
use crate::embed::{native, render::PlayerRenderState, GraphicsConfig, Palette};
use starkit::native_surface::{
    classic::{self, Colors},
    HitRegion, PixelRect as R, Primitive, Surface,
};
use starkit::terminal_graphics::{
    protocol::{Component, Input, Scene, Viewport},
    session::{self, Controller},
};

struct Curve {
    profile: crate::audio::dsp::apo::Profile,
    rate: u32,
    enabled: bool,
    channel: usize,
    width: u16,
    points: Vec<[u16; 2]>,
}
pub struct Rack<'a> {
    app: &'a mut App,
    revision: u64,
    cells: bool,
    paths: bool,
    eq_dirty: bool,
    curve: Option<Curve>,
    surface: Option<Surface>,
    viewport: Viewport,
    offset: u16,
    radius: u16,
    drag: Option<String>,
    max_offset: u16,
    overlay: bool,
    cover_cache: Option<(u64, String)>,
    modules: Vec<(Focus, i32, u16)>,
    folded: std::collections::HashSet<String>,
    skin: crate::embed::skin::PlayerSkin,
    skins: bool,
}
fn color(c: crate::theme::color::Rgb) -> [u8; 3] {
    [c.r, c.g, c.b]
}
fn hex(c: crate::theme::color::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}
fn label(s: &mut Surface, c: &Colors, r: R, text: impl Into<String>, font: u16, bold: bool) {
    classic::label(s, r, text, &c.ink, font, bold);
}
fn key(code: &str) -> Option<KeyCode> {
    if let Some(s) = code.strip_prefix("char:") {
        let mut cs = s.chars();
        let c = cs.next()?;
        return cs.next().is_none().then_some(KeyCode::Char(c));
    }
    if let Some(s) = code.strip_prefix("f:") {
        return s.parse().ok().map(KeyCode::F);
    }
    Some(match code {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ => return None,
    })
}
impl App {
    pub fn run_graphical(&mut self, name: &str) -> Result<()> {
        let cfg = crate::config::Config::load().unwrap_or_default();
        anyhow::ensure!(
            cfg.ui.layout == "classic-rack",
            "unsupported graphical layout: {}",
            cfg.ui.layout
        );
        anyhow::ensure!(
            matches!(cfg.ui.corners.as_str(), "rounded" | "rigid"),
            "ui.corners must be rounded or rigid"
        );
        let root = graphical_root()?;
        session::serve_exclusive(
            &root,
            name,
            Rack {
                app: self,
                revision: 0,
                cells: false,
                paths: true,
                eq_dirty: false,
                curve: None,
                surface: None,
                viewport: Viewport::default(),
                offset: 0,
                radius: if cfg.ui.corners == "rounded" { 8 } else { 0 },
                drag: None,
                max_offset: 0,
                overlay: false,
                cover_cache: None,
                modules: Vec::new(),
                skin: crate::embed::skin::PlayerSkin::new()?,
                skins: false,
                folded: ["eq", "album", "activity"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            },
        )
    }
}
pub fn graphical_root() -> Result<PathBuf> {
    Ok(crate::paths::runtime_dir()?.join("graphical"))
}
impl Rack<'_> {
    fn palette(&self) -> Palette {
        let t = &self.app.look.theme;
        Palette {
            bg: color(t.bg),
            fg: color(t.fg),
            muted: color(t.dim),
            accent: color(t.accent),
            selected: color(t.row_selected_bg),
            border: color(t.border),
            error: color(t.error),
        }
    }
    fn player(&mut self) -> PlayerRenderState {
        use std::sync::atomic::Ordering::Relaxed;
        let underruns = self.app.dropouts_now();
        let a = &self.app;
        let st = &a.player.state;
        let item = a.player.current_item();
        let (repeat, shuffled) = {
            let q = a.player.queue.lock().unwrap();
            (q.repeat(), q.shuffled())
        };
        PlayerRenderState {
            title: item
                .as_ref()
                .and_then(|i| {
                    track_line(i.artist.as_deref(), i.title.as_deref(), i.album.as_deref())
                })
                .unwrap_or_else(|| "Nothing playing".into()),
            subtitle: item
                .as_ref()
                .and_then(|i| i.album.clone())
                .unwrap_or_default(),
            tech: format!(
                "{} · {:.1} kHz · {} bit · {} channels",
                st.codec.load_full(),
                st.sample_rate.load(Relaxed) as f64 / 1000.,
                st.bit_depth.load(Relaxed),
                st.channels.load(Relaxed)
            ),
            state: st.state(),
            position: st.position_secs(),
            duration: st.duration_secs(),
            volume: a.player.volume(),
            repeat,
            shuffled,
            bit_perfect: st.bit_perfect.load(Relaxed),
            focused: a.panels.focus == Focus::Player,
            bands: a.vis.meters.bars().to_vec(),
            peaks: a.vis.meters.peaks().to_vec(),
            underruns,
            wave: a.vis.wave.clone(),
            vis_mode: a.vis.mode,
            seek_style: a.look.seek_style,
            bars: a.vis.bars,
            seek_phase: a.look.seek_phase,
        }
    }
    fn window(&self, w: u16, h: u16, c: &Colors, title: &str, font: u16) -> Surface {
        let mut s = Surface::new(w, h, hex(self.app.look.theme.bg));
        s.background = hex(self.app.look.theme.bg);
        let focus = match title {
            "PARAMETRIC EQUALIZER" => Focus::Equalizer,
            "ALBUM" => Focus::Album,
            "ACTIVITY" => Focus::History,
            _ => Focus::Playlist,
        };
        let mut frame_colors = c.clone();
        if self.app.panels.focus == focus {
            frame_colors.title = c.inset.clone();
        }
        classic::frame(
            &mut s,
            R::new(0, 0, w, h),
            &frame_colors,
            self.radius,
            false,
        );
        s.fill(R::new(7, 7, w - 14, 24), &frame_colors.title, 2);
        label(
            &mut s,
            c,
            R::new(26, 7, w.saturating_sub(148), 24),
            title,
            font,
            true,
        );
        classic::label(
            &mut s,
            R::new(w.saturating_sub(96), 7, 78, 24),
            "settings",
            &c.dim,
            font.saturating_sub(2),
            false,
        );
        s.hits.push(HitRegion {
            rect: R::new(w.saturating_sub(100), 7, 82, 24),
            action: "settings".into(),
        });
        s
    }
    fn fold_module(&self, mut s: Surface, name: &str, c: &Colors) -> Surface {
        let folded = self.folded.contains(name);
        if folded {
            s.height = 38;
            s.nodes.retain(|node| node.rect().y < 32);
            // The frame spans the original height; replace it with a compact header frame.
            s.nodes.retain(|node| node.rect().height <= 32);
            s.hits.retain(|hit| hit.rect.y < 32);
            let header = std::mem::take(&mut s.nodes);
            let width = s.width;
            classic::frame(&mut s, R::new(0, 0, width, 38), c, self.radius, false);
            s.nodes.extend(header);
        }
        let glyph = if folded { "▸" } else { "▾" };
        classic::label(&mut s, R::new(8, 7, 12, 24), glyph, &c.dim, 11, false);
        s.hits.push(HitRegion {
            rect: R::new(7, 7, s.width.saturating_sub(115), 24),
            action: format!("fold:{name}"),
        });
        s
    }
    fn eq(&mut self, w: u16, c: &Colors, font: u16) -> Surface {
        let h = 294.max(font * 16);
        let mut s = self.window(w, h, c, "PARAMETRIC EQUALIZER", font);
        let e = &self.app.eq;
        classic::button(
            &mut s,
            R::new(14, 40, 48, 29),
            c,
            if e.enabled { "ON" } else { "OFF" },
            "eq-enabled",
            font,
            e.enabled,
        );
        let profile_width = w.saturating_sub(570).clamp(100, 420);
        let profile = R::new(76, 40, profile_width, 29);
        classic::frame(&mut s, profile, c, 0, true);
        classic::label(
            &mut s,
            R::new(88, 40, profile_width - 24, 29),
            format!("Profile: {}  ›", e.active().name),
            &c.accent,
            font,
            false,
        );
        s.hits.push(HitRegion {
            rect: profile,
            action: "eq-next".into(),
        });
        for (i, (text, action)) in [
            ("IMPORT", "eq-import"),
            ("EXPORT", "eq-export"),
            ("SAVE AS", "eq-save"),
        ]
        .into_iter()
        .enumerate()
        {
            classic::button(
                &mut s,
                R::new(w - 294 + i as u16 * 92, 40, 82, 29),
                c,
                text,
                action,
                font,
                false,
            );
        }
        let plot = R::new(48, 86, w.saturating_sub(410).max(80), 132);
        classic::frame(&mut s, R::new(22, 78, plot.width + 38, 167), c, 0, true);
        classic::frame(&mut s, plot, c, 0, true);
        for db in [-12, -6, 0, 6, 12] {
            let y = plot.y + ((12 - db) as u16 * (plot.height - 1) / 24);
            s.fill(
                R::new(plot.x, y, plot.width, 1),
                if db == 0 { &c.dim } else { &c.shadow },
                0,
            );
            classic::label(
                &mut s,
                R::new(8, y.saturating_sub(7), 36, 16),
                format!("{db:+}"),
                &c.dim,
                font.saturating_sub(2),
                false,
            );
        }
        for (i, f) in [20., 100., 1000., 10000., 20000.].into_iter().enumerate() {
            let x = plot.x + ((f64::log10(f / 20.) / 3.) * f64::from(plot.width - 1)) as u16;
            s.fill(R::new(x, plot.y, 1, plot.height), &c.shadow, 0);
            classic::label(
                &mut s,
                R::new(
                    x.saturating_sub(if i == 4 { 40 } else { 0 }),
                    plot.y + plot.height + 3,
                    48,
                    20,
                ),
                format!("{f:.0}"),
                &c.dim,
                font.saturating_sub(2),
                false,
            );
        }
        let channel = e
            .active()
            .stages
            .get(e.stage)
            .map(|s| s.channels.0.trailing_zeros().min(63) as usize)
            .unwrap_or(0);
        classic::label(
            &mut s,
            R::new(plot.x, plot.y.saturating_sub(22), plot.width, 20),
            format!("RESPONSE · CHANNEL {}", channel + 1),
            &c.dim,
            font.saturating_sub(3),
            false,
        );
        if self.curve.as_ref().is_none_or(|c| {
            c.profile != *e.active()
                || c.rate != e.compiled_rate
                || c.enabled != e.enabled
                || c.channel != channel
                || c.width != plot.width
        }) {
            let points = (0..plot.width.min(1024))
                .map(|i| {
                    let x = (u32::from(i) * u32::from(plot.width - 1)
                        / u32::from(plot.width.min(1024) - 1)) as u16;
                    let hz = 20. * 1000f64.powf(f64::from(x) / f64::from(plot.width - 1));
                    let db = e
                        .compiled
                        .magnitude_db_at_channel(hz, e.compiled_rate, channel)
                        .clamp(-12., 12.);
                    [x, ((12. - db) / 24. * f64::from(plot.height - 1)) as u16]
                })
                .collect();
            self.curve = Some(Curve {
                profile: e.active().clone(),
                rate: e.compiled_rate,
                enabled: e.enabled,
                channel,
                width: plot.width,
                points,
            });
        }
        let points = self.curve.as_ref().unwrap().points.clone();
        s.nodes.push(Primitive::Path {
            rect: plot,
            points,
            color: c.accent.clone(),
            width: 1,
        });
        s.hits.push(HitRegion {
            rect: plot,
            action: "eq-plot".into(),
        });
        for (i, stage) in e.active().stages.iter().enumerate() {
            if let Filter::Biquad {
                frequency, gain_db, ..
            } = stage.filter
            {
                let x = plot.x
                    + ((frequency.clamp(20., 20000.) / 20.).log10() / 3.
                        * f64::from(plot.width - 9)) as u16;
                let y = plot.y
                    + ((12. - gain_db.clamp(-12., 12.)) / 24. * f64::from(plot.height - 9)) as u16;
                s.fill(
                    R::new(x, y, 9, 9),
                    if i == e.stage { &c.accent } else { &c.dim },
                    2,
                );
                classic::label(
                    &mut s,
                    R::new(x.saturating_sub(3), y.saturating_sub(20), 28, 18),
                    (i + 1).to_string(),
                    &c.ink,
                    font.saturating_sub(3),
                    i == e.stage,
                );
                s.hits.push(HitRegion {
                    rect: R::new(x.saturating_sub(4), y.saturating_sub(4), 17, 17),
                    action: format!("eq-band:{i}"),
                });
            }
        }
        let x = w.saturating_sub(328);
        if let Some(stage) = e.active().stages.get(e.stage) {
            label(
                &mut s,
                c,
                R::new(x, 83, 310, 24),
                format!(
                    "FILTER {} · {}",
                    e.stage + 1,
                    if stage.enabled { "ACTIVE" } else { "BYPASS" }
                ),
                font,
                true,
            );
            let vals = match stage.filter {
                Filter::Biquad {
                    frequency,
                    gain_db,
                    width,
                    ..
                } => vec![
                    (format!("Frequency   {frequency:.1} Hz"), "eq-frequency"),
                    (format!("Gain        {gain_db:+.2} dB"), "eq-gain"),
                    (
                        format!(
                            "Width       {}",
                            match width {
                                Width::Q(q) => format!("Q {q:.3}"),
                                Width::Bandwidth(b) => format!("BW {b:.3}"),
                                Width::Slope(v) => format!("S {v:.3}"),
                            }
                        ),
                        "eq-width",
                    ),
                ],
                Filter::Preamp { gain_db } => {
                    vec![(format!("Preamp      {gain_db:+.2} dB"), "eq-gain")]
                }
                _ => vec![("Edit filter coefficients".into(), "eq-width")],
            };
            for (i, (text, action)) in vals.into_iter().enumerate() {
                let (name, value) = text.split_once("  ").unwrap_or(("Value", &text));
                let y = 111 + i as u16 * 26;
                classic::label(&mut s, R::new(x, y, 108, 24), name, &c.dim, font, false);
                let r = R::new(x + 113, y, 188, 24);
                classic::frame(&mut s, r, c, 0, true);
                classic::label(
                    &mut s,
                    R::new(r.x + 10, y, 168, 24),
                    value.trim(),
                    &c.accent,
                    font,
                    false,
                );
                s.hits.push(HitRegion {
                    rect: r,
                    action: action.into(),
                });
            }
            classic::button(
                &mut s,
                R::new(x, 204, 150, 26),
                c,
                "TYPE",
                "eq-type",
                font,
                false,
            );
            classic::button(
                &mut s,
                R::new(x + 158, 204, 150, 26),
                c,
                "CHANNELS",
                "eq-channels",
                font,
                false,
            );
        }
        for (i, (text, action)) in [
            ("+ FILTER", "eq-add"),
            ("BYPASS", "eq-bypass"),
            ("DELETE", "eq-remove"),
            ("MOVE UP", "eq-up"),
            ("MOVE DOWN", "eq-down"),
        ]
        .into_iter()
        .enumerate()
        {
            classic::button(
                &mut s,
                R::new(16 + i as u16 * 112, h - 40, 104, 28),
                c,
                text,
                action,
                font.saturating_sub(2),
                false,
            );
        }
        if w >= 720 {
            classic::button(
                &mut s,
                R::new(w - 140, h - 40, 60, 28),
                c,
                "PREV",
                "eq-stage-prev",
                font.saturating_sub(2),
                false,
            );
            classic::button(
                &mut s,
                R::new(w - 72, h - 40, 60, 28),
                c,
                "NEXT",
                "eq-stage-next",
                font.saturating_sub(2),
                false,
            );
        }
        s
    }
    fn album(&self, w: u16, c: &Colors, font: u16) -> Surface {
        let mut s = self.window(w, 292.max(font * 17), c, "ALBUM", font);
        let a = self
            .app
            .current_uri()
            .and_then(|u| self.app.art.as_ref()?.album_for(&u));
        let item = self.app.player.current_item();
        if w > 600 {
            classic::frame(&mut s, R::new(16, 46, 180, 180), c, 0, true);
            classic::label(
                &mut s,
                R::new(42, 114, 128, 30),
                "NO COVER",
                &c.dim,
                font,
                false,
            );
        }
        let x = if w > 600 { 214 } else { 16 };
        let detail = a.as_ref().and_then(|a| a.detail.as_ref());
        let lines = vec![
            detail
                .and_then(|d| d.album.clone())
                .or_else(|| item.as_ref().and_then(|i| i.album.clone()))
                .unwrap_or_else(|| "No album metadata".into()),
            detail
                .and_then(|d| d.artist.clone())
                .or_else(|| item.as_ref().and_then(|i| i.artist.clone()))
                .unwrap_or_default(),
            detail
                .map(|d| {
                    format!(
                        "{} · {} tracks · {}:{:02}",
                        d.year.unwrap_or_default(),
                        d.track_count,
                        d.total_ms / 60000,
                        d.total_ms / 1000 % 60
                    )
                })
                .unwrap_or_default(),
            detail
                .map(|d| {
                    format!(
                        "{} · {}",
                        d.codec.as_deref().unwrap_or(""),
                        d.genre.as_deref().unwrap_or("")
                    )
                })
                .unwrap_or_default(),
            a.as_ref()
                .and_then(|a| a.source)
                .map(|v| v.name().to_string())
                .unwrap_or_else(|| "No cover available".into()),
            a.as_ref().and_then(|a| a.art.clone()).unwrap_or_default(),
        ];
        for (i, text) in lines.into_iter().enumerate() {
            let y = 52 + i as u16 * 26;
            classic::label(
                &mut s,
                R::new(x, y, w.saturating_sub(x + 16), 28),
                text,
                if i == 0 || i == 4 {
                    &c.accent
                } else if i == 1 {
                    &c.ink
                } else {
                    &c.dim
                },
                font,
                i == 0,
            );
            if i == 3 {
                s.fill(
                    R::new(x, y + 28, w.saturating_sub(x + 18), 1),
                    &c.highlight,
                    0,
                );
            }
        }
        let bottom = s.height - 44;
        for (x, width, text, action) in [
            (16, 29, "‹", "cover-prev"),
            (167, 29, "›", "cover-next"),
            (214, 99, "CHOOSE", "cover-choose"),
            (323, 81, "RETRY", "cover-retry"),
        ] {
            classic::button(
                &mut s,
                R::new(x, bottom, width, 29),
                c,
                text,
                action,
                font,
                false,
            );
        }
        classic::label(
            &mut s,
            R::new(78, bottom, 74, 29),
            a.as_ref()
                .map(|a| format!("{} / {}", a.choice + 1, a.choices))
                .unwrap_or_else(|| "— / —".into()),
            &c.dim,
            font,
            false,
        );
        for (x, title, action) in [
            (w.saturating_sub(260), "artist ↗", "catalog-artist"),
            (w.saturating_sub(182), "album ↗", "catalog-album"),
        ] {
            let rect = R::new(x, 7, 74, 24);
            classic::label(&mut s, rect, title, &c.dim, font.saturating_sub(2), false);
            s.hits.push(HitRegion {
                rect,
                action: action.into(),
            });
        }
        if w > 600 {
            let rect = R::new(w - 180, bottom, 162, 29);
            classic::label(
                &mut s,
                rect,
                "Click art: original ↗",
                &c.dim,
                font.saturating_sub(2),
                false,
            );
            s.hits.push(HitRegion {
                rect,
                action: "cover-open".into(),
            });
        }
        s
    }
    fn activity(&self, w: u16, c: &Colors, font: u16) -> Surface {
        let mut s = self.window(w, 292.max(font * 17), c, "ACTIVITY", font);
        let snap = self.app.activity.snapshot();
        let n = 3;
        for (i, p) in snap.providers.iter().enumerate() {
            let r = R::new(14 + i as u16 * (w - 28) / n, 42, (w - 28) / n - 6, 43);
            classic::frame(&mut s, r, c, 0, true);
            label(
                &mut s,
                c,
                R::new(r.x + 8, r.y, r.width - 16, 22),
                p.provider.to_string(),
                font,
                true,
            );
            classic::label(
                &mut s,
                R::new(r.x + 8, r.y + 22, r.width - 16, 20),
                if !p.configured {
                    "Not connected".into()
                } else {
                    format!(
                        "{} · {}",
                        if p.enabled { "Enabled" } else { "Disabled" },
                        p.username
                    )
                },
                &c.dim,
                font.saturating_sub(2),
                false,
            );
        }
        let card_w = (w - 28) / 3;
        let r = R::new(14 + card_w * 2, 42, card_w - 6, 43);
        classic::frame(&mut s, r, c, 0, true);
        label(
            &mut s,
            c,
            R::new(r.x + 8, r.y, r.width - 16, 22),
            "LOCAL HISTORY",
            font,
            true,
        );
        classic::label(
            &mut s,
            R::new(r.x + 8, r.y + 22, r.width - 16, 20),
            if self.app.is_mirror() {
                "Following playback owner"
            } else {
                "Recording listens"
            },
            &c.dim,
            font.saturating_sub(2),
            false,
        );
        if snap.recent.is_empty() {
            classic::label(
                &mut s,
                R::new(16, 108, w - 32, 26),
                "No listening activity yet",
                &c.dim,
                font,
                false,
            );
        }
        let count = ((s.height - 150) / 34) as usize;
        for (i, row) in snap
            .recent
            .iter()
            .skip(self.app.panels.history_scroll)
            .take(count)
            .enumerate()
        {
            let y = 100 + i as u16 * 34;
            label(
                &mut s,
                c,
                R::new(16, y, w - 32, 18),
                row.name(),
                font,
                false,
            );
            classic::label(
                &mut s,
                R::new(16, y + 18, w - 32, 16),
                format!(
                    "{} · listened {}:{:02} · {}/{} sent · {} errors",
                    row.state(),
                    row.listened_ms / 60000,
                    row.listened_ms / 1000 % 60,
                    row.sent,
                    row.deliveries,
                    row.errors
                ),
                &c.dim,
                font.saturating_sub(2),
                false,
            );
        }
        let bottom = s.height - 42;
        label(
            &mut s,
            c,
            R::new(16, bottom, w.saturating_sub(160), 28),
            format!("{} pending submissions", snap.pending),
            font,
            false,
        );
        let r = R::new(w.saturating_sub(140), s.height - 42, 124, 28);
        classic::button(
            &mut s,
            r,
            c,
            "RETRY FAILED",
            "activity-retry",
            font.saturating_sub(2),
            false,
        );
        s
    }
    fn playlist(&mut self, w: u16, c: &Colors, font: u16) -> Surface {
        let mut s = self.window(w, 260.max(font * 16), c, "PLAYLIST", font);
        if self.app.queue.rows.rows().is_empty() {
            classic::label(
                &mut s,
                R::new(22, 54, w - 44, 30),
                "No tracks · open a playlist or your library",
                &c.dim,
                font,
                false,
            );
            classic::button(
                &mut s,
                R::new(22, 98, 140, 30),
                c,
                "PLAYLISTS",
                "playlists",
                font,
                false,
            );
            classic::button(
                &mut s,
                R::new(170, 98, 140, 30),
                c,
                "LIBRARY",
                "library",
                font,
                false,
            );
        }
        let well_height = s.height - 82;
        classic::frame(&mut s, R::new(10, 38, w - 20, well_height), c, 0, true);
        let toolbar_y = s.height - 36;
        for (i, (title, action)) in [
            ("ADD", "library"),
            ("REM", "queue-remove"),
            ("SEL", "queue-tag"),
            ("MISC", "settings:playlist"),
            ("LIST", "playlists"),
        ]
        .into_iter()
        .enumerate()
        {
            classic::button(
                &mut s,
                R::new(14 + i as u16 * 63, toolbar_y, 57, 29),
                c,
                title,
                action,
                font,
                false,
            );
        }
        let row_h = font + 10;
        let selected_row = self
            .app
            .queue
            .rows
            .row_of_track(self.app.queue.cursor)
            .unwrap_or(0);
        self.app.queue.scroll = PlaylistView::clamp_scroll(
            selected_row,
            self.app.queue.scroll,
            ((s.height - 86) / row_h) as usize,
        );
        let marked = {
            let q = self.app.player.queue.lock().unwrap();
            q.view()
                .iter()
                .enumerate()
                .filter(|(_, i)| self.app.queue.tagged.contains(i))
                .map(|(slot, _)| slot)
                .collect::<std::collections::HashSet<_>>()
        };
        for (line, row) in self
            .app
            .queue
            .rows
            .rows()
            .iter()
            .skip(self.app.queue.scroll)
            .take(((s.height - 86) / row_h) as usize)
            .enumerate()
        {
            let index = self.app.queue.scroll + line;
            let r = R::new(14, 42 + line as u16 * row_h, w - 28, row_h);
            if index == selected_row {
                s.fill(r, &hex(self.app.look.theme.row_selected_bg), 0);
            }
            let text = match row {
                playlist::Row::Track(i) => self
                    .app
                    .queue
                    .items
                    .get(*i)
                    .map(|item| {
                        format!(
                            "{:02}. {}{}",
                            i + 1,
                            if marked.contains(i) { "● " } else { "" },
                            item.title.as_deref().unwrap_or("Untitled")
                        )
                    })
                    .unwrap_or_default(),
                playlist::Row::Section { label, tracks, .. } => {
                    format!("{label} · {tracks} tracks")
                }
            };
            label(
                &mut s,
                c,
                R::new(r.x + 8, r.y, r.width.saturating_sub(90), r.height),
                text,
                font,
                false,
            );
            if let playlist::Row::Track(i) = row {
                if let Some(seconds) = self
                    .app
                    .queue
                    .items
                    .get(*i)
                    .and_then(|item| item.duration_secs)
                {
                    classic::label(
                        &mut s,
                        R::new(w - 82, r.y, 58, r.height),
                        format!("{:02}:{:02}", seconds / 60, seconds % 60),
                        &c.dim,
                        font,
                        false,
                    );
                }
            }
            s.hits.push(HitRegion {
                rect: r,
                action: format!("playlist:{index}"),
            });
        }
        s
    }
    fn drag_eq(&mut self, x: u16, y: u16) {
        let Some(plot) = self
            .surface
            .as_ref()
            .and_then(|s| s.hits.iter().find(|h| h.action == "eq-plot"))
            .map(|h| h.rect)
        else {
            return;
        };
        let fx =
            f64::from(x.saturating_sub(plot.x).min(plot.width - 1)) / f64::from(plot.width - 1);
        let fy =
            f64::from(y.saturating_sub(plot.y).min(plot.height - 1)) / f64::from(plot.height - 1);
        let index = self.app.eq.stage;
        if let Some(stage) = self.app.eq.active_mut().stages.get_mut(index) {
            if let Filter::Biquad {
                frequency, gain_db, ..
            } = &mut stage.filter
            {
                *frequency = 20. * 1000f64.powf(fx);
                *gain_db = 12. - fy * 24.;
                self.app.eq.enabled = true;
                self.app.apply_eq();
                self.eq_dirty = true;
            }
        }
    }
    fn action(&mut self, name: &str, x: u16, y: u16) {
        if let Some(module) = name.strip_prefix("fold:") {
            if !self.folded.remove(module) {
                self.folded.insert(module.to_owned());
            }
            return;
        }
        let a = &mut self.app;
        match name {
            "previous" => a.handle(Action::Prev),
            "next" => a.handle(Action::Next),
            "play" => {
                if a.player.state.state() != PlayState::Playing {
                    a.handle(Action::PlayPause);
                }
            }
            "pause" => {
                if a.player.state.state() == PlayState::Playing {
                    a.handle(Action::PlayPause);
                }
            }
            "stop" => a.handle(Action::Stop),
            "visualizer" => a.handle(Action::ToggleVisualizer),
            "seek" | "volume" => {
                if let Some(h) = self.surface.as_ref().and_then(|s| s.hit(x, y)) {
                    let fraction =
                        f64::from(x.saturating_sub(h.rect.x)) / f64::from(h.rect.width.max(1));
                    if name == "seek" {
                        a.seek_by(
                            fraction * a.player.state.duration_secs()
                                - a.player.state.position_secs(),
                        );
                    } else {
                        a.set_volume(fraction as f32);
                    }
                }
            }
            "playlists" => a.handle(Action::OpenPlaylistPicker),
            "library" => a.open_library(),
            "queue-remove" => a.handle(Action::RemoveTagged),
            "queue-tag" => a.handle(Action::TagRow),
            "help" => a.handle(Action::Help),
            "repeat" => a.handle(Action::CycleRepeat),
            "shuffle" => a.handle(Action::ToggleShuffle),
            "eq-prev" => a.handle(Action::PrevEqPreset),
            "eq-next" => a.handle(Action::NextEqPreset),
            "eq-stage-prev" => a.handle(Action::EqBandPrev),
            "eq-stage-next" => a.handle(Action::EqBandNext),
            "eq-enabled" => a.apply_setting(Setting::EqEnabled),
            "eq-import" => a.apply_setting(Setting::EqImport),
            "eq-export" => a.apply_setting(Setting::EqExport),
            "eq-save" => a.apply_setting(Setting::EqDuplicateProfile),
            "eq-add" => a.apply_setting(Setting::EqAdd),
            "eq-bypass" => a.apply_setting(Setting::EqStageEnabled),
            "eq-remove" => a.apply_setting(Setting::EqRemove),
            "eq-up" => a.apply_setting(Setting::EqMoveUp),
            "eq-down" => a.apply_setting(Setting::EqMoveDown),
            "eq-type" => a.apply_setting(Setting::EqType),
            "eq-channels" => a.apply_setting(Setting::EqChannels),
            "eq-frequency" => a.begin_eq_value(EqField::Frequency),
            "eq-gain" => a.begin_eq_value(EqField::Gain),
            "eq-width" => a.begin_eq_value(EqField::Width),
            "cover-prev" => a.handle(Action::PrevCover),
            "cover-next" => a.handle(Action::NextCover),
            "cover-choose" => a.handle(Action::ChooseCover),
            "cover-retry" => a.handle(Action::RetryCover),
            "cover-open" => a.handle(Action::OpenCover),
            "catalog-artist" => a.apply_setting(Setting::OpenArtistCatalog),
            "catalog-album" => a.apply_setting(Setting::OpenAlbumCatalog),
            "activity-retry" => a.apply_setting(Setting::RetryScrobbles),
            _ => {
                if let Some(i) = name
                    .strip_prefix("eq-band:")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    a.eq.stage = i;
                    a.panels.focus = Focus::Equalizer;
                }
                if let Some(i) = name
                    .strip_prefix("playlist:")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    a.panels.focus = Focus::Playlist;
                    match a.queue.rows.rows().get(i).cloned() {
                        Some(playlist::Row::Track(track)) => {
                            a.set_cursor(track);
                            if a.edit.last_click.click(0, i as u16) {
                                a.handle(Action::Activate);
                            }
                        }
                        Some(playlist::Row::Section { fold, .. }) => a.toggle_fold(&fold),
                        None => {}
                    }
                }
                if let Some(n) = name.strip_prefix("settings:") {
                    a.open_settings(match n {
                        "eq" => Focus::Equalizer,
                        "album" => Focus::Album,
                        "activity" => Focus::History,
                        "playlist" => Focus::Playlist,
                        _ => Focus::Player,
                    });
                }
            }
        }
    }
}
fn append(target: &mut Surface, mut source: Surface, x: u16, y: i32, minimum: u16, name: &str) {
    // Clip offscreen modules without shrinking their font or altering rack geometry.
    target.assets.append(&mut source.assets);
    let limit = target.height;
    let translate = |r: R| -> Option<R> {
        let yy = i32::from(r.y) + y;
        let start = yy.max(i32::from(minimum));
        let end = (yy + i32::from(r.height)).min(i32::from(limit));
        if end <= start {
            return None;
        }
        Some(R::new(r.x + x, start as u16, r.width, (end - start) as u16))
    };
    for node in &mut source.nodes {
        let original = node.rect();
        let Some(rect) = translate(original) else {
            continue;
        };
        if matches!(
            node,
            Primitive::Text { .. } | Primitive::Icon { .. } | Primitive::Sprite { .. }
        ) && rect.height != original.height
        {
            continue;
        }
        match node {
            Primitive::Fill { rect: r, .. }
            | Primitive::Border { rect: r, .. }
            | Primitive::Text { rect: r, .. }
            | Primitive::Icon { rect: r, .. }
            | Primitive::Sprite { rect: r, .. } => *r = rect,
            Primitive::Path {
                rect: r, points, ..
            } => {
                let delta = rect.y as i32 - (original.y as i32 + y);
                for p in points.iter_mut() {
                    p[1] = (i32::from(p[1]) - delta).clamp(0, i32::from(rect.height - 1)) as u16;
                }
                *r = rect;
            }
        }
        target.nodes.push(node.clone());
    }
    for hit in source.hits {
        if let Some(rect) = translate(hit.rect).filter(|r| r.height == hit.rect.height) {
            target.hits.push(HitRegion {
                rect,
                action: if hit.action == "settings" {
                    format!("settings:{name}")
                } else {
                    hit.action
                },
            });
        }
    }
}
impl Controller for Rack<'_> {
    fn tick(&mut self) {
        self.app.tick_player();
        if self
            .app
            .import
            .as_ref()
            .is_some_and(|m| m.pending.is_some())
        {
            self.app.run_pending_import();
        }
        if self.app.journey.processing {
            self.app.journey.processing = false;
            self.app.shape_upcoming_tracks();
            self.app.refresh_settings();
        }
    }
    fn capabilities(&mut self, c: starkit::terminal_graphics::capabilities::Capabilities) {
        self.paths = c.native_paths;
        self.skins = c.native_skins && c.native_surfaces;
        self.cells = c.cell_presentation.unwrap_or(false);
    }
    fn supports_presentation_switch(&self) -> bool {
        true
    }
    fn presentation(&mut self, cells: bool) {
        if self.eq_dirty {
            self.app.finish_eq_edit();
            self.eq_dirty = false;
        }
        self.cells = cells;
        self.drag = None;
    }
    fn scene(&mut self, v: Viewport) -> Scene {
        self.viewport = v;
        self.revision += 1;
        self.overlay = false;
        self.app.sync_view();
        self.app.prepare_playlist_rows();
        self.app.panels.last_area = Rect::new(0, 0, v.columns, v.rows);
        let area = Rect::new(0, 0, v.columns, v.rows);
        let mut buffer = starkit::ratatui::buffer::Buffer::empty(area);
        if self.cells || !self.paths || v.width < 720 || v.height < 360 {
            self.app.draw(area, &mut buffer);
            self.surface = None;
            return Scene::from_buffer(&buffer, v, self.revision);
        }
        let palette = self.palette();
        let c = Colors::new(
            palette.bg,
            palette.fg,
            palette.muted,
            palette.accent,
            palette.border,
        );
        let font = starkit::native_surface::Metrics::from_cell(
            (v.width / u32::from(v.columns.max(1))) as u16,
            (v.height / u32::from(v.rows.max(1))) as u16,
        )
        .font
        .saturating_mul(3)
        .saturating_div(4)
        .max(10);
        if self.app.over.library.is_some() || self.app.over.files.is_some() {
            self.app.draw(area, &mut buffer);
            let surface = classic::form(
                &buffer,
                v.width.min(4096) as u16,
                v.height.min(1800) as u16,
                font,
                &c,
                self.radius,
            );
            let mut scene = Scene::from_buffer(
                &starkit::ratatui::buffer::Buffer::empty(area),
                v,
                self.revision,
            );
            scene.background = hex(self.app.look.theme.bg);
            scene.foreground = hex(self.app.look.theme.fg);
            scene.components.push(Component::Surface {
                rect: starkit::terminal_graphics::protocol::Rect {
                    x: 0,
                    y: 0,
                    width: v.columns,
                    height: v.rows,
                },
                surface,
            });
            self.surface = None;
            return scene;
        }
        let w = (v.width as u16).min(4096);
        let h = (v.height as u16).min(1800);
        let mut surface = Surface::new(w, h, hex(self.app.look.theme.bg));
        let density =
            if self.skins && w >= 1456 && h >= 600 && v.height / u32::from(v.rows.max(1)) >= 32 {
                2
            } else {
                1
            };
        let player_h = (230 * density).max(font * 13).min(h - 40);
        let graphics = GraphicsConfig {
            cell_width: (font * 3 / 5).max(1),
            cell_height: font + 4,
        };
        let state = self.player();
        let player = if self.skins {
            self.skin
                .surface_at_density(
                    w - 16,
                    player_h,
                    graphics,
                    &palette,
                    &state,
                    self.radius,
                    density,
                )
                .unwrap_or_else(|error| {
                    tracing::error!(%error,"Cannot compose player skin");
                    native::classic_surface(
                        w - 16,
                        player_h,
                        graphics,
                        &palette,
                        &state,
                        self.radius,
                    )
                })
        } else {
            native::classic_surface(w - 16, player_h, graphics, &palette, &state, self.radius)
        };
        append(&mut surface, player, 8, 8, 0, "player");
        classic::label(
            &mut surface,
            R::new(w - 100, 15, 80, 24),
            "settings",
            &c.dim,
            font.saturating_sub(2),
            false,
        );
        surface.hits.push(HitRegion {
            rect: R::new(w - 104, 15, 84, 24),
            action: "settings:player".into(),
        });
        let top = player_h + 16;
        self.modules.clear();
        let mut y = i32::from(top) - i32::from(self.offset);
        if self.app.panels.eq {
            let s = self.eq(w - 16, &c, font);
            let s = self.fold_module(s, "eq", &c);
            let hh = s.height;
            self.modules
                .push((Focus::Equalizer, y + i32::from(self.offset), hh));
            append(&mut surface, s, 8, y, top, "eq");
            y += i32::from(hh + 8);
        }
        let module_w = w - 16;
        let album_y = y;
        if self.app.panels.album {
            let s = self.album(module_w, &c, font);
            let s = self.fold_module(s, "album", &c);
            let row_height = s.height;
            self.modules
                .push((Focus::Album, y + i32::from(self.offset), row_height));
            append(&mut surface, s, 8, y, top, "album");
            y += i32::from(row_height + 8);
        }
        if self.app.panels.history {
            let s = self.activity(module_w, &c, font);
            let s = self.fold_module(s, "activity", &c);
            let row_height = s.height;
            self.modules
                .push((Focus::History, y + i32::from(self.offset), s.height));
            append(&mut surface, s, 8, y, top, "activity");
            y += i32::from(row_height + 8);
        }
        if self.app.panels.playlist {
            let s = self.playlist(w - 16, &c, font);
            let s = self.fold_module(s, "playlist", &c);
            let hh = s.height;
            self.modules
                .push((Focus::Playlist, y + i32::from(self.offset), hh));
            append(&mut surface, s, 8, y, top, "playlist");
            y += i32::from(hh + 8);
        }
        self.max_offset =
            (y + i32::from(self.offset) - i32::from(h) + i32::from(font + 24)).max(0) as u16;
        self.offset = self.offset.min(self.max_offset);
        // Footer always stays visible; it uses the same status text as ASCII.
        self.app
            .draw_status(Rect::new(0, 0, area.width, 1), &mut buffer);
        let status = self
            .app
            .status
            .as_ref()
            .filter(|(_, t)| t.elapsed() < Duration::from_secs(4))
            .map(|(msg, _)| msg.clone())
            .unwrap_or_else(|| {
                (0..area.width)
                    .map(|x| buffer[(x, 0)].symbol())
                    .collect::<String>()
            });
        surface.fill(
            R::new(0, h - font - 12, w, font + 12),
            &hex(self.app.look.theme.bg),
            0,
        );
        label(
            &mut surface,
            &c,
            R::new(12, h - font - 12, w - 24, font + 12),
            status,
            font,
            false,
        );
        surface.hits.push(HitRegion {
            rect: R::new(12, h - font - 12, font * 6, font + 12),
            action: "help".into(),
        });
        if self.app.overlay_open() {
            self.overlay = true;
            let mut modal = starkit::ratatui::buffer::Buffer::empty(area);
            self.app.draw_overlays(area, &mut modal);
            let cw = f64::from(w) / f64::from(v.columns.max(1));
            let ch = f64::from(h) / f64::from(v.rows.max(1));
            let meaningful = |cell: &starkit::ratatui::buffer::Cell| {
                cell.symbol() != " " || cell.bg != Color::Reset
            };
            let points = (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .filter(|&(x, y)| meaningful(&modal[(x, y)]))
                .collect::<Vec<_>>();
            if let (Some(left), Some(right), Some(top), Some(bottom)) = (
                points.iter().map(|p| p.0).min(),
                points.iter().map(|p| p.0).max(),
                points.iter().map(|p| p.1).min(),
                points.iter().map(|p| p.1).max(),
            ) {
                let r = R::new(
                    (f64::from(left) * cw) as u16,
                    (f64::from(top) * ch) as u16,
                    ((f64::from(right + 1) * cw) as u16)
                        .saturating_sub((f64::from(left) * cw) as u16),
                    ((f64::from(bottom + 1) * ch) as u16)
                        .saturating_sub((f64::from(top) * ch) as u16),
                );
                classic::frame(&mut surface, r, &c, self.radius, false);
                for y in top..=bottom {
                    let mut x = left;
                    while x <= right {
                        let cell = &modal[(x, y)];
                        let fg = cell.fg;
                        let bg = cell.bg;
                        let mods = cell.modifier;
                        let start = x;
                        let mut text = String::new();
                        while x <= right
                            && modal[(x, y)].fg == fg
                            && modal[(x, y)].bg == bg
                            && modal[(x, y)].modifier == mods
                        {
                            let symbol = modal[(x, y)].symbol();
                            if symbol.chars().all(|c| "│║─━═┌┐└┘┏┓┗┛╔╗╚╝┬┴├┤┼".contains(c))
                            {
                                text.push(' ');
                            } else {
                                text.push_str(symbol);
                            }
                            x += 1;
                        }
                        if text.trim().is_empty() {
                            continue;
                        }
                        let rect = R::new(
                            (f64::from(start) * cw) as u16,
                            (f64::from(y) * ch) as u16,
                            ((f64::from(x) * cw) as u16)
                                .saturating_sub((f64::from(start) * cw) as u16),
                            ch.ceil() as u16,
                        );
                        if let Color::Rgb(r, g, b) = bg {
                            surface.fill(rect, &format!("#{r:02x}{g:02x}{b:02x}"), 0);
                        }
                        let ink = if let Color::Rgb(r, g, b) = fg {
                            format!("#{r:02x}{g:02x}{b:02x}")
                        } else {
                            c.ink.clone()
                        };
                        classic::label(
                            &mut surface,
                            rect,
                            text,
                            &ink,
                            font,
                            mods.contains(starkit::ratatui::style::Modifier::BOLD),
                        );
                    }
                }
                surface.hits.clear();
                surface.hits.push(HitRegion {
                    rect: r,
                    action: "modal".into(),
                });
            }
        }
        let mut scene = Scene::from_buffer(
            &starkit::ratatui::buffer::Buffer::empty(area),
            v,
            self.revision,
        );
        scene.components.push(Component::Surface {
            rect: starkit::terminal_graphics::protocol::Rect {
                x: 0,
                y: 0,
                width: v.columns,
                height: v.rows,
            },
            surface: surface.clone(),
        });
        if !self.overlay
            && self.app.panels.album
            && module_w > 600
            && !self.folded.contains("album")
            && album_y + 46 >= i32::from(top)
            && album_y + 226 < i32::from(h - font - 12)
        {
            if let Some(album) = self
                .app
                .current_uri()
                .and_then(|u| self.app.art.as_ref()?.album_for(&u))
            {
                if let Some(image) = &album.image {
                    let serial = self
                        .app
                        .art
                        .as_ref()
                        .map(|a| a.serial())
                        .unwrap_or_default();
                    if self
                        .cover_cache
                        .as_ref()
                        .is_none_or(|(old, _)| *old != serial)
                    {
                        self.cover_cache = starkit::terminal_graphics::assets::encode_png(image)
                            .ok()
                            .map(|png| (serial, png));
                    }
                    if let Some((_, png)) = &self.cover_cache {
                        let xx = 24u16;
                        let yy = (album_y + 46) as u16;
                        let to_cell = |r: R| starkit::terminal_graphics::protocol::Rect {
                            x: (u32::from(r.x) * u32::from(v.columns) / v.width) as u16,
                            y: (u32::from(r.y) * u32::from(v.rows) / v.height) as u16,
                            width: (u32::from(r.width) * u32::from(v.columns) / v.width).max(1)
                                as u16,
                            height: (u32::from(r.height) * u32::from(v.rows) / v.height).max(1)
                                as u16,
                        };
                        scene.components.push(Component::Image {
                            rect: to_cell(R::new(xx, yy, 180, 180)),
                            id: format!("album-{serial}"),
                            png: Some(png.clone()),
                            scale: Default::default(),
                            zoom: 100,
                        });
                        surface.hits.push(HitRegion {
                            rect: R::new(xx, yy, 180, 180),
                            action: "cover-open".into(),
                        });
                    }
                }
            }
        }
        if let Some(Component::Surface { surface: drawn, .. }) = scene.components.first_mut() {
            drawn.hits.clone_from(&surface.hits);
        }
        use std::hash::{Hash, Hasher};
        let mut interaction = std::collections::hash_map::DefaultHasher::new();
        self.app
            .player
            .queue
            .lock()
            .unwrap()
            .revision()
            .hash(&mut interaction);
        for hit in &surface.hits {
            (
                hit.rect.x,
                hit.rect.y,
                hit.rect.width,
                hit.rect.height,
                &hit.action,
            )
                .hash(&mut interaction);
        }
        scene.interaction = interaction.finish();
        let mut scroll = std::collections::hash_map::DefaultHasher::new();
        (
            self.overlay,
            self.cells,
            self.app.panels.eq,
            self.app.panels.album,
            self.app.panels.history,
            self.app.panels.playlist,
            v.width,
            v.height,
        )
            .hash(&mut scroll);
        scene.scroll_interaction = Some(scroll.finish());
        scene.background = hex(self.app.look.theme.bg);
        scene.foreground = hex(self.app.look.theme.fg);
        scene.accent = hex(self.app.look.theme.accent);
        scene.border = hex(self.app.look.theme.border);
        self.surface = Some(surface);
        scene
    }
    fn input(&mut self, input: Input) {
        match input {
            Input::Key { code, modifiers } => {
                if let Some(code) = key(&code) {
                    let before = self.app.panels.focus;
                    let module = match before {
                        Focus::Equalizer => Some("eq"),
                        Focus::Album => Some("album"),
                        Focus::History => Some("activity"),
                        Focus::Playlist => Some("playlist"),
                        _ => None,
                    };
                    if !self.overlay && code == KeyCode::Enter {
                        if let Some(name) = module.filter(|name| self.folded.contains(*name)) {
                            self.action(&format!("fold:{name}"), 0, 0);
                            return;
                        }
                    }
                    self.app.dispatch_key(KeyEvent::new(
                        code,
                        KeyModifiers::from_bits_truncate(modifiers),
                    ));
                    if before != self.app.panels.focus {
                        if let Some((_, start, height)) = self
                            .modules
                            .iter()
                            .find(|(focus, _, _)| *focus == self.app.panels.focus)
                        {
                            let top = self
                                .modules
                                .first()
                                .map(|(_, start, _)| *start as u16)
                                .unwrap_or(246);
                            let end = *start + i32::from(*height);
                            if *start - i32::from(self.offset) < i32::from(top) {
                                self.offset = (*start - i32::from(top)).max(0) as u16;
                            } else if end - i32::from(self.offset)
                                > self.viewport.height as i32 - 40
                            {
                                self.offset =
                                    (end - self.viewport.height as i32 + 40).max(0) as u16;
                            }
                            self.offset = self.offset.min(self.max_offset);
                        } else if self.app.panels.focus == Focus::Player {
                            self.offset = 0;
                        }
                    }
                }
            }
            Input::Paste { text } => {
                if !self.app.import_artist_paste(&text) && !self.app.auth_paste(&text) {
                    self.app.eq_value_paste(&text);
                }
            }
            Input::Pointer {
                action,
                button,
                x,
                y,
                pixel,
                modifiers,
            } => {
                if self.surface.is_none() || self.overlay {
                    let kind = match action.as_str() {
                        "down" => MouseEventKind::Down(if button == 1 {
                            MouseButton::Left
                        } else {
                            MouseButton::Right
                        }),
                        "up" => MouseEventKind::Up(MouseButton::Left),
                        "drag" => MouseEventKind::Drag(MouseButton::Left),
                        "move" => MouseEventKind::Moved,
                        "scroll_up" => MouseEventKind::ScrollUp,
                        "scroll_down" => MouseEventKind::ScrollDown,
                        _ => return,
                    };
                    self.app.handle_mouse(
                        MouseEvent {
                            kind,
                            column: x,
                            row: y,
                            modifiers: KeyModifiers::from_bits_truncate(modifiers),
                        },
                        Rect::new(0, 0, self.viewport.columns, self.viewport.rows),
                    );
                    return;
                }
                let [px, py] = pixel.unwrap_or([
                    u32::from(x) * self.viewport.width / u32::from(self.viewport.columns.max(1)),
                    u32::from(y) * self.viewport.height / u32::from(self.viewport.rows.max(1)),
                ]);
                if matches!(action.as_str(), "scroll_up" | "scroll_down") {
                    if let Some(index) = self
                        .surface
                        .as_ref()
                        .and_then(|s| s.hit(px as u16, py as u16))
                        .and_then(|h| h.action.strip_prefix("eq-band:"))
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        self.app.eq.stage = index;
                        if let Some(stage) = self.app.eq.active_mut().stages.get_mut(index) {
                            if let Filter::Biquad {
                                width: Width::Q(q), ..
                            } = &mut stage.filter
                            {
                                *q = (*q * if action == "scroll_up" { 1.1 } else { 1. / 1.1 })
                                    .clamp(0.05, 100.);
                                self.app.finish_eq_edit();
                            }
                        }
                        return;
                    }
                }
                if action == "scroll_up" {
                    self.offset = self.offset.saturating_sub(48);
                    return;
                }
                if action == "scroll_down" {
                    self.offset = self.offset.saturating_add(48).min(self.max_offset);
                    return;
                }
                let hit = self
                    .surface
                    .as_ref()
                    .and_then(|s| s.hit(px as u16, py as u16))
                    .map(|h| h.action.clone());
                self.skin.hovered = hit.clone();
                if action == "down" && button == 1 {
                    self.skin.pressed = hit.clone();
                    self.drag = hit.clone();
                    if let Some(name) = hit {
                        self.action(&name, px as u16, py as u16);
                    }
                }
                if action == "drag" {
                    if let Some(name) = self.drag.clone() {
                        if matches!(name.as_str(), "seek" | "volume") {
                            self.action(&name, px as u16, py as u16);
                        }
                        if name.starts_with("eq-band:") {
                            self.drag_eq(px as u16, py as u16);
                        }
                    }
                }
                if action == "up" {
                    self.skin.pressed = None;
                    if self.eq_dirty {
                        self.app.finish_eq_edit();
                        self.eq_dirty = false;
                    }
                    self.drag = None;
                }
            }
            Input::Detach => self.app.quit = true,
            Input::CancelPointer => {
                self.skin.pressed = None;
                self.skin.hovered = None;
                if self.eq_dirty {
                    self.app.finish_eq_edit();
                    self.eq_dirty = false;
                }
                self.drag = None;
            }
            _ => {}
        }
    }
    fn closed(&self) -> bool {
        self.app.quit
    }
    fn shutdown(&mut self) {
        self.app.shutdown();
    }
}

impl App {
    /// Deterministic production rendering for review; callers use an isolated
    /// STARAMP_DIR so no real listening history or player settings are touched.
    pub fn render_rack_references(output: &std::path::Path) -> Result<()> {
        use std::sync::atomic::Ordering::Relaxed;
        anyhow::ensure!(
            std::env::var_os("STARAMP_DIR").is_some(),
            "reference rendering requires an isolated STARAMP_DIR"
        );
        std::fs::create_dir_all(output)?;
        let cfg = crate::config::Config::default();
        let vfs = Arc::new(crate::vfs::Vfs::local(PathBuf::from("/")));
        let player = Arc::new(Player::new_embedded(vfs, None, None)?);
        let mut items = Vec::new();
        for title in [
            "The Dead of the Sea (Orchestral Version)",
            "Where the Sky and Ocean Blend",
            "Hellfire",
            "The Land of the Free",
            "Tonight I'm Alive",
        ] {
            let mut item = QueueItem::new(crate::playlist::uri::TrackUri::parse("/fixture.flac"));
            item.title = Some(title.into());
            item.artist = Some("Visions of Atlantis".into());
            item.album = Some("Armada · An Orchestral Voyage".into());
            item.year = Some(2026);
            item.duration_secs = Some(298);
            items.push(item);
        }
        let mut app = Self::with_player(player, items, &cfg)?;
        // Deterministic presentation data only: no device or playback worker is
        // started. These samples exercise the same state consumed during playback.
        app.player.state.sample_rate.store(44100, Relaxed);
        app.player.state.bit_depth.store(16, Relaxed);
        app.player.state.duration_frames.store(298 * 44100, Relaxed);
        app.player.state.position_frames.store(69 * 44100, Relaxed);
        app.player.state.codec.store(Arc::new("FLAC".into()));
        app.player.state.playing.store(true, Relaxed);
        app.player.state.bit_perfect.store(true, Relaxed);
        app.vis.mode = VisMode::Leds;
        let bands = (0..56)
            .map(|i| 0.15 + 0.75 * (i as f32 * 0.11 + 1.).sin().abs())
            .collect::<Vec<_>>();
        app.vis.meters.update(&bands, 0.016);

        app.panels.eq = true;
        app.panels.album = true;
        app.panels.history = true;
        app.panels.playlist = true;
        let mut rack = Rack {
            app: &mut app,
            revision: 0,
            cells: false,
            paths: true,
            eq_dirty: false,
            curve: None,
            surface: None,
            viewport: Viewport::default(),
            offset: 0,
            radius: 8,
            drag: None,
            max_offset: 0,
            overlay: false,
            cover_cache: None,
            modules: Vec::new(),
            skin: crate::embed::skin::PlayerSkin::new()?,
            skins: true,
            folded: ["eq", "album", "activity"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        for id in crate::theme::builtin::ids() {
            rack.app.look.theme = crate::theme::builtin::load(id).unwrap();
            for (name, width, height, columns, rows) in
                [("full", 1400, 1200, 140, 60), ("compact", 900, 700, 90, 35)]
            {
                let v = Viewport {
                    width,
                    height,
                    columns,
                    rows,
                    generation: 0,
                };
                let scene = rack.scene(v);
                for component in &scene.components {
                    if let Component::Surface { surface, .. } = component {
                        surface.validate()?;
                    }
                }
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("{id}-{name}.png")))?;
                // The player stays at the same physical position while the
                // lower rack scrolls, and every target remains inside bounds.
                let player_hit = rack
                    .surface
                    .as_ref()
                    .and_then(|s| s.hits.iter().find(|h| h.action == "play"))
                    .map(|h| h.rect);
                rack.offset = 240;
                let scrolled = rack.scene(v);
                assert_eq!(
                    player_hit,
                    rack.surface
                        .as_ref()
                        .and_then(|s| s.hits.iter().find(|h| h.action == "play"))
                        .map(|h| h.rect)
                );
                for component in &scrolled.components {
                    if let Component::Surface { surface, .. } = component {
                        surface.validate()?;
                    }
                }
                rack.offset = 0;
            }
        }
        // Exercise native hit routing, modal dismissal, and presentation changes
        // against the same queue owner used for the reference captures.
        let v = Viewport {
            width: 1400,
            height: 1200,
            columns: 140,
            rows: 60,
            generation: 0,
        };
        rack.scene(v);
        let closed_height = rack
            .modules
            .iter()
            .find(|(focus, _, _)| *focus == Focus::Equalizer)
            .unwrap()
            .2;
        assert_eq!(closed_height, 38);
        rack.action("fold:eq", 0, 0);
        rack.scene(v);
        assert!(
            rack.modules
                .iter()
                .find(|(focus, _, _)| *focus == Focus::Equalizer)
                .unwrap()
                .2
                > closed_height
        );
        rack.action("fold:eq", 0, 0);
        rack.scene(v);
        let hit = rack
            .surface
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|h| h.action == "playlist:1")
            .unwrap()
            .rect;
        rack.input(Input::Pointer {
            action: "down".into(),
            button: 1,
            x: hit.x / 10,
            y: hit.y / 20,
            pixel: Some([u32::from(hit.x + 2), u32::from(hit.y + 2)]),
            modifiers: 0,
        });
        assert_eq!(rack.app.queue.cursor, 1);
        rack.app.handle(Action::TagRow);
        let cursor = rack.app.queue.cursor;
        let tags = rack.app.queue.tagged.clone();
        let owner = Arc::clone(&rack.app.player);
        rack.presentation(true);
        rack.scene(v);
        assert_eq!(rack.app.queue.cursor, cursor);
        assert_eq!(rack.app.queue.tagged, tags);
        assert!(Arc::ptr_eq(&owner, &rack.app.player));
        rack.presentation(false);
        rack.scene(v);
        rack.action("settings:eq", 0, 0);
        rack.scene(v);
        assert!(rack.overlay);
        rack.input(Input::Key {
            code: "escape".into(),
            modifiers: 0,
        });
        rack.scene(v);
        assert!(!rack.overlay);
        rack.app.shutdown();
        Ok(())
    }
}
