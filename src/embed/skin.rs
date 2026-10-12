//! AMP-owned player artwork. Standalone and embedding use the same real state,
//! controls and geometry; KIT owns sprites, density and transport.
use super::{native, render::PlayerRenderState, GraphicsConfig, Palette};
use anyhow::{Context, Result};
use starkit::native_surface::{
    skin::assets::{AssetCache, Layer, Sprite},
    PixelRect, Primitive, Surface,
};
use std::collections::BTreeMap;

pub(crate) use super::skin_palette::{
    button_rect, calibrated, colors, generic, inactive_led, tone, REFERENCE_PALETTE, SILVER_PALETTE,
};

fn button_background(palette: &Palette, kind: &str) -> String {
    let c = colors(palette);
    match kind {
        "button-pressed" => c.title,
        "button-hover" if *palette == REFERENCE_PALETTE => "#383d49".into(),
        "button-hover" => {
            use starkit::theme::color::Rgb;
            Rgb::parse_hex(&c.panel)
                .unwrap()
                .mix(Rgb::parse_hex(&c.ink).unwrap(), 0.06)
                .to_hex()
        }
        _ => c.panel,
    }
}

fn transport_button(action: &str) -> bool {
    matches!(action, "previous" | "play" | "pause" | "stop" | "next")
}

/// Three satin planes across the blue-gray chassis, behind the original
/// fixed-corner bitmap outline. Bounded strips scale with native pixel density.
pub(crate) fn silver_panel_color(y: u16, height: u16) -> starkit::theme::color::Rgb {
    use starkit::theme::color::Rgb;
    let inner_height = height.saturating_sub(2).max(1);
    let step = inner_height.div_ceil(96);
    let row = y.saturating_sub(1).min(inner_height - 1) / step * step;
    let t = f64::from(row) / f64::from(inner_height.saturating_sub(1).max(1));
    let top = Rgb::new(33, 33, 50);
    let mid = Rgb::new(53, 53, 77);
    let bottom = Rgb::new(32, 32, 48);
    if t <= 0.48 {
        top.mix(mid, t / 0.48)
    } else {
        mid.mix(bottom, (t - 0.48) / 0.52)
    }
}
fn silver_panel(s: &mut Surface, rect: PixelRect, rigid: bool) {
    if rect.width <= 8 || rect.height <= 8 {
        return;
    }
    let radius = if rigid { 0 } else { 3 };
    let inner = PixelRect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
    // Rounded caps let the gradient reach the single-pixel outline while
    // retaining the fixed corner shape. Middle strips meet its straight sides.
    s.fill(inner, &silver_panel_color(1, rect.height).to_hex(), radius);
    s.fill(
        PixelRect::new(inner.x, rect.y + rect.height - 7, inner.width, 6),
        &silver_panel_color(rect.height - 2, rect.height).to_hex(),
        radius,
    );
    let step = (rect.height - 2).div_ceil(96);
    for row in (1..rect.height - 1).step_by(usize::from(step)) {
        let start = row.max(4);
        let end = (row + step).min(rect.height - 4);
        if end <= start {
            continue;
        }
        let shade = silver_panel_color(start, rect.height);
        s.fill(
            PixelRect::new(inner.x, rect.y + start, inner.width, end - start),
            &shade.to_hex(),
            0,
        );
    }
}

/// Give wide instrument displays a recessed glass surface. The original
/// bitmap bevel remains at the outside; all shading sits behind the content.
fn display_relief(s: &mut Surface, rect: PixelRect, palette: &Palette) {
    if rect.width < 320 || rect.height < 64 {
        return;
    }
    use starkit::theme::color::Rgb;
    let c = colors(palette);
    let base = Rgb::parse_hex(&c.inset).unwrap();
    let accent = Rgb::new(palette.accent[0], palette.accent[1], palette.accent[2]);
    let black = Rgb::new(0, 0, 0);
    let inner = PixelRect::new(rect.x + 2, rect.y + 2, rect.width - 4, rect.height - 4);
    let shade = |row: u16| {
        let t = f64::from(row) / f64::from(inner.height - 1);
        let glow = (1. - (2. * t - 1.).powi(2)) * 0.045;
        let shadow = (1. - t).powi(4) * 0.45 + t.powi(6) * 0.18;
        base.mix(accent, glow).mix(black, shadow)
    };
    // Bound node counts even for a tall embedded player. Geometry is logical
    // pixels and is scaled with the rest of the original-density artwork.
    let step = inner.height.div_ceil(64);
    for row in (0..inner.height).step_by(usize::from(step)) {
        s.fill(
            PixelRect::new(
                inner.x,
                inner.y + row,
                inner.width,
                step.min(inner.height - row),
            ),
            &shade(row).to_hex(),
            0,
        );
    }
    for row in (2..inner.height).step_by(usize::from(step.max(4))) {
        s.fill(
            PixelRect::new(inner.x, inner.y + row, inner.width, 1),
            &shade(row).mix(black, 0.06).to_hex(),
            0,
        );
    }
    for (edge, amount) in [0.50, 0.34, 0.20, 0.10].into_iter().enumerate() {
        let edge = edge as u16;
        let shadow = base.mix(black, amount).to_hex();
        s.fill(
            PixelRect::new(inner.x + edge, inner.y, 1, inner.height),
            &shadow,
            0,
        );
        s.fill(
            PixelRect::new(inner.x, inner.y + edge, inner.width, 1),
            &shadow,
            0,
        );
    }
    let lip = base
        .mix(Rgb::parse_hex(&c.highlight).unwrap(), 0.20)
        .to_hex();
    s.fill(
        PixelRect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
        &lip,
        0,
    );
    s.fill(
        PixelRect::new(inner.x + inner.width - 1, inner.y, 1, inner.height),
        &lip,
        0,
    );
}
fn role_color(role: &str, original: &str, palette: &Palette) -> String {
    if *palette != REFERENCE_PALETTE {
        use starkit::theme::color::Rgb;
        let c = colors(palette);
        let blend = |a: &str, b: &str, amount| {
            Rgb::parse_hex(a)
                .unwrap()
                .mix(Rgb::parse_hex(b).unwrap(), amount)
                .to_hex()
        };
        return match role {
            "panel" | "quiet_panel" | "disabled" | "quiet_disabled" => c.panel,
            "background" => native::hex(palette.bg),
            "well" => c.inset,
            "shadow" | "edge_shadow" | "well_shadow" | "control_border" => c.shadow,
            "highlight" | "control_highlight" | "well_highlight" => c.highlight,
            "active" | "quiet_active" => blend(&c.panel, &c.accent, 0.16),
            "title" => c.title,
            "pressed" => blend(&c.panel, &c.accent, 0.08),
            "ink" => c.ink,
            "accent" => c.accent,
            "control" => c.raised,
            "hover" | "quiet_hover" => blend(&c.raised, &c.accent, 0.06),
            "well_border" | "border" => native::hex(palette.border),
            _ => unreachable!("unknown skin role {role}"),
        };
    }
    calibrated(
        original,
        &role_base(role, &REFERENCE_PALETTE),
        &role_base(role, palette),
    )
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
    label_ink: BTreeMap<&'static str, PixelRect>,
    readouts: BTreeMap<(u16, char), ReadoutGlyph>,
}
struct ReadoutGlyph {
    source: PixelRect,
    bearing: u16,
    advance: u16,
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
            let mut label_ink = BTreeMap::new();
            let mut readouts = BTreeMap::new();
            if *name == "minimal-labels" {
                for (label, y, width, height) in super::skin_assets::MINIMAL_LABELS {
                    let (mut left, mut top) = (width * density, height * density);
                    let (mut right, mut bottom) = (0, 0);
                    for yy in 0..height * density {
                        for xx in 0..width * density {
                            if pixels
                                .get_pixel(u32::from(xx), u32::from(y * density + yy))
                                .0[3]
                                > 0
                            {
                                left = left.min(xx);
                                top = top.min(yy);
                                right = right.max(xx + 1);
                                bottom = bottom.max(yy + 1);
                            }
                        }
                    }
                    anyhow::ensure!(right > left && bottom > top, "Empty button label: {label}");
                    // Keep source rectangles in whole logical pixels so the
                    // original 1x/2x lettering is never stretched or resampled.
                    label_ink.insert(
                        *label,
                        PixelRect::new(
                            left / density,
                            y + top / density,
                            right.div_ceil(*density) - left / density,
                            bottom.div_ceil(*density) - top / density,
                        ),
                    );
                }
            }
            if *name == "readout-glyphs" {
                for cell in super::skin_assets::READOUT_CELLS {
                    let r = cell.rect;
                    let (mut left, mut top) = (r.width * density, r.height * density);
                    let (mut right, mut bottom) = (0, 0);
                    for y in 0..r.height * density {
                        for x in 0..r.width * density {
                            if pixels
                                .get_pixel(
                                    u32::from(r.x * density + x),
                                    u32::from(r.y * density + y),
                                )
                                .0[3]
                                > 0
                            {
                                left = left.min(x);
                                top = top.min(y);
                                right = right.max(x + 1);
                                bottom = bottom.max(y + 1);
                            }
                        }
                    }
                    anyhow::ensure!(right > left && bottom > top, "Empty readout glyph");
                    readouts.insert(
                        (cell.size, cell.character),
                        ReadoutGlyph {
                            source: PixelRect::new(
                                r.x + left / density,
                                r.y + top / density,
                                right.div_ceil(*density) - left / density,
                                bottom.div_ceil(*density) - top / density,
                            ),
                            bearing: left / density,
                            advance: cell.advance,
                        },
                    );
                }
            }
            let (id, png) = starkit::terminal_graphics::assets::encode_skin(&pixels)?;
            glyphs.insert(
                (*density, *name),
                GlyphAsset {
                    id,
                    png,
                    label_ink,
                    readouts,
                },
            );
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
            let value = role_color(role, original, palette);
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
            "well-pane",
            "button-normal",
            "button-active",
            "button-hover",
            "button-pressed",
            "button-focus",
            "button-disabled",
        ] {
            let key = format!(
                "{density}/{}",
                if name == "well-pane" { "well" } else { name }
            );
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
            let mut pane_roles;
            let sprite_roles = if name == "well-pane" {
                pane_roles = roles.clone();
                let inset = colors(&Palette {
                    inset_bg: None,
                    ..*palette
                })
                .inset;
                pane_roles.insert(
                    "well".into(),
                    [
                        u8::from_str_radix(&inset[1..3], 16)?,
                        u8::from_str_radix(&inset[3..5], 16)?,
                        u8::from_str_radix(&inset[5..7], 16)?,
                    ],
                );
                &pane_roles
            } else {
                &roles
            };
            let pixels = self
                .source
                .compose(24 * density, 24 * density, layers, sprite_roles)?;
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
    pub(crate) fn pane_well(&self, s: &mut Surface, rect: PixelRect) {
        self.paint(s, rect, "well-pane");
    }
    pub(crate) fn focus_outline(&self, s: &mut Surface, accent: &str) {
        let name = if self.rigid {
            "focus-rigid"
        } else {
            "focus-rounded"
        };
        let asset = &self.glyphs[&(self.density, name)];
        s.assets.insert(asset.id.clone(), Some(asset.png.clone()));
        s.nodes.push(Primitive::Sprite {
            rect: PixelRect::new(0, 0, s.width, s.height),
            asset: asset.id.clone(),
            source: PixelRect::new(0, 0, 24 * self.density, 24 * self.density),
            insets: Some([6 * self.density; 4]),
            tint: Some(accent.into()),
        });
    }
    pub(crate) fn button(&self, s: &mut Surface, rect: PixelRect, action: &str, active: bool) {
        <Self as native::PlayerArtwork>::button(self, s, rect, action, active);
    }
    pub(crate) fn ink(&self, action: &str, active: bool) -> String {
        <Self as native::PlayerArtwork>::button_ink(self, action, active)
    }
    pub(crate) fn button_label(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        text: &str,
        tint: &str,
    ) -> bool {
        let Some(source) = self.glyphs[&(self.density, "minimal-labels")]
            .label_ink
            .get(text)
        else {
            return false;
        };
        if source.width > rect.width || source.height > rect.height {
            return false;
        }
        // Center the actual ink, rather than the atlas cell's transparent padding.
        self.glyph(
            s,
            PixelRect::new(
                rect.x + (rect.width - source.width) / 2,
                rect.y + (rect.height - source.height) / 2,
                source.width,
                source.height,
            ),
            "minimal-labels",
            *source,
            tint,
        );
        true
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
    fn readout_advance(&self, size: u16) -> Option<u16> {
        self.glyphs[&(self.density, "readout-glyphs")]
            .readouts
            .get(&(size, '0'))
            .map(|glyph| glyph.advance)
    }
    fn readout_width(&self, text: &str, size: u16) -> Option<u16> {
        let glyphs = &self.glyphs[&(self.density, "readout-glyphs")].readouts;
        let first = glyphs.get(&(size, text.chars().next()?))?;
        let mut advance = 0;
        let mut right = 0;
        for character in text.chars() {
            let glyph = glyphs.get(&(size, character))?;
            right = advance + glyph.bearing + glyph.source.width;
            advance += glyph.advance;
        }
        Some(right - first.bearing)
    }
    fn readout(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        text: &str,
        color: &str,
        size: u16,
    ) -> bool {
        let Some(width) = self.readout_width(text, size) else {
            return false;
        };
        if width > rect.width {
            return false;
        }
        let glyphs = &self.glyphs[&(self.density, "readout-glyphs")].readouts;
        let first = glyphs[&(size, text.chars().next().unwrap())].bearing;
        let mut advance = 0;
        for character in text.chars() {
            let glyph = &glyphs[&(size, character)];
            self.glyph(
                s,
                PixelRect::new(
                    rect.x + advance + glyph.bearing - first,
                    rect.y + rect.height.saturating_sub(24) / 2 + glyph.source.y % 24,
                    glyph.source.width,
                    glyph.source.height,
                ),
                "readout-glyphs",
                glyph.source,
                color,
            );
            advance += glyph.advance;
        }
        true
    }
    fn fixed_label(
        &self,
        s: &mut Surface,
        rect: PixelRect,
        text: &str,
        color: &str,
        centered: bool,
    ) -> bool {
        let _ = centered;
        self.button_label(s, rect, text, color)
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
        let inactive = if p == SILVER_PALETTE {
            "#203226".into()
        } else if p == REFERENCE_PALETTE {
            calibrated("#203226", &reference, &inactive)
        } else {
            inactive_led(&p)
        };
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
        if !inset && self.palette == Some(SILVER_PALETTE) {
            let name = if self.rigid {
                "silver-panel-rigid"
            } else {
                "silver-panel-rounded"
            };
            let asset = &self.glyphs[&(self.density, name)];
            s.assets.insert(asset.id.clone(), Some(asset.png.clone()));
            s.nodes.push(Primitive::Sprite {
                rect,
                asset: asset.id.clone(),
                source: PixelRect::new(0, 0, 24 * self.density, 24 * self.density),
                insets: Some([6 * self.density; 4]),
                tint: None,
            });
        } else {
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
        if inset {
            display_relief(s, rect, &self.palette.expect("prepared skin palette"));
        } else if self.palette == Some(SILVER_PALETTE) {
            silver_panel(s, rect, self.rigid);
        }
    }
    fn button_ink(&self, action: &str, active: bool) -> String {
        use starkit::theme::color::Rgb;
        let p = self.palette.expect("prepared skin palette");
        let kind = self.button_kind(action, active);
        if p == SILVER_PALETTE && transport_button(action) {
            return if kind == "button-disabled" {
                "#404f5e"
            } else {
                "#273743"
            }
            .into();
        }
        if kind == "button-disabled" {
            // Disabled controls are intentionally subdued, not promoted to
            // bright active ink by the enabled-control contrast correction.
            return native::hex(p.muted);
        }
        let color = if transport_button(action) {
            let role = match kind {
                "button-pressed" => "pressed",
                "button-hover" => "hover",
                "button-active" => "active",
                _ => "control",
            };
            role_color(role, &self.manifest.roles[role], &p)
        } else {
            button_background(&p, kind)
        };
        let background = Rgb::parse_hex(&color).expect("derived skin color");
        let ink = if transport_button(action) && p == REFERENCE_PALETTE {
            Rgb::parse_hex(if active { "#10161a" } else { "#e0e2d5" }).unwrap()
        } else if active && !transport_button(action) {
            Rgb::new(p.accent[0], p.accent[1], p.accent[2])
        } else {
            Rgb::new(p.fg[0], p.fg[1], p.fg[2])
        };
        if background.contrast(ink) >= 4.5 {
            ink.to_hex()
        } else {
            background
                .best_contrast_against(&[starkit::theme::BLACK, starkit::theme::WHITE])
                .to_hex()
        }
    }
    fn button(&self, s: &mut Surface, rect: PixelRect, action: &str, active: bool) {
        let kind = self.button_kind(action, active);
        if transport_button(action) {
            if self.palette == Some(SILVER_PALETTE) {
                let name = format!("silver-{kind}");
                let asset = &self.glyphs[&(self.density, name.as_str())];
                s.assets.insert(asset.id.clone(), Some(asset.png.clone()));
                s.nodes.push(Primitive::Sprite {
                    rect,
                    asset: asset.id.clone(),
                    source: PixelRect::new(0, 0, 29 * self.density, 29 * self.density),
                    insets: None,
                    tint: None,
                });
                return;
            }
            self.paint(s, rect, kind);
            return;
        }
        if kind == "button-disabled" {
            return;
        }
        let p = self.palette.expect("prepared skin palette");
        let rect = button_rect(rect);
        if matches!(kind, "button-hover" | "button-pressed") {
            s.fill(rect, &button_background(&p, kind), 0);
            s.nodes.push(Primitive::Border {
                rect,
                color: if p == REFERENCE_PALETTE {
                    if kind == "button-pressed" {
                        "#11131a"
                    } else {
                        "#555c6b"
                    }
                    .into()
                } else {
                    native::hex(p.border)
                },
                radius: 0,
            });
        }
        if active || self.focused.as_deref() == Some(action) {
            s.nodes.push(Primitive::Border {
                rect,
                color: native::hex(p.accent),
                radius: 0,
            });
        }
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
            loading: false,
            position: 30.,
            buffered: 30.,
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
            inset_bg: None,
            fg: [215, 217, 207],
            muted: [152, 158, 172],
            accent: [164, 215, 145],
            selected: [48, 51, 64],
            border: [116, 121, 134],
            error: [240, 60, 60],
        }
    }
    #[test]
    fn thicker_seek_bar_renders_played_buffered_and_empty_at_both_densities() {
        use starkit::terminal_graphics::renderer::SurfaceOverlayRenderer;
        let mut skin = PlayerSkin::new().unwrap();
        let mut state = state();
        state.buffered = 84.;
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
            let seek = surface
                .hits
                .iter()
                .find(|hit| hit.action == "seek")
                .unwrap()
                .rect;
            let image = SurfaceOverlayRenderer::new("monospace")
                .render(
                    &starkit::image::RgbaImage::new(
                        u32::from(surface.width),
                        u32::from(surface.height),
                    ),
                    &surface,
                )
                .unwrap();
            let x = |fraction: f64| u32::from(seek.x) + (f64::from(seek.width) * fraction) as u32;
            let y = u32::from(seek.y + seek.height / 2);
            let played = image.get_pixel(x(0.1), y);
            let buffered = image.get_pixel(x(0.5), y);
            let empty = image.get_pixel(x(0.9), y);
            assert_ne!(played, buffered);
            assert_ne!(buffered, empty);
            for dy in 0..3 * density {
                assert_eq!(image.get_pixel(x(0.5), y + u32::from(dy)), buffered);
            }
        }
    }

    #[test]
    fn silver_shading_and_metal_buttons_render_at_both_densities() {
        let mut skin = PlayerSkin::new().unwrap();
        let mut renderer =
            starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new("monospace");
        for density in [1, 2] {
            skin.prepare(&SILVER_PALETTE, density, false).unwrap();
            let mut surface = Surface::new(300, 240, "#000000".into());
            skin.frame(&mut surface, PixelRect::new(0, 0, 300, 240), false);
            for (i, kind) in ["normal", "hover", "pressed", "focus", "active", "disabled"]
                .iter()
                .enumerate()
            {
                skin.hovered = (*kind == "hover").then(|| "play".into());
                skin.pressed = (*kind == "pressed").then(|| "play".into());
                skin.focused = (*kind == "focus").then(|| "play".into());
                skin.disabled.clear();
                if *kind == "disabled" {
                    skin.disabled.insert("play");
                }
                skin.button(
                    &mut surface,
                    PixelRect::new(10 + i as u16 * 40, 180, 29, 29),
                    "play",
                    *kind == "active",
                );
            }
            let surface = surface.at_density(density).unwrap();
            surface.validate().unwrap();
            let scale = u32::from(density);
            let image = renderer
                .render(
                    &starkit::image::RgbaImage::new(300 * scale, 240 * scale),
                    &surface,
                )
                .unwrap();
            let pixel = |x, y| image.get_pixel(x * scale, y * scale).0;
            for (x, y, color) in [
                (280, 0, [96, 101, 124, 255]),
                (280, 239, [96, 101, 124, 255]),
                (0, 130, [96, 101, 124, 255]),
                (299, 130, [96, 101, 124, 255]),
                (280, 1, [33, 33, 50, 255]),
                (280, 2, [33, 33, 50, 255]),
                (280, 3, [33, 33, 50, 255]),
            ] {
                assert_eq!(pixel(x, y), color, "thin outline at {x},{y}");
            }
            for y in [12, 130, 230] {
                assert_eq!(pixel(1, y), pixel(280, y), "left body reaches outline");
                assert_eq!(pixel(298, y), pixel(280, y), "right body reaches outline");
            }
            let top = pixel(280, 12);
            let middle = pixel(280, 115);
            let bottom = pixel(280, 230);
            assert!(middle[0] > top[0] + 12 && middle[0] > bottom[0] + 12);
            assert!(top[2] > top[0] && middle[2] > middle[0] && bottom[2] > bottom[0]);
            let face_fall = pixel(24, 185)[0].saturating_sub(pixel(24, 203)[0]);
            assert!((10..=30).contains(&face_fall), "restrained silver gradient");
            assert_eq!(pixel(10, 180), [16, 20, 30, 255], "opaque square corner");
            assert!(
                pixel(11, 181)[0] > pixel(37, 207)[0] + 100,
                "hard one-pixel bevel"
            );
            assert!(pixel(24, 203)[0] > 100, "button face should remain silver");
            assert_eq!(pixel(184, 206), [107, 152, 91, 255], "active underline");
        }
    }
    #[test]
    fn reference_palette_keeps_original_chrome_with_recessed_display() {
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
            (700, 104, [20, 30, 23, 255]),
            (20, 210, [71, 77, 93, 255]),
            (55, 210, [107, 114, 130, 255]),
        ] {
            assert_eq!(image.get_pixel(x, y).0, color, "reference color at {x},{y}");
        }
    }
    #[test]
    fn led_segments_stay_fixed_as_levels_change_at_both_densities() {
        let mut skin = PlayerSkin::new().unwrap();
        let mut state = state();
        state.vis_mode = VisMode::Leds;
        let accent = native::hex(REFERENCE_PALETTE.accent);
        let off = inactive_led(&REFERENCE_PALETTE);
        for density in [1, 2] {
            let mut fixed_rects = None;
            let mut previous_lit = 0;
            for level in [0., 0.17, 0.38, 0.71, 1.] {
                state.bands.fill(level);
                let surface = skin
                    .surface_at_density(
                        1352 * density,
                        230 * density,
                        GraphicsConfig {
                            cell_width: 8 * density,
                            cell_height: 16 * density,
                        },
                        &REFERENCE_PALETTE,
                        &state,
                        8,
                        density,
                    )
                    .unwrap();
                let cells = surface
                    .nodes
                    .iter()
                    .filter_map(|node| match node {
                        Primitive::Fill { rect, color, .. }
                            if color == &accent || color == &off =>
                        {
                            Some((*rect, color == &accent))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let rects = cells.iter().map(|(rect, _)| *rect).collect::<Vec<_>>();
                if let Some(fixed) = &fixed_rects {
                    assert_eq!(&rects, fixed, "LED cells moved at level {level}");
                } else {
                    fixed_rects = Some(rects);
                }
                let lit = cells.iter().filter(|(_, lit)| *lit).count();
                assert!(lit >= previous_lit);
                if level == 1. {
                    assert!(lit > previous_lit);
                }
                previous_lit = lit;
            }
        }
    }
    #[test]
    fn kanagawa_displays_and_inactive_leds_follow_the_theme_background() {
        let t = crate::theme::builtin::load("kanagawa").unwrap();
        let rgb = |c: starkit::theme::color::Rgb| [c.r, c.g, c.b];
        let p = Palette {
            bg: rgb(t.bg),
            inset_bg: None,
            fg: rgb(t.fg),
            muted: rgb(t.dim),
            accent: rgb(t.accent),
            selected: rgb(t.row_selected_bg),
            border: rgb(t.border),
            error: rgb(t.error),
        };
        let c = colors(&p);
        assert_eq!(c.panel, t.bg.to_hex());
        assert_eq!(
            c.inset,
            t.bg.mix(starkit::theme::color::Rgb::new(0, 0, 0), 0.06)
                .to_hex()
        );
        assert_eq!(c.accent, t.accent.to_hex());
        assert_eq!(role_color("panel", "#303340", &p), c.panel);
        assert_eq!(role_color("quiet_panel", "#20212a", &p), c.panel);
        assert_eq!(role_color("well", "#101813", &p), c.inset);
        assert_eq!(role_color("background", "#171820", &p), native::hex(p.bg));
        let off = starkit::theme::color::Rgb::parse_hex(&inactive_led(&p)).unwrap();
        assert!(
            off.b > off.g && off.b > off.r,
            "Kanagawa's unlit LEDs retain its blue background hue"
        );
        let mut skin = PlayerSkin::new().unwrap();
        let surface = skin
            .surface_at_density(
                1352,
                230,
                GraphicsConfig {
                    cell_width: 8,
                    cell_height: 16,
                },
                &p,
                &state(),
                8,
                1,
            )
            .unwrap();
        let image = starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new("monospace")
            .render(&starkit::image::RgbaImage::new(1352, 230), &surface)
            .unwrap();
        let expected = starkit::theme::color::Rgb::parse_hex(&c.inset).unwrap();
        let body = image.get_pixel(700, 104).0;
        let top = image.get_pixel(700, 45).0;
        assert_ne!(body, [expected.r, expected.g, expected.b, 255]);
        assert!(
            body[2] > body[0] && body[2] > body[1],
            "display relief retains Kanagawa's blue hue"
        );
        assert!(
            top[..3].iter().map(|v| u16::from(*v)).sum::<u16>()
                < body[..3].iter().map(|v| u16::from(*v)).sum::<u16>(),
            "the inset's top shadow must be darker than its glass"
        );
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
    fn eq_button_ink_is_centered_in_hover_outlines_at_both_densities() {
        let mut skin = PlayerSkin::new().unwrap();
        for density in [1, 2] {
            skin.prepare(&SILVER_PALETTE, density, true).unwrap();
            let png = super::super::skin_assets::GLYPHS
                .iter()
                .find(|(d, name, _)| *d == density && *name == "minimal-labels")
                .unwrap()
                .2;
            let atlas = starkit::image::load_from_memory(png).unwrap().to_rgba8();
            for (text, width) in [
                ("SAVE", 64),
                ("SAVE AS", 88),
                ("IMPORT", 82),
                ("EXPORT", 96),
                ("NEW", 60),
                ("LOAD", 152),
                ("RENAME", 80),
                ("REVERT", 80),
                ("CANCEL", 96),
                ("APPLY", 100),
                ("BYPASS", 100),
                ("DELETE", 100),
            ] {
                let rect = PixelRect::new(12, 12, width, 29);
                let mut surface = Surface::new(200, 80, "#000000".into());
                skin.hovered = Some("eq-action".into());
                skin.button(&mut surface, rect, "eq-action", false);
                assert!(
                    skin.button_label(&mut surface, button_rect(rect), text, "#ffffff"),
                    "{text} must use the common lettering"
                );
                let physical = surface.at_density(density).unwrap();
                let outline = physical
                    .nodes
                    .iter()
                    .find_map(|n| match n {
                        Primitive::Border { rect, .. } => Some(*rect),
                        _ => None,
                    })
                    .unwrap();
                let (drawn, source) = physical
                    .nodes
                    .iter()
                    .find_map(|n| match n {
                        Primitive::Sprite { rect, source, .. } => Some((*rect, *source)),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(
                    (drawn.width, drawn.height),
                    (source.width, source.height),
                    "lettering must retain original density pixels"
                );
                let mut ink = Vec::new();
                for y in 0..source.height {
                    for x in 0..source.width {
                        if atlas
                            .get_pixel(u32::from(source.x + x), u32::from(source.y + y))
                            .0[3]
                            > 0
                        {
                            ink.push((drawn.x + x, drawn.y + y));
                        }
                    }
                }
                let left = ink.iter().map(|p| p.0).min().unwrap();
                let right = ink.iter().map(|p| p.0).max().unwrap() + 1;
                let top = ink.iter().map(|p| p.1).min().unwrap();
                let bottom = ink.iter().map(|p| p.1).max().unwrap() + 1;
                for (a, b, origin, size) in [
                    (left, right, outline.x, outline.width),
                    (top, bottom, outline.y, outline.height),
                ] {
                    let error =
                        (i32::from(a) + i32::from(b) - i32::from(origin) * 2 - i32::from(size))
                            .abs();
                    assert!(
                        error <= i32::from(density),
                        "{text} at {density}x is off-center by {error}/2 physical pixels"
                    );
                }
            }
        }
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
                for action in ["play", "shuffle"] {
                    for kind in [
                        "button-normal",
                        "button-active",
                        "button-hover",
                        "button-focus",
                        "button-pressed",
                    ] {
                        skin.hovered = (kind == "button-hover").then(|| action.into());
                        skin.focused = (kind == "button-focus").then(|| action.into());
                        skin.pressed = (kind == "button-pressed").then(|| action.into());
                        let color = skin.button_ink(action, kind == "button-active");
                        let foreground = Rgb::parse_hex(&color).unwrap();
                        let mut surface = Surface::new(24, 24, colors(&palette).panel);
                        skin.button(
                            &mut surface,
                            PixelRect::new(0, 0, 24, 24),
                            action,
                            kind == "button-active",
                        );
                        if action == "play" {
                            assert!(
                                matches!(surface.nodes.as_slice(), [Primitive::Sprite { asset, .. }] if asset == &skin.prepared[kind].id),
                                "transport artwork changed: {kind}"
                            );
                        } else if kind == "button-normal" {
                            assert!(surface.nodes.is_empty(), "idle buttons must be transparent");
                        } else if action == "shuffle"
                            && matches!(kind, "button-active" | "button-focus")
                        {
                            assert!(matches!(
                                surface.nodes.as_slice(),
                                [Primitive::Border { .. }]
                            ));
                        }
                        let bg =
                            starkit::theme::color::Rgb::parse_hex(&colors(&palette).panel).unwrap();
                        let base = starkit::image::RgbaImage::from_pixel(
                            24,
                            24,
                            starkit::image::Rgba([bg.r, bg.g, bg.b, 255]),
                        );
                        let image =
                            starkit::terminal_graphics::renderer::SurfaceOverlayRenderer::new(
                                "monospace",
                            )
                            .render(&base, &surface)
                            .unwrap();
                        let pixel = image.get_pixel(12, 12);
                        let background = Rgb::new(pixel[0], pixel[1], pixel[2]);
                        assert!(foreground.contrast(background) >= 4.5, "{kind}");
                    }
                }
            }
        }
    }
    #[test]
    fn readout_ink_has_equal_track_gaps_at_both_densities() {
        use starkit::terminal_graphics::renderer::SurfaceOverlayRenderer;
        let mut skin = PlayerSkin::new().unwrap();
        let graphics = GraphicsConfig {
            cell_width: 8,
            cell_height: 16,
        };
        let mut state = state();
        for density in [1, 2] {
            let mut previous_volume = None;
            for (position, duration, volume_level) in
                [(117., 584., 0.08), (599., 600., 0.42), (600., 6222., 1.)]
            {
                state.position = position;
                state.duration = duration;
                state.volume = volume_level;
                let surface = skin
                    .logical_surface(1200, 230, graphics, &palette(), &state, 8, density)
                    .unwrap()
                    .at_density(density)
                    .unwrap();
                let seek = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "seek")
                    .unwrap()
                    .rect;
                let volume = surface
                    .hits
                    .iter()
                    .find(|h| h.action == "volume")
                    .unwrap()
                    .rect;
                if let Some(before) = previous_volume {
                    assert_eq!(before, volume);
                }
                previous_volume = Some(volume);
                let id = &skin.glyphs[&(density, "readout-glyphs")].id;
                let mut readouts = surface.clone();
                readouts.background = "transparent".into();
                readouts
                    .nodes
                    .retain(|n| matches!(n, Primitive::Sprite { asset, .. } if asset == id));
                assert!(!readouts.nodes.is_empty());
                for node in &readouts.nodes {
                    if let Primitive::Sprite { rect, source, .. } = node {
                        assert_eq!(
                            (rect.width, rect.height),
                            (source.width, source.height),
                            "resampled readout glyph"
                        );
                    }
                }
                let mut previous_pixels = None;
                for family in ["Liberation Mono", "JetBrainsMono Nerd Font"] {
                    let base = starkit::image::RgbaImage::new(
                        u32::from(surface.width),
                        u32::from(surface.height),
                    );
                    let image = SurfaceOverlayRenderer::new(family)
                        .render(&base, &readouts)
                        .unwrap();
                    if let Some(before) = previous_pixels {
                        assert!(before == image, "terminal font changed the readout");
                    }
                    previous_pixels = Some(image.clone());
                    let bounds = |r: PixelRect| {
                        let mut left = u16::MAX;
                        let mut right = 0;
                        for y in r.y..r.y + r.height {
                            for x in r.x..r.x + r.width {
                                if image.get_pixel(u32::from(x), u32::from(y)).0[3] > 0 {
                                    left = left.min(x);
                                    right = right.max(x + 1);
                                }
                            }
                        }
                        assert!(right > left, "readout is empty");
                        (left, right)
                    };
                    let elapsed = bounds(PixelRect::new(0, seek.y, seek.x, seek.height));
                    let total = bounds(PixelRect::new(
                        seek.x + seek.width,
                        seek.y,
                        surface.width - seek.x - seek.width,
                        seek.height,
                    ));
                    let caption = bounds(PixelRect::new(0, volume.y, volume.x, volume.height));
                    let value = bounds(PixelRect::new(
                        volume.x + volume.width,
                        volume.y,
                        surface.width - volume.x - volume.width,
                        volume.height,
                    ));
                    for (gap, expected) in [
                        (seek.x - elapsed.1, 24),
                        (total.0 - seek.x - seek.width, 24),
                        (volume.x - caption.1, 18),
                        (value.0 - volume.x - volume.width, 18),
                    ] {
                        assert!(
                            gap.abs_diff(expected * density) < density,
                            "unequal visible ink gap: {gap}, density {density}"
                        );
                    }
                    assert!(
                        (16 * density..20 * density).contains(&elapsed.0),
                        "elapsed field overflows its inset"
                    );
                    assert!(
                        (surface.width - total.1).abs_diff(16 * density) < density,
                        "total timestamp lost its outer inset"
                    );
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
