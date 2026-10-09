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
}
impl PlayerSkin {
    pub fn new() -> Result<Self> {
        let mut source = AssetCache::new(8_000_000);
        for (id, png) in super::skin_assets::PNGS {
            source.insert_png(id, png)?;
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
        })
    }
    fn prepare(&mut self, palette: &Palette, density: u16, rigid: bool) -> Result<()> {
        if self.palette.as_ref() == Some(palette) && self.density == density && self.rigid == rigid
        {
            return Ok(());
        }
        let c = Colors::new(
            palette.bg,
            palette.fg,
            palette.muted,
            palette.accent,
            palette.border,
        );
        let hover = starkit::theme::color::Rgb::new(
            palette.selected[0],
            palette.selected[1],
            palette.selected[2],
        )
        .mix(
            starkit::theme::color::Rgb::new(
                palette.accent[0],
                palette.accent[1],
                palette.accent[2],
            ),
            0.16,
        )
        .to_hex();
        let mut roles = BTreeMap::new();
        for role in self.manifest.roles.keys() {
            let value = match role.as_str() {
                "panel" | "quiet_panel" | "disabled" | "quiet_disabled" => &c.panel,
                "shadow" | "edge_shadow" | "well_shadow" | "control_border" => &c.shadow,
                "highlight" | "control_highlight" | "well_highlight" => &c.highlight,
                "title" => &c.title,
                "ink" => &c.ink,
                "accent" => &c.accent,
                "well" | "background" => &c.inset,
                "control" => &c.raised,
                "hover" | "quiet_hover" => &hover,
                "active" | "quiet_active" => &c.highlight,
                "pressed" => &c.title,
                "well_border" | "border" => &super::native::hex(palette.border),
                _ => anyhow::bail!("Unmapped skin theme role: {role}"),
            };
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
        if self.pressed.as_deref() == Some(action) {
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
        let c = Colors::new(p.bg, p.fg, p.muted, p.accent, p.border);
        let kind = self.button_kind(action, active);
        let color = if kind == "button-pressed" {
            c.title
        } else if kind == "button-hover" {
            Rgb::new(p.selected[0], p.selected[1], p.selected[2])
                .mix(Rgb::new(p.accent[0], p.accent[1], p.accent[2]), 0.16)
                .to_hex()
        } else if kind == "button-active" {
            c.highlight
        } else {
            c.raised
        };
        let background = Rgb::parse_hex(&color).expect("derived skin color");
        let ink = Rgb::new(p.fg[0], p.fg[1], p.fg[2]);
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
                    let c = Colors::new(
                        palette.bg,
                        palette.fg,
                        palette.muted,
                        palette.accent,
                        palette.border,
                    );
                    let background = match kind {
                        "button-active" => Rgb::parse_hex(&c.highlight).unwrap(),
                        "button-pressed" => Rgb::parse_hex(&c.title).unwrap(),
                        "button-hover" => Rgb::new(
                            palette.selected[0],
                            palette.selected[1],
                            palette.selected[2],
                        )
                        .mix(Rgb::new(accent[0], accent[1], accent[2]), 0.16),
                        _ => Rgb::parse_hex(&c.raised).unwrap(),
                    };
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
        assert_eq!(first.hits, paused.hits);
        assert_ne!(first.nodes, paused.nodes);
        // The host and standalone do not get independent implementations.
        let embedded = PlayerSkin::new()
            .unwrap()
            .surface(1352, 230, g, &palette(), &state, 8)
            .unwrap();
        assert_eq!(paused, embedded);
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
