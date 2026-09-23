//! Render the main player's body into the host's cell grid.
//!
//! The ordinary `PlayerView` remains the source of the clock, analyzer,
//! metadata, seek bar, and transport artwork. We render it offscreen at its
//! native height, then send only its borderless body. Short hosts select its
//! useful rows rather than introducing a second player design.

use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Color;
use starkit::ratatui::widgets::Widget;
use starkit::theme::{Resolve, ThemeFile};

use crate::audio::player::PlayState;
use crate::playlist::queue::RepeatMode;
use crate::theme::color::Rgb;
use crate::theme::Theme;
use crate::ui::panels::faces::{self, Button};
use crate::ui::panels::player::{self, Glyphs, PlayerView, SeekStyle};
use crate::ui::panels::visualizer::BarLayout;
use crate::vis::mode::VisMode;

use super::{Cell, GraphicsConfig, Palette, TransportImage};

const MAX_IMAGE_PIXELS: usize = 65_536;
const MAX_TOTAL_RGBA: usize = 262_144;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ImageKey {
    width: u16,
    height: u16,
    palette: Palette,
    state: PlayState,
    repeat: RepeatMode,
    graphics: GraphicsConfig,
}

/// Rasters change on resize, palette or active transport state, not each
/// analyzer frame. Keep the same five pixel buffers until one of those moves.
#[derive(Default)]
pub struct TransportImageCache {
    key: Option<ImageKey>,
    images: Vec<TransportImage>,
}

impl TransportImageCache {
    pub fn images(
        &mut self,
        width: u16,
        height: u16,
        palette: &Palette,
        player: &PlayerRenderState,
        graphics: GraphicsConfig,
    ) -> &[TransportImage] {
        let key = ImageKey {
            width,
            height,
            palette: *palette,
            state: player.state,
            repeat: player.repeat,
            graphics,
        };
        if self.key != Some(key) {
            self.images = transport_images(width, height, palette, player, graphics);
            self.key = Some(key);
        }
        &self.images
    }
}

fn transport_images(
    width: u16,
    height: u16,
    palette: &Palette,
    player: &PlayerRenderState,
    graphics: GraphicsConfig,
) -> Vec<TransportImage> {
    if width == 0
        || height == 0
        || graphics.cell_width == 0
        || graphics.cell_height == 0
        || graphics.cell_width > 64
        || graphics.cell_height > 128
    {
        return Vec::new();
    }
    let outer = Rect::new(0, 0, width.saturating_add(2), player::PANEL_ROWS);
    let Some(g) = player::geometry(
        outer,
        player.position,
        player.duration,
        player.repeat,
        Glyphs::ASCII,
    ) else {
        return Vec::new();
    };
    let t = theme(palette);
    let bg = t.bg;
    let plate = t.transport_button_bg;
    let plate_lit = t.transport_button_active_bg;
    let ink = t.transport_button_fg;
    let ink_lit = t.transport_button_active_fg;
    let c = &g.controls;
    let buttons = [
        (Button::Prev, c.prev, false),
        (Button::Play, c.play, player.state == PlayState::Playing),
        (Button::Pause, c.pause, player.state == PlayState::Paused),
        (Button::Stop, c.stop, player.state == PlayState::Stopped),
        (Button::Next, c.next, false),
    ];
    let body = body_rows(height);
    let mut images = Vec::with_capacity(5);
    let mut total_bytes = 0usize;
    for (button, rect, lit) in buttons {
        if rect.width == 0 || rect.height == 0 || rect.x == 0 {
            continue;
        }
        let x = rect.x - 1;
        if x.checked_add(rect.width).is_none_or(|right| right > width) {
            continue;
        }
        let visible = body
            .iter()
            .enumerate()
            .filter_map(|(host_y, source_row)| {
                let source_y = source_row + 1;
                (source_y >= rect.y && source_y < rect.bottom()).then_some(host_y as u16)
            })
            .collect::<Vec<_>>();
        let Some(&y) = visible.first() else { continue };
        let shown_height = visible.len() as u16;
        if !visible.windows(2).all(|pair| pair[1] == pair[0] + 1)
            || y.checked_add(shown_height)
                .is_none_or(|bottom| bottom > height)
        {
            continue;
        }
        let Some(pixel_width) = rect.width.checked_mul(graphics.cell_width) else {
            continue;
        };
        let Some(pixel_height) = shown_height.checked_mul(graphics.cell_height) else {
            continue;
        };
        let pixels = usize::from(pixel_width) * usize::from(pixel_height);
        let Some(bytes) = pixels.checked_mul(4) else {
            continue;
        };
        if pixels == 0 || pixels > MAX_IMAGE_PIXELS || bytes > MAX_TOTAL_RGBA - total_bytes {
            continue;
        }
        let (fg, plate_color): (Rgb, Rgb) = if lit {
            (ink_lit, plate_lit)
        } else {
            (ink, plate)
        };
        let rgba = faces::raster(
            button,
            u32::from(pixel_width),
            u32::from(pixel_height),
            fg,
            plate_color,
            bg,
        )
        .into_raw();
        total_bytes += rgba.len();
        images.push(TransportImage {
            x,
            y,
            width: rect.width,
            height: shown_height,
            pixel_width,
            pixel_height,
            rgba,
        });
    }
    images
}

/// The playback fields needed to paint one frame. Captured by the embed
/// service, away from the audio callback.
pub struct PlayerRenderState {
    pub title: String,
    pub subtitle: String,
    pub tech: String,
    pub state: PlayState,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub bit_perfect: bool,
    pub focused: bool,
    pub bands: Vec<f32>,
    pub peaks: Vec<f32>,
    pub underruns: u64,
    pub wave: Vec<f32>,
    pub vis_mode: VisMode,
    pub seek_style: SeekStyle,
    pub bars: BarLayout,
    pub seek_phase: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitTarget {
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    /// Absolute position in seconds.
    Seek(f64),
    /// Normalized volume, 0 through 1.
    Volume(f32),
    Visualizer,
    SeekRow,
}

fn theme(palette: &Palette) -> Theme {
    use starkit::theme::color::Rgb;
    let rgb = |c: [u8; 3]| Rgb::new(c[0], c[1], c[2]);
    let mut file = ThemeFile::default();
    file.meta.name = "Embedded STAR/AMP".into();
    file.app.bg = Some(rgb(palette.bg));
    file.app.fg = Some(rgb(palette.fg));
    file.app.dim = Some(rgb(palette.muted));
    file.app.accent = Some(rgb(palette.accent));
    file.app.error = Some(rgb(palette.error));
    file.chrome.border = Some(rgb(palette.border));
    file.chrome.border_focused = Some(rgb(palette.accent));
    file.panel.bg = Some(rgb(palette.bg));
    file.panel.fg = Some(rgb(palette.fg));
    file.row.fg = Some(rgb(palette.fg));
    file.row.bg = Some(rgb(palette.bg));
    file.row.meta_fg = Some(rgb(palette.muted));
    file.row.selected_fg = Some(rgb(palette.fg));
    file.row.selected_bg = Some(rgb(palette.selected));
    Theme::resolve(&file)
}

/// Native body row indices to present at a requested height. The full body
/// is four analyzer rows followed by title, tech, seek and three controls.
fn body_rows(height: u16) -> Vec<u16> {
    let h = height.min(player::BODY_ROWS);
    match h {
        0 => vec![],
        1 => vec![4],
        2 => vec![4, 8],
        3 => vec![4, 6, 8],
        4 => vec![3, 4, 6, 8],
        5 => vec![3, 4, 5, 6, 8],
        6 => vec![3, 4, 6, 7, 8, 9],
        7..=9 => ((3 - (h - 7))..10).collect(),
        _ => (0..10).collect(),
    }
}

fn source_buffer(width: u16, palette: &Palette, player: &PlayerRenderState) -> Buffer {
    let t = theme(palette);
    let outer = Rect::new(0, 0, width.saturating_add(2), player::PANEL_ROWS);
    let mut buffer = Buffer::empty(outer);
    PlayerView {
        theme: &t,
        title: player.title.clone(),
        subtitle: player.subtitle.clone(),
        tech: player.tech.clone(),
        state: player.state,
        position: player.position,
        duration: player.duration,
        volume: player.volume,
        repeat: player.repeat,
        bit_perfect: player.bit_perfect,
        focused: player.focused,
        mirroring: false,
        marquee_offset: 0,
        bands: &player.bands,
        peaks: &player.peaks,
        wave: &player.wave,
        vis_mode: player.vis_mode,
        bars: player.bars,
        glyphs: Glyphs::ASCII,
        seek_phase: player.seek_phase,
        seek_style: player.seek_style,
        underruns: player.underruns,
    }
    .render(outer, &mut buffer);
    buffer
}

fn color(color: Color, fallback: [u8; 3]) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        _ => fallback,
    }
}

/// A row-major cell grid, exactly `width * height` cells, with no STAR/AMP
/// border or header. Any height beyond the native body is blank host colour.
pub fn render_frame(
    width: u16,
    height: u16,
    palette: &Palette,
    player: &PlayerRenderState,
) -> Vec<Cell> {
    if width == 0 || height == 0 {
        return vec![];
    }
    let buffer = source_buffer(width, palette, player);
    let rows = body_rows(height);
    let mut cells = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let source = rows.get(y as usize).map(|row| &buffer[(x + 1, row + 1)]);
            let cell = source.map_or_else(
                || Cell {
                    symbol: " ".into(),
                    fg: palette.fg,
                    bg: palette.bg,
                    modifiers: 0,
                },
                |cell| Cell {
                    symbol: cell.symbol().to_string(),
                    fg: color(cell.fg, palette.fg),
                    bg: color(cell.bg, palette.bg),
                    modifiers: cell.modifier.bits(),
                },
            );
            cells.push(cell);
        }
    }
    if height < player::BODY_ROWS {
        if let Some(clock_row) = rows.iter().position(|row| *row == 3) {
            // The native clock is three rows tall. In compact mode its last
            // row becomes a one-line readout before the title.
            let row = &mut cells[clock_row * width as usize..(clock_row + 1) * width as usize];
            for cell in row.iter_mut() {
                cell.symbol = " ".into();
                cell.fg = palette.fg;
                cell.bg = palette.bg;
                cell.modifiers = 0;
            }
            let clock = if player.state == PlayState::Stopped {
                "--:--".to_string()
            } else {
                crate::ui::digits::clock_padded(player.position)
            };
            for (cell, ch) in row.iter_mut().skip(1).zip(clock.chars()) {
                cell.symbol = ch.to_string();
                cell.fg = palette.accent;
            }
        }
    }
    cells
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && x >= rect.x
        && x < rect.right()
        && y >= rect.y
        && y < rect.bottom()
}

/// Hit-test the same geometry `PlayerView` used to draw the source cells.
pub fn hit_test(
    width: u16,
    height: u16,
    x: u16,
    y: u16,
    player: &PlayerRenderState,
) -> Option<HitTarget> {
    if x >= width || y >= height {
        return None;
    }
    let source_y = *body_rows(height).get(y as usize)? + 1;
    let source_x = x + 1;
    let outer = Rect::new(0, 0, width.saturating_add(2), player::PANEL_ROWS);
    let g = player::geometry(
        outer,
        player.position,
        player.duration,
        player.repeat,
        Glyphs::ASCII,
    )?;
    let c = &g.controls;
    if !(height < player::BODY_ROWS && source_y == 4) && contains(g.visualizer, source_x, source_y)
    {
        return Some(HitTarget::Visualizer);
    }
    for (rect, target) in [
        (c.prev, HitTarget::Previous),
        (c.play, HitTarget::Play),
        (c.pause, HitTarget::Pause),
        (c.stop, HitTarget::Stop),
        (c.next, HitTarget::Next),
    ] {
        if contains(rect, source_x, source_y) {
            return Some(target);
        }
    }
    if let Some(seek) = g.seek {
        if contains(seek, source_x, source_y)
            && player.duration.is_finite()
            && player.duration > 0.0
        {
            let frac = (source_x - seek.x) as f64 / seek.width.max(1) as f64;
            return Some(HitTarget::Seek(
                (frac * player.duration).clamp(0.0, player.duration),
            ));
        }
    }
    if contains(g.seek_row, source_x, source_y) {
        return Some(HitTarget::SeekRow);
    }
    if let Some(volume) = c.volume {
        if contains(volume, source_x, source_y) {
            let frac = (source_x - volume.x + 1) as f32 / volume.width.max(1) as f32;
            return Some(HitTarget::Volume(frac.clamp(0.0, 1.0)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette {
            bg: [10, 20, 30],
            fg: [230, 220, 210],
            muted: [120, 130, 140],
            accent: [200, 100, 50],
            selected: [30, 40, 50],
            border: [70, 80, 90],
            error: [240, 50, 60],
        }
    }

    fn playing() -> PlayerRenderState {
        PlayerRenderState {
            title: "The Track".into(),
            subtitle: "The Album".into(),
            tech: "FLAC".into(),
            state: PlayState::Playing,
            position: 30.0,
            duration: 120.0,
            volume: 0.5,
            repeat: RepeatMode::Off,
            bit_perfect: false,
            focused: true,
            bands: vec![0.5; 20],
            peaks: vec![0.6; 20],
            underruns: 0,
            wave: vec![0.0; 1024],
            vis_mode: VisMode::Bars,
            seek_style: SeekStyle::default(),
            bars: BarLayout::default(),
            seek_phase: 0.0,
        }
    }

    #[test]
    fn full_body_uses_host_palette_and_contains_player_controls() {
        let cells = render_frame(64, 10, &palette(), &playing());
        assert_eq!(cells.len(), 640);
        assert!(cells.iter().any(|cell| cell.bg == palette().bg));
        assert!(cells.iter().any(|cell| cell.fg == palette().accent));
        assert!(cells.iter().any(|cell| cell.symbol == "T"));
        assert!(cells.iter().any(|cell| cell.symbol == "|"));
    }

    #[test]
    fn compact_and_tiny_heights_keep_exact_grid_size() {
        for width in [0, 1, 2, 3, 4, 8, 40] {
            for height in 0..=12 {
                let cells = render_frame(width, height, &palette(), &playing());
                assert_eq!(cells.len(), width as usize * height as usize);
                if width == 40 && height > 0 {
                    assert!(cells.iter().any(|cell| cell.symbol == "T"));
                }
            }
        }
    }

    #[test]
    fn full_and_compact_transport_hits_use_drawn_button_rows() {
        let player = playing();
        for height in [2, 3, 5, 10] {
            let y = if height == 10 { 8 } else { height - 1 };
            assert_eq!(
                hit_test(64, height, 2, y, &player),
                Some(HitTarget::Previous)
            );
        }
        assert_eq!(hit_test(64, 10, 64, 8, &player), None);
    }

    #[test]
    fn five_row_compact_body_has_clock_title_seek_and_transport_with_volume() {
        let player = playing();
        let cells = render_frame(64, 5, &palette(), &player);
        let row = |y: usize| -> String {
            cells[y * 64..(y + 1) * 64]
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect()
        };
        assert!(row(0).contains("00:30"));
        assert!(row(1).contains("The Track"));
        assert!(row(3).contains("00:30"));
        assert!(row(4).contains("VOL"));
        assert_eq!(hit_test(64, 5, 2, 4, &player), Some(HitTarget::Previous));
    }

    #[test]
    fn image_controls_match_main_panel_geometry_and_hit_targets() {
        let player = playing();
        let graphics = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let mut cache = TransportImageCache::default();
        let images = cache.images(64, 10, &palette(), &player, graphics);
        assert_eq!(images.len(), 5);
        for (image, target) in images.iter().zip([
            HitTarget::Previous,
            HitTarget::Play,
            HitTarget::Pause,
            HitTarget::Stop,
            HitTarget::Next,
        ]) {
            assert_eq!(image.pixel_width, image.width * graphics.cell_width);
            assert_eq!(image.pixel_height, image.height * graphics.cell_height);
            assert_eq!(
                image.rgba.len(),
                usize::from(image.pixel_width) * usize::from(image.pixel_height) * 4
            );
            assert_eq!(
                hit_test(
                    64,
                    10,
                    image.x + image.width / 2,
                    image.y + image.height / 2,
                    &player
                ),
                Some(target)
            );
            assert!(image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255));
        }
    }

    #[test]
    fn compact_images_use_the_single_visible_transport_row() {
        let player = playing();
        let mut cache = TransportImageCache::default();
        let images = cache.images(
            64,
            5,
            &palette(),
            &player,
            GraphicsConfig {
                cell_width: 8,
                cell_height: 16,
            },
        );
        assert_eq!(images.len(), 5);
        assert!(images.iter().all(|image| image.y == 4 && image.height == 1));
        assert_eq!(
            hit_test(64, 5, images[0].x + 1, images[0].y, &player),
            Some(HitTarget::Previous)
        );
    }

    #[test]
    fn image_budget_and_active_state_are_respected() {
        let mut player = playing();
        let mut cache = TransportImageCache::default();
        let graphics = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let playing_images = cache.images(64, 10, &palette(), &player, graphics).to_vec();
        player.state = PlayState::Paused;
        let paused_images = cache.images(64, 10, &palette(), &player, graphics);
        assert_ne!(playing_images[1].rgba, paused_images[1].rgba);
        assert_ne!(playing_images[2].rgba, paused_images[2].rgba);

        let oversized = cache.images(
            64,
            5,
            &palette(),
            &player,
            GraphicsConfig {
                cell_width: 64,
                cell_height: 128,
            },
        );
        assert!(oversized.len() <= 5);
        assert!(oversized.iter().all(|image| {
            usize::from(image.pixel_width) * usize::from(image.pixel_height) <= MAX_IMAGE_PIXELS
        }));
        assert!(
            oversized
                .iter()
                .map(|image| image.rgba.len())
                .sum::<usize>()
                <= MAX_TOTAL_RGBA
        );
    }

    #[test]
    fn style_modes_change_native_cells_and_compact_hits_only_visible_analyzer() {
        let mut player = playing();
        let bars = render_frame(64, 10, &palette(), &player);
        player.vis_mode = VisMode::Off;
        let off = render_frame(64, 10, &palette(), &player);
        assert_ne!(
            bars[..64 * 4].iter().map(|c| &c.symbol).collect::<Vec<_>>(),
            off[..64 * 4].iter().map(|c| &c.symbol).collect::<Vec<_>>()
        );
        player.seek_style = SeekStyle::BLOCKS;
        let blocks = render_frame(64, 10, &palette(), &player);
        assert_ne!(
            off[64 * 6..64 * 7]
                .iter()
                .map(|c| &c.symbol)
                .collect::<Vec<_>>(),
            blocks[64 * 6..64 * 7]
                .iter()
                .map(|c| &c.symbol)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            hit_test(64, 10, 30, 0, &player),
            Some(HitTarget::Visualizer)
        );
        assert_eq!(hit_test(64, 5, 30, 0, &player), None);
        assert_eq!(hit_test(64, 8, 30, 0, &player), Some(HitTarget::Visualizer));
        assert_eq!(hit_test(64, 10, 0, 6, &player), Some(HitTarget::SeekRow));
    }
}
