//! Classic Rack composition. This adapter shares the ordinary application's
//! playback owner, state, actions, services and modal key dispatch.
use super::*;
use unicode_width::UnicodeWidthStr;
mod eq_profiles;
mod popup;
use crate::audio::dsp::apo::{Filter, Width};
use crate::embed::{native, render::PlayerRenderState, GraphicsConfig, Palette};
use starkit::native_surface::{
    classic::{self, Colors},
    HitRegion, HoverFill, PixelRect as R, Primitive, Surface,
};
use starkit::terminal_graphics::{
    protocol::{Component, Input, Scene, Viewport},
    session::{self, Controller},
};

const EQ_HEIGHT: u16 = 258;
const LEDGER_HEIGHT: u16 = 320;
const LEDGER_ROWS: usize = 6;
const ALBUM_COVER: u16 = 160;
const ALBUM_COVER_Y: u16 = 50;

fn filter_caption(filter: &Filter) -> &'static str {
    use crate::audio::dsp::apo::BiquadKind as K;
    match filter {
        Filter::Preamp { .. } => "GAIN",
        Filter::Biquad { kind, .. } => match kind {
            K::Peaking => "PEAKING",
            K::LowShelf => "LOW SHELF",
            K::HighShelf => "HIGH SHELF",
            K::LowPass => "LOW PASS",
            K::HighPass => "HIGH PASS",
            K::BandPass => "BAND PASS",
            K::Notch => "NOTCH",
            K::AllPass => "ALL PASS",
        },
        Filter::GraphicEq { .. } => "GRAPHIC EQ",
        Filter::Iir { .. } => "CUSTOM IIR",
    }
}

fn provider_state(
    snapshot: &crate::activity::Snapshot,
    provider: crate::activity::Provider,
) -> (&'static str, bool) {
    match snapshot
        .providers
        .iter()
        .find(|status| status.provider == provider)
    {
        Some(status) if status.configured && status.enabled => ("Enabled", true),
        Some(status) if status.configured => ("Off", false),
        _ => ("Set up", false),
    }
}

struct Curve {
    profile: crate::audio::dsp::apo::Profile,
    rate: u32,
    enabled: bool,
    channel: usize,
    width: u16,
    points: Vec<[u16; 2]>,
}
#[derive(PartialEq, Eq)]
struct CoverKey {
    serial: u64,
    canvas: [u32; 2],
    bounds: R,
    border: u16,
    background: String,
    silver_gradient: bool,
    outline: String,
}
struct CoverCache {
    key: CoverKey,
    id: String,
    png: String,
}

/// Enclose the physical artwork slot in cells, retaining its exact position
/// inside the transported raster. One-to-one image presentation avoids another
/// resampling pass that would blur the two-pixel outline on fractional grids.
fn cover_layout(v: Viewport, slot: R) -> (starkit::terminal_graphics::protocol::Rect, [u32; 2], R) {
    let x = u32::from(slot.x) * u32::from(v.columns) / v.width;
    let y = u32::from(slot.y) * u32::from(v.rows) / v.height;
    let right = (u32::from(slot.x + slot.width) * u32::from(v.columns)).div_ceil(v.width);
    let bottom = (u32::from(slot.y + slot.height) * u32::from(v.rows)).div_ceil(v.height);
    let rect = starkit::terminal_graphics::protocol::Rect {
        x: x as u16,
        y: y as u16,
        width: (right - x) as u16,
        height: (bottom - y) as u16,
    };
    let cw = v.width as f32 / f32::from(v.columns);
    let ch = v.height as f32 / f32::from(v.rows);
    let width = (f32::from(rect.width) * cw).floor() as u32;
    let height = (f32::from(rect.height) * ch).floor() as u32;
    let left = (x as f32 * cw + (f32::from(rect.width) * cw - width as f32) / 2.).round();
    let top = (y as f32 * ch + (f32::from(rect.height) * ch - height as f32) / 2.).round();
    (
        rect,
        [width, height],
        R::new(
            slot.x - left as u16,
            slot.y - top as u16,
            slot.width,
            slot.height,
        ),
    )
}

fn cover_raster(image: &starkit::image::RgbaImage, key: &CoverKey) -> starkit::image::RgbaImage {
    use starkit::image::{imageops, Rgba, RgbaImage};
    let color = |s: &str| {
        let c = crate::theme::color::Rgb::parse_hex(s).expect("artwork theme color");
        Rgba([c.r, c.g, c.b, 255])
    };
    let border = u32::from(key.border);
    let bounds = key.bounds;
    let ratio = ((u32::from(bounds.width) - 2 * border) as f32 / image.width() as f32)
        .min((u32::from(bounds.height) - 2 * border) as f32 / image.height() as f32);
    let width = (image.width() as f32 * ratio).round().max(1.) as u32;
    let height = (image.height() as f32 * ratio).round().max(1.) as u32;
    let x = u32::from(bounds.x) + (u32::from(bounds.width) - width) / 2;
    let y = u32::from(bounds.y) + (u32::from(bounds.height) - height) / 2;
    let mut canvas = RgbaImage::new(key.canvas[0], key.canvas[1]);
    let well = RgbaImage::from_fn(u32::from(bounds.width), u32::from(bounds.height), |_, y| {
        if key.silver_gradient {
            let shade = crate::embed::skin::silver_panel_color(
                ALBUM_COVER_Y + y as u16 / (key.border / 2),
                LEDGER_HEIGHT,
            );
            Rgba([shade.r, shade.g, shade.b, 255])
        } else {
            color(&key.background)
        }
    });
    imageops::overlay(&mut canvas, &well, i64::from(bounds.x), i64::from(bounds.y));
    let outline =
        RgbaImage::from_pixel(width + 2 * border, height + 2 * border, color(&key.outline));
    imageops::overlay(
        &mut canvas,
        &outline,
        i64::from(x - border),
        i64::from(y - border),
    );
    let image = imageops::resize(image, width, height, imageops::FilterType::Triangle);
    // Transparent artwork composites onto the pane color, not the outline.
    let backdrop = imageops::crop_imm(
        &well,
        x - u32::from(bounds.x),
        y - u32::from(bounds.y),
        width,
        height,
    )
    .to_image();
    imageops::overlay(&mut canvas, &backdrop, i64::from(x), i64::from(y));
    imageops::overlay(&mut canvas, &image, i64::from(x), i64::from(y));
    canvas
}

// Pointer-relative parameters avoid jumping from a band's own gain to the
// combined response (preamp and neighboring filters) drawn under its handle.
struct EqDrag {
    stage: usize,
    origin: [u16; 2],
    plot: R,
    frequency: f64,
    gain_db: f64,
}

pub struct Rack<'a> {
    app: &'a mut App,
    audio: super::ssh_audio::RemoteAudio,
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
    eq_drag: Option<EqDrag>,
    max_offset: u16,
    overlay: bool,
    modal_focus: Option<R>,
    context_anchor: Option<[u32; 2]>,
    playlist_bars: Scrollbars<()>,
    playlist_visible: usize,
    playlist_scrolled: Option<usize>,
    cover_cache: Option<CoverCache>,
    modules: Vec<(Focus, i32, u16)>,
    folded: std::collections::HashSet<String>,
    skin: crate::embed::skin::PlayerSkin,
    skins: bool,
    local_hover: bool,
    reference_colors: bool,
    density: u16,
    resume_scroll: usize,
    resume_fresh: bool,
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
        anyhow::ensure!(
            matches!(cfg.ui.graphical_palette.as_str(), "classic" | "theme"),
            "ui.graphical_palette must be classic or theme"
        );
        if !cfg.ui.graphical_rack_initialized {
            self.panels.eq = true;
            self.panels.album = true;
            self.panels.history = true;
            self.panels.playlist = true;
            self.panels.focus = Focus::Player;
            self.vis.mode = VisMode::Leds;
            self.saving.pending.insert(
                ("ui".into(), "graphical_rack_initialized".into()),
                crate::config::edit::Value::Bool(true),
            );
        }
        let root = graphical_root()?;
        session::serve_exclusive(
            &root,
            name,
            Rack {
                audio: Default::default(),
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
                eq_drag: None,
                max_offset: 0,
                overlay: false,
                modal_focus: None,
                context_anchor: None,
                playlist_bars: Scrollbars::new(),
                playlist_visible: 0,
                playlist_scrolled: None,
                cover_cache: None,
                modules: Vec::new(),
                skin: crate::embed::skin::PlayerSkin::new()?,
                skins: false,
                local_hover: false,
                reference_colors: cfg.ui.graphical_palette == "classic",
                density: 1,
                resume_scroll: 0,
                resume_fresh: false,
                folded: Default::default(),
            },
        )
    }
}
pub fn graphical_root() -> Result<PathBuf> {
    Ok(crate::paths::runtime_dir()?.join("graphical"))
}
impl Rack<'_> {
    fn palette(&self) -> Palette {
        if self.app.look.theme.id == "winamp-silver" {
            return crate::embed::skin::SILVER_PALETTE;
        }
        // Winamp Classic is the selected stacked-rack treatment, including its
        // original bitmap colors. Passing the legacy terminal palette through
        // the generic recoloring path turns its slate panels black, its green
        // phosphor neon, and its selected rows blue.
        if self.reference_colors || self.app.look.theme.id == "winamp-classic" {
            return crate::embed::skin::REFERENCE_PALETTE;
        }
        let t = &self.app.look.theme;
        Palette {
            // Kanagawa's raised background gives large graphical panes more
            // separation than its terminal base without washing out the hue.
            bg: color(if t.name == "Kanagawa" {
                t.header_bg
            } else {
                t.bg
            }),
            inset_bg: (t.name == "Kanagawa")
                .then(|| color(t.bg.mix(crate::theme::color::Rgb::new(0, 0, 0), 0.12))),
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
        let (repeat, shuffled, has_items) = {
            let q = a.player.queue.lock().unwrap();
            (q.repeat(), q.shuffled(), !q.is_empty())
        };
        PlayerRenderState {
            has_items,
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
            tech: if st.sample_rate.load(Relaxed) == 0 {
                String::new()
            } else {
                format!(
                    "{} · {:.1} kHz · {} bit · {} channels",
                    st.codec.load_full(),
                    st.sample_rate.load(Relaxed) as f64 / 1000.,
                    st.bit_depth.load(Relaxed),
                    st.channels.load(Relaxed)
                )
            },
            state: st.state(),
            loading: st.is_loading(),
            position: st.position_secs(),
            buffered: st.buffered_secs(),
            remote_audio: self.audio.active() || crate::audio::relay::enabled(),
            duration: st.duration_secs(),
            volume: a.player.volume(),
            repeat,
            shuffled,
            bit_perfect: st.bit_perfect.load(Relaxed),
            focused: a.panels.focus == Focus::Player && !a.overlay_open(),
            bands: a.vis.meters.bars().to_vec(),
            peaks: a.vis.meters.peaks().to_vec(),
            underruns,
            wave: a.vis.wave.clone(),
            vis_mode: a.vis.mode,
            seek_style: a.look.seek_style,
            bars: a.vis.bars,
            seek_phase: a.look.seek_phase,
        }
        .clear_empty_idle()
    }
    fn frame(&self, s: &mut Surface, r: R, c: &Colors, radius: u16, inset: bool) {
        if self.skins {
            self.skin.frame(s, r, inset);
        } else {
            classic::frame(s, r, c, radius, inset);
        }
    }
    fn selection_color(&self) -> String {
        let palette = self.palette();
        native::hex(if palette == crate::embed::skin::SILVER_PALETTE {
            palette.fg
        } else {
            palette.accent
        })
    }
    fn focus_outline(&self, s: &mut Surface) {
        self.focus_outline_at(s, R::new(0, 0, s.width, s.height));
    }
    fn focus_outline_at(&self, s: &mut Surface, rect: R) {
        let accent = self.selection_color();
        if self.skins {
            self.skin.focus_outline(s, rect, &accent);
        } else {
            native::focus_outline(s, rect, &accent, self.radius);
        }
    }
    fn modal_frame(&mut self, s: &mut Surface, rect: R, c: &Colors, inset: bool) {
        self.frame(s, rect, c, self.radius, inset);
        // A nested confirmation replaces its parent as the focused modal.
        self.modal_focus = Some(rect);
    }
    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        s: &mut Surface,
        r: R,
        c: &Colors,
        text: &str,
        action: &str,
        font: u16,
        active: bool,
    ) {
        if !self.skins {
            classic::button(s, r, c, text, action, font, active);
            return;
        }
        self.skin.button(s, r, action, active);
        let ink = self.skin.ink(action, active);
        let face = crate::embed::skin::button_rect(r);
        if !self.skin.button_label(s, face, text, &ink) {
            // Keep room for the presenting terminal's wider monospace font.
            let width = (text.chars().count() as u16 * 8 + 10).min(face.width);
            classic::label(
                s,
                R::new(
                    face.x + (face.width - width) / 2,
                    face.y,
                    width,
                    face.height,
                ),
                text,
                &ink,
                11,
                true,
            );
        }
        s.hits.push(HitRegion {
            rect: r,
            action: action.into(),
        });
    }
    fn window(&self, w: u16, h: u16, c: &Colors, title: &str, font: u16) -> Surface {
        let mut s = Surface::new(w, h, native::hex(self.palette().bg));
        s.background = native::hex(self.palette().bg);
        let focus = match title {
            "PARAMETRIC EQUALIZER" => Focus::Equalizer,
            "ALBUM" => Focus::Album,
            "ACTIVITY" => Focus::History,
            _ => Focus::Playlist,
        };
        self.frame(&mut s, R::new(0, 0, w, h), c, self.radius, false);
        // Pane content may use a different well palette; chrome always uses
        // the same title strip as EQ and the shared native player.
        let header_colors = crate::embed::skin::colors(&self.palette());
        native::panel_header(&mut s, w, &header_colors);
        let title_rect = R::new(18, 7, w.saturating_sub(166), 24);
        if !self.skins
            || !self.skin.label(
                &mut s,
                title_rect,
                title,
                &crate::embed::skin::tone(&self.palette(), "#bbc6d5", true),
                false,
            )
        {
            label(&mut s, &header_colors, title_rect, title, 12, true);
        }
        classic::label(
            &mut s,
            R::new(w.saturating_sub(114), 7, 78, 24),
            "settings",
            &c.dim,
            font.saturating_sub(2),
            false,
        );
        s.hits.push(HitRegion {
            rect: R::new(w.saturating_sub(118), 7, 82, 24),
            action: "settings".into(),
        });
        let action = format!(
            "close:{}",
            match focus {
                Focus::Equalizer => "eq",
                Focus::Album => "album",
                Focus::History => "activity",
                _ => "playlist",
            }
        );
        native::close_button(
            &mut s,
            R::new(w - 40, 7, 30, 24),
            c,
            &action,
            self.skin.hovered.as_deref() == Some(&action),
        );
        s
    }
    fn fold_module(&self, mut s: Surface, name: &str, c: &Colors) -> Surface {
        let folded = self.folded.contains(name);
        if folded {
            s.height = 38;
            s.hover_fills.clear();
            s.nodes.retain(|node| node.rect().y < 32);
            // The frame spans the original height; replace it with a compact header frame.
            s.nodes.retain(|node| node.rect().height <= 32);
            s.hits.retain(|hit| hit.rect.y < 32);
            let header = std::mem::take(&mut s.nodes);
            let width = s.width;
            self.frame(&mut s, R::new(0, 0, width, 38), c, self.radius, false);
            s.nodes.extend(header);
        }
        let glyph = if folded { "▸" } else { "▾" };
        if folded {
            classic::label(&mut s, R::new(8, 7, 9, 24), glyph, &c.dim, 9, false);
        }
        // Header controls sit on top of the foldable title strip. Hits are
        // resolved back to front, just like the painted controls.
        s.hits.insert(
            0,
            HitRegion {
                rect: R::new(7, 7, s.width.saturating_sub(140), 24),
                action: format!("fold:{name}"),
            },
        );
        let focus = match name {
            "eq" => Focus::Equalizer,
            "album" => Focus::Album,
            "activity" => Focus::History,
            _ => Focus::Playlist,
        };
        s.hits.insert(
            0,
            HitRegion {
                rect: R::new(0, 0, s.width, s.height),
                action: format!("focus:{name}"),
            },
        );
        if self.app.panels.focus == focus && !self.app.overlay_open() {
            self.focus_outline(&mut s);
        }
        s
    }
    fn eq(&mut self, w: u16, c: &Colors, font: u16) -> Surface {
        let eq_colors = Colors {
            inset: if self.palette() == crate::embed::skin::SILVER_PALETTE {
                crate::embed::skin::colors(&crate::embed::skin::REFERENCE_PALETTE).inset
            } else {
                c.inset.clone()
            },
            ..c.clone()
        };
        let c = &eq_colors;
        let mut s = self.window(w, EQ_HEIGHT, c, "PARAMETRIC EQUALIZER", font);
        let e = &self.app.eq;
        self.button(
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
        self.frame(&mut s, profile, c, 0, true);
        s.fill(
            R::new(
                profile.x + 2,
                profile.y + 2,
                profile.width - 4,
                profile.height - 4,
            ),
            &c.inset,
            0,
        );
        classic::label(
            &mut s,
            R::new(88, 40, profile_width - 48, 29),
            format!(
                "Profile: {}{}",
                e.active().name,
                if e.modified(e.profile) { " *" } else { "" }
            ),
            &c.accent,
            font.saturating_sub(2).max(1),
            false,
        );
        // A separate raster chevron cannot be mistaken for part of the name
        // and does not depend on the text font having a dropdown glyph.
        for row in 0..4 {
            s.fill(
                R::new(
                    profile.x + profile.width - 20 + row,
                    profile.y + 12 + row,
                    8 - row * 2,
                    1,
                ),
                &c.accent,
                0,
            );
        }
        s.hits.push(HitRegion {
            rect: profile,
            action: "eq-profiles".into(),
        });
        for (offset, width, text, action) in [
            (370, 64, "SAVE", "eq-save-current"),
            (298, 88, "SAVE AS", "eq-save"),
            (202, 82, "IMPORT", "eq-import"),
            (110, 96, "EXPORT", "eq-export"),
        ]
        .into_iter()
        {
            self.button(
                &mut s,
                R::new(w - offset, 40, width, 29),
                c,
                text,
                action,
                font,
                false,
            );
        }
        let preamp = e
            .active()
            .stages
            .iter()
            .find_map(|s| {
                if let Filter::Preamp { gain_db } = s.filter {
                    Some(gain_db)
                } else {
                    None
                }
            })
            .unwrap_or(0.);
        let preamp_x = profile.x + profile.width + 20;
        let preamp_width = w.saturating_sub(390 + preamp_x);
        let rect = R::new(preamp_x, 40, preamp_width, 29);
        classic::label(
            &mut s,
            rect,
            if preamp_width > 260 {
                format!("Preamp {preamp:+.1} dB · {}", e.profile_status(e.profile))
            } else if preamp_width > 130 {
                format!("Preamp {preamp:+.1} dB")
            } else {
                format!("Pre {preamp:+.1}dB")
            },
            &c.dim,
            if preamp_width > 130 { 12 } else { 10 },
            false,
        );
        s.hits.push(HitRegion {
            rect,
            action: "eq-preamp".into(),
        });
        let plot = R::new(55, 86, w.saturating_sub(410).max(80), 132);
        self.frame(&mut s, R::new(22, 78, plot.width + 44, 167), c, 0, true);
        s.fill(R::new(24, 80, plot.width + 40, 163), &c.inset, 0);
        let well = crate::theme::color::Rgb::parse_hex(&c.inset).unwrap();
        let green = crate::theme::color::Rgb::parse_hex(&c.accent).unwrap();
        let grid = well.mix(green, 0.12).to_hex();
        let zero = well.mix(green, 0.28).to_hex();

        for db in [-12, -6, 0, 6, 12] {
            let y = plot.y + ((12 - db) as u16 * (plot.height - 1) / 24);
            s.fill(
                R::new(plot.x, y, plot.width, 1),
                if db == 0 { &zero } else { &grid },
                0,
            );
            classic::label(
                &mut s,
                R::new(28, y.saturating_sub(7).max(82), 22, 16),
                format!("{db:+}"),
                &c.dim,
                font.saturating_sub(2),
                false,
            );
        }
        for (i, (f, tick)) in [
            (20., "20"),
            (50., "50"),
            (100., "100"),
            (200., "200"),
            (500., "500"),
            (1000., "1k"),
            (2000., "2k"),
            (5000., "5k"),
            (10000., "10k"),
            (20000., "20k"),
        ]
        .into_iter()
        .enumerate()
        {
            let x = plot.x + ((f64::log10(f / 20.) / 3.) * f64::from(plot.width - 1)) as u16;
            s.fill(R::new(x, plot.y, 1, plot.height), &grid, 0);
            classic::label(
                &mut s,
                R::new(
                    x.saturating_sub(if i == 9 { 20 } else { 8 }),
                    plot.y + plot.height + 3,
                    48,
                    20,
                ),
                tick,
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
            width: 2,
        });
        s.hits.push(HitRegion {
            rect: plot,
            action: "eq-plot".into(),
        });
        for (i, stage) in e.active().stages.iter().enumerate() {
            if let Filter::Biquad { frequency, .. } = stage.filter {
                let cx = plot.x
                    + ((frequency.clamp(20., 20000.) / 20.).log10() / 3.
                        * f64::from(plot.width - 1)) as u16;
                let db = e
                    .compiled
                    .magnitude_db_at_channel(frequency, e.compiled_rate, channel)
                    .clamp(-12., 12.);
                let cy = plot.y + ((12. - db) / 24. * f64::from(plot.height - 1)) as u16;
                let size = if i == e.stage { 18 } else { 16 };
                let r = R::new(
                    cx.saturating_sub(size / 2),
                    cy.saturating_sub(size / 2),
                    size,
                    size,
                );
                s.fill(r, if stage.enabled { &c.accent } else { &c.dim }, size / 2);
                s.nodes.push(Primitive::Border {
                    rect: r,
                    color: if i == e.stage {
                        c.ink.clone()
                    } else {
                        c.accent.clone()
                    },
                    radius: size / 2,
                });
                classic::label(
                    &mut s,
                    R::new(r.x + 4, r.y, size - 4, size),
                    e.band_number(i).to_string(),
                    &c.inset,
                    10,
                    true,
                );
                s.hits.push(HitRegion {
                    rect: R::new(cx.saturating_sub(12), cy.saturating_sub(12), 24, 24),
                    action: format!("eq-band:{i}"),
                });
            }
        }
        let x = w.saturating_sub(328);
        if let Some(stage) = e.active().stages.get(e.stage) {
            classic::label(
                &mut s,
                R::new(x, 83, 310, 24),
                format!(
                    "{}{} · {}{}",
                    if matches!(stage.filter, Filter::Preamp { .. }) {
                        "PREAMP"
                    } else {
                        "BAND "
                    },
                    if matches!(stage.filter, Filter::Preamp { .. }) {
                        String::new()
                    } else {
                        e.band_number(e.stage).to_string()
                    },
                    filter_caption(&stage.filter),
                    if stage.enabled { "" } else { " · BYPASS" }
                ),
                &c.accent,
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
                            "{}       {}",
                            match width {
                                Width::Q(_) => "Q",
                                Width::Bandwidth(_) => "Bandwidth",
                                Width::Slope(_) => "Slope",
                            },
                            match width {
                                Width::Q(q) => format!("{q:.2}"),
                                Width::Bandwidth(b) => format!("{b:.2}"),
                                Width::Slope(v) => format!("{v:.2}"),
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
                let y = 111 + i as u16 * 24;
                classic::label(&mut s, R::new(x, y, 108, 24), name, &c.dim, font, false);
                let r = R::new(x + 113, y, 188, 22);
                self.frame(&mut s, r, c, 0, true);
                s.fill(
                    R::new(r.x + 2, r.y + 2, r.width - 4, r.height - 4),
                    &c.inset,
                    0,
                );
                classic::label(
                    &mut s,
                    R::new(r.x + 10, y, 168, 24),
                    value.trim(),
                    &c.accent,
                    font.saturating_sub(2).max(1),
                    false,
                );
                s.hits.push(HitRegion {
                    rect: r,
                    action: action.into(),
                });
            }
            let r = R::new(x + 113, 183, 188, 22);
            self.frame(&mut s, r, c, 0, true);
            s.fill(
                R::new(r.x + 2, r.y + 2, r.width - 4, r.height - 4),
                &c.inset,
                0,
            );
            classic::label(
                &mut s,
                R::new(x, 183, 108, 22),
                "Channels",
                &c.dim,
                12,
                false,
            );
            let channels = if stage.channels == crate::audio::dsp::apo::ChannelMask::ALL {
                "L + R".to_string()
            } else {
                format!("0x{:x}", stage.channels.0)
            };
            classic::label(
                &mut s,
                R::new(r.x + 12, r.y, 164, r.height),
                channels,
                &c.accent,
                10,
                false,
            );
            s.hits.push(HitRegion {
                rect: r,
                action: "eq-channels".into(),
            });
            s.hits.push(HitRegion {
                rect: R::new(x, 83, 310, 24),
                action: "eq-type".into(),
            });
            self.button(
                &mut s,
                R::new(x, 218, 91, 29),
                c,
                "BYPASS",
                "eq-bypass",
                14,
                !stage.enabled,
            );
            self.button(
                &mut s,
                R::new(x + 101, 218, 91, 29),
                c,
                "DELETE",
                "eq-remove",
                14,
                false,
            );
        }

        s
    }
    fn album(&self, w: u16, c: &Colors, font: u16) -> Surface {
        let mut s = self.window(w, LEDGER_HEIGHT, c, "ALBUM", font);
        let album = self
            .app
            .current_uri()
            .and_then(|u| self.app.art.as_ref()?.album_for(&u));
        let item = self.app.player.current_item();
        let artwork = R::new(16, ALBUM_COVER_Y, ALBUM_COVER, ALBUM_COVER);
        if album.as_ref().is_none_or(|a| a.image.is_none()) {
            if self.skins {
                self.skin.pane_well(&mut s, artwork);
            } else {
                self.frame(&mut s, artwork, c, 0, true);
            }
            classic::label(
                &mut s,
                R::new(
                    24,
                    ALBUM_COVER_Y + ALBUM_COVER / 2 - 12,
                    ALBUM_COVER - 16,
                    24,
                ),
                "NO COVER",
                &c.dim,
                11,
                false,
            );
        }
        let detail = album.as_ref().and_then(|a| a.detail.as_ref());
        let x = ALBUM_COVER + 40;
        let text_width = w.saturating_sub(x + 16);
        let title = detail
            .and_then(|d| d.album.clone())
            .or_else(|| item.as_ref().and_then(|i| i.album.clone()))
            .unwrap_or_else(|| "No album metadata".into());
        let artist = detail
            .and_then(|d| d.artist.clone())
            .or_else(|| item.as_ref().and_then(|i| i.artist.clone()))
            .unwrap_or_default();
        classic::label(
            &mut s,
            R::new(x, 50, text_width, 28),
            title,
            &c.accent,
            16,
            true,
        );
        classic::label(
            &mut s,
            R::new(x, 80, text_width, 24),
            artist,
            &c.ink,
            13,
            false,
        );
        if let Some(genre) = detail.and_then(|d| d.genre.as_deref()) {
            classic::label(
                &mut s,
                R::new(x, 116, text_width, 22),
                genre,
                &c.dim,
                11,
                false,
            );
        }
        use std::sync::atomic::Ordering::Relaxed;
        let rate = self.app.player.state.sample_rate.load(Relaxed);
        let depth = self.app.player.state.bit_depth.load(Relaxed);
        let format = if rate > 0 {
            format!(
                "{} · {:.1} kHz · {}-bit",
                self.app.player.state.codec.load_full(),
                rate as f64 / 1000.,
                depth
            )
        } else {
            detail.and_then(|d| d.codec.clone()).unwrap_or_default()
        };
        classic::label(
            &mut s,
            R::new(x, 144, text_width, 22),
            format,
            &c.ink,
            11,
            false,
        );
        // The larger artwork owns the left column; its selector slides beside
        // it instead of consuming space below the image and the facts row.
        for (left, caption, action) in [(x, "‹", "cover-prev"), (x + 123, "›", "cover-next")] {
            self.button(
                &mut s,
                R::new(left, 178, 29, 29),
                c,
                caption,
                action,
                font,
                false,
            );
        }
        let choices = album
            .as_ref()
            .map(|a| format!("{} / {}", a.choice + 1, a.choices))
            .unwrap_or_else(|| "— / —".into());
        classic::label(
            &mut s,
            R::new(x + 48, 178, 64, 29),
            choices,
            &c.dim,
            11,
            false,
        );
        let album_items = self
            .app
            .queue
            .items
            .iter()
            .filter(|i| {
                item.as_ref().is_some_and(|current| {
                    i.album == current.album && i.album_artist == current.album_artist
                })
            })
            .collect::<Vec<_>>();
        let seconds = detail.map(|d| d.total_ms / 1000).unwrap_or_else(|| {
            album_items
                .iter()
                .filter_map(|i| i.duration_secs)
                .map(|v| v.max(0))
                .sum()
        });
        let year = detail
            .map(|d| d.original_year)
            .unwrap_or_else(|| {
                item.as_ref().and_then(|i| {
                    crate::library::release_year::from_context(i.uri.backing_path(), i.year)
                })
            })
            .map(|y| y.to_string())
            .unwrap_or_else(|| "—".into());
        let count = if item.is_some() {
            detail
                .map(|d| d.track_count)
                .unwrap_or(album_items.len())
                .to_string()
        } else {
            "—".into()
        };
        let duration = if item.is_some() {
            format!("{}:{:02}", seconds / 60, seconds % 60)
        } else {
            "—".into()
        };
        s.fill(R::new(16, 220, w - 32, 1), &c.raised, 0);
        let column = (w - 32) / 3;
        for (i, (heading, value)) in [
            ("ORIGINAL YEAR", year),
            (
                if detail.is_some() {
                    "TRACKS"
                } else {
                    "QUEUED TRACKS"
                },
                count,
            ),
            ("DURATION", duration),
        ]
        .into_iter()
        .enumerate()
        {
            let left = 16 + i as u16 * column;
            classic::label(
                &mut s,
                R::new(left, 226, column - 12, 18),
                heading,
                &c.dim,
                9,
                false,
            );
            classic::label(
                &mut s,
                R::new(left, 246, column - 12, 24),
                value,
                &c.ink,
                14,
                false,
            );
        }
        s.fill(R::new(16, 277, w - 32, 1), &c.raised, 0);
        let provenance = album
            .as_ref()
            .and_then(|a| a.source)
            .map(|source| format!("Artwork: {}", source.name()))
            .unwrap_or_else(|| "No cover available".into());
        classic::label(
            &mut s,
            R::new(16, 286, w.saturating_sub(244), 24),
            provenance,
            &c.dim,
            10,
            false,
        );
        for (left, width, caption, action) in [
            (w - 218, 100, "CHOOSE", "cover-choose"),
            (w - 108, 90, "RETRY", "cover-retry"),
        ] {
            self.button(
                &mut s,
                R::new(left, 284, width, 29),
                c,
                caption,
                action,
                font,
                false,
            );
        }
        for (left, caption, action) in [
            (w.saturating_sub(260), "artist ↗", "catalog-artist"),
            (w.saturating_sub(182), "album ↗", "catalog-album"),
        ] {
            let rect = R::new(left, 7, 74, 24);
            classic::label(&mut s, rect, caption, &c.dim, 10, false);
            s.hits.push(HitRegion {
                rect,
                action: action.into(),
            });
        }
        s
    }
    fn activity(&self, w: u16, c: &Colors, font: u16) -> Surface {
        self.activity_view(w, c, font, &self.app.activity.snapshot())
    }
    fn activity_view(
        &self,
        w: u16,
        c: &Colors,
        font: u16,
        snap: &crate::activity::Snapshot,
    ) -> Surface {
        use crate::activity::Provider;
        let mut s = self.window(w, LEDGER_HEIGHT, c, "ACTIVITY", font);
        let column = (w - 32) / 3;
        for (i, (heading, status, enabled)) in [
            ("LOCAL HISTORY", "Recording", true),
            (
                "LAST.FM",
                provider_state(snap, Provider::Lastfm).0,
                provider_state(snap, Provider::Lastfm).1,
            ),
            (
                "LISTENBRAINZ",
                provider_state(snap, Provider::Listenbrainz).0,
                provider_state(snap, Provider::Listenbrainz).1,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 16 + i as u16 * column;
            classic::label(
                &mut s,
                R::new(x, 42, column - 10, 18),
                heading,
                &c.dim,
                9,
                true,
            );
            classic::label(
                &mut s,
                R::new(x, 61, column - 10, 22),
                status,
                if enabled { &c.accent } else { &c.dim },
                11,
                false,
            );
        }
        s.fill(R::new(16, 87, w - 32, 1), &c.raised, 0);
        for (x, width, heading) in [
            (16, 72, "LISTENED"),
            (96, w - 212, "TRACK / ARTIST"),
            (w - 100, 84, "DELIVERY"),
        ] {
            classic::label(&mut s, R::new(x, 96, width, 20), heading, &c.dim, 9, true);
        }
        if snap.recent.is_empty() {
            classic::label(
                &mut s,
                R::new(96, 122, w - 112, 24),
                "No listening activity yet",
                &c.dim,
                11,
                false,
            );
        }
        let scroll = self
            .app
            .panels
            .history_scroll
            .min(snap.recent.len().saturating_sub(LEDGER_ROWS));
        for (i, row) in snap
            .recent
            .iter()
            .skip(scroll)
            .take(LEDGER_ROWS)
            .enumerate()
        {
            let y = 121 + i as u16 * 23;
            let action = format!("activity-row:{}", scroll + i);
            if self.skin.hovered.as_deref() == Some(&action) {
                s.fill(R::new(10, y, w - 20, 23), &c.title, 0);
            }
            classic::label(
                &mut s,
                R::new(16, y, 72, 23),
                format!(
                    "{}:{:02}",
                    row.listened_ms / 60000,
                    row.listened_ms / 1000 % 60
                ),
                &c.dim,
                11,
                false,
            );
            let title = row.title.as_deref().unwrap_or(&row.uri);
            let track = row
                .artist
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(|artist| format!("{title} — {artist}"))
                .unwrap_or_else(|| title.to_string());
            classic::label(&mut s, R::new(96, y, w - 212, 23), track, &c.ink, 11, false);
            let state = row.state();
            let color = if row.errors > 0 {
                native::hex(self.palette().error)
            } else if state == "playing" {
                c.accent.clone()
            } else if matches!(state.as_str(), "queued" | "partial") {
                crate::embed::skin::tone(&self.palette(), "#d7b579", true)
            } else {
                c.dim.clone()
            };
            classic::label(
                &mut s,
                R::new(w - 100, y, 84, 23),
                state.to_uppercase(),
                &color,
                9,
                true,
            );
            s.hits.push(HitRegion {
                rect: R::new(10, y, w - 20, 23),
                action,
            });
            if i + 1 < LEDGER_ROWS {
                s.fill(R::new(16, y + 23, w - 32, 1), &c.raised, 0);
            }
        }
        s.fill(R::new(16, 277, w - 32, 1), &c.raised, 0);
        let errors = snap.recent.iter().filter(|row| row.errors > 0).count();
        let pending = if errors > 0 {
            format!("{} pending · {errors} recent errors", snap.pending)
        } else {
            format!("{} pending submissions", snap.pending)
        };
        classic::label(
            &mut s,
            R::new(16, 286, w.saturating_sub(180), 24),
            pending,
            &c.dim,
            10,
            false,
        );
        self.button(
            &mut s,
            R::new(w - 156, 284, 140, 29),
            c,
            "RETRY FAILED",
            "activity-retry",
            font,
            false,
        );
        s
    }
    fn playlist(&mut self, w: u16, h: u16, c: &Colors, font: u16) -> Surface {
        let playlist_colors = Colors {
            inset: if self.palette() == crate::embed::skin::SILVER_PALETTE {
                crate::embed::skin::colors(&crate::embed::skin::REFERENCE_PALETTE).inset
            } else {
                c.inset.clone()
            },
            ..c.clone()
        };
        let c = &playlist_colors;
        let silver = self.palette() == crate::embed::skin::SILVER_PALETTE;
        let mut s = self.window(w, h, c, "PLAYLIST", font);
        let heading = if self.app.filter_query().trim().is_empty() {
            format!("· {}", self.app.queue.name)
        } else {
            format!(
                "· {} · /{}",
                self.app.queue.name,
                self.app.filter_query().trim()
            )
        };
        classic::label(
            &mut s,
            R::new(86, 7, w.saturating_sub(500), 24),
            crate::ui::panels::player::truncate(&heading, usize::from(w.saturating_sub(500) / 8)),
            &crate::embed::skin::tone(&self.palette(), "#bbc6d5", true),
            12,
            false,
        );
        // Match the gaps between the monospace labels, rather than their
        // starting positions: jump is three characters shorter than its peers.
        const HEADER_ADVANCE: u16 = 6;
        const HEADER_GAP: u16 = 38;
        let mut right = w.saturating_sub(114); // settings starts here
        for (text, action) in [
            ("gravity", "gravity"),
            ("sorting", "sorting"),
            ("jump", "playlist:jump"),
        ] {
            let width = text.len() as u16 * HEADER_ADVANCE;
            let x = right.saturating_sub(width + HEADER_GAP);
            let rect = R::new(x, 7, width + HEADER_GAP / 2, 24);
            right = x;
            classic::label(&mut s, rect, text, &c.dim, 10, false);
            s.hits.push(HitRegion {
                rect,
                action: action.into(),
            });
        }
        let well_top = 42;
        let well_height = s.height - well_top - 44;
        // Silver retains the earlier Classic dark playlist surface.
        // Start with the first row so no exposed strip forms a header divider.
        s.fill(R::new(10, well_top, w - 20, well_height), &c.inset, 0);
        if self.app.queue.items.is_empty() {
            classic::label(
                &mut s,
                R::new(22, 54, w - 44, 30),
                "No tracks · open a playlist or your library",
                &c.dim,
                font,
                false,
            );
            self.button(
                &mut s,
                R::new(22, 98, 140, 30),
                c,
                "PLAYLISTS",
                "playlists",
                font,
                false,
            );
            self.button(
                &mut s,
                R::new(170, 98, 140, 30),
                c,
                "LIBRARY",
                "library",
                font,
                false,
            );
        }
        let toolbar_y = s.height - 35;
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
            self.button(
                &mut s,
                R::new(14 + i as u16 * 63, toolbar_y, 57, 29),
                c,
                title,
                action,
                font,
                false,
            );
        }
        let total = self
            .app
            .queue
            .items
            .iter()
            .filter_map(|i| i.duration_secs)
            .map(|seconds| seconds.max(0))
            .sum::<i64>();
        let summary = if self.app.filter_query().trim().is_empty() {
            format!(
                "{} tracks · {}:{:02}",
                self.app.queue.items.len(),
                total / 60,
                total % 60
            )
        } else {
            format!(
                "{} of {} tracks",
                self.app.queue.matched_count,
                self.app.queue.items.len()
            )
        };
        let width = (summary.chars().count() as u16 * 7).min(w - 350);
        let summary_y = s.height - 30;
        classic::label(
            &mut s,
            R::new(w - 18 - width, summary_y, width, 24),
            summary,
            &c.dim,
            11,
            false,
        );
        let row_font = font.saturating_sub(1);
        let row_h = 22;
        let visible = ((s.height - 86) / row_h) as usize;
        self.playlist_visible = visible;
        let rows = &self.app.queue.rows;
        let selected_row = rows
            .row_of_track(self.app.queue.cursor)
            .unwrap_or_else(|| rows.anchor_row(self.app.queue.cursor));
        self.app.queue.scroll = self
            .app
            .queue
            .scroll
            .min(rows.len().saturating_sub(visible));
        // A grabbed scrollbar owns the viewport. Keep its position after
        // release too, including a viewport containing only folded headings.
        if self.playlist_scrolled != Some(self.app.queue.cursor) {
            self.playlist_scrolled = None;
            self.app.queue.scroll =
                PlaylistView::clamp_scroll(selected_row, self.app.queue.scroll, visible);
            if visible > 1 {
                let anchor = rows.anchor_row(self.app.queue.cursor);
                self.app.queue.scroll =
                    PlaylistView::clamp_scroll(anchor, self.app.queue.scroll, visible)
                        .min(self.app.queue.scroll);
            }
        }
        let (marked, playing) = {
            let q = self.app.player.queue.lock().unwrap();
            (
                q.view()
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| self.app.queue.tagged.contains(i))
                    .map(|(slot, _)| slot)
                    .collect::<std::collections::HashSet<_>>(),
                q.current_index().map(|_| q.view_cursor()),
            )
        };
        let grouped = self.app.session_grouped();
        let index_width = self.app.queue.items.len().to_string().len().max(4) as u16 * 7 + 7;
        let body = R::new(14, well_top, w - 44, visible as u16 * row_h);
        // Blend the selection into this playlist's actual well so it reads as
        // a light overlay across themes, including Silver's Classic well.
        let [r, g, b] = self.palette().selected;
        let selection_fill = crate::theme::color::Rgb::parse_hex(&c.inset)
            .expect("playlist well color")
            .mix(crate::theme::color::Rgb::new(r, g, b), 0.50)
            .to_hex();
        s.hits.push(HitRegion {
            rect: body,
            action: "playlist:body".into(),
        });
        for (line, row) in self
            .app
            .queue
            .rows
            .rows()
            .iter()
            .skip(self.app.queue.scroll)
            .take(visible)
            .enumerate()
        {
            let index = self.app.queue.scroll + line;
            let r = R::new(body.x, body.y + line as u16 * row_h, body.width, row_h);
            let action = format!("playlist:{index}");
            let hovered = !self.local_hover && self.skin.hovered.as_deref() == Some(&action);
            if let playlist::Row::Section {
                label: title,
                tracks,
                folded,
                playing,
                ..
            } = row
            {
                // Same seam as ASCII: fold control, year/album label, an
                // uninterrupted rule across the spare space, then track count.
                if self.local_hover {
                    s.hover_fills.push(HoverFill {
                        node: s.nodes.len() as u16,
                        color: c.title.clone(),
                    });
                    s.fill(r, &c.inset, 0);
                } else if hovered {
                    s.fill(r, &c.title, 0);
                }
                native::heading_depth(&mut s, r, if hovered { &c.title } else { &c.inset }, &c.ink);
                let count = playlist::section_count(*tracks, *folded);
                // Leave room for the presenting terminal's wider mono font.
                let count_w = count.width() as u16 * 8 + 12;
                let count_x = r.x + r.width - count_w - 6;
                let title_x = r.x + 32;
                let title_w = count_x.saturating_sub(title_x + 24);
                let title_node = s.nodes.len() as u16;
                classic::label(
                    &mut s,
                    R::new(title_x, r.y, title_w, row_h),
                    title.clone(),
                    if *playing { &c.accent } else { &c.ink },
                    row_font,
                    true,
                );
                s.text_rules.push(starkit::native_surface::TextRule {
                    node: title_node,
                    end: count_x - 8,
                    gap: 10,
                    width: 1,
                    color: c.raised.clone(),
                });
                classic::label(
                    &mut s,
                    R::new(count_x, r.y, count_w, row_h),
                    count,
                    &c.dim,
                    row_font,
                    false,
                );
                // Pixel geometry keeps both signs centered on the same axis;
                // font bearings made the minus sit above the section's seam.
                let fold_hit = R::new(r.x + 7, r.y + 2, 18, 18);
                let fold = R::new(r.x + 9, r.y + 4, 14, 14);
                if hovered {
                    s.fill(fold, &c.title, 0);
                    s.nodes.push(Primitive::Border {
                        rect: fold,
                        color: c.dim.clone(),
                        radius: 0,
                    });
                }
                s.fill(R::new(fold.x + 4, fold.y + 6, 6, 2), &c.ink, 0);
                if *folded {
                    s.fill(R::new(fold.x + 6, fold.y + 4, 2, 6), &c.ink, 0);
                }
                s.hits.push(HitRegion {
                    rect: fold_hit,
                    action: action.clone(),
                });
            } else if let playlist::Row::Track(i) = row {
                let Some(item) = self.app.queue.items.get(*i) else {
                    continue;
                };
                let selected = index == selected_row;
                let active = playing == Some(*i);
                if self.local_hover {
                    s.hover_fills.push(HoverFill {
                        node: s.nodes.len() as u16,
                        color: if selected {
                            selection_fill.clone()
                        } else {
                            c.title.clone()
                        },
                    });
                    s.fill(r, if selected { &selection_fill } else { &c.inset }, 0);
                } else if selected {
                    s.fill(r, &selection_fill, 0);
                } else if hovered {
                    s.fill(r, &c.title, 0);
                }
                let ink = if item.unplayable {
                    &c.dim
                } else if active {
                    &c.accent
                } else {
                    &c.ink
                };
                let column_ink = |silver_ink: &str| -> String {
                    if active || item.unplayable {
                        ink.clone()
                    } else if silver {
                        silver_ink.into()
                    } else if selected {
                        ink.clone()
                    } else {
                        c.dim.clone()
                    }
                };
                let x = r.x + 6;
                classic::label(
                    &mut s,
                    R::new(x, r.y, 12, row_h),
                    if active {
                        ">"
                    } else if item.unplayable {
                        "!"
                    } else {
                        ""
                    },
                    ink,
                    row_font,
                    active,
                );
                classic::label(
                    &mut s,
                    R::new(x + 14, r.y, index_width, row_h),
                    format!("{:>4}{}", i + 1, if marked.contains(i) { '+' } else { '.' }),
                    &column_ink("#9daec4"),
                    row_font,
                    active,
                );
                let mut name_x = x + 14 + index_width + 8;
                if grouped {
                    s.fill(R::new(name_x, r.y, 1, row_h), &c.raised, 0);
                    name_x += 10;
                    classic::label(
                        &mut s,
                        R::new(name_x, r.y, 42, row_h),
                        format!("{:>5}", rows.album_index(*i, item)),
                        &column_ink("#b8b09f"),
                        row_font,
                        active,
                    );
                    name_x += 52;
                }
                let duration_x = r.x + r.width - 62;
                classic::label(
                    &mut s,
                    R::new(name_x, r.y, duration_x.saturating_sub(name_x + 10), row_h),
                    playlist::track_name(item),
                    ink,
                    row_font,
                    active,
                );
                classic::label(
                    &mut s,
                    R::new(duration_x, r.y, 58, row_h),
                    item.duration_secs
                        .map(|seconds| crate::ui::digits::clock(seconds as f64))
                        .unwrap_or_else(|| "-:--".into()),
                    &column_ink("#95acb5"),
                    row_font,
                    active,
                );
            }
            s.hits.push(HitRegion { rect: r, action });
        }
        if rows.is_empty() && !self.app.queue.items.is_empty() {
            classic::label(
                &mut s,
                R::new(body.x + 12, body.y + 8, body.width - 24, 28),
                "No tracks match this filter. Press / to change or clear it.",
                &c.dim,
                row_font,
                false,
            );
        }
        // Shared KIT geometry measures rendered rows (headings and folds
        // included). Only the thumb is painted, inside the well's border.
        let track = Rect::new(w - 28, body.y, 16, body.height);
        let mut bars = Scrollbars::new();
        bars.record_viewport_with_minimum(
            (),
            track,
            rows.len() as u32,
            self.app.queue.scroll as u32,
            visible as u32,
            24,
        );
        if bars.visible().next().is_some() {
            s.hits.push(HitRegion {
                rect: R::new(track.x, track.y, track.width, track.height),
                action: "playlist:scroll-track".into(),
            });
        }

        s
    }
    fn scroll_playlist_to(&mut self, above: u32) {
        self.app.panels.focus = Focus::Playlist;
        let rows = &self.app.queue.rows;
        self.app.queue.scroll =
            (above as usize).min(rows.len().saturating_sub(self.playlist_visible));
        // Browsing retains selection for an explicit jump back to the track.
        self.playlist_scrolled = Some(self.app.queue.cursor);
    }

    fn begin_eq_drag(&mut self, x: u16, y: u16) {
        self.eq_drag = None;
        let Some(plot) = self
            .surface
            .as_ref()
            .and_then(|s| s.hits.iter().find(|h| h.action == "eq-plot"))
            .map(|h| h.rect)
            .filter(|r| r.width > 1 && r.height > 1)
        else {
            return;
        };
        let stage = self.app.eq.stage;
        if let Some(Filter::Biquad {
            frequency, gain_db, ..
        }) = self.app.eq.active().stages.get(stage).map(|s| &s.filter)
        {
            self.eq_drag = Some(EqDrag {
                stage,
                origin: [x, y],
                plot,
                frequency: *frequency,
                gain_db: *gain_db,
            });
        }
    }

    fn drag_eq(&mut self, x: u16, y: u16) {
        let Some(drag) = &self.eq_drag else { return };
        let dx = f64::from(x) - f64::from(drag.origin[0]);
        let dy = f64::from(y) - f64::from(drag.origin[1]);
        // Keep full precision, including when returning to the press position.
        // The graph spans three frequency decades and 24 dB vertically.
        let frequency = if dx == 0. {
            drag.frequency
        } else {
            (drag.frequency * 1000f64.powf(dx / f64::from(drag.plot.width - 1))).clamp(20., 20000.)
        };
        let gain_db = if dy == 0. {
            drag.gain_db
        } else {
            (drag.gain_db - dy * 24. / f64::from(drag.plot.height - 1)).clamp(
                -crate::audio::dsp::apo::MAX_GAIN_DB,
                crate::audio::dsp::apo::MAX_GAIN_DB,
            )
        };
        if let Some(stage) = self.app.eq.active_mut().stages.get_mut(drag.stage) {
            if let Filter::Biquad {
                frequency: hz,
                gain_db: db,
                ..
            } = &mut stage.filter
            {
                if *hz == frequency && *db == gain_db {
                    return;
                }
                *hz = frequency;
                *db = gain_db;
                self.app.eq.enabled = true;
                self.app.apply_eq();
                self.eq_dirty = true;
            }
        }
    }
    /// Preserve ordinary seek/module shortcuts while making drawn buttons
    /// reachable without a pointer. Order comes from the emitted surface.
    fn player_control_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.overlay || self.cells || !self.skins || self.app.panels.focus != Focus::Player {
            return false;
        }
        if modifiers == KeyModifiers::ALT && matches!(code, KeyCode::Left | KeyCode::Right) {
            let mut actions = Vec::new();
            if let Some(surface) = &self.surface {
                for hit in &surface.hits {
                    if matches!(
                        hit.action.as_str(),
                        "previous" | "play" | "pause" | "stop" | "next" | "shuffle" | "repeat"
                    ) && !actions.contains(&hit.action)
                    {
                        actions.push(hit.action.clone());
                    }
                }
            }
            if actions.is_empty() {
                return false;
            }
            let current = self
                .skin
                .focused
                .as_ref()
                .and_then(|name| actions.iter().position(|a| a == name));
            let index = match (current, code) {
                (Some(i), KeyCode::Left) => (i + actions.len() - 1) % actions.len(),
                (Some(i), _) => (i + 1) % actions.len(),
                (None, KeyCode::Left) => actions.len() - 1,
                (None, _) => 0,
            };
            self.skin.focused = Some(actions[index].clone());
            return true;
        }
        if modifiers.is_empty() && code == KeyCode::Enter {
            if let Some(name) = self.skin.focused.clone() {
                // Recheck the current surface: a resize can remove a control.
                if self
                    .surface
                    .as_ref()
                    .is_some_and(|s| s.hits.iter().any(|h| h.action == name))
                {
                    self.action(&name, 0, 0);
                    return true;
                }
                self.skin.focused = None;
            }
        }
        if code == KeyCode::Esc {
            self.skin.focused = None;
        }
        false
    }
    fn use_theme_palette(&mut self) {
        self.reference_colors = false;
        self.app.saving.pending.insert(
            ("ui".into(), "graphical_palette".into()),
            crate::config::edit::Value::Str("theme".into()),
        );
    }
    fn action(&mut self, name: &str, x: u16, y: u16) {
        tracing::debug!(action = name, x, y, "Graphical control activated");
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
            "play" => a.play(),
            "pause" => a.pause(),
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
            "sorting" => a.open_filter(),
            "gravity" => a.open_gravity(),
            "queue-remove" => a.handle(Action::RemoveTagged),
            "queue-tag" => a.handle(Action::TagRow),
            "playlist:jump" => {
                let has_active_track = a.player.state.state() != PlayState::Stopped
                    && a.player.queue.lock().unwrap().current_index().is_some();
                if has_active_track {
                    a.focus_playing_track();
                } else {
                    a.focus_selected_track();
                }
                self.playlist_scrolled = None;
            }
            "help" => a.handle(Action::Help),
            "theme-next" => {
                a.handle(Action::NextTheme);
                self.use_theme_palette();
            }
            "close:app" => a.handle(Action::Quit),
            "close:eq" => a.handle(Action::ToggleEqPanel),
            "close:album" => a.handle(Action::ToggleAlbumPanel),
            "close:activity" => a.handle(Action::ToggleHistoryPanel),
            "close:playlist" => a.handle(Action::TogglePlaylistPanel),
            "repeat" => a.handle(Action::CycleRepeat),
            "shuffle" => a.handle(Action::ToggleShuffle),
            "eq-prev" => a.handle(Action::PrevEqPreset),
            "eq-next" => a.handle(Action::NextEqPreset),
            "eq-profiles" => a.open_eq_profiles(),
            "eq-save-current" => a.save_eq_profile(),
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
            "eq-preamp" => {
                if let Some(i) =
                    a.eq.active()
                        .stages
                        .iter()
                        .position(|s| matches!(s.filter, Filter::Preamp { .. }))
                {
                    a.eq.stage = i;
                    a.begin_eq_value(EqField::Gain);
                }
            }
            _ => {
                if let Some(i) = name
                    .strip_prefix("eq-band:")
                    .or_else(|| name.strip_prefix("eq-select:"))
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    a.eq.stage = i;
                    a.panels.focus = Focus::Equalizer;
                }
                if name.starts_with("activity-row:") {
                    a.panels.focus = Focus::History;
                }
                if let Some(i) = name
                    .strip_prefix("playlist:")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    a.panels.focus = Focus::Playlist;
                    match a.queue.rows.rows().get(i).cloned() {
                        Some(playlist::Row::Track(track)) => {
                            a.set_cursor(track);
                            self.playlist_scrolled = Some(track);
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
fn append(
    target: &mut Surface,
    mut source: Surface,
    x: u16,
    y: i32,
    visible: std::ops::Range<u16>,
    density: u16,
    name: &str,
) {
    // Clip offscreen modules without shrinking their font or altering rack geometry.
    target.assets.append(&mut source.assets);
    let limit = target.height.min(visible.end);
    let translate = |r: R| -> Option<R> {
        let yy = i32::from(r.y) + y;
        let start = yy.max(i32::from(visible.start));
        let end = (yy + i32::from(r.height)).min(i32::from(limit));
        if end <= start {
            return None;
        }
        Some(R::new(r.x + x, start as u16, r.width, (end - start) as u16))
    };
    let mut node_indices = vec![None; source.nodes.len()];
    for (index, node) in source.nodes.iter_mut().enumerate() {
        let original = node.rect();
        let Some(rect) = translate(original) else {
            continue;
        };
        if matches!(
            node,
            Primitive::Text { .. }
                | Primitive::Icon { .. }
                | Primitive::Sprite { insets: None, .. }
        ) && rect.height != original.height
        {
            continue;
        }
        if let Primitive::Sprite {
            source,
            insets: Some(insets),
            ..
        } = node
        {
            if rect.height != original.height {
                let cut_top = rect.y as i32 - (original.y as i32 + y);
                let cut_bottom = original.height as i32 - cut_top - rect.height as i32;
                // Atlas crops and corner insets remain physical pixels even
                // while destination rectangles use the logical rack density.
                let height = rect.height * density;
                let top = insets[1]
                    .saturating_sub(cut_top as u16 * density)
                    .min(height);
                let bottom = insets[3]
                    .saturating_sub(cut_bottom as u16 * density)
                    .min(height - top);
                source.y += insets[1] - top;
                source.height -= insets[1] - top + insets[3] - bottom;
                insets[1] = top;
                insets[3] = bottom;
            }
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
        node_indices[index] = Some(target.nodes.len() as u16);
        target.nodes.push(node.clone());
    }
    for mut rule in source.text_rules {
        if let Some(node) = node_indices[usize::from(rule.node)] {
            rule.node = node;
            rule.end += x;
            target.text_rules.push(rule);
        }
    }
    for hover in source.hover_fills {
        if let Some(node) = node_indices[usize::from(hover.node)] {
            target.hover_fills.push(HoverFill {
                node,
                color: hover.color,
            });
        }
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
        self.audio.capabilities(self.app, c);
        self.paths = c.native_paths;
        self.local_hover = c.surface_hover;
        self.skins = c.native_skins && c.native_surfaces;
        self.cells = c.cell_presentation.unwrap_or(false);
    }
    fn detached(&mut self) {
        self.audio.detached(self.app);
    }
    fn media(&mut self, message: starkit::terminal_graphics::media::ToHost) {
        self.audio.receive(self.app, message);
    }
    fn media_chunks(&mut self) -> Vec<starkit::terminal_graphics::protocol::ServerMessage> {
        self.audio.chunks(self.app)
    }
    fn effects(&mut self) -> Vec<starkit::terminal_graphics::protocol::ServerMessage> {
        self.audio.effects(self.app)
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
        self.playlist_bars.release();
        self.drag = None;
        self.eq_drag = None;
    }
    fn scene(&mut self, v: Viewport) -> Scene {
        if v != self.viewport {
            if self.eq_dirty {
                self.app.finish_eq_edit();
                self.eq_dirty = false;
            }
            self.drag = None;
            self.eq_drag = None;
            self.playlist_bars.release();
            self.skin.hovered = None;
        }
        self.viewport = v;
        self.revision += 1;
        self.overlay = false;
        self.modal_focus = None;
        if self.app.over.resume.is_none() {
            self.resume_scroll = 0;
            self.resume_fresh = false;
        }
        self.playlist_bars.begin_frame();
        if self.app.over.playlist_add.is_none() {
            self.context_anchor = None;
        }
        self.app.sync_view();
        // Native scenes bypass App::draw, including its initial/open-panel
        // artwork lookup. Track revision changes alone do not cover these.
        self.app.pump_album();
        self.app.prepare_playlist_rows();
        self.app.panels.cell_geometry = None;
        self.app.panels.last_area = Rect::new(0, 0, v.columns, v.rows);
        let area = Rect::new(0, 0, v.columns, v.rows);
        let mut buffer = starkit::ratatui::buffer::Buffer::empty(area);
        if self.cells || !self.paths || v.width < 720 || v.height < 360 {
            self.app.draw(area, &mut buffer);
            self.surface = None;
            return Scene::from_buffer(&buffer, v, self.revision);
        }
        let palette = self.palette();
        let c = crate::embed::skin::colors(&palette);
        let font = 12;
        if self.app.over.library.is_some() || self.app.over.files.is_some() {
            self.app.draw(area, &mut buffer);
            let surface = classic::form(
                &buffer,
                v.width as u16,
                v.height as u16,
                font,
                &c,
                self.radius,
            );
            let mut scene = Scene::from_buffer(
                &starkit::ratatui::buffer::Buffer::empty(area),
                v,
                self.revision,
            );
            scene.background = native::hex(self.palette().bg);
            scene.foreground = native::hex(self.palette().fg);
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
        let density = if self.skins
            && v.width >= 1472
            && v.height >= 600
            && v.height / u32::from(v.rows.max(1)) >= 32
        {
            2
        } else {
            1
        };
        self.density = density;
        let w = v.width as u16 / density;
        let h = v.height as u16 / density;
        let footer_y = h - font - 12;
        let mut surface = Surface::new(w, h, "transparent".into());
        if self.skins {
            if let Err(error) = self.skin.prepare(&palette, density, self.radius == 0) {
                tracing::error!(%error,"Cannot prepare rack artwork");
                self.skins = false;
            }
        }
        let player_h = 230.min(h - 40);
        let graphics = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let state = self.player();
        let mut player = if self.skins {
            self.skin
                .logical_surface(
                    w,
                    player_h,
                    graphics,
                    &palette,
                    &state,
                    self.radius,
                    density,
                )
                .unwrap_or_else(|error| {
                    tracing::error!(%error,"Cannot compose player skin");
                    native::classic_surface(w, player_h, graphics, &palette, &state, self.radius)
                })
        } else {
            native::classic_surface(w, player_h, graphics, &palette, &state, self.radius)
        };
        player.hits.insert(
            0,
            HitRegion {
                rect: R::new(0, 0, w, player_h),
                action: "focus:player".into(),
            },
        );
        if self.app.panels.focus == Focus::Player && !self.app.overlay_open() {
            self.focus_outline(&mut player);
        }
        append(&mut surface, player, 0, 0, 0..footer_y, density, "player");
        classic::label(
            &mut surface,
            R::new(w - 114, 7, 70, 24),
            "settings",
            &c.dim,
            font.saturating_sub(2),
            false,
        );
        surface.hits.push(HitRegion {
            rect: R::new(w - 118, 7, 78, 24),
            action: "settings:player".into(),
        });
        native::close_button(
            &mut surface,
            R::new(w - 40, 7, 30, 24),
            &c,
            "close:app",
            self.skin.hovered.as_deref() == Some("close:app"),
        );
        let top = player_h + 8;
        self.modules.clear();
        let mut modules = Vec::new();
        let mut y = i32::from(top);
        if self.app.panels.eq {
            let s = self.eq(w, &c, font);
            let s = self.fold_module(s, "eq", &c);
            let hh = s.height;
            self.modules.push((Focus::Equalizer, y, hh));
            modules.push((s, 0, y, "eq"));
            y += i32::from(hh + 8);
        }
        let module_w = w;
        // Album and Activity retain their original surfaces; the darker display
        // wells belong only to the player, EQ and playlist.
        let detail_colors = crate::embed::skin::colors(&Palette {
            inset_bg: None,
            ..palette
        });
        let album_y = y;
        let paired = module_w >= 1040 && self.app.panels.album && self.app.panels.history;
        let album_w = if paired { (module_w - 8) / 2 } else { module_w };
        let mut album_height = 0;
        if self.app.panels.album {
            let s = self.album(album_w, &detail_colors, font);
            let s = self.fold_module(s, "album", &detail_colors);
            album_height = s.height;
            self.modules.push((Focus::Album, y, s.height));
            modules.push((s, 0, y, "album"));
            if !paired {
                y += i32::from(album_height + 8);
            }
        }
        if self.app.panels.history {
            let activity_w = if paired {
                module_w - album_w - 8
            } else {
                module_w
            };
            let s = self.activity(activity_w, &detail_colors, font);
            let s = self.fold_module(s, "activity", &detail_colors);
            let row_height = if paired {
                s.height.max(album_height)
            } else {
                s.height
            };
            self.modules.push((Focus::History, y, s.height));
            modules.push((s, if paired { 8 + album_w } else { 0 }, y, "activity"));
            y += i32::from(row_height + 8);
        }
        if self.app.panels.playlist {
            // Fill the remaining rack space above the status footer. Use the
            // unscrolled position so scrolling never changes the module size.
            let playlist_height = (i32::from(h) - i32::from(font + 20) - y).max(260) as u16;
            let s = self.playlist(w, playlist_height, &c, font);
            let s = self.fold_module(s, "playlist", &c);
            let hh = s.height;
            self.modules.push((Focus::Playlist, y, hh));
            modules.push((s, 0, y, "playlist"));
            y += i32::from(hh + 8);
        }
        self.max_offset = (y - i32::from(h) + i32::from(font + 12)).max(0) as u16;
        self.offset = self.offset.min(self.max_offset);
        // Clamp against the new layout before painting, so resizing or closing
        // a module cannot leave a frame at the previous scroll offset.
        for (module, x, y, name) in modules {
            append(
                &mut surface,
                module,
                x,
                y - i32::from(self.offset),
                top..footer_y,
                density,
                name,
            );
        }
        let album_y = album_y - i32::from(self.offset);
        // Use the ASCII status model, but lay out native segments in pixels.
        // A flattened terminal row can clip away the right-hand indicators
        // when the native and terminal font advances differ.
        let (shuffled, repeat) = {
            let queue = self.app.player.queue.lock().unwrap();
            (queue.shuffled(), queue.repeat())
        };
        let segments = status_indicators(
            self.app.vis.mode,
            shuffled,
            repeat,
            self.app.player.crossfade(),
        );
        let advance = 8;
        let status_width = indicator_width(&segments) * advance;
        let status_x = w.saturating_sub(status_width + 8);
        classic::label(
            &mut surface,
            R::new(8, footer_y, 64, font + 12),
            "? help",
            &c.accent,
            font,
            false,
        );
        let theme = format!(
            "Theme: {}",
            if self.reference_colors {
                "Classic"
            } else {
                &self.app.look.theme.name
            }
        );
        let theme_width = (theme.width() as u16 * advance).min(status_x.saturating_sub(104));
        let theme_rect = R::new(88, footer_y, theme_width, font + 12);
        classic::label(&mut surface, theme_rect, theme, &c.ink, font, false);
        surface.hits.push(HitRegion {
            rect: theme_rect,
            action: "theme-next".into(),
        });
        let message_x = 88 + theme_width + 24;
        let mut x = status_x;
        for (index, (text, lit)) in segments.iter().enumerate() {
            if index > 0 {
                let width = SEPARATOR.width() as u16 * advance;
                classic::label(
                    &mut surface,
                    R::new(x, footer_y, width, font + 12),
                    SEPARATOR,
                    &c.dim,
                    font,
                    false,
                );
                x += width;
            }
            let width = text.width() as u16 * advance;
            let ink = if palette == crate::embed::skin::REFERENCE_PALETTE {
                if *lit {
                    c.accent.clone()
                } else {
                    c.dim.clone()
                }
            } else {
                hex(if *lit {
                    self.app.look.theme.transport_toggle_on_fg
                } else {
                    self.app.look.theme.transport_toggle_off_fg
                })
            };
            classic::label(
                &mut surface,
                R::new(x, footer_y, width, font + 12),
                text,
                &ink,
                font,
                false,
            );
            x += width;
        }
        if let Some(status) = self.app.audio_output.as_deref().or_else(|| {
            self.app
                .status
                .as_ref()
                .filter(|(_, t)| t.elapsed() < Duration::from_secs(4))
                .map(|(s, _)| s.as_str())
        }) {
            classic::label(
                &mut surface,
                R::new(
                    message_x,
                    footer_y,
                    status_x.saturating_sub(message_x + 16),
                    font + 12,
                ),
                status,
                &c.dim,
                font,
                false,
            );
        }
        surface.hits.push(HitRegion {
            rect: R::new(0, footer_y, 72, font + 12),
            action: "help".into(),
        });
        if self.app.overlay_open()
            && !self.eq_value_popup(&mut surface, &c)
            && !self.eq_library_popup(&mut surface, &c)
            && !self.resume_popup(&mut surface, &c)
            && !self.filter_popup(&mut surface, &c)
            && !self.list_popup(&mut surface, &c)
        {
            self.overlay = true;
            let mut modal = starkit::ratatui::buffer::Buffer::empty(area);
            // Ordinary mouse dispatch uses the padded application area.
            // Render the source cells there too, including at 2x density.
            let modal_area = self.app.regions(area).map_or(area, |regions| regions.area);
            self.app.draw_overlays(modal_area, &mut modal);
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
                self.modal_frame(&mut surface, r, &c, false);
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
                surface.hover_fills.clear();
                surface.hits.clear();
                surface.hits.push(HitRegion {
                    rect: r,
                    action: "modal".into(),
                });
            }
        }
        if self.overlay {
            surface.hover_fills.clear();
            if let Some(rect) = self.modal_focus {
                self.focus_outline_at(&mut surface, rect);
            }
        }
        if density > 1 {
            surface = surface.at_density(density).expect("bounded rack density");
        }
        if let Some(hit) = surface
            .hits
            .iter()
            .find(|h| h.action == "playlist:scroll-track")
        {
            let r = hit.rect;
            self.playlist_bars.record_viewport_with_minimum(
                (),
                Rect::new(r.x, r.y, r.width, r.height),
                self.app.queue.rows.len() as u32,
                self.app.queue.scroll as u32,
                self.playlist_visible as u32,
                24 * density,
            );
            if let Some((track, thumb)) = self.playlist_bars.visible().next() {
                surface.fill(
                    R::new(
                        track.x + 6 * density,
                        track.y + thumb.start,
                        4 * density,
                        thumb.len,
                    ),
                    &self.selection_color(),
                    2 * density,
                );
            }
        }
        // Odd viewport dimensions leave a one-pixel gutter at 2x density.
        // Pad the canvas instead of scaling every fixed atlas crop to fit it.
        surface.width = v.width as u16;
        surface.height = v.height as u16;
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
            && !self.folded.contains("album")
            && album_y + i32::from(ALBUM_COVER_Y) >= i32::from(top)
            && album_y + i32::from(ALBUM_COVER_Y) + i32::from(ALBUM_COVER)
                < i32::from(h - font - 12)
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
                    let cover = ALBUM_COVER;
                    let slot = R::new(
                        16 * density,
                        (album_y + i32::from(ALBUM_COVER_Y)) as u16 * density,
                        cover * density,
                        cover * density,
                    );
                    let (rect, canvas, bounds) = cover_layout(v, slot);
                    let key = CoverKey {
                        serial,
                        canvas,
                        bounds,
                        border: 2 * density,
                        background: c.panel.clone(),
                        silver_gradient: palette == crate::embed::skin::SILVER_PALETTE,
                        outline: native::hex(palette.border),
                    };
                    if self.cover_cache.as_ref().is_none_or(|old| old.key != key) {
                        self.cover_cache = starkit::terminal_graphics::assets::encode_png(
                            &cover_raster(image, &key),
                        )
                        .ok()
                        .map(|png| CoverCache {
                            key,
                            id: format!("album-{serial}-{}", self.revision),
                            png,
                        });
                    }
                    if let Some(cached) = &self.cover_cache {
                        scene.components.push(Component::Image {
                            rect,
                            id: cached.id.clone(),
                            png: Some(cached.png.clone()),
                            scale: starkit::terminal_graphics::protocol::ImageScale::One,
                            zoom: 100,
                        });
                        surface.hits.push(HitRegion {
                            rect: slot,
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
        // Preserve the terminal around pane borders, between modules and in
        // the footer; only the panes themselves own opaque themed backgrounds.
        scene.background = "transparent".into();
        scene.spans.clear();
        scene.foreground = native::hex(self.palette().fg);
        scene.accent = native::hex(self.palette().accent);
        scene.border = native::hex(self.palette().border);
        self.surface = Some(surface);
        scene
    }
    fn input(&mut self, input: Input) {
        match input {
            Input::Key { code, modifiers } => {
                if let Some(code) = key(&code) {
                    let modifiers = KeyModifiers::from_bits_truncate(modifiers);
                    if self.resume_key(code, modifiers) {
                        return;
                    }
                    if self.player_control_key(code, modifiers) {
                        return;
                    }
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
                    let theme_before = self.app.look.theme.name.clone();
                    self.app.dispatch_key(KeyEvent::new(code, modifiers));
                    if !self.overlay && self.app.panels.focus == Focus::History {
                        self.app.panels.history_scroll = self.app.panels.history_scroll.min(
                            self.app
                                .activity
                                .snapshot()
                                .recent
                                .len()
                                .saturating_sub(LEDGER_ROWS),
                        );
                    }
                    if theme_before != self.app.look.theme.name {
                        self.use_theme_palette();
                    }
                    if before != self.app.panels.focus {
                        self.skin.focused = None;
                        if let Some((_, start, height)) = self
                            .modules
                            .iter()
                            .find(|(focus, _, _)| *focus == self.app.panels.focus)
                        {
                            let top = self
                                .modules
                                .first()
                                .map(|(_, start, _)| *start as u16)
                                .unwrap_or(238);
                            let end = *start + i32::from(*height);
                            if *start - i32::from(self.offset) < i32::from(top) {
                                self.offset = (*start - i32::from(top)).max(0) as u16;
                            } else if end - i32::from(self.offset)
                                > self.viewport.height as i32 / i32::from(self.density) - 40
                            {
                                self.offset = (end
                                    - self.viewport.height as i32 / i32::from(self.density)
                                    + 40)
                                    .max(0) as u16;
                            }
                            self.offset = self.offset.min(self.max_offset);
                        } else if self.app.panels.focus == Focus::Player {
                            self.offset = 0;
                        }
                    }
                }
            }
            Input::Paste { text } => {
                if let Some(library) = &mut self.app.edit.eq_profiles {
                    library.query.push_str(text.trim());
                    library.cursor = 0;
                    library.scroll = 0;
                    return;
                }
                if !self.app.filter_paste(&text)
                    && !self.app.import_artist_paste(&text)
                    && !self.app.auth_paste(&text)
                {
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
                if action != "move" {
                    tracing::debug!(%action, button, x, y, ?pixel, overlay = self.overlay, "Graphical pointer input");
                }
                if self.overlay
                    && (self.eq_profile_pointer(&action, button, x, y, pixel)
                        || self.resume_pointer(&action, button, x, y, pixel)
                        || self.filter_pointer(&action, button, x, y, pixel)
                        || self.popup_pointer(&action, button, x, y, pixel))
                {
                    return;
                }
                if self.surface.is_none() || self.overlay {
                    // KIT's pointer protocol is zero based: left, right, middle.
                    let mouse_button = match button {
                        0 => MouseButton::Left,
                        1 => MouseButton::Right,
                        2 => MouseButton::Middle,
                        _ => return,
                    };
                    let kind = match action.as_str() {
                        "down" => MouseEventKind::Down(mouse_button),
                        "up" => MouseEventKind::Up(mouse_button),
                        "drag" => MouseEventKind::Drag(mouse_button),
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
                // Global controls act immediately regardless of pane focus.
                if action == "down" && button == 0 {
                    if let Some(name) = self
                        .surface
                        .as_ref()
                        .and_then(|s| s.hit(px as u16, py as u16))
                        .filter(|hit| {
                            matches!(hit.action.as_str(), "close:app" | "theme-next" | "help")
                        })
                        .map(|hit| hit.action.clone())
                    {
                        self.action(&name, px as u16, py as u16);
                        return;
                    }
                }
                if action == "down" {
                    if let Some(name) = self
                        .surface
                        .as_ref()
                        .and_then(|s| {
                            s.hits.iter().find(|hit| {
                                hit.action.starts_with("focus:")
                                    && hit.rect.contains(px as u16, py as u16)
                            })
                        })
                        .map(|hit| hit.action.as_str())
                    {
                        let focus = match name {
                            "focus:player" => Focus::Player,
                            "focus:eq" => Focus::Equalizer,
                            "focus:album" => Focus::Album,
                            "focus:activity" => Focus::History,
                            _ => Focus::Playlist,
                        };
                        if self.app.panels.focus != focus {
                            self.app.panels.focus = focus;
                            self.skin.focused = None;
                            self.skin.pressed = None;
                            self.drag = None;
                            self.eq_drag = None;
                            self.playlist_bars.release();
                            // Row selection and transport buttons act on the
                            // first left press. Keep focus-only presses for
                            // sliders and graphs.
                            let immediate = button == 0
                                && self
                                    .surface
                                    .as_ref()
                                    .and_then(|s| s.hit(px as u16, py as u16))
                                    .is_some_and(|hit| {
                                        hit.action
                                            .strip_prefix("playlist:")
                                            .is_some_and(|row| row.parse::<usize>().is_ok())
                                            || matches!(
                                                hit.action.as_str(),
                                                "previous" | "play" | "pause" | "stop" | "next"
                                            )
                                    });
                            if !immediate {
                                return;
                            }
                        }
                    }
                }
                if action == "down" && button == 0 {
                    if let Some(((), above)) = self.playlist_bars.press(px as u16, py as u16) {
                        self.scroll_playlist_to(above);
                        return;
                    }
                }
                if action == "drag" && button == 0 {
                    if let Some(((), above)) = self.playlist_bars.drag(py as u16) {
                        self.scroll_playlist_to(above);
                        return;
                    }
                }
                if action == "up" && self.playlist_bars.release() {
                    return;
                }
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
                if matches!(action.as_str(), "scroll_up" | "scroll_down") {
                    let target = self
                        .surface
                        .as_ref()
                        .and_then(|s| s.hit(px as u16, py as u16))
                        .map(|h| h.action.clone());
                    if target
                        .as_deref()
                        .is_some_and(|a| a.starts_with("playlist:"))
                    {
                        self.scroll_playlist_to(if action == "scroll_up" {
                            self.app.queue.scroll.saturating_sub(3) as u32
                        } else {
                            self.app.queue.scroll.saturating_add(3) as u32
                        });
                        return;
                    }
                    if target
                        .as_deref()
                        .is_some_and(|a| a.starts_with("activity-row:"))
                    {
                        self.app.panels.focus = Focus::History;
                        if action == "scroll_up" {
                            self.app.panels.history_scroll =
                                self.app.panels.history_scroll.saturating_sub(1);
                        } else {
                            self.app.panels.history_scroll = (self.app.panels.history_scroll + 1)
                                .min(
                                    self.app
                                        .activity
                                        .snapshot()
                                        .recent
                                        .len()
                                        .saturating_sub(LEDGER_ROWS),
                                );
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
                self.skin.hovered = if self.local_hover
                    && hit.as_deref().is_some_and(|name| {
                        name.strip_prefix("playlist:")
                            .is_some_and(|row| row.parse::<usize>().is_ok())
                    }) {
                    None
                } else {
                    hit.clone()
                };
                if action == "down" && button == 0 {
                    self.skin.pressed = hit.clone();
                    self.drag = hit.clone();
                    self.eq_drag = None;
                    if let Some(name) = hit.as_deref() {
                        self.action(name, px as u16, py as u16);
                        if name.starts_with("eq-band:") {
                            self.begin_eq_drag(px as u16, py as u16);
                        }
                    }
                }
                if action == "down" && button == 1 {
                    if let Some(index) = hit
                        .as_deref()
                        .and_then(|name| name.strip_prefix("playlist:"))
                        .and_then(|row| row.parse::<usize>().ok())
                        .and_then(|row| match self.app.queue.rows.rows().get(row) {
                            Some(playlist::Row::Track(index)) => Some(*index),
                            _ => None,
                        })
                    {
                        if let Some(source) = self.app.queue.items.get(index).cloned() {
                            self.context_anchor = Some([px, py]);
                            self.app.panels.focus = Focus::Playlist;
                            self.app.set_cursor(index);
                            if self.app.import.is_some() {
                                self.app.open_import_album_menu(source, index);
                            } else {
                                self.app.open_track_menu(source, index);
                            }
                        }
                    }
                }
                if action == "drag" && button == 0 {
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
                    self.eq_drag = None;
                }
            }
            Input::Detach => self.app.quit = true,
            Input::CancelPointer => {
                self.playlist_bars.release();
                self.skin.pressed = None;
                self.skin.hovered = None;
                if self.eq_dirty {
                    self.app.finish_eq_edit();
                    self.eq_dirty = false;
                }
                self.drag = None;
                self.eq_drag = None;
            }
            _ => {}
        }
    }
    fn closed(&self) -> bool {
        self.app.quit
    }
    fn shutdown(&mut self) {
        self.audio.detached(self.app);
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
        app.player.state.buffered_frames.store(105 * 44100, Relaxed);
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
            audio: Default::default(),
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
            eq_drag: None,
            max_offset: 0,
            overlay: false,
            modal_focus: None,
            context_anchor: None,
            playlist_bars: Scrollbars::new(),
            playlist_visible: 0,
            playlist_scrolled: None,
            cover_cache: None,
            modules: Vec::new(),
            skin: crate::embed::skin::PlayerSkin::new()?,
            skins: true,
            local_hover: true,
            reference_colors: true,
            density: 1,
            resume_scroll: 0,
            resume_fresh: false,
            folded: Default::default(),
        };
        // A native scene must request artwork even when playback has not
        // changed. Unlike the text view, it does not call App::draw.
        {
            let library = tempfile::tempdir()?;
            let index = library.path().join("index.sqlite");
            let db = crate::library::db::Db::open(&index)?;
            db.conn.execute_batch(
                "INSERT INTO dir (id, rel_path, scan_gen) VALUES (1, '', 1);
                 INSERT INTO file (id, dir_id, rel_path, size, mtime_ns, kind, scan_gen)
                 VALUES (1, 1, 'fixture.flac', 0, 0, 0, 1),
                        (2, 1, 'fixture.png', 0, 0, 2, 1);",
            )?;
            let uri = rack.app.current_uri().expect("fixture current track");
            db.conn.execute(
                "INSERT INTO track (uri, file_id, album, codec, added_at, modified_at, scan_gen)
                 VALUES (?1, 1, 'Artwork fixture', 'flac', 0, 0, 1)",
                [&uri],
            )?;
            starkit::image::RgbaImage::from_pixel(
                32,
                24,
                starkit::image::Rgba([164, 215, 145, 255]),
            )
            .save(library.path().join("fixture.png"))?;
            rack.app.art = crate::library::art::Watcher::spawn(
                index,
                Arc::new(crate::vfs::Vfs::local(library.path())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            );
            let viewport = Viewport {
                width: 1400,
                height: 1800,
                columns: 140,
                rows: 90,
                generation: 0,
            };
            // A desktop media service also requests covers with Album closed.
            // This fixture exercises the panel-only lazy lookup path.
            rack.app.mpris = None;
            rack.app.panels.album = false;
            rack.scene(viewport);
            assert!(rack.app.art_uri.is_none(), "closed Album stays idle");
            rack.app.panels.album = true;
            rack.scene(viewport);
            assert_eq!(rack.app.art_uri.as_deref(), Some(uri.as_str()));
            let watcher = rack.app.art.as_ref().expect("fixture artwork worker");
            let deadline = Instant::now() + Duration::from_secs(5);
            let album = loop {
                if let Some(album) = watcher.album_for(&uri) {
                    break album;
                }
                assert!(
                    Instant::now() < deadline,
                    "initial artwork lookup timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(album.image.is_some(), "cover loads without Retry");
            let scene = rack.scene(viewport);
            assert!(scene
                .components
                .iter()
                .any(|c| matches!(c, Component::Image { .. })));
            // Check the production raster, including fractional terminal grids:
            // fitted artwork has a 2px logical outline and no dark empty well.
            let previous_theme = rack.app.look.theme.clone();
            let mut previous_id = String::new();
            for theme in ["winamp-classic", "kanagawa", "winamp-silver"] {
                rack.app.look.theme = crate::theme::builtin::load(theme).unwrap();
                rack.reference_colors = false;
                for (width, height, columns, rows) in [
                    (1400, 1800, 140, 90),
                    (2800, 3600, 140, 90),
                    (2737, 1801, 171, 55),
                ] {
                    let scene = rack.scene(Viewport {
                        width,
                        height,
                        columns,
                        rows,
                        generation: 0,
                    });
                    let cached = rack.cover_cache.as_ref().unwrap();
                    assert_ne!(
                        cached.id, previous_id,
                        "changed artwork raster needs a new transport ID"
                    );
                    previous_id = cached.id.clone();
                    let slot = rack
                        .surface
                        .as_ref()
                        .unwrap()
                        .hits
                        .iter()
                        .rev()
                        .find(|h| h.action == "cover-open")
                        .unwrap()
                        .rect;
                    assert_eq!(slot.width, ALBUM_COVER * rack.density);
                    let selector = rack
                        .surface
                        .as_ref()
                        .unwrap()
                        .hits
                        .iter()
                        .find(|h| h.action == "cover-next")
                        .unwrap()
                        .rect;
                    assert!(
                        selector.x >= slot.x + slot.width,
                        "art selector must sit beside the enlarged cover"
                    );
                    let rendered = starkit::terminal_graphics::renderer::render_reference(&scene)?;
                    let pixel = |x: u16, y: u16| rendered.get_pixel(u32::from(x), u32::from(y)).0;
                    let rgb = |s: &str| {
                        let c = crate::theme::color::Rgb::parse_hex(s).unwrap();
                        [c.r, c.g, c.b, 255]
                    };
                    let middle_x = slot.x + slot.width / 2;
                    let middle_y = slot.y + slot.height / 2;
                    let outline = rgb(&cached.key.outline);
                    let background = if theme == "winamp-silver" {
                        rgb(
                            &crate::embed::skin::silver_panel_color(ALBUM_COVER_Y, LEDGER_HEIGHT)
                                .to_hex(),
                        )
                    } else {
                        rgb(&cached.key.background)
                    };
                    assert_eq!(pixel(middle_x, slot.y + 1), background);
                    if theme == "winamp-silver" {
                        assert_eq!(
                            pixel(middle_x, slot.y + 1),
                            pixel(slot.x + slot.width + 1, slot.y + 1),
                            "artwork padding must match the panel gradient"
                        );
                    }
                    let xs = (slot.x..slot.x + slot.width)
                        .filter(|x| pixel(*x, middle_y) == outline)
                        .collect::<Vec<_>>();
                    let ys = (slot.y..slot.y + slot.height)
                        .filter(|y| pixel(middle_x, *y) == outline)
                        .collect::<Vec<_>>();
                    let border = cached.key.border;
                    assert_eq!(
                        xs.len(),
                        usize::from(2 * border),
                        "left/right border at {width}x{height}"
                    );
                    assert_eq!(
                        ys.len(),
                        usize::from(2 * border),
                        "top/bottom border at {width}x{height}"
                    );
                    for i in 0..border {
                        assert_eq!(pixel(xs[0] + i, middle_y), outline);
                        assert_eq!(pixel(*xs.last().unwrap() - i, middle_y), outline);
                        assert_eq!(pixel(middle_x, ys[0] + i), outline);
                        assert_eq!(pixel(middle_x, *ys.last().unwrap() - i), outline);
                    }
                    assert_eq!(pixel(xs[0] + border, middle_y), [164, 215, 145, 255]);
                    assert_eq!(pixel(middle_x, ys[0] + border), [164, 215, 145, 255]);
                    assert!(
                        (i32::from(xs[0] + xs.last().unwrap() + 1)
                            - i32::from(2 * slot.x + slot.width))
                        .abs()
                            <= 1
                    );
                    assert!(
                        (i32::from(ys[0] + ys.last().unwrap() + 1)
                            - i32::from(2 * slot.y + slot.height))
                        .abs()
                            <= 1
                    );
                    starkit::image::imageops::crop_imm(
                        &rendered,
                        u32::from(slot.x - 4),
                        u32::from(slot.y - 4),
                        u32::from(slot.width + 8),
                        u32::from(slot.height + 8),
                    )
                    .to_image()
                    .save(output.join(format!("album-artwork-{theme}-{width}.png")))?;
                }
            }
            rack.app.look.theme = previous_theme;
            rack.reference_colors = true;
            let serial = rack.app.art.as_ref().unwrap().serial();
            rack.app.panels.album = false;
            rack.scene(viewport);
            rack.app.panels.album = true;
            rack.scene(viewport);
            assert_eq!(rack.app.art.as_ref().unwrap().serial(), serial);
            rack.app.art = None;
            rack.app.art_uri = None;
            rack.cover_cache = None;
        }
        // Resume is a native, wrapped choice dialog, sharing the existing
        // acceptance/decline actions. A detached player keeps this check silent.
        {
            let mut saved = rack.app.snapshot().expect("fixture session");
            saved.artist = "Black Sabbath".into();
            saved.title = "Wasp/Behind the Wall of Sleep/Bassically/N.I.B. — a complete medley title that must stay readable instead of being clipped into a terminal row".into();
            saved.playlist_name = "queue".into();
            saved.index = 78;
            saved.total = 29615;
            saved.position = 134.55;
            saved.duration = 584.56;
            saved.folded = vec!["first record".into(), "second record".into()];
            saved.volume = 0.42;
            saved.playlist = Some(output.join("missing-playlist.m3u"));
            let items = rack.app.player.queue.lock().unwrap().tracks().to_vec();
            let original = std::mem::replace(&mut rack.app.player, Arc::new(Player::detached()));
            rack.app.player.set_queue_tracks(items);
            let status = rack.app.status.take();
            let folded = rack.app.queue.folded.clone();
            let fold_gen = rack.app.queue.fold_gen;
            let cursor = rack.app.queue.cursor;
            let scroll = rack.app.queue.scroll;
            let text = |rack: &Rack<'_>| {
                let surface = rack.surface.as_ref().unwrap();
                let start = surface
                    .nodes
                    .iter()
                    .rposition(
                        |n| matches!(n, Primitive::Text { text, .. } if text == "RESUME SESSION"),
                    )
                    .unwrap();
                surface.nodes[start..]
                    .iter()
                    .filter_map(|n| match n {
                        Primitive::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            for density in [1, 2] {
                rack.app.offer_resume(saved.clone());
                let viewport = Viewport {
                    width: 800 * density,
                    height: 420 * density,
                    columns: 100,
                    rows: 26,
                    generation: 0,
                };
                let scene = rack.scene(viewport);
                assert!(rack.overlay);
                let surface = rack.surface.as_ref().unwrap();
                assert!(surface.hits.iter().all(|h| h.action.starts_with("resume:")));
                let contents = text(&rack);
                for value in [
                    "Black Sabbath",
                    "N.I.B.",
                    "terminal row",
                    "track 79 of 29615",
                    "2:14",
                    "9:44",
                    "2 records folded",
                    "unavailable",
                    "Start fresh",
                    "Resume",
                ] {
                    assert!(
                        contents.contains(value),
                        "resume dialog lost {value}: {contents}"
                    );
                }
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("resume-session-{density}x.png")))?;
                // Kitty uses larger glyph metrics than the bundled reference
                // font. Compare actual clipped label boxes with an unrestricted
                // rendering at a larger size; neither may introduce an ellipsis.
                for caption in ["Start fresh", "Resume"] {
                    let label = surface
                        .nodes
                        .iter()
                        .rev()
                        .find_map(|node| match node {
                            Primitive::Text { rect, text, .. } if text == caption => Some(*rect),
                            _ => None,
                        })
                        .unwrap();
                    let base = starkit::image::RgbaImage::new(220, 30);
                    let mut label_surface = Surface::new(220, 30, "transparent".into());
                    classic::label(
                        &mut label_surface,
                        R::new(0, 0, label.width / density as u16, 30),
                        caption,
                        "#ffffff",
                        14,
                        true,
                    );
                    let bounded = starkit::terminal_graphics::renderer::render_surface_overlay(
                        &base,
                        &label_surface,
                    )?;
                    if let Primitive::Text { rect, .. } = &mut label_surface.nodes[0] {
                        rect.width = 220;
                    }
                    let complete = starkit::terminal_graphics::renderer::render_surface_overlay(
                        &base,
                        &label_surface,
                    )?;
                    assert_eq!(bounded, complete, "resume button truncates {caption}");
                }
                saved.save()?;
                rack.input(Input::Key {
                    code: "tab".into(),
                    modifiers: 0,
                });
                assert!(rack.resume_fresh);
                rack.input(Input::Key {
                    code: "enter".into(),
                    modifiers: 0,
                });
                assert!(rack.app.over.resume.is_none());
                assert!(!crate::session::Session::path()?.exists());
                rack.scene(viewport);
            }
            rack.app.offer_resume(saved.clone());
            rack.scene(rack.viewport);
            press_button(&mut rack, "resume:accept", 0);
            assert!(rack.app.over.resume.is_none());
            assert!(rack.app.queue.folded.contains("first record"));
            assert!((rack.app.player.volume() - 0.42).abs() < 0.001);
            assert_eq!(rack.app.current_uri().as_deref(), Some(saved.uri.as_str()));
            rack.scene(rack.viewport);
            rack.app.offer_resume(saved.clone());
            rack.scene(rack.viewport);
            press_button(&mut rack, "resume:fresh", 0);
            assert!(rack.app.over.resume.is_none());
            rack.scene(rack.viewport);
            saved.title = format!("{} LAST WORD", "A long saved title 世界 ".repeat(150));
            rack.app.offer_resume(saved);
            rack.scene(rack.viewport);
            rack.input(Input::Key {
                code: "end".into(),
                modifiers: 0,
            });
            rack.scene(rack.viewport);
            assert!(rack.app.over.resume.is_some());
            assert!(rack.resume_scroll > 0);
            let contents = text(&rack).split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                contents.contains("LAST WORD"),
                "scroll missed title ending: {contents}"
            );
            rack.app.over.resume = None;
            rack.app.player = original;
            rack.app.status = status;
            rack.app.queue.folded = folded;
            rack.app.queue.fold_gen = fold_gen;
            rack.app.queue.cursor = cursor;
            rack.app.queue.scroll = scroll;
            rack.resume_scroll = 0;
            rack.resume_fresh = false;
        }
        // Measure the actual shared component against option 01 separately
        // from the full application's fixture. No alternate drawing path.
        let mut selected = rack.player();
        selected.title = "Terra Atlantica — Through the Water and the Waves".into();
        selected.subtitle = "Oceans".into();
        selected.tech = "Oceans · FLAC · 1063 kbps · 44.1 kHz · 16-bit".into();
        selected.position = 69.;
        selected.duration = 298.;
        selected.volume = 0.8;
        selected.repeat = crate::playlist::queue::RepeatMode::All;
        let selected_palette = Palette {
            bg: [23, 24, 32],
            inset_bg: None,
            fg: [215, 217, 207],
            muted: [152, 158, 172],
            accent: [164, 215, 145],
            selected: [48, 63, 61],
            border: [116, 121, 134],
            error: [240, 60, 60],
        };
        let mut renderer =
            starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new("JetBrains Mono");
        for density in [1, 2] {
            for width in [900, 1352] {
                let surface = rack.skin.surface_at_density(
                    width * density,
                    230 * density,
                    GraphicsConfig {
                        cell_width: 8 * density,
                        cell_height: 16 * density,
                    },
                    &selected_palette,
                    &selected,
                    8 * density,
                    density,
                )?;
                surface.validate()?;
                let base = starkit::image::RgbaImage::new(
                    u32::from(surface.width),
                    u32::from(surface.height),
                );
                renderer
                    .render(&base, &surface)?
                    .save(output.join(format!("selected-player-{width}-{density}x.png")))?;
            }
        }
        let selected_view = Viewport {
            width: 1368,
            height: 1148,
            columns: 171,
            rows: 71,
            generation: 0,
        };
        let playlist_geometry = |rack: &Rack<'_>| {
            rack.modules
                .iter()
                .find(|(focus, _, _)| *focus == Focus::Playlist)
                .map(|(_, y, height)| (*y, *height))
                .unwrap()
        };
        // Album and Activity share a row whenever both readable modules fit,
        // including at Retina density. Compact Album retains its artwork.
        for density in [1, 2] {
            for width in [1000, 1100, 1400] {
                let viewport = Viewport {
                    width: width * density,
                    height: 1800 * density,
                    columns: 140,
                    rows: 90,
                    generation: 0,
                };
                let scene = rack.scene(viewport);
                let module_y = |focus| rack.modules.iter().find(|(f, _, _)| *f == focus).unwrap().1;
                assert_eq!(
                    module_y(Focus::Album) == module_y(Focus::History),
                    width >= 1056
                );
                if width == 1100 {
                    starkit::terminal_graphics::renderer::render_reference(&scene)?
                        .save(output.join(format!("album-activity-paired-{density}x.png")))?;
                }
            }
        }
        let activity = rack.activity(520, &crate::embed::skin::colors(&rack.palette()), 12);
        let activity_text = activity
            .nodes
            .iter()
            .filter_map(|n| match n {
                Primitive::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        for heading in [
            "LOCAL HISTORY",
            "LAST.FM",
            "LISTENBRAINZ",
            "LISTENED",
            "TRACK / ARTIST",
            "DELIVERY",
        ] {
            assert!(activity_text.contains(heading));
        }
        // Render populated, scrollable ledgers using the same production view.
        // No provider setup or network requests are involved in the fixture.
        {
            use crate::activity::{Provider, ProviderStatus, Recent, Snapshot};
            let scroll = rack.app.panels.history_scroll;
            let mut snapshot = Snapshot {
                providers: vec![
                    ProviderStatus {
                        provider: Provider::Lastfm,
                        configured: true,
                        enabled: true,
                        username: "fixture".into(),
                    },
                    ProviderStatus {
                        provider: Provider::Listenbrainz,
                        configured: true,
                        enabled: false,
                        username: "fixture".into(),
                    },
                ],
                recent: (0..10)
                    .map(|i| Recent {
                        artist: Some("Black Sabbath".into()),
                        title: Some(format!("Listen {i} — 世界")),
                        listened_ms: 132000,
                        outcome: "completed".into(),
                        deliveries: 1,
                        sent: i64::from(i % 3 == 0),
                        errors: i64::from(i % 3 == 2),
                        ..Recent::default()
                    })
                    .collect(),
                pending: 2,
            };
            assert_eq!(
                provider_state(&snapshot, Provider::Lastfm),
                ("Enabled", true)
            );
            assert_eq!(
                provider_state(&snapshot, Provider::Listenbrainz),
                ("Off", false)
            );
            snapshot.providers[1].configured = false;
            snapshot.providers[1].enabled = true;
            assert_eq!(
                provider_state(&snapshot, Provider::Listenbrainz),
                ("Set up", false)
            );
            rack.app.panels.history_scroll = usize::MAX;
            for density in [1, 2] {
                rack.skin
                    .prepare(&rack.palette(), density, rack.radius == 0)?;
                let ledger = rack.activity_view(
                    592,
                    &crate::embed::skin::colors(&rack.palette()),
                    12,
                    &snapshot,
                );
                let rows = ledger
                    .hits
                    .iter()
                    .filter(|h| h.action.starts_with("activity-row:"))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), LEDGER_ROWS);
                assert_eq!(rows[0].action, "activity-row:4");
                assert_eq!(rows[5].action, "activity-row:9");
                assert!(rows.iter().all(|h| h.rect.y + h.rect.height < 277));
                let ledger = ledger.at_density(density)?;
                ledger.validate()?;
                starkit::terminal_graphics::renderer::render_surface_overlay(
                    &starkit::image::RgbaImage::new(
                        u32::from(ledger.width),
                        u32::from(ledger.height),
                    ),
                    &ledger,
                )?
                .save(output.join(format!("compact-ledger-activity-{density}x.png")))?;
            }
            rack.app.panels.history_scroll = scroll;
        }
        for (width, height, rows) in [(1400, 2000, 100), (2737, 2401, 70)] {
            let v = Viewport {
                width,
                height,
                columns: 171,
                rows,
                generation: 0,
            };
            rack.scene(v);
            let (start, expanded_height) = playlist_geometry(&rack);
            assert!(expanded_height > 260);
            assert_eq!(
                start + i32::from(expanded_height),
                (height / u32::from(rack.density)) as i32 - 32
            );
            assert_eq!(rack.max_offset, 0);
            rack.app.panels.eq = false;
            rack.scene(v);
            assert_eq!(playlist_geometry(&rack).1, expanded_height + EQ_HEIGHT + 8);
            rack.app.panels.eq = true;
            rack.folded.insert("eq".into());
            rack.scene(v);
            assert_eq!(playlist_geometry(&rack).1, expanded_height + EQ_HEIGHT - 38);
            rack.folded.remove("eq");
            rack.folded.insert("playlist".into());
            rack.scene(v);
            assert_eq!(playlist_geometry(&rack).1, 38);
            rack.folded.remove("playlist");
        }
        let compact = Viewport {
            width: 900,
            height: 700,
            columns: 90,
            rows: 35,
            generation: 0,
        };
        rack.scene(compact);
        assert_eq!(playlist_geometry(&rack).1, 260);
        rack.offset = 96;
        rack.scene(compact);
        assert_eq!(playlist_geometry(&rack).1, 260);
        // A resize must paint at the newly clamped offset immediately.
        let tall = Viewport {
            width: 1400,
            height: 2000,
            columns: 140,
            rows: 100,
            generation: 0,
        };
        rack.scene(tall);
        assert_eq!(rack.offset, 0);
        let (start, height) = playlist_geometry(&rack);
        let toolbar = rack
            .surface
            .as_ref()
            .unwrap()
            .hits
            .iter()
            .find(|hit| hit.action == "queue-remove")
            .unwrap();
        assert_eq!(i32::from(toolbar.rect.y), start + i32::from(height) - 35);
        // The production terminal uses real pixel dimensions, including tall,
        // odd-sized and 4K windows. Fixed glyphs must never be rescaled to fit.
        // Resize through partial pane corners as well as complete layouts.
        // A one-pixel fragment must not retain a full nine-slice corner, and
        // the scrolling modules must leave the status footer unobstructed.
        for density in [1u16, 2] {
            for height in 480..535u16 {
                let v = Viewport {
                    width: u32::from(900 * density),
                    height: u32::from(height * density),
                    columns: 90,
                    rows: height / 20,
                    generation: u64::from(height),
                };
                let scene = rack.scene(v);
                assert_eq!(rack.density, density);
                let footer = (height - 24) * density;
                for node in &rack.surface.as_ref().unwrap().nodes {
                    let r = node.rect();
                    assert!(r.y + r.height <= footer || r.y >= footer);
                }
                starkit::terminal_graphics::renderer::render_reference(&scene)?;
            }
        }
        for (width, height, rows) in [(1400, 2000, 100), (2737, 1801, 56), (3840, 2160, 67)] {
            let v = Viewport {
                width,
                height,
                columns: 171,
                rows,
                generation: 0,
            };
            let scene = rack.scene(v);
            let surface = rack.surface.as_ref().unwrap();
            assert_eq!(
                (u32::from(surface.width), u32::from(surface.height)),
                (width, height)
            );
            surface.validate()?;
            // Rendering is the gate: surface validation alone cannot detect a
            // host stretching a fixed atlas crop to a different pixel size.
            starkit::terminal_graphics::renderer::render_reference(&scene)?;
        }
        let selected_scene = rack.scene(selected_view);
        assert!(rack
            .surface
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|node| matches!(node, Primitive::Text { text, .. } if text == "Theme: Classic")));
        starkit::terminal_graphics::renderer::render_reference(&selected_scene)?
            .save(output.join("selected-rack.png"))?;
        rack.reference_colors = false;
        // Compare actual header pixels across the composed production modules.
        // Album/Activity wells must not change their title strip's palette.
        let assert_common_headers = |rack: &Rack<'_>, image: &starkit::image::RgbaImage| {
            let scale = u32::from(rack.density);
            for action in [
                "focus:player",
                "fold:eq",
                "fold:album",
                "fold:activity",
                "fold:playlist",
            ] {
                let hit = rack
                    .surface
                    .as_ref()
                    .unwrap()
                    .hits
                    .iter()
                    .find(|hit| hit.action == action)
                    .unwrap_or_else(|| panic!("missing header hit: {action}"));
                // Whole-panel focus hits are clipped away when the body is
                // below the viewport; title-strip hits remain available.
                let inset = if action == "focus:player" {
                    0
                } else {
                    7 * scale
                };
                for row in [7, 15, 30] {
                    assert_eq!(
                        image.get_pixel(
                            u32::from(hit.rect.x) - inset + 250 * scale,
                            u32::from(hit.rect.y) - inset + row * scale
                        ),
                        image.get_pixel(250 * scale, row * scale),
                        "{action} header differs from Player/EQ at row {row}, density {scale}"
                    );
                }
            }
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
                let theme_label = format!("Theme: {}", rack.app.look.theme.name);
                assert!(rack.surface.as_ref().unwrap().nodes.iter().any(|node| {
                    matches!(node, Primitive::Text { text, rect, .. }
                        if text == &theme_label && usize::from(rect.width) >= theme_label.width() * 8)
                }));
                let status_area = Rect::new(0, 0, columns, 1);
                let mut status = starkit::ratatui::buffer::Buffer::empty(status_area);
                rack.app.look.theme_selection_until = Some(Instant::now() + Duration::from_secs(3));
                rack.app.draw_status(status_area, &mut status);
                let status_text = (0..columns)
                    .map(|x| status[(x, 0)].symbol())
                    .collect::<String>();
                assert!(status_text.contains(&format!("theme: {}", rack.app.look.theme.name)));
                rack.app.look.theme_selection_until = None;
                for component in &scene.components {
                    if let Component::Surface { surface, .. } = component {
                        surface.validate()?;
                    }
                }
                let image = starkit::terminal_graphics::renderer::render_reference(&scene)?;
                if name == "full" {
                    assert_common_headers(&rack, &image);
                }
                if id == "winamp-classic" {
                    // Theme selection must reproduce the actual reference-mode
                    // artwork, not just a similar palette. Compare the rendered
                    // rack at both responsive widths; only the footer's theme
                    // name intentionally differs.
                    rack.reference_colors = true;
                    let reference =
                        starkit::terminal_graphics::renderer::render_reference(&rack.scene(v))?;
                    rack.reference_colors = false;
                    let body_height = height - 32;
                    assert!(
                        starkit::image::imageops::crop_imm(&image, 0, 0, width, body_height)
                            .to_image()
                            == starkit::image::imageops::crop_imm(
                                &reference,
                                0,
                                0,
                                width,
                                body_height
                            )
                            .to_image(),
                        "Winamp Classic diverged from the stacked-rack artwork at {width}x{height}"
                    );
                    assert!(
                        !starkit::image::imageops::crop_imm(&image, 0, body_height, width, 32)
                            .to_image()
                            .pixels()
                            .any(|pixel| pixel.0[..3] == [0, 255, 0] && pixel.0[3] > 0),
                        "Winamp Classic footer still uses the legacy neon green"
                    );
                    rack.scene(v);
                }
                let theme_rect = rack
                    .surface
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find_map(|node| match node {
                        Primitive::Text { text, rect, .. } if text == &theme_label => Some(*rect),
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    (theme_rect.y..theme_rect.y + theme_rect.height).any(|y| {
                        (theme_rect.x..theme_rect.x + theme_rect.width)
                            .any(|x| image.get_pixel(u32::from(x), u32::from(y))[3] > 0)
                    }),
                    "{id}/{name}: theme footer must remain visible on a transparent canvas"
                );
                image.save(output.join(format!("{id}-{name}.png")))?;
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
        let v2 = Viewport {
            width: 2736,
            height: 1800,
            columns: 171,
            rows: 56,
            generation: 0,
        };
        let high_density = rack.scene(v2);
        for component in &high_density.components {
            if let Component::Surface { surface, .. } = component {
                surface.validate()?;
            }
        }
        assert_eq!(rack.density, 2);
        assert_eq!(
            rack.surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .find(|h| h.action == "play")
                .unwrap()
                .rect,
            R::new(102, 374, 58, 58)
        );
        rack.reference_colors = true;
        starkit::terminal_graphics::renderer::render_reference(&rack.scene(v2))?
            .save(output.join("selected-rack-2x.png"))?;
        rack.reference_colors = false;
        rack.app.look.theme = crate::theme::builtin::load("winamp-silver").unwrap();
        let silver_2x = starkit::terminal_graphics::renderer::render_reference(&rack.scene(v2))?;
        assert_common_headers(&rack, &silver_2x);
        silver_2x.save(output.join("winamp-silver-full-2x.png"))?;
        // Wide layouts share a row; narrow layouts stack, with no overlapping
        // drawing or hit geometry. Closing a module never stops its service.
        let vwide = Viewport {
            width: 1368,
            height: 1148,
            columns: 171,
            rows: 71,
            generation: 0,
        };
        rack.scene(vwide);
        let album_start = rack
            .modules
            .iter()
            .find(|(f, _, _)| *f == Focus::Album)
            .unwrap()
            .1;
        assert_eq!(
            album_start,
            rack.modules
                .iter()
                .find(|(f, _, _)| *f == Focus::History)
                .unwrap()
                .1
        );
        rack.action("close:album", 0, 0);
        assert!(!rack.app.panels.album);
        assert!(rack.app.panels.history);
        rack.action("close:album", 0, 0);
        rack.scene(Viewport {
            width: 900,
            height: 1100,
            columns: 90,
            rows: 55,
            generation: 0,
        });
        assert!(
            rack.modules
                .iter()
                .find(|(f, _, _)| *f == Focus::History)
                .unwrap()
                .1
                > rack
                    .modules
                    .iter()
                    .find(|(f, _, _)| *f == Focus::Album)
                    .unwrap()
                    .1
        );
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
        rack.app.panels.focus = Focus::Player;
        rack.input(Input::Key {
            code: "right".into(),
            modifiers: KeyModifiers::ALT.bits(),
        });
        assert_eq!(rack.skin.focused.as_deref(), Some("previous"));
        assert!(rack.player_control_key(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(rack.skin.focused.as_deref(), Some("play"));
        assert!(!rack.player_control_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(rack.skin.focused.as_deref(), Some("play"));
        assert!(rack.player_control_key(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(rack.skin.focused.as_deref(), Some("previous"));
        assert!(!rack.player_control_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(rack.skin.focused.is_none());
        rack.skin.focused = Some("shuffle".into());
        let shuffled = rack.app.player.queue.lock().unwrap().shuffled();
        rack.input(Input::Key {
            code: "enter".into(),
            modifiers: 0,
        });
        assert_ne!(rack.app.player.queue.lock().unwrap().shuffled(), shuffled);
        rack.input(Input::Key {
            code: "tab".into(),
            modifiers: 0,
        });
        assert!(rack.skin.focused.is_none());
        assert_ne!(rack.app.panels.focus, Focus::Player);
        assert!(!rack.player_control_key(KeyCode::Right, KeyModifiers::ALT));
        rack.app.panels.focus = Focus::Player;
        rack.overlay = true;
        assert!(!rack.player_control_key(KeyCode::Right, KeyModifiers::ALT));
        rack.overlay = false;
        let closed_height = rack
            .modules
            .iter()
            .find(|(focus, _, _)| *focus == Focus::Equalizer)
            .unwrap()
            .2;
        assert_eq!(closed_height, EQ_HEIGHT);
        rack.action("fold:eq", 0, 0);
        rack.scene(v);
        assert!(
            rack.modules
                .iter()
                .find(|(focus, _, _)| *focus == Focus::Equalizer)
                .unwrap()
                .2
                == 38
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
            button: 0,
            x: hit.x / 10,
            y: hit.y / 20,
            pixel: Some([u32::from(hit.x + 2), u32::from(hit.y + 2)]),
            modifiers: 0,
        });
        assert_eq!(rack.app.panels.focus, Focus::Playlist);
        assert_eq!(rack.app.queue.cursor, 1);
        assert_eq!(rack.drag.as_deref(), Some("playlist:1"));
        rack.input(Input::Pointer {
            action: "up".into(),
            button: 0,
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
        let full = Rect::new(0, 0, v.columns, v.rows);
        let regions = rack.app.regions(full).unwrap();
        let volume_rect = rack
            .app
            .player_geometry(&regions)
            .unwrap()
            .controls
            .volume
            .unwrap();
        let volume = rack.app.player.volume();
        rack.app.player.set_volume(0.1);
        rack.app.panels.focus = Focus::Playlist;
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: volume_rect.x + volume_rect.width / 2,
            row: volume_rect.y,
            modifiers: KeyModifiers::empty(),
        };
        rack.app.handle_mouse(mouse, full);
        assert_eq!(rack.app.panels.focus, Focus::Player);
        assert!((rack.app.player.volume() - 0.1).abs() < 0.001);
        rack.app.handle_mouse(mouse, full);
        assert!(rack.app.player.volume() > 0.1);
        rack.app.edit.drag = None;
        rack.app.player.set_volume(volume);
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
        // Use the producer's actual button numbers (left = 0), through the
        // complete pointer route. Calling action() directly missed the wire
        // mismatch, and a header-wide fold hit used to swallow these links.
        fn click(rack: &mut Rack<'_>, name: &str) {
            click_button(rack, name, 0);
        }
        fn click_button(rack: &mut Rack<'_>, name: &str, button: u8) {
            let focus = rack.app.panels.focus;
            press_button(rack, name, button);
            if rack.app.panels.focus != focus
                && rack.app.over.settings.is_none()
                && !matches!(name, "previous" | "play" | "pause" | "stop" | "next")
                && !(button == 0
                    && name
                        .strip_prefix("playlist:")
                        .is_some_and(|row| row.parse::<usize>().is_ok()))
            {
                rack.scene(rack.viewport);
                press_button(rack, name, button);
            }
        }
        fn press_button(rack: &mut Rack<'_>, name: &str, button: u8) {
            let surface = rack.surface.as_ref().unwrap();
            let rect = surface
                .hits
                .iter()
                .find(|hit| hit.action == name)
                .unwrap()
                .rect;
            let (x, y) = (rect.x + rect.width / 2, rect.y + rect.height / 2);
            assert_eq!(surface.hit(x, y).unwrap().action, name);
            rack.input(Input::Pointer {
                action: "down".into(),
                button,
                x: (u32::from(x) * u32::from(rack.viewport.columns) / rack.viewport.width) as u16,
                y: (u32::from(y) * u32::from(rack.viewport.rows) / rack.viewport.height) as u16,
                pixel: Some([u32::from(x), u32::from(y)]),
                modifiers: 0,
            });
        }
        // Footer clicks cycle once without first focusing the pane underneath,
        // including wraparound and switching out of reference palette mode.
        {
            let original_theme = rack.app.look.theme.clone();
            let original_index = rack.app.look.index;
            let original_reference = rack.reference_colors;
            let original_focus = rack.app.panels.focus;
            for density in [1, 2] {
                rack.app.look.index = rack.app.look.ids.len() - 1;
                rack.reference_colors = true;
                rack.app.panels.focus = Focus::Equalizer;
                let viewport = Viewport {
                    width: 1400 * density,
                    height: 1200 * density,
                    columns: 140,
                    rows: 60,
                    generation: 0,
                };
                for index in 0..rack.app.look.ids.len() {
                    rack.scene(viewport);
                    press_button(&mut rack, "theme-next", 0);
                    assert_eq!(rack.app.look.index, index);
                    assert_eq!(
                        rack.app.look.theme.id,
                        crate::theme::builtin::resolve_named(&rack.app.look.ids[index])
                            .0
                            .id
                    );
                    assert!(!rack.reference_colors);
                    assert_eq!(rack.app.panels.focus, Focus::Equalizer);
                }
            }
            let area = Rect::new(4, 40, 140, 1);
            rack.app.status_click(area, area.x + 9);
            assert_eq!(rack.app.look.index, 0, "cell footer cycles too");
            rack.app.look.theme = original_theme;
            rack.app.look.index = original_index;
            rack.reference_colors = original_reference;
            rack.app.panels.focus = original_focus;
            rack.scene(v);
        }
        // A restored high-frequency profile must match the audio response on
        // the first playback frame, without selecting and loading it again.
        {
            use crate::audio::dsp::apo::Profile;
            use std::sync::atomic::Ordering::Relaxed;
            let mut bob = Profile::legacy("Bob", 0., &[0.; 10]);
            for (stage, hz, gain) in [(1, 33.4, 2.47), (5, 1217.4, 2.47), (10, 16998., -6.32)] {
                if let Filter::Biquad {
                    frequency, gain_db, ..
                } = &mut bob.stages[stage].filter
                {
                    *frequency = hz;
                    *gain_db = gain;
                }
            }
            let file = bob.save_new_managed()?;
            let mut restored_cfg = cfg.clone();
            restored_cfg.eq.preset = "Bob".into();
            restored_cfg.eq.enabled = true;
            let original_eq =
                std::mem::replace(&mut rack.app.eq, EqState::from_config(&restored_cfg));
            let original_rate = rack.app.player.state.sample_rate.load(Relaxed);
            let original_output_rate = rack.app.player.eq.load().sample_rate;
            rack.app.player.state.sample_rate.store(0, Relaxed);
            rack.app.apply_eq();
            // The audio worker rebuilds at startup and when output formats
            // change. Source metadata can remain 44.1 kHz under resampling.
            for rate in [44100, 48000, 96000] {
                rack.app.player.eq.rebuild(rate);
                rack.app.player.state.sample_rate.store(44100, Relaxed);
                rack.tick();
                assert_eq!(rack.app.eq.active(), &bob);
                assert_eq!(rack.app.eq.compiled_rate, rate);
                let audio = rack.app.player.eq.load();
                assert_eq!(audio.sample_rate, rate);
                for hz in [16998., 1217.4, 33.4] {
                    let drawn = rack
                        .app
                        .eq
                        .compiled
                        .magnitude_db_at(hz, rack.app.eq.compiled_rate);
                    let heard = audio.magnitude_db_at(hz, rate);
                    assert!((drawn - heard).abs() < 1e-6, "restored Bob response differs at {hz} Hz: graph {drawn} dB, audio {heard} dB");
                }
                rack.scene(v);
                let restored_curve = rack.curve.as_ref().unwrap().points.clone();
                rack.app.apply_eq();
                rack.scene(v);
                assert_eq!(
                    rack.curve.as_ref().unwrap().points,
                    restored_curve,
                    "reapplying Bob must not repair or alter the restored graph"
                );
                assert_eq!(rack.app.player.eq.load().sample_rate, rate);
            }
            // Stopping playback must not turn a valid high-frequency profile
            // into an 8 kHz display or change the saved file.
            rack.app.player.state.sample_rate.store(0, Relaxed);
            rack.tick();
            assert_eq!(rack.app.eq.compiled_rate, 96000);
            assert_eq!(Profile::parse_file(&file)?, bob);
            assert!(rack
                .surface
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|n| matches!(n, Primitive::Text{text,..} if text == "Profile: Bob")));
            rack.app.eq = original_eq;
            rack.app
                .player
                .state
                .sample_rate
                .store(original_rate, Relaxed);
            rack.app.player.eq.rebuild(original_output_rate);
            rack.app.apply_eq();
            std::fs::remove_file(file)?;
        }
        // Named profile workflows run against the isolated fixture directory,
        // through the production native input path, never the user's EQ files.
        {
            use crate::audio::dsp::apo::{BiquadKind as K, ChannelMask, Profile, Stage};
            let original_eq = std::mem::replace(&mut rack.app.eq, EqState::from_config(&cfg));
            let original_theme = rack.app.look.theme.clone();
            let original_reference = rack.reference_colors;
            let original_offset = rack.offset;
            rack.reference_colors = false;
            rack.app.look.theme = crate::theme::builtin::load("winamp-silver").unwrap();
            rack.offset = 0;
            rack.app.panels.focus = Focus::Equalizer;
            rack.app.eq.profile = 0;
            let stage = |filter| Stage {
                enabled: true,
                channels: ChannelMask::ALL,
                filter,
            };
            rack.app.eq.active_mut().stages = vec![
                stage(Filter::Preamp { gain_db: -4. }),
                stage(Filter::Biquad {
                    kind: K::LowShelf,
                    frequency: 70.,
                    gain_db: 2.6,
                    width: Width::Q(0.7),
                    corner_frequency: false,
                }),
                stage(Filter::Biquad {
                    kind: K::Peaking,
                    frequency: 560.,
                    gain_db: -3.8,
                    width: Width::Q(1.4),
                    corner_frequency: false,
                }),
                stage(Filter::Biquad {
                    kind: K::HighShelf,
                    frequency: 6300.,
                    gain_db: 1.4,
                    width: Width::Q(0.7),
                    corner_frequency: false,
                }),
            ];
            rack.app.finish_eq_edit();
            let viewport = Viewport {
                width: 1400,
                height: 1200,
                columns: 140,
                rows: 60,
                generation: 0,
            };
            // A handle sits on the combined curve, not at its individual
            // gain. Grabbing it off-center must never rewrite the profile.
            let initial = rack.app.eq.active().clone();
            for density in [1u16, 2] {
                let view = Viewport {
                    width: viewport.width * u32::from(density),
                    height: viewport.height * u32::from(density),
                    ..viewport
                };
                rack.scene(view);
                let hits = &rack.surface.as_ref().unwrap().hits;
                let handle = hits.iter().find(|h| h.action == "eq-band:2").unwrap().rect;
                let plot = hits.iter().find(|h| h.action == "eq-plot").unwrap().rect;
                let origin = [
                    handle.x + handle.width / 2 + 4 * density,
                    handle.y + handle.height / 2 - 3 * density,
                ];
                let pointer = |rack: &mut Rack<'_>, action: &str, point: [u16; 2]| {
                    rack.input(Input::Pointer {
                        action: action.into(),
                        button: 0,
                        x: 0,
                        y: 0,
                        pixel: Some(point.map(u32::from)),
                        modifiers: 0,
                    });
                };
                pointer(&mut rack, "down", origin);
                assert_eq!(rack.app.eq.stage, 2);
                pointer(&mut rack, "drag", origin);
                assert_eq!(*rack.app.eq.active(), initial, "pickup must not snap");
                assert!(!rack.eq_dirty, "stationary pickup must not create an edit");
                // Move one logical pixel, then beyond the original hit region.
                // Repainting during a drag must not replace the original anchor.
                for distance in [1, 30] {
                    pointer(
                        &mut rack,
                        "drag",
                        [
                            origin[0] + distance * density,
                            origin[1] - distance * density,
                        ],
                    );
                    let Filter::Biquad {
                        frequency, gain_db, ..
                    } = rack.app.eq.active().stages[2].filter
                    else {
                        unreachable!()
                    };
                    let expected_hz = 560.
                        * 1000f64.powf(f64::from(distance * density) / f64::from(plot.width - 1));
                    let expected_db =
                        -3.8 + f64::from(distance * density) * 24. / f64::from(plot.height - 1);
                    assert!((frequency - expected_hz).abs() < 1e-9);
                    assert!((gain_db - expected_db).abs() < 1e-9);
                    rack.scene(view);
                }
                pointer(&mut rack, "drag", origin);
                assert_eq!(
                    *rack.app.eq.active(),
                    initial,
                    "returning to pickup must restore exact unrounded parameters"
                );
                pointer(&mut rack, "up", origin);
                assert!(rack.eq_drag.is_none() && !rack.eq_dirty);
                pointer(&mut rack, "drag", [origin[0] + 20, origin[1]]);
                assert_eq!(*rack.app.eq.active(), initial, "release must end capture");
                pointer(&mut rack, "down", origin);
                pointer(&mut rack, "drag", [origin[0], origin[1] + density]);
                rack.input(Input::CancelPointer);
                assert!(rack.eq_drag.is_none() && !rack.eq_dirty);
                let cancelled = rack.app.eq.active().clone();
                pointer(&mut rack, "drag", origin);
                assert_eq!(*rack.app.eq.active(), cancelled, "cancel must end capture");
                *rack.app.eq.active_mut() = initial.clone();
                rack.app.finish_eq_edit();
            }
            rack.scene(viewport);
            click(&mut rack, "eq-save-current");
            assert!(matches!(
                rack.app.edit.eq_value.as_ref().unwrap().field,
                EqField::SaveAsProfile
            ));
            rack.input(Input::Key {
                code: "char:u".into(),
                modifiers: KeyModifiers::CONTROL.bits(),
            });
            rack.input(Input::Paste {
                text: "Headphone reference".into(),
            });
            let stage_before = rack.app.eq.stage;
            rack.input(Input::Key {
                code: "right".into(),
                modifiers: 0,
            });
            assert_eq!(
                rack.app.eq.stage, stage_before,
                "name forms must own navigation keys"
            );
            for density in [1, 2] {
                let view = Viewport {
                    width: 1400 * density,
                    height: 1200 * density,
                    columns: 140,
                    rows: 60,
                    generation: 0,
                };
                let scene = rack.scene(view);
                assert!(rack
                    .surface
                    .as_ref()
                    .unwrap()
                    .hits
                    .iter()
                    .any(|h| h.action == "eq-form:accept"));
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("eq-save-as-{density}x.png")))?;
            }
            click(&mut rack, "eq-form:accept");
            assert!(rack.app.edit.eq_value.is_none());
            assert_eq!(rack.app.eq.active().name, "Headphone reference");
            let headphone = rack.app.eq.profile;
            let path = rack.app.eq.active().managed_path()?;
            let saved = std::fs::read(&path)?;
            if let Filter::Biquad { gain_db, .. } = &mut rack.app.eq.active_mut().stages[2].filter {
                *gain_db = -4.2;
            }
            rack.app.finish_eq_edit();
            assert_eq!(
                std::fs::read(&path)?,
                saved,
                "editing must not overwrite a saved profile"
            );
            assert!(rack.app.eq.modified(headphone));
            let recovered = EqState::from_config(&cfg);
            assert_eq!(
                recovered.active(),
                rack.app.eq.active(),
                "restart must recover the working curve"
            );
            assert!(recovered.modified(recovered.profile));
            rack.app.select_eq_profile(0);
            rack.app.select_eq_profile(headphone);
            assert!(
                rack.app.eq.modified(headphone),
                "switching must retain unsaved edits"
            );
            rack.app.save_eq_profile();
            assert!(!rack.app.eq.modified(headphone));
            assert_ne!(std::fs::read(&path)?, saved);
            assert_eq!(Profile::parse_file(&path)?, *rack.app.eq.active());
            // Save As detects collisions and leaves both the name form and
            // original file intact, so the user can fix the name or cancel.
            rack.app.begin_eq_name(EqField::SaveAsProfile);
            rack.app.edit.eq_value.as_mut().unwrap().value = "Headphone reference".into();
            rack.app
                .eq_value_type(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(!rack.app.edit.eq_value.as_ref().unwrap().error.is_empty());
            rack.app
                .eq_value_type(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            rack.app
                .commit_eq_name(EqField::SaveAsProfile, "Studio speakers", headphone)?;
            let speakers = rack.app.eq.profile;
            rack.app.select_eq_profile(headphone);
            rack.app
                .commit_eq_name(EqField::RenameProfile, "Studio monitors", speakers)?;
            assert_eq!(
                rack.app.eq.profile, headphone,
                "renaming another profile must not load it"
            );
            assert!(!crate::paths::equalizer_dir()?
                .join("Studio speakers.txt")
                .exists());
            assert!(crate::paths::equalizer_dir()?
                .join("Studio monitors.txt")
                .exists());
            // Search owns letter keys while the modal is open; choosing a row
            // does not change audio until Load, and delete requires confirmation.
            rack.app.open_eq_profiles();
            rack.input(Input::Paste {
                text: "STUDIO monitors".into(),
            });
            rack.scene(viewport);
            assert_eq!(
                rack.app
                    .edit
                    .eq_profiles
                    .as_ref()
                    .unwrap()
                    .matches(&rack.app.eq),
                [speakers]
            );
            click(&mut rack, "eq-library:row:0");
            assert_eq!(rack.app.eq.profile, headphone);
            click(&mut rack, "eq-library:delete");
            rack.scene(viewport);
            click(&mut rack, "eq-library:cancel-delete");
            assert!(crate::paths::equalizer_dir()?
                .join("Studio monitors.txt")
                .exists());
            rack.scene(viewport);
            click(&mut rack, "eq-library:delete");
            rack.scene(viewport);
            click(&mut rack, "eq-library:confirm-delete");
            assert!(!crate::paths::equalizer_dir()?
                .join("Studio monitors.txt")
                .exists());
            rack.app.eq_library_action("close");
            // Reverting is explicit and confirmed, and leaves the saved file intact.
            if let Filter::Biquad { gain_db, .. } = &mut rack.app.eq.active_mut().stages[2].filter {
                *gain_db = -8.;
            }
            rack.app.finish_eq_edit();
            rack.app.open_eq_profiles();
            rack.scene(viewport);
            click(&mut rack, "eq-library:revert");
            rack.scene(viewport);
            click(&mut rack, "eq-library:cancel-delete");
            assert!(rack.app.eq.modified(headphone));
            rack.scene(viewport);
            click(&mut rack, "eq-library:revert");
            rack.scene(viewport);
            click(&mut rack, "eq-library:confirm-revert");
            assert!(!rack.app.eq.modified(headphone));
            assert_eq!(Profile::parse_file(&path)?, *rack.app.eq.active());
            rack.app.eq_library_action("close");
            // External changes are never silently overwritten or deleted.
            let disk_before = std::fs::read(&path)?;
            let mut external = rack.app.eq.active().clone();
            if let Filter::Preamp { gain_db } = &mut external.stages[0].filter {
                *gain_db = -6.;
            }
            std::fs::write(&path, external.to_apo())?;
            let external_bytes = std::fs::read(&path)?;
            rack.app.save_eq_profile();
            assert_eq!(std::fs::read(&path)?, external_bytes);
            std::fs::write(&path, &disk_before)?;
            // A file absent from the in-memory library still cannot be clobbered by Save As.
            let occupied = crate::paths::equalizer_dir()?.join("Occupied.txt");
            std::fs::write(&occupied, "external file")?;
            assert!(rack
                .app
                .commit_eq_name(EqField::SaveAsProfile, "Occupied", headphone)
                .is_err());
            assert_eq!(std::fs::read_to_string(&occupied)?, "external file");
            std::fs::remove_file(occupied)?;
            // Restore the reference sample's gain for native visual captures.
            if let Filter::Biquad { gain_db, .. } = &mut rack.app.eq.active_mut().stages[2].filter {
                *gain_db = -3.8;
            }
            rack.app.finish_eq_edit();
            rack.app.save_eq_profile();
            rack.app.eq.stage = 2;
            for density in [1, 2] {
                let view = Viewport {
                    width: 1400 * density,
                    height: 1200 * density,
                    columns: 140,
                    rows: 60,
                    generation: 0,
                };
                let scene = rack.scene(view);
                let image = starkit::terminal_graphics::renderer::render_reference(&scene)?;
                image.save(output.join(format!("eq-reference-rack-{density}x.png")))?;
                starkit::image::imageops::crop_imm(
                    &image,
                    0,
                    238 * density,
                    1400 * density,
                    u32::from(EQ_HEIGHT) * density,
                )
                .to_image()
                .save(output.join(format!("eq-reference-{density}x.png")))?;
                let surface = rack.surface.as_ref().unwrap();
                assert_eq!(
                    surface
                        .hits
                        .iter()
                        .filter(|h| h.action.starts_with("eq-band:"))
                        .count(),
                    3
                );
                assert!(surface
                    .nodes
                    .iter()
                    .any(|n| matches!(n,Primitive::Text{text,..}if text=="BAND 2 · PEAKING")));
                for action in ["eq-frequency", "eq-gain", "eq-width", "eq-channels"] {
                    assert!(surface.hits.iter().any(|h| h.action == action));
                }
                click(&mut rack, "eq-profiles");
                let scene = rack.scene(view);
                let surface = rack.surface.as_ref().unwrap();
                assert!(surface.hits.iter().any(|h| h.action == "eq-library:load"));
                assert!(
                    !surface.hits.iter().any(|h| h.action == "eq-plot"),
                    "modal must hide underlying controls"
                );
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("eq-profiles-{density}x.png")))?;
                click(&mut rack, "eq-library:close");
            }
            for density in [1, 2] {
                let view = Viewport {
                    width: 900 * density,
                    height: 700 * density,
                    columns: 90,
                    rows: 35,
                    generation: 0,
                };
                let scene = rack.scene(view);
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("eq-reference-narrow-{density}x.png")))?;
                click(&mut rack, "eq-profiles");
                let scene = rack.scene(view);
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("eq-profiles-narrow-{density}x.png")))?;
                click(&mut rack, "eq-library:close");
            }
            rack.app.eq = original_eq;
            rack.app.look.theme = original_theme;
            rack.reference_colors = original_reference;
            rack.offset = original_offset;
            rack.app.apply_eq();
            rack.scene(v);
        }
        rack.app.player.queue.lock().unwrap().set_shuffle(false);
        for density in [1, 2] {
            let viewport = Viewport {
                width: 1400 * density,
                height: 1800 * density,
                columns: 140,
                rows: 90,
                generation: 0,
            };
            rack.app.panels.padding_x = 3;
            rack.app.panels.padding_y = 2;
            for (name, focus) in [
                ("settings:player", Focus::Player),
                ("settings:eq", Focus::Equalizer),
                ("settings:album", Focus::Album),
                ("settings:activity", Focus::History),
                ("settings:playlist", Focus::Playlist),
            ] {
                rack.app.panels.focus = if focus == Focus::Player {
                    Focus::Playlist
                } else {
                    Focus::Player
                };
                rack.scene(viewport);
                press_button(&mut rack, name, 0);
                assert_eq!(rack.app.panels.focus, focus);
                assert!(
                    rack.app.over.settings.is_none(),
                    "first click only selects {name}"
                );
                assert!(rack.drag.is_none());
                assert!(rack.skin.pressed.is_none());
                rack.scene(viewport);
                click(&mut rack, name);
                let settings = rack.app.over.settings.as_ref().expect(name);
                assert_eq!(settings.panel, focus);
                assert_eq!(settings.kind, Overlay::Settings);
                assert!(rack.folded.is_empty());
                rack.scene(viewport);
                assert!(rack.overlay);
                rack.input(Input::Key {
                    code: "escape".into(),
                    modifiers: 0,
                });
            }
            rack.scene(viewport);
            // Catalog links also share the foldable header, but this check
            // must not launch a browser or contact a catalog service.
            for name in ["catalog-artist", "catalog-album"] {
                let surface = rack.surface.as_ref().unwrap();
                let rect = surface
                    .hits
                    .iter()
                    .find(|hit| hit.action == name)
                    .unwrap()
                    .rect;
                assert_eq!(
                    surface
                        .hit(rect.x + rect.width / 2, rect.y + rect.height / 2)
                        .unwrap()
                        .action,
                    name
                );
            }
            click(&mut rack, "gravity");
            assert_eq!(
                rack.app.over.settings.as_ref().unwrap().kind,
                Overlay::Gravity
            );
            rack.input(Input::Key {
                code: "escape".into(),
                modifiers: 0,
            });
            rack.scene(viewport);
            click(&mut rack, "sorting");
            assert_eq!(
                rack.app.over.settings.as_ref().unwrap().kind,
                Overlay::Filter
            );
            rack.scene(viewport);
            let grouping = rack.app.player.queue.lock().unwrap().grouping();
            click(&mut rack, "popup:row:0");
            assert_ne!(rack.app.player.queue.lock().unwrap().grouping(), grouping);
            rack.input(Input::Key {
                code: "escape".into(),
                modifiers: 0,
            });
            rack.scene(viewport);
            let enabled = rack.app.eq.enabled;
            click(&mut rack, "eq-enabled");
            assert_ne!(rack.app.eq.enabled, enabled);
            let volume = rack.app.player.volume();
            rack.app.player.set_volume(0.1);
            rack.app.panels.focus = Focus::Playlist;
            rack.scene(viewport);
            press_button(&mut rack, "volume", 0);
            assert_eq!(rack.app.panels.focus, Focus::Player);
            let rect = rack
                .surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .find(|hit| hit.action == "volume")
                .unwrap()
                .rect;
            for action in ["drag", "up"] {
                rack.input(Input::Pointer {
                    action: action.into(),
                    button: 0,
                    x: 0,
                    y: 0,
                    pixel: Some([u32::from(rect.x + rect.width - 1), u32::from(rect.y)]),
                    modifiers: 0,
                });
            }
            assert!(
                (rack.app.player.volume() - 0.1).abs() < 0.001,
                "selecting the player must not start a volume drag"
            );
            click(&mut rack, "volume");
            assert!((rack.app.player.volume() - 0.5).abs() < 0.05);
            rack.app.player.set_volume(volume);
        }
        rack.app.panels.padding_x = 0;
        rack.app.panels.padding_y = 0;
        rack.scene(v);
        // The actual worker consumes transport commands even without a device.
        // Exercise pause, resume and stop rather than only checking hit bounds.
        fn wait_state(rack: &Rack<'_>, expected: PlayState) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while rack.app.player.state.state() != expected && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(rack.app.player.state.state(), expected);
        }
        rack.app.panels.focus = Focus::Playlist;
        rack.scene(v);
        click(&mut rack, "pause");
        wait_state(&rack, PlayState::Paused);
        assert_eq!(rack.app.panels.focus, Focus::Player);
        rack.app.panels.focus = Focus::Equalizer;
        rack.scene(v);
        // One click must resume from another pane; repeated Play presses
        // before the worker responds must never become queued toggles.
        for _ in 0..5 {
            click(&mut rack, "play");
        }
        wait_state(&rack, PlayState::Playing);
        assert_eq!(rack.app.panels.focus, Focus::Player);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(rack.app.player.state.state(), PlayState::Playing);
        rack.app.panels.focus = Focus::Album;
        rack.scene(v);
        click(&mut rack, "stop");
        wait_state(&rack, PlayState::Stopped);

        // Exercise the displayed groups and track context actions, not just
        // the sorting switch. Interleaved albums must become two real groups
        // while retaining the current decoder's queue occurrence.
        let grouped_items = (0..6)
            .map(|i| {
                let mut item = QueueItem::new(crate::playlist::uri::TrackUri::parse(&format!(
                    "album-{}/track-{i}.flac",
                    i % 2
                )));
                item.title = Some(format!("Song {i}"));
                item.artist = Some("Artist".into());
                item.album = Some(
                    if i % 2 == 0 {
                        "First album"
                    } else {
                        "Second album"
                    }
                    .into(),
                );
                item.year = Some(if i % 2 == 0 { 1990 } else { 2000 });
                item.track_no = Some(i + 1);
                item
            })
            .collect();
        {
            let mut queue = rack.app.player.queue.lock().unwrap();
            queue.set_tracks(grouped_items);
            queue.set_grouping(None);
            queue.set_shuffle(true);
        }
        rack.app.queue.folded.clear();
        let current = rack.app.player.current_item().unwrap().uri;
        let viewport = Viewport {
            width: 1400,
            height: 1800,
            columns: 140,
            rows: 90,
            generation: 0,
        };
        rack.scene(viewport);
        click(&mut rack, "sorting");
        rack.scene(viewport);
        click(&mut rack, "popup:row:0");
        rack.scene(viewport);
        assert!(rack.app.session_grouped());
        assert!(!rack.app.player.queue.lock().unwrap().shuffled());
        assert_eq!(rack.app.player.current_item().unwrap().uri, current);
        let albums = |rack: &Rack<'_>| {
            rack.app
                .queue
                .rows
                .rows()
                .iter()
                .filter_map(|row| {
                    if let playlist::Row::Section { label, .. } = row {
                        Some(label.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let ascending = albums(&rack);
        assert_eq!(ascending.len(), 2);
        assert!(ascending[0].contains("First album"));
        assert!(ascending[1].contains("Second album"));
        click(&mut rack, "popup:row:1");
        rack.scene(viewport);
        assert_eq!(
            albums(&rack),
            ascending.into_iter().rev().collect::<Vec<_>>()
        );
        click(&mut rack, "popup:close");
        let grouped_scene = rack.scene(viewport);
        starkit::terminal_graphics::renderer::render_reference(&grouped_scene)?
            .save(output.join("album-grouping.png"))?;
        // Check actual raster colors and mouse focus after composition and
        // density scaling, including paired panes and their blank interiors.
        let previous_theme = rack.app.look.theme.clone();
        let previous_reference = rack.reference_colors;
        rack.app.look.theme = crate::theme::builtin::load("kanagawa").unwrap();
        rack.reference_colors = false;
        for density in [1, 2] {
            let viewport = Viewport {
                width: 1400 * density,
                height: 1800 * density,
                columns: 140,
                rows: 90,
                generation: 0,
            };
            for (name, focus) in [
                ("player", Focus::Player),
                ("eq", Focus::Equalizer),
                ("album", Focus::Album),
                ("activity", Focus::History),
                ("playlist", Focus::Playlist),
            ] {
                rack.scene(viewport);
                let rect = rack
                    .surface
                    .as_ref()
                    .unwrap()
                    .hits
                    .iter()
                    .find(|h| h.action == format!("focus:{name}"))
                    .unwrap()
                    .rect;
                let pixel = [
                    u32::from(rect.x + rect.width - 5 * density as u16),
                    u32::from(rect.y + rect.height - 5 * density as u16),
                ];
                rack.input(Input::Pointer {
                    action: "down".into(),
                    button: 0,
                    x: 0,
                    y: 0,
                    pixel: Some(pixel),
                    modifiers: 0,
                });
                assert_eq!(rack.app.panels.focus, focus);
                let scene = rack.scene(viewport);
                let image = starkit::terminal_graphics::renderer::render_reference(&scene)?;
                let accent = rack.palette().accent;
                assert_eq!(
                    image
                        .get_pixel(u32::from(rect.x + rect.width / 2), u32::from(rect.y))
                        .0,
                    [accent[0], accent[1], accent[2], 255]
                );
                assert_ne!(
                    image
                        .get_pixel(
                            u32::from(rect.x + rect.width / 2),
                            u32::from(rect.y) + density
                        )
                        .0,
                    [accent[0], accent[1], accent[2], 255],
                    "{name}: focus border must be one logical pixel thick"
                );
                if name == "playlist" {
                    image.save(output.join(format!("kanagawa-grouped-focus-{density}x.png")))?;
                }
            }
        }
        rack.app.look.theme = previous_theme;
        rack.reference_colors = previous_reference;
        rack.scene(viewport);
        assert_eq!(rack.app.queue.rows.rows().len(), 8);
        click(&mut rack, "playlist:0");
        rack.scene(viewport);
        assert_eq!(
            rack.app.queue.rows.rows().len(),
            5,
            "fold only the clicked album"
        );
        click(&mut rack, "playlist:0");
        rack.scene(viewport);
        assert_eq!(rack.app.queue.rows.rows().len(), 8);
        // A single row press must both focus the queue and select that row;
        // no second press or double-click timeout is required for feedback.
        let target = match rack.app.queue.rows.rows()[2] {
            playlist::Row::Track(index) => index,
            _ => panic!("expected selectable track"),
        };
        let original_cursor = rack.app.queue.cursor;
        let original_click = rack.app.edit.last_click;
        let playing_uri = rack.app.player.current_item().map(|item| item.uri);
        for density in [1, 2] {
            let view = Viewport {
                width: viewport.width * density,
                height: viewport.height * density,
                ..viewport
            };
            rack.app.panels.focus = Focus::Player;
            rack.app.queue.cursor = usize::from(target == 0);
            rack.app.edit.last_click = Default::default();
            let before = rack.scene(view);
            let row = rack
                .surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .find(|h| h.action == "playlist:2")
                .unwrap()
                .rect;
            let point = (
                u32::from(row.x + row.width - 2),
                u32::from(row.y + row.height / 2),
            );
            let before = starkit::terminal_graphics::renderer::render_reference(&before)?;
            press_button(&mut rack, "playlist:2", 0);
            assert_eq!(rack.app.panels.focus, Focus::Playlist);
            assert_eq!(
                rack.app.queue.cursor, target,
                "first click must select immediately"
            );
            let scene = rack.scene(view);
            let after = starkit::terminal_graphics::renderer::render_reference(&scene)?;
            assert_ne!(
                before.get_pixel(point.0, point.1),
                after.get_pixel(point.0, point.1),
                "the next frame must paint the new row highlight"
            );
            assert_eq!(
                rack.app.player.current_item().map(|item| item.uri),
                playing_uri,
                "a single selection must not start playback"
            );
        }
        rack.app.queue.cursor = original_cursor;
        rack.app.edit.last_click = original_click;
        rack.scene(viewport);
        for local in [true, false] {
            rack.local_hover = local;
            rack.skin.hovered = None;
            let before = rack.scene(viewport);
            let row = rack
                .surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .find(|h| h.action == "playlist:2")
                .unwrap()
                .rect;
            let cursor = rack.app.queue.cursor;
            rack.input(Input::Pointer {
                action: "move".into(),
                button: 0,
                x: row.x / 10,
                y: row.y / 20,
                pixel: Some([u32::from(row.x + 2), u32::from(row.y + 2)]),
                modifiers: 0,
            });
            assert_eq!(rack.app.queue.cursor, cursor, "hover cannot select a row");
            let after = rack.scene(viewport);
            if local {
                assert!(rack.skin.hovered.is_none());
                assert!(
                    before.same_content(&after),
                    "local hover needs no new controller scene"
                );
                assert!(!rack.surface.as_ref().unwrap().hover_fills.is_empty());
            } else {
                assert_eq!(rack.skin.hovered.as_deref(), Some("playlist:2"));
                assert!(rack.surface.as_ref().unwrap().hover_fills.is_empty());
            }
        }
        rack.local_hover = true;
        rack.skin.hovered = None;
        rack.scene(viewport);
        let source = match rack.app.queue.rows.rows()[2] {
            playlist::Row::Track(i) => rack.app.queue.items[i].clone(),
            _ => panic!("expected track"),
        };
        rack.app.panels.focus = Focus::Player;
        let cursor = rack.app.queue.cursor;
        press_button(&mut rack, "playlist:2", 1);
        assert_eq!(rack.app.panels.focus, Focus::Playlist);
        assert_eq!(rack.app.queue.cursor, cursor);
        assert!(
            rack.app.over.playlist_add.is_none(),
            "first right-click only selects"
        );
        click_button(&mut rack, "playlist:2", 1);
        let context_scene = rack.scene(viewport);
        assert_eq!(
            rack.app.over.playlist_add.as_ref().unwrap().source.uri,
            source.uri
        );
        for i in 0..6 {
            assert!(rack
                .surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .any(|h| h.action == format!("popup:row:{i}")));
        }
        starkit::terminal_graphics::renderer::render_reference(&context_scene)?
            .save(output.join("track-context-menu.png"))?;
        // Pointer anchoring, edge flipping, hover and outside dismissal must
        // work at both physical pixel densities without moving the playlist.
        for density in [1, 2] {
            let viewport = Viewport {
                width: 900 * density,
                height: 700 * density,
                columns: 100,
                rows: 35,
                generation: 0,
            };
            for [x, y] in [[100, 100], [895, 695]] {
                rack.context_anchor = Some([x * density, y * density]);
                let scene = rack.scene(viewport);
                let surface = rack.surface.as_ref().unwrap();
                let body = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "popup:body")
                    .unwrap()
                    .rect;
                assert!(u32::from(body.x + body.width) <= viewport.width);
                assert!(u32::from(body.y + body.height) <= viewport.height);
                if x == 100 {
                    assert_eq!(u32::from(body.x), (x + 2) * density);
                } else {
                    assert!(u32::from(body.x) < x * density);
                }
                let row = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "popup:row:4")
                    .unwrap()
                    .rect;
                rack.input(Input::Pointer {
                    action: "move".into(),
                    button: 0,
                    x: 0,
                    y: 0,
                    pixel: Some([u32::from(row.x + 8), u32::from(row.y + 8)]),
                    modifiers: 0,
                });
                assert_eq!(rack.app.over.playlist_add.as_ref().unwrap().cursor, 4);
                assert_eq!(
                    rack.app.player.queue.lock().unwrap().len(),
                    6,
                    "hover never executes an action"
                );
                if x == 895 {
                    starkit::terminal_graphics::renderer::render_reference(&scene)?
                        .save(output.join(format!("track-context-edge-{density}x.png")))?;
                }
            }
        }
        rack.scene(viewport);
        let view_index = rack.app.over.playlist_add.as_ref().unwrap().view_index;
        rack.input(Input::Pointer {
            action: "down".into(),
            button: 0,
            x: 0,
            y: 0,
            pixel: Some([0, 0]),
            modifiers: 0,
        });
        assert!(rack.app.over.playlist_add.is_none());
        rack.app.open_track_menu(source.clone(), view_index);
        rack.scene(viewport);
        click(&mut rack, "popup:row:1");
        assert!(rack.app.over.playlist_add.is_none());
        assert_eq!(rack.app.player.queue.lock().unwrap().len(), 5);
        assert!(!rack
            .app
            .player
            .queue
            .lock()
            .unwrap()
            .tracks()
            .iter()
            .any(|i| i.uri == source.uri));

        // Long values must survive native layout in full, and the final row
        // of a scrolling menu must remain reachable at either pixel density.
        for density in [1, 2] {
            let viewport = Viewport {
                width: 800 * density,
                height: 600 * density,
                columns: 100,
                rows: 30,
                generation: 0,
            };
            rack.app.open_settings(Focus::Equalizer);
            let state = rack.app.over.settings.as_mut().unwrap();
            state.title =
                "A long equalizer profile title that must wrap rather than disappear".into();
            state.rows = (0..40)
                .map(|i| {
                    settings::Row::setting(
                        &format!("Setting {i}"),
                        format!("{}: complete-value-{i}", "long profile value ".repeat(6)),
                    )
                })
                .collect();
            state.items = vec![Setting::Visualizer; 40];
            state.cursor = 39;
            let scene = rack.scene(viewport);
            let surface = rack.surface.as_ref().unwrap();
            let texts = surface
                .nodes
                .iter()
                .filter_map(|node| {
                    if let Primitive::Text { text, .. } = node {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<String>();
            assert!(texts.contains("complete-value-39"));
            assert!(surface.hits.iter().any(|h| h.action == "popup:row:39"));
            assert!(rack.app.over.settings.as_ref().unwrap().scroll > 0);
            starkit::terminal_graphics::renderer::render_reference(&scene)?
                .save(output.join(format!("settings-long-values-{density}x.png")))?;
            click(&mut rack, "popup:close");
        }
        // Graphical playlist uses the ASCII section/track model, including
        // disc numbers, undated records and untagged tracks at the bottom.
        let mut items = Vec::new();
        for (album, year, disc, number) in [
            (None, Some(1901), None, None),
            (Some("Later album"), Some(2005), Some(1), Some(1)),
            (Some("Early album"), Some(1990), Some(2), Some(2)),
            (Some("Undated album"), None, None, Some(1)),
            (Some("Early album"), Some(1990), Some(1), Some(1)),
        ] {
            let mut item = QueueItem::new(crate::playlist::uri::TrackUri::parse(&format!(
                "section-{}.flac",
                items.len()
            )));
            item.album = album.map(str::to_owned);
            item.year = year;
            item.disc_no = disc;
            item.track_no = number;
            item.artist = Some("Artist".into());
            item.title = Some(format!("Song {}", items.len()));
            items.push(item);
        }
        {
            let mut queue = rack.app.player.queue.lock().unwrap();
            queue.set_tracks(items);
            queue.set_shuffle(false);
            queue.set_grouping(Some(false));
        }
        rack.app.queue.cursor = 0;
        rack.app.queue.scroll = 0;
        rack.app.queue.folded.clear();
        rack.app.queue.fold_gen += 1;
        rack.playlist_scrolled = None;
        rack.offset = 0;
        for density in [1, 2] {
            let v = Viewport {
                width: 1400 * density,
                height: 1800 * density,
                columns: 140,
                rows: 90,
                generation: 0,
            };
            let scene = rack.scene(v);
            assert_eq!(
                albums(&rack),
                [
                    "1990 · Early album",
                    "2005 · Later album",
                    "Undated album",
                    "no album"
                ]
            );
            let surface = rack.surface.as_ref().unwrap();
            let texts = surface
                .nodes
                .iter()
                .filter_map(|n| match n {
                    Primitive::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(texts.iter().any(|t| t.trim() == "1.01"));
            assert!(texts.iter().any(|t| t.trim() == "2.02"));
            assert!(texts.contains(&"Artist — Song 4"));
            assert!(texts.contains(&"2 tracks"));
            assert!(
                surface.nodes.iter().any(|n| matches!(n,
                Primitive::Fill { rect, .. } if rect.height == density as u16 && rect.width > 100))
            );
            assert!(rack.playlist_bars.visible().next().is_none());
            for h in surface
                .hits
                .iter()
                .filter(|h| h.action.starts_with("close:"))
            {
                assert_eq!(
                    (h.rect.width, h.rect.height),
                    (30 * density as u16, 24 * density as u16)
                );
            }
            starkit::terminal_graphics::renderer::render_reference(&scene)?
                .save(output.join(format!("playlist-album-sections-{density}x.png")))?;
        }
        rack.app
            .player
            .queue
            .lock()
            .unwrap()
            .set_grouping(Some(true));
        rack.scene(viewport);
        assert_eq!(
            albums(&rack),
            [
                "2005 · Later album",
                "1990 · Early album",
                "Undated album",
                "no album"
            ]
        );

        // Filter the actual graphical row model, not just the predicate. The
        // draft previews live; cancel restores the committed query; keyboard,
        // paste and mouse Apply/Clear all share the ASCII editing model.
        rack.app.panels.eq = false;
        rack.app.panels.album = false;
        rack.app.panels.history = false;
        for density in [1, 2] {
            let v = Viewport {
                width: 900 * density,
                height: 700 * density,
                columns: 90,
                rows: 35,
                generation: 0,
            };
            rack.app.edit.words.clear();
            rack.app.edit.gen += 1;
            rack.offset = 0;
            rack.scene(v);
            let before = rack.app.player.current_item().unwrap().uri;
            rack.input(Input::Key {
                code: "char:/".into(),
                modifiers: 0,
            });
            rack.scene(v);
            assert!(rack.app.edit.typing.is_some());
            rack.input(Input::Paste {
                text: "Early".into(),
            });
            let scene = rack.scene(v);
            assert_eq!(rack.app.queue.matched_count, 2);
            assert_eq!(albums(&rack), ["1990 · Early album"]);
            assert!(
                rack.app.edit.words.is_empty(),
                "the live draft is not yet committed"
            );
            let surface = rack.surface.as_ref().unwrap();
            assert!(surface.hits.iter().any(|h| h.action == "filter:apply"));
            assert!(
                !surface
                    .hits
                    .iter()
                    .any(|h| h.action.starts_with("playlist:")),
                "modal clicks cannot leak to the playlist"
            );
            let texts = surface
                .nodes
                .iter()
                .filter_map(|n| match n {
                    Primitive::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(texts.contains(&"FILTER PLAYLIST"));
            assert!(texts.contains(&"Match words in artist, title, album, year or path."));
            assert!(texts.contains(&"Enter apply · Esc cancel · Ctrl+U clear"));
            surface.validate()?;
            starkit::terminal_graphics::renderer::render_reference(&scene)?
                .save(output.join(format!("filter-live-{density}x.png")))?;
            click(&mut rack, "filter:cancel");
            rack.scene(v);
            assert_eq!(rack.app.queue.matched_count, 5);
            rack.app.open_filter_box();
            rack.scene(v);
            let query = format!("{}末尾é", "long-query ".repeat(20));
            rack.input(Input::Paste {
                text: query.clone(),
            });
            let scene = rack.scene(v);
            assert_eq!(rack.app.edit.typing.as_ref().unwrap().text(), query);
            assert_eq!(rack.app.queue.matched_count, 0);
            assert!(rack
                .surface
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|n| matches!(n,
                Primitive::Text { text, .. } if text.starts_with("No tracks match"))));
            starkit::terminal_graphics::renderer::render_reference(&scene)?
                .save(output.join(format!("filter-long-no-results-{density}x.png")))?;
            click(&mut rack, "filter:clear");
            rack.scene(v);
            assert_eq!(rack.app.queue.matched_count, 5);
            rack.input(Input::Paste {
                text: "Artist\nSong 4\r\n".into(),
            });
            rack.scene(v);
            assert_eq!(rack.app.queue.matched_count, 1);
            click(&mut rack, "filter:apply");
            rack.scene(v);
            assert!(rack.app.edit.typing.is_none());
            assert_eq!(rack.app.edit.words, "Artist Song 4");
            assert_eq!(rack.app.queue.matched_count, 1);
            assert_eq!(
                rack.app.player.current_item().unwrap().uri,
                before,
                "filtering never changes playback"
            );
            rack.app.open_filter_box();
            rack.scene(v);
            assert_eq!(
                rack.app.edit.typing.as_ref().unwrap().text(),
                "Artist Song 4"
            );
            click(&mut rack, "filter:clear");
            rack.scene(v);
            click(&mut rack, "filter:close");
            rack.scene(v);
            assert_eq!(
                rack.app.queue.matched_count, 1,
                "closing restores the committed filter"
            );
            rack.app.open_filter_box();
            rack.scene(v);
            click(&mut rack, "filter:clear");
            rack.scene(v);
            rack.input(Input::Key {
                code: "enter".into(),
                modifiers: 0,
            });
            let scene = rack.scene(v);
            assert_eq!(rack.app.queue.matched_count, 5);
            assert!(rack.app.edit.words.is_empty());
            assert_eq!(scene.background, "transparent");
            let surface = rack.surface.as_ref().unwrap();
            assert_eq!(surface.background, "transparent");
            let frame = starkit::terminal_graphics::renderer::render_reference(&scene)?;
            assert_eq!(
                frame.get_pixel(0, 0)[3],
                0,
                "rounded pane corners reveal the terminal"
            );
            assert_eq!(
                frame.get_pixel(v.width / 2, 232 * density)[3],
                0,
                "rack gaps reveal the terminal"
            );
            assert_eq!(
                frame.get_pixel(v.width / 2, 2 * density)[3],
                255,
                "pane starts at the top edge"
            );
        }
        rack.app.panels.eq = true;
        rack.app.panels.album = true;
        rack.app.panels.history = true;
        // Drag the real scrollbar at both densities. Headings count toward
        // thumb size and an all-folded list retains the viewport on release.
        let long_items = (0..30_002)
            .map(|i| {
                let mut item = QueueItem::new(crate::playlist::uri::TrackUri::parse(&format!(
                    "scroll-{i}.flac"
                )));
                item.title = Some(format!("Song {i}"));
                if i < 30_000 {
                    item.album = Some(format!("Album {:02}", i / 5));
                    item.year = Some(1980 + i / 5);
                    item.track_no = Some((i % 5 + 1) as u32);
                }
                item
            })
            .collect();
        {
            let mut queue = rack.app.player.queue.lock().unwrap();
            queue.set_tracks(long_items);
            queue.set_grouping(Some(false));
        }
        rack.app.panels.eq = false;
        rack.app.panels.album = false;
        rack.app.panels.history = false;
        for density in [1, 2] {
            let v = Viewport {
                width: 1400 * density,
                height: 700 * density,
                columns: 140,
                rows: 42,
                generation: 0,
            };
            for folded in [false, true] {
                rack.app.queue.folded.clear();
                rack.app.queue.fold_gen += 1;
                rack.app.queue.cursor = 0;
                rack.app.queue.scroll = 0;
                rack.app.panels.focus = Focus::Playlist;
                rack.playlist_scrolled = None;
                rack.scene(v);
                if folded {
                    rack.app.queue.folded = rack
                        .app
                        .queue
                        .rows
                        .rows()
                        .iter()
                        .filter_map(|r| match r {
                            playlist::Row::Section { fold, .. } => Some(fold.clone()),
                            _ => None,
                        })
                        .collect();
                    rack.app.queue.fold_gen += 1;
                    rack.scene(v);
                }
                let (track, thumb) = rack.playlist_bars.visible().next().unwrap();
                let maximum = rack.app.queue.rows.len() - rack.playlist_visible;
                // KIT owns proportional sizing, clipping and drag arithmetic.
                assert!(thumb.len >= 24 * density as u16 && thumb.len <= track.height / 4);
                let painted = rack.surface.as_ref().unwrap().nodes.iter().any(|node| {
                    matches!(node,
                    Primitive::Fill { rect, color, .. } if rect.x == track.x + 6 * density as u16
                        && rect.y == track.y + thumb.start && rect.height == thumb.len
                        && color == &rack.selection_color())
                });
                assert!(
                    painted,
                    "paint and input must use the exact same physical thumb"
                );
                let x = u32::from(track.x + track.width / 2);
                for (action, y) in [
                    ("down", track.y + thumb.start + thumb.len / 2),
                    ("drag", track.y + track.height - 1),
                    ("up", track.y + track.height - 1),
                ] {
                    rack.input(Input::Pointer {
                        action: action.into(),
                        button: 0,
                        x: 0,
                        y: 0,
                        pixel: Some([x, u32::from(y)]),
                        modifiers: 0,
                    });
                }
                let scene = rack.scene(v);
                if folded {
                    let surface = rack.surface.as_ref().unwrap();
                    assert!(!surface.text_rules.is_empty());
                    surface.validate()?;
                    for rule in &surface.text_rules {
                        assert_eq!(rule.gap, 10 * rack.density);
                        assert!(matches!(
                            surface.nodes[usize::from(rule.node)],
                            Primitive::Text { .. }
                        ));
                    }
                }
                assert_eq!(
                    rack.app.queue.scroll, maximum,
                    "drag remains at the bottom after release"
                );
                let (track, thumb) = rack.playlist_bars.visible().next().unwrap();
                assert_eq!(thumb.start + thumb.len, track.height);
                assert!(rack
                    .surface
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|n| matches!(n,
                    Primitive::Text { text, .. } if text == "no album")));
                starkit::terminal_graphics::renderer::render_reference(&scene)?
                    .save(output.join(format!("playlist-scroll-{folded}-{density}x.png")))?;
                let offset = rack.offset;
                rack.input(Input::Pointer {
                    action: "scroll_up".into(),
                    button: 0,
                    x: 0,
                    y: 0,
                    pixel: Some([x, u32::from(track.y + 2)]),
                    modifiers: 0,
                });
                rack.scene(v);
                assert_eq!(rack.app.queue.scroll, maximum.saturating_sub(3));
                assert_eq!(rack.offset, offset, "playlist wheel does not move the rack");
                if !folded {
                    rack.scroll_playlist_to(500);
                    rack.scene(v);
                    let scroll = rack.app.queue.scroll;
                    let (row, selected) = rack
                        .app
                        .queue
                        .rows
                        .rows()
                        .iter()
                        .enumerate()
                        .skip(scroll + 2)
                        .take(rack.playlist_visible.saturating_sub(2))
                        .find_map(|(row, line)| match line {
                            playlist::Row::Track(track) => Some((row, *track)),
                            _ => None,
                        })
                        .unwrap();
                    let action = format!("playlist:{row}");
                    rack.app.edit.last_click = Default::default();
                    press_button(&mut rack, &action, 0);
                    rack.scene(v);
                    assert_eq!(
                        rack.app.queue.scroll, scroll,
                        "selection preserves the browsed viewport"
                    );
                    press_button(&mut rack, &action, 0);
                    assert_eq!(
                        rack.app.queue.scroll, scroll,
                        "double-click playback preserves scroll before redraw"
                    );
                    rack.scene(v);
                    assert_eq!(
                        rack.app.queue.scroll, scroll,
                        "double-click playback preserves scroll after redraw"
                    );
                    assert_eq!(rack.app.queue.cursor, selected);
                    assert_eq!(rack.app.panels.focus, Focus::Playlist);
                }
            }
        }
        rack.app.queue.folded.clear();
        rack.app.queue.fold_gen += 1;
        rack.playlist_scrolled = None;
        rack.app.queue.cursor = 0;
        rack.app.queue.scroll = 0;
        rack.app.panels.eq = true;
        rack.app.panels.album = true;
        rack.app.panels.history = true;
        // A real separate playback owner, at library scale. Local preferences
        // must not re-sort its queue, and the open menu must reflect each
        // acknowledged change immediately. Include duplicate occurrences.
        let owner = Arc::new(Player::detached());
        owner.state.playing.store(true, Relaxed);
        let tracks = (0..30_000)
            .map(|i| {
                let album = i % 1000;
                let mut item = QueueItem::new(crate::playlist::uri::TrackUri::parse(&format!(
                    "album-{album}/{}.flac",
                    i / 2
                )));
                item.title = Some(format!("Song {i}"));
                item.album = Some(format!("Album {album:04}"));
                item.year = Some(1900 + album % 100);
                item.track_no = Some((i / 1000 + 1) as u32);
                item
            })
            .collect();
        owner.set_queue_tracks(tracks);
        owner.queue.lock().unwrap().jump_to(4592);
        let playing = owner.current_item().unwrap().uri;
        let socket = std::env::temp_dir().join(format!("amp-rack-{}.sock", std::process::id()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        crate::ipc::spawn_at(
            socket.clone(),
            Arc::clone(&owner),
            crate::view::shared(),
            Arc::clone(&stop),
        )
        .expect("isolated playback owner");
        {
            let mut q = rack.app.player.queue.lock().unwrap();
            q.set_grouping(Some(true));
            q.set_manual_order(vec!["album 0999".into()]);
            q.set_shuffle(true);
        }
        rack.app
            .set_mirror(crate::mirror::Mirror::connect_at(socket.clone()).unwrap());
        // The UI retries a failed snapshot on its next tick. At 30k tracks,
        // one poll may time out under build load; wait for acknowledgement
        // before comparing orders rather than asserting against the old queue.
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            rack.app.poll_mirror();
            if rack.app.session.revision == owner.queue.lock().unwrap().revision() {
                break;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "owner queue snapshot did not arrive"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let assert_owner = |rack: &Rack<'_>| {
            let q = rack.app.player.queue.lock().unwrap();
            let leader = owner.queue.lock().unwrap();
            let expected = leader
                .view()
                .iter()
                .map(|&i| &leader.tracks()[i])
                .collect::<Vec<_>>();
            let actual = q.view().iter().map(|&i| &q.tracks()[i]).collect::<Vec<_>>();
            assert_eq!(actual, expected, "mirror preserves exact owner order");
            assert_eq!(
                q.revision(),
                leader.revision(),
                "queue edits use the owner's revision"
            );
            assert_eq!(q.current().unwrap().uri, playing);
        };
        assert_owner(&rack);
        crate::ipc::handle(&owner, &crate::view::shared(), "set-group album");
        rack.app.poll_mirror();
        rack.scene(v);
        let playing_cursor = rack.app.player.queue.lock().unwrap().view_cursor();
        let fold = playlist::fold_for_track(&rack.app.queue.items, playing_cursor).unwrap();
        rack.app.queue.folded.insert(fold.clone());
        rack.app.queue.fold_gen += 1;
        rack.app.edit.words = "no-match-for-reveal-test".into();
        rack.app.edit.gen += 1;
        rack.app.queue.cursor = 0;
        rack.playlist_scrolled = Some(0);
        rack.scene(v);
        click(&mut rack, "playlist:jump");
        rack.scene(v);
        assert_eq!(rack.app.queue.cursor, playing_cursor);
        assert!(rack.app.edit.words.is_empty());
        assert!(!rack.app.queue.folded.contains(&fold));
        let row = rack.app.queue.rows.row_of_track(playing_cursor).unwrap();
        assert!(
            (rack.app.queue.scroll..rack.app.queue.scroll + rack.playlist_visible).contains(&row)
        );
        assert_owner(&rack);
        let selected_cursor = (playing_cursor + 30) % rack.app.queue.items.len();
        rack.app.queue.cursor = selected_cursor;
        rack.scene(v);
        rack.scroll_playlist_to(u32::MAX);
        rack.scene(v);
        assert_eq!(
            rack.app.queue.cursor, selected_cursor,
            "scrolling retains selection"
        );
        let selected_row = rack.app.queue.rows.row_of_track(selected_cursor).unwrap();
        assert!(
            !(rack.app.queue.scroll..rack.app.queue.scroll + rack.playlist_visible)
                .contains(&selected_row)
        );
        owner.state.playing.store(false, Relaxed);
        rack.app.poll_mirror();
        click(&mut rack, "playlist:jump");
        rack.scene(v);
        assert_eq!(rack.app.queue.cursor, selected_cursor);
        let row = rack.app.queue.rows.row_of_track(selected_cursor).unwrap();
        assert!(
            (rack.app.queue.scroll..rack.app.queue.scroll + rack.playlist_visible).contains(&row)
        );
        assert_owner(&rack);
        owner.state.playing.store(true, Relaxed);
        crate::ipc::handle(&owner, &crate::view::shared(), "set-group off");
        rack.app.poll_mirror();
        rack.app.queue.group_desc = false;
        rack.app.open_filter();
        rack.scene(v);
        click(&mut rack, "popup:row:0");
        assert_owner(&rack);
        assert!(rack.app.session_grouped());
        assert_eq!(
            rack.app.over.settings.as_ref().unwrap().rows[0].value,
            "album by year"
        );
        assert_eq!(
            rack.app.over.settings.as_ref().unwrap().rows[1].value,
            "oldest first"
        );
        rack.scene(v);
        click(&mut rack, "popup:row:1");
        assert_owner(&rack);
        assert_eq!(
            rack.app.over.settings.as_ref().unwrap().rows[1].value,
            "newest first"
        );
        // External changes are reflected while the dialog remains open too.
        crate::ipc::handle(&owner, &crate::view::shared(), "set-group album");
        rack.app.poll_mirror();
        assert_owner(&rack);
        assert_eq!(
            rack.app.over.settings.as_ref().unwrap().rows[1].value,
            "oldest first"
        );
        crate::ipc::handle(&owner, &crate::view::shared(), "set-shuffle on");
        rack.app.poll_mirror();
        assert_owner(&rack);
        rack.scene(v);
        click(&mut rack, "popup:row:0");
        assert_owner(&rack);
        assert!(!owner.queue.lock().unwrap().shuffled());
        rack.scene(v);
        click(&mut rack, "popup:row:0");
        assert_owner(&rack);
        assert!(!rack.app.session_grouped());
        assert_eq!(
            rack.app.over.settings.as_ref().unwrap().rows[0].value,
            "playlist order"
        );
        rack.app.session.link = None;
        stop.store(true, Relaxed);
        let _ = crate::ipc::connect(&socket);
        rack.app.handle(Action::CloseOverlay);

        // Empty is rendered from real queue membership, not inferred from a
        // zero duration (which is also normal during decoder startup).
        rack.app.player.queue.lock().unwrap().set_tracks(Vec::new());
        rack.app.player.state.playing.store(false, Relaxed);
        rack.app.player.state.paused.store(false, Relaxed);
        rack.app.player.state.duration_frames.store(0, Relaxed);
        rack.app.player.state.position_frames.store(0, Relaxed);
        rack.app.player.state.sample_rate.store(0, Relaxed);
        rack.app.look.theme = crate::theme::builtin::load("catppuccin-mocha").unwrap();
        let empty = rack.scene(v);
        let hits = &rack.surface.as_ref().unwrap().hits;
        for action in ["previous", "play", "pause", "stop", "next", "seek"] {
            assert!(
                !hits.iter().any(|h| h.action == action),
                "empty queue target: {action}"
            );
        }
        starkit::terminal_graphics::renderer::render_reference(&empty)?
            .save(output.join("empty-rack.png"))?;
        // The real rack keeps a foreground disk wait visible at both bitmap
        // densities, even with animation effects and the visualizer disabled.
        rack.app.player.queue.lock().unwrap().set_tracks(vec![{
            let mut item = crate::playlist::queue::QueueItem::new(
                crate::playlist::uri::TrackUri::parse("/sleeping-drive/song.flac"),
            );
            item.artist = Some("Queen".into());
            item.title = Some("Another One Bites the Dust".into());
            item
        }]);
        rack.app.player.state.loading.store(true, Relaxed);
        rack.app.vis.mode = VisMode::Off;
        rack.app.fx.active = false;
        rack.app.advance_seek_phase(0.2);
        assert!(rack.app.look.seek_phase > 0.);
        rack.app.look.theme = crate::theme::builtin::load("winamp-silver").unwrap();
        for density in [1, 2] {
            let scene = rack.scene(Viewport {
                width: 1200 * density,
                height: 900 * density,
                columns: 120,
                rows: 45,
                generation: 0,
            });
            let contents = rack
                .surface
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .filter_map(|n| match n {
                    Primitive::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(contents.contains("LOADING TRACK"));
            assert!(contents.contains("Another One Bites the Dust"));
            assert!(!rack
                .surface
                .as_ref()
                .unwrap()
                .hits
                .iter()
                .any(|h| h.action == "seek"));
            starkit::terminal_graphics::renderer::render_reference(&scene)?
                .save(output.join(format!("loading-track-{density}x.png")))?;
        }
        rack.app.player.state.loading.store(false, Relaxed);
        rack.app.player.queue.lock().unwrap().set_tracks(Vec::new());
        // The main header closes the app even while another pane has focus.
        // Check actual pointer projection and non-overlapping settings at both
        // bitmap densities, including the artwork-independent renderer.
        for skins in [true, false] {
            rack.skins = skins;
            for density in [1, 2] {
                let viewport = Viewport {
                    width: 1200 * density,
                    height: 900 * density,
                    columns: 120,
                    rows: 45,
                    generation: 0,
                };
                rack.app.quit = false;
                rack.app.panels.focus = Focus::Playlist;
                rack.scene(viewport);
                let surface = rack.surface.as_ref().unwrap();
                let close = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "close:app")
                    .unwrap()
                    .rect;
                let settings = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "settings:player")
                    .unwrap()
                    .rect;
                assert!(settings.x + settings.width <= close.x);
                assert!(!rack.closed());
                let pixel = [
                    u32::from(close.x + close.width / 2),
                    u32::from(close.y + close.height / 2),
                ];
                rack.input(Input::Pointer {
                    action: "down".into(),
                    button: 0,
                    x: (pixel[0] * u32::from(viewport.columns) / viewport.width) as u16,
                    y: (pixel[1] * u32::from(viewport.rows) / viewport.height) as u16,
                    pixel: Some(pixel),
                    modifiers: 0,
                });
                assert!(
                    rack.closed(),
                    "main close failed: skins={skins}, density={density}"
                );
            }
        }
        rack.app.shutdown();
        Ok(())
    }
}
