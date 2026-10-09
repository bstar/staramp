//! AMP-owned player artwork. Standalone and embedding use the same real state,
//! controls and geometry; KIT owns sprites, density and transport.
use super::{native, render::PlayerRenderState, GraphicsConfig, Palette};
use anyhow::{Context, Result};
use starkit::native_surface::{
    classic::Colors,
    skin::assets::{AssetCache, Layer, Sprite},
    PixelRect, Primitive, Surface,
};
use std::collections::BTreeMap;

/// Theme adaptation is calibrated against the selected study, rather than
/// replacing every independently designed surface with one generic blend.
pub(crate) const REFERENCE_PALETTE: Palette = Palette {
    bg: [23, 24, 32],
    fg: [215, 217, 207],
    muted: [152, 158, 172],
    accent: [164, 215, 145],
    selected: [48, 63, 61],
    border: [116, 121, 134],
    error: [224, 152, 133],
};
pub(crate) fn tone(p: &Palette, original: &str, ink: bool) -> String {
    calibrated(
        original,
        &native::hex(if ink {
            REFERENCE_PALETTE.fg
        } else {
            REFERENCE_PALETTE.bg
        }),
        &native::hex(if ink { p.fg } else { p.bg }),
    )
}
fn generic(p: &Palette) -> Colors {
    Colors::new(p.bg, p.fg, p.muted, p.accent, p.border)
}
fn calibrated(original: &str, reference: &str, actual: &str) -> String {
    use starkit::theme::color::Rgb;
    let a = Rgb::parse_hex(original).expect("skin source color");
    let b = Rgb::parse_hex(reference).expect("reference theme color");
    let c = Rgb::parse_hex(actual).expect("resolved theme color");
    let channel = |a, b, c| (i16::from(a) + i16::from(c) - i16::from(b)).clamp(0, 255) as u8;
    Rgb::new(
        channel(a.r, b.r, c.r),
        channel(a.g, b.g, c.g),
        channel(a.b, b.b, c.b),
    )
    .to_hex()
}
pub(crate) fn colors(p: &Palette) -> Colors {
    let a = generic(p);
    let r = generic(&REFERENCE_PALETTE);
    Colors {
        panel: calibrated("#303340", &r.panel, &a.panel),
        inset: calibrated("#101813", &r.inset, &a.inset),
        ink: native::hex(p.fg),
        dim: native::hex(p.muted),
        accent: native::hex(p.accent),
        highlight: calibrated("#7c8294", &r.highlight, &a.highlight),
        shadow: calibrated("#101117", &r.shadow, &a.shadow),
        raised: calibrated("#474d5d", &r.raised, &a.raised),
        title: calibrated("#242733", &r.title, &a.title),
    }
}
fn role_base(role: &str, p: &Palette) -> String {
    let c = generic(p);
    match role {
        "panel" | "quiet_panel" | "disabled" | "quiet_disabled" => c.panel,
        "shadow" | "edge_shadow" | "well_shadow" | "control_border" => c.shadow,
        "highlight" | "control_highlight" | "well_highlight" | "active" | "quiet_active" => {
            c.highlight
        }
        "title" | "pressed" => c.title,
        "ink" => c.ink,
        "accent" => c.accent,
        "well" | "background" => c.inset,
        "control" => c.raised,
        "hover" | "quiet_hover" => {
            use starkit::theme::color::Rgb;
            Rgb::new(p.selected[0], p.selected[1], p.selected[2])
                .mix(Rgb::new(p.accent[0], p.accent[1], p.accent[2]), 0.16)
                .to_hex()
        }
        "well_border" | "border" => native::hex(p.border),
        _ => unreachable!("unknown skin role {role}"),
    }
}

#[derive(serde::Deserialize)]
struct Manifest {
    sprites: BTreeMap<String, Sprite>,
    layers: BTreeMap<String, Vec<Layer>>,
    roles: BTreeMap<String, String>,
}
struct Prepared {
    id: String,
    png: String,
    source: PixelRect,
    insets: [u16; 4],
}
struct GlyphAsset {
    id: String,
    png: String,
}
pub(crate) struct PlayerSkin {
    source: AssetCache,
    manifest: Manifest,
    prepared: BTreeMap<String, Prepared>,
    palette: Option<Palette>,
    density: u16,
    rigid: bool,
    pub hovered: Option<String>,
    pub pressed: Option<String>,
    pub focused: Option<String>,
    disabled: std::collections::BTreeSet<&'static str>,
    glyphs: BTreeMap<(u16, &'static str), GlyphAsset>,
}
impl PlayerSkin {
    pub fn new() -> Result<Self> {
        let mut source = AssetCache::new(8_000_000);
        for (id, png) in super::skin_assets::PNGS {
            source.insert_png(id, png)?;
        }
        let mut glyphs = BTreeMap::new();
        for (density, name, png) in super::skin_assets::GLYPHS {
            let pixels = starkit::image::load_from_memory(png)?.to_rgba8();
            let (id, png) = starkit::terminal_graphics::assets::encode_skin(&pixels)?;
            glyphs.insert((*density, *name), GlyphAsset { id, png });
        }
        Ok(Self {
            source,
            manifest: serde_json::from_str(include_str!(
                "../../assets/skins/classic/manifest.json"
            ))?,
            prepared: BTreeMap::new(),
            palette: None,
            density: 1,
            rigid: false,
            hovered: None,
            pressed: None,
            focused: None,
            disabled: Default::default(),
            glyphs,
        })
    }
    pub(crate) fn prepare(&mut self, palette: &Palette, density: u16, rigid: bool) -> Result<()> {
        if self.palette.as_ref() == Some(palette) && self.density == density && self.rigid == rigid
        {
            return Ok(());
        }
        let mut roles = BTreeMap::new();
        for (role, original) in &self.manifest.roles {
            let value = calibrated(
                original,
                &role_base(role, &REFERENCE_PALETTE),
                &role_base(role, palette),
            );
            roles.insert(
                role.clone(),
                [
                    u8::from_str_radix(&value[1..3], 16)?,
                    u8::from_str_radix(&value[3..5], 16)?,
                    u8::from_str_radix(&value[5..7], 16)?,
                ],
            );
        }
        let mut prepared = BTreeMap::new();
        for name in [
            "panel",
            "panel-rigid",
            "well",
            "button-normal",
            "button-active",
            "button-hover",
            "button-pressed",
            "button-focus",
            "button-disabled",
        ] {
            let key = format!("{density}/{name}");
            let sprite = self
                .manifest
                .sprites
                .get(&key)
                .context("Missing skin sprite")?;
            let layers = self
                .manifest
                .layers
                .get(&key)
                .context("Missing skin layers")?;
            let pixels = self
                .source
                .compose(24 * density, 24 * density, layers, &roles)?;
            // encode_png does not thumbnail or resize these original assets.
            let (id, png) = starkit::terminal_graphics::assets::encode_skin(&pixels)?;
            let i = sprite.nine_slice.context("Missing slice metadata")?.insets;
            prepared.insert(
                name.into(),
                Prepared {
                    id,
                    png,
                    source: PixelRect::new(0, 0, 24 * density, 24 * density),
                    insets: [i.left, i.top, i.right, i.bottom],
                },
            );
        }
        self.prepared = prepared;
        self.palette = Some(*palette);
        self.density = density;
        self.rigid = rigid;
        Ok(())
    }
    fn button_kind(&self, action: &str, active: bool) -> &'static str {
        if self.disabled.contains(action) {
            "button-disabled"
        } else if self.pressed.as_deref() == Some(action) {
            "button-pressed"
        } else if self.focused.as_deref() == Some(action) {
            "button-focus"
        } else if self.hovered.as_deref() == Some(action) {
            "button-hover"
        } else if active {
            "button-active"
        } else {
            "button-normal"
        }
    }
    fn paint(&self, surface: &mut Surface, rect: PixelRect, name: &str) {
        let art = &self.prepared[name];
        surface.assets.insert(art.id.clone(), Some(art.png.clone()));
        surface.nodes.push(Primitive::Sprite {
            rect,
            asset: art.id.clone(),
            source: art.source,
            insets: Some(art.insets),
            tint: None,
        });
    }
    pub(crate) fn frame(&self, s: &mut Surface, rect: PixelRect, inset: bool) {
        <Self as native::PlayerArtwork>::frame(self, s, rect, inset);
    }
    pub(crate) fn button(&self, s: &mut Surface, rect: PixelRect, action: &str, active: bool) {
        <Self as native::PlayerArtwork>::button(self, s, rect, action, active);
    }
    pub(crate) fn ink(&self, action: &str, active: bool) -> String {
        <Self as native::PlayerArtwork>::button_ink(self, action, active)
    }
    pub(crate) fn label(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        text: &str,
        tint: &str,
        centered: bool,
    ) -> bool {
        let Some((_, y, width, height)) = super::skin_assets::LABELS
            .iter()
            .find(|(label, _, _, _)| *label == text)
        else {
            return false;
        };
        if *width > rect.width {
            return false;
        }
        let x = rect.x
            + if centered {
                (rect.width - width) / 2
            } else {
                0
            };
        self.glyph(
            s,
            PixelRect::new(x, rect.y, *width, *height),
            "fixed-labels",
            PixelRect::new(0, *y, *width, *height),
            tint,
        );
        true
    }
    fn glyph(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        name: &'static str,
        source: PixelRect,
        tint: &str,
    ) {
        let asset = &self.glyphs[&(self.density, name)];
        s.assets.insert(asset.id.clone(), Some(asset.png.clone()));
        s.nodes.push(Primitive::Sprite {
            rect,
            asset: asset.id.clone(),
            source: PixelRect::new(
                source.x * self.density,
                source.y * self.density,
                source.width * self.density,
                source.height * self.density,
            ),
            insets: None,
            tint: Some(tint.into()),
        });
    }
    #[allow(clippy::too_many_arguments)]
    pub fn surface(
        &mut self,
        width: u16,
        height: u16,
        graphics: GraphicsConfig,
        palette: &Palette,
        state: &PlayerRenderState,
        radius: u16,
    ) -> Result<Surface> {
        self.surface_at_density(
            width,
            height,
            graphics,
            palette,
            state,
            radius,
            density(width, height, graphics),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn logical_surface(
        &mut self,
        width: u16,
        height: u16,
        graphics: GraphicsConfig,
        palette: &Palette,
        state: &PlayerRenderState,
        radius: u16,
        density: u16,
    ) -> Result<Surface> {
        self.disabled = ["previous", "play", "pause", "stop", "next", "seek"]
            .into_iter()
            .filter(|action| !state.control_enabled(action))
            .collect();
        self.prepare(palette, density, radius == 0)?;
        Ok(native::player_surface(
            width,
            height,
            graphics,
            palette,
            state,
            radius,
            Some(self),
        ))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn surface_at_density(
        &mut self,
        width: u16,
        height: u16,
        graphics: GraphicsConfig,
        palette: &Palette,
        state: &PlayerRenderState,
        radius: u16,
        density: u16,
    ) -> Result<Surface> {
        anyhow::ensure!((1..=2).contains(&density), "Unsupported player density");
        self.disabled = ["previous", "play", "pause", "stop", "next", "seek"]
            .into_iter()
            .filter(|action| !state.control_enabled(action))
            .collect();
        if width / density < 720 || height / density < 230 {
            // Defined compatibility presentation for hosts too short for the
            // full player. Its controls remain the ordinary AMP controls.
            let mut surface =
                native::classic_surface(width, height, graphics, palette, state, radius);
            if let Some(rect) = self.focused.as_ref().and_then(|action| {
                surface
                    .hits
                    .iter()
                    .find(|h| &h.action == action)
                    .map(|h| h.rect)
            }) {
                surface.nodes.push(Primitive::Border {
                    rect,
                    color: format!(
                        "#{:02x}{:02x}{:02x}",
                        palette.accent[0], palette.accent[1], palette.accent[2]
                    ),
                    radius: radius.min(3),
                });
            }
            return Ok(surface);
        }
        self.prepare(palette, density, radius == 0)?;
        let surface = native::player_surface(
            width / density,
            height / density,
            GraphicsConfig {
                cell_width: (graphics.cell_width / density).max(1),
                cell_height: (graphics.cell_height / density).max(1),
            },
            palette,
            state,
            radius / density,
            Some(self),
        );
        surface.at_density(density)
    }
}
/// UI density selects original artwork and fresh text sizes. Window width only
/// expands the flexible display; it never rescales a completed player bitmap.
pub(crate) fn density(width: u16, height: u16, graphics: GraphicsConfig) -> u16 {
    if graphics.cell_height >= 32 && width >= 1440 && height >= 460 {
        2
    } else {
        1
    }
}
impl native::PlayerArtwork for PlayerSkin {
    fn fixed_label(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        text: &str,
        color: &str,
        centered: bool,
    ) -> bool {
        self.label(s, rect, text, color, centered)
    }
    fn clock(&self, s: &mut Surface, origin: [u16; 2], value: &str, color: &str) {
        let p = self.palette.expect("prepared skin palette");
        let inactive = starkit::theme::color::Rgb::new(p.bg[0], p.bg[1], p.bg[2])
            .mix(
                starkit::theme::color::Rgb::new(p.accent[0], p.accent[1], p.accent[2]),
                0.14,
            )
            .to_hex();
        let r = REFERENCE_PALETTE;
        let reference = starkit::theme::color::Rgb::new(r.bg[0], r.bg[1], r.bg[2])
            .mix(
                starkit::theme::color::Rgb::new(r.accent[0], r.accent[1], r.accent[2]),
                0.14,
            )
            .to_hex();
        let inactive = calibrated("#203226", &reference, &inactive);
        let mut offset = 0u16;
        for character in value.chars() {
            let (index, width, advance) = match character {
                '0'..='9' => (character as u16 - '0' as u16, 35, 341),
                ':' => (10, 15, 143),
                _ => continue,
            };
            let x = origin[0] + offset / 10;
            if u32::from(x) + u32::from(width) > u32::from(s.width) {
                break;
            }
            let rect = PixelRect::new(x, origin[1], width, 54);
            let source = PixelRect::new(index * 35, (offset % 10) * 54, width, 54);
            self.glyph(s, rect, "clock-off", source, &inactive);
            self.glyph(s, rect, "clock-on", source, color);
            offset += advance;
        }
    }
    fn transport_icon(&self, s: &mut Surface, rect: PixelRect, action: &str, active: bool) {
        let index = match action {
            "previous" => 0,
            "play" => 1,
            "pause" => 2,
            "stop" => 3,
            "next" => 4,
            _ => return,
        };
        self.glyph(
            s,
            rect,
            "transport-glyphs",
            PixelRect::new(index * 29, 0, 29, 29),
            &self.button_ink(action, active),
        );
    }
    fn frame(&self, s: &mut Surface, rect: PixelRect, inset: bool) {
        self.paint(
            s,
            rect,
            if inset {
                "well"
            } else if self.rigid {
                "panel-rigid"
            } else {
                "panel"
            },
        );
    }
    fn button_ink(&self, action: &str, active: bool) -> String {
        use starkit::theme::color::Rgb;
        let p = self.palette.expect("prepared skin palette");
        let kind = self.button_kind(action, active);
        if kind == "button-disabled" {
            // Disabled controls are intentionally subdued, not promoted to
            // bright active ink by the enabled-control contrast correction.
            return native::hex(p.muted);
        }
        let role = match kind {
            "button-pressed" => "pressed",
            "button-hover" => "hover",
            "button-active" => "active",
            _ => "control",
        };
        let color = calibrated(
            &self.manifest.roles[role],
            &role_base(role, &REFERENCE_PALETTE),
            &role_base(role, &p),
        );
        let background = Rgb::parse_hex(&color).expect("derived skin color");
        let ink = Rgb::parse_hex(&calibrated(
            if active { "#10161a" } else { "#e0e2d5" },
            &native::hex(if active {
                REFERENCE_PALETTE.bg
            } else {
                REFERENCE_PALETTE.fg
            }),
            &native::hex(if active { p.bg } else { p.fg }),
        ))
        .unwrap();
        if background.contrast(ink) >= 4.5 {
            ink.to_hex()
        } else {
            background
                .best_contrast_against(&[starkit::theme::BLACK, starkit::theme::WHITE])
                .to_hex()
        }
    }
    fn button(&self, s: &mut Surface, rect: PixelRect, action: &str, active: bool) {
        let name = self.button_kind(action, active);
        self.paint(s, rect, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::player::PlayState,
        playlist::queue::RepeatMode,
        ui::panels::{player::SeekStyle, visualizer::BarLayout},
        vis::mode::VisMode,
    };
    fn state() -> PlayerRenderState {
        PlayerRenderState {
            has_items: true,
            title: "Björk · Jóga — 東京".into(),
            subtitle: "Homogenic".into(),
            tech: "FLAC · 44.1 kHz · 16-bit".into(),
            state: PlayState::Playing,
            position: 30.,
            duration: 120.,
            volume: 0.5,
            repeat: RepeatMode::Off,
            shuffled: false,
            bit_perfect: true,
            focused: true,
            bands: vec![0.5; 56],
            peaks: vec![0.6; 56],
            underruns: 0,
            wave: vec![0.; 128],
            vis_mode: VisMode::Bars,
            seek_style: SeekStyle::default(),
            bars: BarLayout::default(),
            seek_phase: 0.,
        }
    }
    fn palette() -> Palette {
        Palette {
            bg: [23, 24, 32],
            fg: [215, 217, 207],
            muted: [152, 158, 172],
            accent: [164, 215, 145],
            selected: [48, 51, 64],
            border: [116, 121, 134],
            error: [240, 60, 60],
        }
    }
    #[test]
    fn reference_palette_keeps_the_original_artwork_colors() {
        let mut skin = PlayerSkin::new().unwrap();
        let surface = skin
            .surface_at_density(
                1352,
                230,
                GraphicsConfig {
                    cell_width: 8,
                    cell_height: 16,
                },
                &REFERENCE_PALETTE,
                &state(),
                8,
                1,
            )
            .unwrap();
        let image = starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new("monospace")
            .render(&starkit::image::RgbaImage::new(1352, 230), &surface)
            .unwrap();
        for (x, y, color) in [
            (700, 180, [48, 51, 64, 255]),
            (700, 104, [16, 24, 19, 255]),
            (20, 210, [71, 77, 93, 255]),
            (55, 210, [107, 114, 130, 255]),
        ] {
            assert_eq!(image.get_pixel(x, y).0, color, "reference color at {x},{y}");
        }
    }
    #[test]
    fn bitmap_clock_and_transport_preserve_reference_geometry_at_both_densities() {
        let mut skin = PlayerSkin::new().unwrap();
        let mut state = state();
        state.position = 69.;
        for density in [1, 2] {
            let surface = skin
                .surface_at_density(
                    1352 * density,
                    230 * density,
                    GraphicsConfig {
                        cell_width: 8 * density,
                        cell_height: 16 * density,
                    },
                    &palette(),
                    &state,
                    8,
                    density,
                )
                .unwrap();
            surface.validate().unwrap();
            let clock_id = &skin.glyphs[&(density, "clock-on")].id;
            let clock = surface
                .nodes
                .iter()
                .filter_map(|node| match node {
                    Primitive::Sprite {
                        rect,
                        source,
                        asset,
                        insets: None,
                        ..
                    } if asset == clock_id => Some((*rect, *source)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(clock.len(), 4);
            for ((rect, source), (x, sx, sy, width)) in clock.iter().zip([
                (28, 35, 0, 35),
                (62, 350, 54, 15),
                (76, 0, 216, 35),
                (110, 315, 270, 35),
            ]) {
                assert_eq!(
                    *rect,
                    PixelRect::new(x * density, 58 * density, width * density, 54 * density)
                );
                assert_eq!(
                    *source,
                    PixelRect::new(sx * density, sy * density, width * density, 54 * density)
                );
            }
            let icon_id = &skin.glyphs[&(density, "transport-glyphs")].id;
            let icons = surface
                .nodes
                .iter()
                .filter_map(|node| match node {
                    Primitive::Sprite {
                        rect,
                        source,
                        asset,
                        insets: None,
                        ..
                    } if asset == icon_id => Some((*rect, *source)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(icons.len(), 5);
            for (i, (rect, source)) in icons.iter().enumerate() {
                assert_eq!(
                    *rect,
                    PixelRect::new(
                        (16 + i as u16 * 35) * density,
                        187 * density,
                        29 * density,
                        29 * density
                    )
                );
                assert_eq!(
                    *source,
                    PixelRect::new(i as u16 * 29 * density, 0, 29 * density, 29 * density)
                );
            }
        }
    }
    #[test]
    fn compact_fallback_keeps_keyboard_focus_visible_without_changing_hits() {
        let mut skin = PlayerSkin::new().unwrap();
        let graphics = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let before = skin
            .surface_at_density(640, 180, graphics, &palette(), &state(), 8, 1)
            .unwrap();
        skin.focused = Some("play".into());
        let focused = skin
            .surface_at_density(640, 180, graphics, &palette(), &state(), 8, 1)
            .unwrap();
        focused.validate().unwrap();
        assert_eq!(before.hits, focused.hits);
        let rect = focused
            .hits
            .iter()
            .find(|h| h.action == "play")
            .unwrap()
            .rect;
        assert!(
            matches!(focused.nodes.last(), Some(Primitive::Border { rect: drawn, .. }) if *drawn == rect)
        );
        assert!(focused.assets.is_empty());
    }
    #[test]
    fn active_hover_pressed_and_focus_ink_remains_readable_across_palettes() {
        use native::PlayerArtwork;
        use starkit::theme::color::Rgb;
        let mut skin = PlayerSkin::new().unwrap();
        for mut palette in [
            palette(),
            Palette {
                bg: [245; 3],
                fg: [30; 3],
                ..palette()
            },
        ] {
            for accent in [[164, 215, 145], [20, 60, 220], [250, 170, 210]] {
                palette.accent = accent;
                skin.prepare(&palette, 1, false).unwrap();
                for kind in [
                    "button-normal",
                    "button-active",
                    "button-hover",
                    "button-focus",
                    "button-pressed",
                ] {
                    skin.hovered = (kind == "button-hover").then(|| "play".into());
                    skin.focused = (kind == "button-focus").then(|| "play".into());
                    skin.pressed = (kind == "button-pressed").then(|| "play".into());
                    let color = skin.button_ink("play", kind == "button-active");
                    let foreground = Rgb::parse_hex(&color).unwrap();
                    let mut surface = Surface::new(24, 24, "#000000".into());
                    skin.button(
                        &mut surface,
                        PixelRect::new(0, 0, 24, 24),
                        "play",
                        kind == "button-active",
                    );
                    let image = starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new(
                        "monospace",
                    )
                    .render(&starkit::image::RgbaImage::new(24, 24), &surface)
                    .unwrap();
                    let pixel = image.get_pixel(12, 12);
                    let background = Rgb::new(pixel[0], pixel[1], pixel[2]);
                    assert!(foreground.contrast(background) >= 4.5, "{kind}");
                }
            }
        }
    }
    #[test]
    fn live_state_changes_keep_artwork_and_control_geometry_stable() {
        let mut skin = PlayerSkin::new().unwrap();
        let g = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let mut state = state();
        let first = skin.surface(1352, 230, g, &palette(), &state, 8).unwrap();
        first.validate().unwrap();
        assert!(!first.assets.is_empty());
        for action in [
            "previous",
            "play",
            "pause",
            "stop",
            "next",
            "shuffle",
            "repeat",
            "volume",
            "seek",
            "visualizer",
        ] {
            assert!(
                first.hits.iter().any(|h| h.action == action),
                "missing {action}"
            );
        }
        assert!(first
            .nodes
            .iter()
            .any(|n| matches!(n,Primitive::Text{text,..} if text.contains("Björk"))));
        state.position = 77.;
        state.bands.fill(0.8);
        let advanced = skin.surface(1352, 230, g, &palette(), &state, 8).unwrap();
        assert_ne!(first.nodes, advanced.nodes);
        assert_eq!(first.assets, advanced.assets);
        assert_eq!(first.hits, advanced.hits);
        state.state = PlayState::Paused;
        let paused = skin.surface(1352, 230, g, &palette(), &state, 8).unwrap();
        let enabled = first
            .hits
            .iter()
            .filter(|h| h.action != "pause")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(enabled, paused.hits);
        assert_eq!(skin.button_kind("pause", true), "button-disabled");
        assert_ne!(first.nodes, paused.nodes);
        // The host and standalone do not get independent implementations.
        let embedded = PlayerSkin::new()
            .unwrap()
            .surface(1352, 230, g, &palette(), &state, 8)
            .unwrap();
        assert_eq!(paused, embedded);
    }
    #[test]
    fn empty_idle_clears_stale_decoder_and_meter_data_but_keeps_active_playback() {
        let mut idle = state();
        idle.has_items = false;
        idle.state = PlayState::Stopped;
        let idle = idle.clear_empty_idle();
        assert_eq!(idle.duration, 0.);
        assert_eq!(idle.position, 0.);
        assert!(idle.tech.is_empty());
        assert!(!idle.bit_perfect);
        assert!(idle
            .bands
            .iter()
            .chain(&idle.peaks)
            .chain(&idle.wave)
            .all(|v| *v == 0.));
        let mut active = state();
        active.has_items = false;
        let active = active.clear_empty_idle();
        assert_eq!(active.duration, 120.);
        assert!(!active.tech.is_empty());
        assert!(active.bit_perfect);
        assert!(active.control_enabled("stop"));
    }
    #[test]
    fn unavailable_controls_have_disabled_art_and_no_pointer_targets() {
        let g = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        for (has_items, playback, duration, disabled) in [
            (
                false,
                PlayState::Stopped,
                0.,
                vec!["previous", "play", "pause", "stop", "next", "seek"],
            ),
            (true, PlayState::Stopped, 0., vec!["pause", "stop", "seek"]),
            (true, PlayState::Paused, 120., vec!["pause"]),
            (true, PlayState::Playing, 0., vec!["seek"]),
            (
                false,
                PlayState::Playing,
                120.,
                vec!["previous", "play", "next", "seek"],
            ),
        ] {
            let mut state = state();
            state.has_items = has_items;
            state.state = playback;
            state.duration = duration;
            let mut skin = PlayerSkin::new().unwrap();
            skin.focused = Some("pause".into());
            skin.hovered = Some("pause".into());
            skin.pressed = Some("pause".into());
            for (width, height) in [(1352, 230), (640, 180)] {
                let surface = skin
                    .surface(width, height, g, &palette(), &state, 8)
                    .unwrap();
                surface.validate().unwrap();
                for action in &disabled {
                    assert!(
                        !surface.hits.iter().any(|h| &h.action == action),
                        "{action}"
                    );
                    assert_eq!(skin.button_kind(action, true), "button-disabled");
                    if width == 1352 {
                        assert_eq!(
                            native::PlayerArtwork::button_ink(&skin, action, true),
                            native::hex(palette().muted)
                        );
                    }
                }
                assert!(surface.hits.iter().any(|h| h.action == "volume"));
                if playback == PlayState::Playing {
                    for action in ["pause", "stop"] {
                        assert!(surface.hits.iter().any(|h| h.action == action));
                    }
                }
                if width == 1352 {
                    for action in ["shuffle", "repeat", "visualizer"] {
                        assert!(surface.hits.iter().any(|h| h.action == action));
                    }
                    if !has_items {
                        let disabled_id = &skin.prepared["button-disabled"].id;
                        assert!(surface.nodes.iter().any(|node| matches!(node, Primitive::Sprite { asset, .. } if asset == disabled_id)));
                    }
                }
            }
        }
    }
    #[test]
    fn density_theme_corners_and_pointer_states_use_original_assets() {
        let mut skin = PlayerSkin::new().unwrap();
        let state = state();
        let g = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let first = skin.surface(1352, 230, g, &palette(), &state, 8).unwrap();
        let double = skin
            .surface(
                2704,
                460,
                GraphicsConfig {
                    cell_width: 16,
                    cell_height: 32,
                },
                &palette(),
                &state,
                8,
            )
            .unwrap();
        assert_eq!(double.hits[0].rect.x, first.hits[0].rect.x * 2);
        assert!(double
            .nodes
            .iter()
            .any(|n| matches!(n,Primitive::Sprite{source,..} if source.width==48)));
        let rigid = skin.surface(1352, 230, g, &palette(), &state, 0).unwrap();
        assert_ne!(rigid.nodes[0], first.nodes[0]);
        skin.hovered = Some("next".into());
        let hover = skin.surface(1352, 230, g, &palette(), &state, 8).unwrap();
        assert_eq!(hover.hits, first.hits);
        assert_ne!(hover.nodes, first.nodes);
        let mut custom = palette();
        custom.accent = [200, 100, 250];
        custom.bg = [240, 240, 240];
        let themed = skin.surface(1352, 230, g, &custom, &state, 8).unwrap();
        themed.validate().unwrap();
        assert_ne!(themed.assets, hover.assets);
    }
}
