//! Offline visual proof. Does not change the installed AMP interface.
use anyhow::Result;
use starkit::native_surface::HitRegion;
use starkit::{
    image::{self, Rgba, RgbaImage},
    native_surface::{
        skin::{
            assets::{AssetCache, Layer, Sprite},
            BitmapFont,
        },
        PixelRect, Primitive, Surface,
    },
    terminal_graphics::renderer::SurfaceOverlayRenderer,
};
use std::collections::BTreeMap;
use std::path::Path;
#[derive(Clone)]
struct State {
    theme: usize,
    classic: bool,
    bitmap: bool,
    density: u16,
    rigid: bool,
    hover: Option<usize>,
    pressed: Option<usize>,
    focus: Option<usize>,
    playing: bool,
    shuffle: bool,
    repeat: bool,
    volume: u16,
    position: u16,
    unicode: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            theme: 0,
            classic: true,
            bitmap: false,
            density: 1,
            rigid: false,
            hover: None,
            pressed: None,
            focus: None,
            playing: true,
            shuffle: false,
            repeat: true,
            volume: 80,
            position: 23,
            unicode: false,
        }
    }
}
#[derive(serde::Deserialize)]
struct Manifest {
    sprites: BTreeMap<String, Sprite>,
    layers: BTreeMap<String, Vec<Layer>>,
    roles: BTreeMap<String, String>,
}
struct Proof {
    cache: AssetCache,
    manifest: Manifest,
    themed_assets: BTreeMap<String, RgbaImage>,
    palette: BTreeMap<String, String>,
    active_theme: usize,
    font: BitmapFont,
    renderer: SurfaceOverlayRenderer,
}
impl Proof {
    fn new() -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skins/classic");
        let mut cache = AssetCache::new(8_000_000);
        for density in [1, 2] {
            for entry in std::fs::read_dir(root.join(format!("{density}x")))? {
                let entry = entry?;
                // The production clock atlases are not used by this isolated
                // proof. Keep them out of its independent bounded cache.
                if matches!(
                    entry.path().file_stem().and_then(|s| s.to_str()),
                    Some("clock-on" | "clock-off" | "transport-glyphs" | "fixed-labels")
                ) {
                    continue;
                }
                if entry.path().extension().is_some_and(|e| e == "png") {
                    let id = format!(
                        "{density}/{}",
                        entry.path().file_stem().unwrap().to_string_lossy()
                    );
                    cache.insert_png(&id, &std::fs::read(entry.path())?)?;
                }
            }
        }
        Ok(Self {
            cache,
            manifest: serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?,
            themed_assets: BTreeMap::new(),
            palette: BTreeMap::new(),
            active_theme: usize::MAX,
            font: serde_json::from_slice(&std::fs::read(root.join("control-font.json"))?)?,
            renderer: SurfaceOverlayRenderer::new("Liberation Mono"),
        })
    }
    fn prepare_theme(&mut self, index: usize) -> Result<()> {
        if self.active_theme == index {
            return Ok(());
        }
        self.palette.clear();
        self.themed_assets.clear();
        if index == 0 {
            self.active_theme = index;
            return Ok(());
        }
        let builtin = starkit::theme::BUILTINS
            .get(index - 1)
            .ok_or_else(|| anyhow::anyhow!("Invalid theme index"))?;
        let theme =
            starkit::theme::Theme::resolve(&starkit::theme::ThemeFile::parse(builtin.toml)?);
        let mut colors = BTreeMap::new();
        for (role, original) in &self.manifest.roles {
            let c = match role.as_str() {
                "panel" | "quiet_panel" => theme.panel_bg,
                "background" => theme.bg,
                "shadow" | "edge_shadow" | "well_shadow" => {
                    theme.bg.mix(starkit::theme::BLACK, 0.3)
                }
                "highlight" | "control_highlight" => theme.panel_bg.mix(theme.fg, 0.35),
                "title" => theme.header_bg,
                "ink" => theme.fg,
                "accent" | "active" | "quiet_active" => theme.accent,
                "well" => theme.bg,
                "well_border" | "control_border" | "border" => theme.border,
                "well_highlight" => theme.border.mix(theme.fg, 0.2),
                "control" => theme.panel_bg.mix(theme.fg, 0.08),
                "hover" | "quiet_hover" => theme.panel_bg.mix(theme.accent, 0.15),
                "pressed" => theme.panel_bg.mix(theme.accent, 0.25),
                "disabled" | "quiet_disabled" => theme.panel_bg.mix(theme.bg, 0.5),
                _ => anyhow::bail!("Unknown artwork theme role {role}"),
            };
            colors.insert(role.clone(), [c.r, c.g, c.b]);
            self.palette.insert(original.clone(), c.to_hex());
        }
        for (original, c) in [
            ("#989eac", theme.dim),
            ("#bbc6d5", theme.header_fg),
            ("#a5aaba", theme.dim),
            ("#e0e2d5", theme.fg),
            ("#10161a", {
                let candidate =
                    if theme.accent.contrast(theme.bg) >= theme.accent.contrast(theme.fg) {
                        theme.bg
                    } else {
                        theme.fg
                    };
                if theme.accent.contrast(candidate) >= 4.5 {
                    candidate
                } else if theme.accent.contrast(starkit::theme::BLACK)
                    >= theme.accent.contrast(starkit::theme::WHITE)
                {
                    starkit::theme::BLACK
                } else {
                    starkit::theme::WHITE
                }
            }),
            ("#151920", theme.border),
            ("#203226", theme.bg.mix(theme.accent, 0.14)),
            ("#1c2a20", theme.bg.mix(theme.accent, 0.10)),
        ] {
            self.palette.insert(original.into(), c.to_hex());
        }
        for (id, layers) in &self.manifest.layers {
            let d = layers
                .first()
                .ok_or_else(|| anyhow::anyhow!("Empty artwork layers"))?
                .sprite
                .density;
            self.themed_assets.insert(
                id.clone(),
                self.cache.compose(24 * d, 24 * d, layers, &colors)?,
            );
        }
        self.active_theme = index;
        Ok(())
    }
}
struct Raster {
    palette: BTreeMap<String, String>,
    pixels: RgbaImage,
    density: u16,
}
fn regions(width: u16) -> Vec<HitRegion> {
    let mut hits: Vec<_> = ["previous", "play", "pause", "stop", "next"]
        .iter()
        .enumerate()
        .map(|(i, a)| HitRegion {
            rect: PixelRect::new(16 + i as u16 * 35, 187, 29, 29),
            action: (*a).into(),
        })
        .collect();
    hits.extend(
        [
            (237, 187, 100, 29, "shuffle"),
            (346, 187, 100, 29, "repeat"),
            (78, 160, width - 156, 18, "seek"),
            (width - 180, 192, 130, 22, "volume"),
        ]
        .map(|(x, y, w, h, a)| HitRegion {
            rect: PixelRect::new(x, y, w, h),
            action: a.into(),
        }),
    );
    hits
}
fn button_state(state: &State, index: usize, on: bool) -> &'static str {
    if !state.classic {
        return if state.pressed == Some(index) {
            "button-fold-pressed"
        } else if state.focus == Some(index) {
            "button-fold-focus"
        } else if state.hover == Some(index) {
            "button-fold-hover"
        } else if on {
            "button-fold-active"
        } else {
            "button-fold-normal"
        };
    }
    if state.pressed == Some(index) {
        "button-pressed"
    } else if state.focus == Some(index) {
        "button-focus"
    } else if state.hover == Some(index) {
        "button-hover"
    } else if on {
        "button-active"
    } else {
        "button-normal"
    }
}
fn color(s: &str) -> Rgba<u8> {
    Rgba([
        u8::from_str_radix(&s[1..3], 16).unwrap(),
        u8::from_str_radix(&s[3..5], 16).unwrap(),
        u8::from_str_radix(&s[5..7], 16).unwrap(),
        255,
    ])
}
fn rect(im: &mut Raster, x: u16, y: u16, w: u16, h: u16, c: &str) {
    for yy in y * im.density..(y + h) * im.density {
        for xx in x * im.density..(x + w) * im.density {
            im.pixels.put_pixel(
                xx.into(),
                yy.into(),
                color(im.palette.get(c).map(String::as_str).unwrap_or(c)),
            );
        }
    }
}
fn slice(im: &mut Raster, proof: &Proof, name: &str, r: PixelRect, n: u16) -> Result<()> {
    let d = im.density;
    let id = format!("{d}/{name}");
    let nine = proof.manifest.sprites[&id]
        .nine_slice
        .ok_or_else(|| anyhow::anyhow!("Missing slice metadata"))?;
    anyhow::ensure!(nine.insets.left == n * d, "Skin slice metadata mismatch");
    nine.paint(
        if proof.active_theme == 0 {
            proof.cache.image(&id)?
        } else {
            &proof.themed_assets[&format!("{d}/{name}")]
        },
        &mut im.pixels,
        PixelRect::new(r.x * d, r.y * d, r.width * d, r.height * d),
    )
}

#[allow(clippy::too_many_arguments)]
fn text(s: &mut Surface, x: u16, baseline: u16, w: u16, t: &str, size: u16, c: &str, bold: bool) {
    s.nodes.push(Primitive::Text {
        rect: PixelRect::new(x, baseline - size, w, size + 4),
        text: t.into(),
        color: c.into(),
        size,
        bold,
        mono: true,
    });
}
fn player(proof: &mut Proof, width: u16, state: &State) -> Result<RgbaImage> {
    anyhow::ensure!(
        (720..=3600).contains(&width) && (1..=2).contains(&state.density),
        "Invalid player geometry"
    );
    proof.prepare_theme(state.theme)?;
    let d = state.density;
    let mut im = Raster {
        palette: proof.palette.clone(),
        pixels: RgbaImage::from_pixel(
            u32::from(width * d),
            u32::from(230 * d),
            color(
                proof
                    .palette
                    .get("#171820")
                    .map(String::as_str)
                    .unwrap_or("#171820"),
            ),
        ),
        density: d,
    };
    slice(
        &mut im,
        proof,
        if state.rigid && !state.classic {
            "panel-fold-rigid"
        } else if state.rigid {
            "panel-rigid"
        } else if state.classic {
            "panel"
        } else {
            "panel-fold"
        },
        PixelRect::new(0, 0, width, 230),
        6,
    )?;
    if state.classic {
        rect(&mut im, 7, 7, width - 14, 24, "#242733");
    }
    slice(
        &mut im,
        proof,
        if state.classic { "well" } else { "well-fold" },
        PixelRect::new(14, 43, width - 28, 110),
        3,
    )?;
    let mut s = Surface::new(width, 230, "#171820".into());
    text(&mut s, 18, 24, 250, "S T A R / A M P", 12, "#bbc6d5", true);
    text(
        &mut s,
        width - 225,
        24,
        210,
        "bit-perfect   −  ×",
        12,
        "#989eac",
        false,
    );
    let segments = [
        (3, 0, 17, 3),
        (20, 3, 3, 20),
        (20, 26, 3, 20),
        (3, 46, 17, 3),
        (0, 26, 3, 20),
        (0, 3, 3, 20),
        (3, 23, 17, 3),
    ];
    let mut x = 28.;
    for ch in "1:09".chars() {
        if ch == ':' {
            for y in [15., 33.] {
                rect(
                    &mut im,
                    (x + 3.3) as u16,
                    (58. + y * 1.1) as u16,
                    3,
                    3,
                    "#a4d791",
                );
            }
            x += 14.3;
            continue;
        }
        let mask = match ch {
            '1' => 0b0000110,
            '0' => 0b0111111,
            _ => 0b1101111,
        };
        for (i, (xx, y, w, h)) in segments.iter().enumerate() {
            rect(
                &mut im,
                (x + *xx as f32 * 1.1) as u16,
                58 + (*y as f32 * 1.1) as u16,
                (*w as f32 * 1.1) as u16,
                (*h as f32 * 1.1) as u16,
                if mask & (1 << i) != 0 {
                    "#a4d791"
                } else {
                    "#203226"
                },
            );
        }
        x += 34.1;
    }
    text(
        &mut s,
        28,
        138,
        145,
        if state.playing {
            "PLAY · STEREO"
        } else {
            "PAUSED · STEREO"
        },
        11,
        "#a4d791",
        false,
    );
    for i in 0..56 {
        let count = (3.
            + 12. * (i as f32 * 0.11 + 1.).sin().abs() * (0.7 + 0.3 * (i as f32 * 0.37).cos()))
            as u16;
        if !state.classic {
            let step = (width - 220) as f32 / 56.;
            rect(
                &mut im,
                190 + (i as f32 * step) as u16,
                100 - count * 2,
                (step - 5.).max(1.) as u16,
                count * 2,
                "#a4d791",
            );
            continue;
        }
        for j in 0..16 {
            let step = (width - 220) as f32 / 56.;
            rect(
                &mut im,
                190 + (i as f32 * step) as u16,
                100 - j * 3 - 2,
                (step - 3.) as u16,
                2,
                if j < count { "#a4d791" } else { "#1c2a20" },
            );
        }
    }
    text(
        &mut s,
        190,
        122,
        width - 220,
        if state.unicode {
            "Björk · Jóga — 東京 / Ελληνικά"
        } else {
            "Terra Atlantica — Through the Water and the Waves"
        },
        13,
        "#a4d791",
        false,
    );
    text(
        &mut s,
        190,
        141,
        width - 220,
        "Oceans · FLAC · 1063 kbps · 44.1 kHz · 16-bit",
        11,
        "#989eac",
        false,
    );
    text(&mut s, 16, 173, 60, "01:09", 12, "#989eac", false);
    text(&mut s, width - 52, 173, 50, "04:58", 12, "#989eac", false);
    rect(&mut im, 78, 168, width - 156, 3, "#747986");
    rect(
        &mut im,
        78,
        168,
        ((width - 156) as u32 * u32::from(state.position) / 100) as u16,
        3,
        "#a4d791",
    );
    for i in 0..5 {
        let x = 16 + i * 35;
        slice(
            &mut im,
            proof,
            button_state(state, i as usize, i == 1 && state.playing),
            PixelRect::new(x, 187, 29, 29),
            4,
        )?;
        let ink = if i == 1 { "#10161a" } else { "#e0e2d5" };
        match i {
            2 => {
                rect(&mut im, x + 10, 195, 3, 12, ink);
                rect(&mut im, x + 16, 195, 3, 12, ink);
            }
            3 => rect(&mut im, x + 9, 196, 10, 10, ink),
            _ => {
                for row in 0..12u16 {
                    let n = 6 - row.abs_diff(6);
                    let start = if i == 0 { x + 16 - n } else { x + 9 };
                    rect(&mut im, start, 195 + row, n + 1, 1, ink);
                }
                if i == 0 {
                    rect(&mut im, x + 8, 195, 2, 12, ink);
                }
                if i == 4 {
                    rect(&mut im, x + 19, 195, 2, 12, ink);
                }
            }
        }
    }
    let font = &proof.font;
    let atlas = proof.cache.image("1/control-font")?;
    for (index, (x, label, on)) in [
        (237, "SHUFFLE", state.shuffle),
        (346, "REP ALL", state.repeat),
    ]
    .into_iter()
    .enumerate()
    {
        slice(
            &mut im,
            proof,
            button_state(state, index + 5, on),
            PixelRect::new(x, 187, 100, 29),
            4,
        )?;
        if state.bitmap {
            let mut labelim = RgbaImage::new(font.measure(label)?, 7);
            let lw = labelim.width() as u16;
            font.paint(atlas, &mut labelim, PixelRect::new(0, 0, lw, 7), label, {
                let original = if on { "#10161a" } else { "#e0e2d5" };
                let c = color(
                    proof
                        .palette
                        .get(original)
                        .map(String::as_str)
                        .unwrap_or(original),
                );
                [c[0], c[1], c[2]]
            })?;
            let scaled = image::imageops::resize(
                &labelim,
                lw as u32 * 2 * u32::from(d),
                14 * u32::from(d),
                image::imageops::FilterType::Nearest,
            );
            image::imageops::overlay(
                &mut im.pixels,
                &scaled,
                i64::from((x + (100 - lw * 2) / 2) * d),
                i64::from(195 * d),
            );
        } else {
            text(
                &mut s,
                x + 15,
                207,
                85,
                label,
                14,
                if on { "#10161a" } else { "#e0e2d5" },
                true,
            );
        }
    }
    text(&mut s, width - 220, 207, 35, "VOL", 11, "#989eac", false);
    rect(&mut im, width - 180, 199, 130, 6, "#151920");
    rect(
        &mut im,
        width - 180,
        200,
        130 * state.volume / 100,
        4,
        "#a4d791",
    );
    text(
        &mut s,
        width - 31,
        207,
        25,
        &state.volume.to_string(),
        11,
        "#d7d9cf",
        false,
    );
    s.hits = regions(width);
    for node in &mut s.nodes {
        if let Primitive::Text { color, .. } = node {
            if let Some(mapped) = proof.palette.get(color) {
                *color = mapped.clone();
            }
        }
    }
    if d != 1 {
        s.width *= d;
        s.height *= d;
        for node in &mut s.nodes {
            match node {
                Primitive::Text { rect, size, .. } => {
                    rect.x *= d;
                    rect.y *= d;
                    rect.width *= d;
                    rect.height *= d;
                    *size *= d;
                }
                _ => unreachable!(),
            }
        }
        for hit in &mut s.hits {
            hit.rect.x *= d;
            hit.rect.y *= d;
            hit.rect.width *= d;
            hit.rect.height *= d;
        }
    }
    proof.renderer.render(&im.pixels, &s)
}
fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("STAR/AMP skin proof (isolated sample; no audio)\nUsage: staramp-skin-proof --kitty\n       staramp-skin-proof [output-directory]\nKeys: q quit, Tab focus, Enter activate, arrows seek/volume, Alt+t theme, c chrome, b labels, r corners, d density, u Unicode");
        return Ok(());
    }
    let mut proof = Proof::new()?;
    if std::env::args().any(|a| a == "--kitty") {
        return live(&mut proof);
    }

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/staramp-skin-proof".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;
    for w in [900, 1352, 1800] {
        for bitmap in [false, true] {
            player(
                &mut proof,
                w,
                &State {
                    classic: true,
                    bitmap,
                    ..Default::default()
                },
            )?
            .save(out.join(format!(
                "player-{w}-{}.png",
                if bitmap { "bitmap" } else { "outline" }
            )))?;
        }
    }
    for density in [1, 2] {
        player(
            &mut proof,
            1352,
            &State {
                density,
                ..Default::default()
            },
        )?
        .save(out.join(format!("player-1352-{density}x.png")))?;
    }
    for (name, state) in [
        (
            "hover",
            State {
                hover: Some(1),
                ..Default::default()
            },
        ),
        (
            "pressed",
            State {
                pressed: Some(1),
                ..Default::default()
            },
        ),
        (
            "focus",
            State {
                focus: Some(1),
                ..Default::default()
            },
        ),
        (
            "rigid",
            State {
                rigid: true,
                ..Default::default()
            },
        ),
        (
            "unicode",
            State {
                unicode: true,
                ..Default::default()
            },
        ),
    ] {
        player(&mut proof, 1352, &state)?.save(out.join(format!("player-{name}.png")))?;
    }
    player(
        &mut proof,
        1352,
        &State {
            classic: false,
            ..Default::default()
        },
    )?
    .save(out.join("player-fold-rack.png"))?;
    for (i, theme) in starkit::theme::BUILTINS.iter().enumerate() {
        player(
            &mut proof,
            1352,
            &State {
                theme: i + 1,
                ..Default::default()
            },
        )?
        .save(out.join(format!("theme-{}.png", theme.id)))?;
    }
    let reference = image::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/graphical/skin-proof/approved-player.png"),
    )?
    .to_rgba8();
    let rendered = player(
        &mut proof,
        1352,
        &State {
            classic: true,
            ..Default::default()
        },
    )?;
    let mut overlay = rendered.clone();
    let mut difference = rendered.clone();
    for ((o, d), (a, b)) in overlay
        .pixels_mut()
        .zip(difference.pixels_mut())
        .zip(reference.pixels().zip(rendered.pixels()))
    {
        for c in 0..3 {
            o[c] = ((u16::from(a[c]) + u16::from(b[c])) / 2) as u8;
            d[c] = a[c].abs_diff(b[c]);
        }
        o[3] = 255;
        d[3] = 255;
    }
    overlay.save(out.join("overlay.png"))?;
    difference.save(out.join("difference.png"))?;
    println!("Proof images: {}", out.display());
    Ok(())
}

fn activate(state: &mut State, index: usize) {
    match index {
        1 => state.playing = !state.playing,
        2 | 3 => state.playing = false,
        5 => state.shuffle = !state.shuffle,
        6 => state.repeat = !state.repeat,
        _ => {}
    }
}
fn adjust(state: &mut State, index: usize, x: u16, width: u16) {
    let hits = regions(width);
    let r = hits[index].rect;
    let value = ((u32::from(x.saturating_sub(r.x)) * 100 / u32::from(r.width)).min(100)) as u16;
    match index {
        7 => state.position = value,
        8 => state.volume = value,
        _ => {}
    }
}
fn live(proof: &mut Proof) -> Result<()> {
    use starkit::crossterm::{
        cursor::{Hide, MoveTo, Show},
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
            MouseButton, MouseEventKind,
        },
        execute,
        terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use starkit::terminal_graphics::{protocol::Viewport, renderer::KittyPresenter};
    use std::{
        io::{self, IsTerminal, Write},
        sync::Arc,
        time::Duration,
    };
    anyhow::ensure!(
        io::stdout().is_terminal(),
        "Run --kitty in a Kitty terminal (including through SSH)"
    );
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                DisableMouseCapture,
                Show,
                LeaveAlternateScreen
            );
            let _ = write!(io::stdout(), "\x1b]22;default\x1b\\");
        }
    }
    // Probe before raw input ownership; SSH PTYs often omit pixel dimensions.
    let graphics = starkit::graphics::Graphics::probe(starkit::graphics::Mode::Kitty);
    let cell = graphics.cell_size().unwrap_or((8, 16));
    terminal::enable_raw_mode()?;
    let _restore = Restore;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture, Hide)?;
    let mut trace = std::env::var_os("STAR_SKIN_PROOF_TRACE")
        .map(std::fs::File::create)
        .transpose()?;
    let mut presenter = KittyPresenter::default();
    let mut state = State::default();
    let mut dirty = true;
    let mut theme_notice: Option<std::time::Instant> = None;
    let mut generation = 1;
    let result = (|| -> Result<()> {
        loop {
            if theme_notice.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(2)) {
                theme_notice = None;
                dirty = true;
            }
            let size = terminal::window_size()?;
            let physical_width = if size.width == 0 {
                size.columns.saturating_mul(cell.0)
            } else {
                size.width
            };
            let physical_height = if size.height == 0 {
                size.rows.saturating_mul(cell.1)
            } else {
                size.height
            };
            let width = (physical_width / state.density).clamp(720, 3600);
            if dirty {
                let started = std::time::Instant::now();
                let player = player(proof, width, &state)?;
                let mut frame = RgbaImage::from_pixel(
                    physical_width.into(),
                    physical_height.into(),
                    color(
                        proof
                            .palette
                            .get("#171820")
                            .map(String::as_str)
                            .unwrap_or("#171820"),
                    ),
                );
                image::imageops::overlay(&mut frame, &player, 0, 0);
                if physical_width >= 720 && physical_height > 270 {
                    let mut caption =
                        Surface::new(physical_width, physical_height, "#171820".into());
                    let notice = theme_notice.map(|_| {
                        format!(
                            "Theme: {}",
                            if state.theme == 0 {
                                "reference"
                            } else {
                                starkit::theme::BUILTINS[state.theme - 1].id
                            }
                        )
                    });
                    text(&mut caption,16,250,physical_width-32,notice.as_deref().unwrap_or("SKIN PROOF · sample data, no audio · Tab focus · Enter activate · Alt+t theme · c chrome · b labels · r corners · d density · u Unicode · q quit"),11,"#989eac",false);
                    for node in &mut caption.nodes {
                        if let Primitive::Text { color, .. } = node {
                            if let Some(mapped) = proof.palette.get(color) {
                                *color = mapped.clone();
                            }
                        }
                    }
                    frame = proof.renderer.render(&frame, &caption)?;
                }
                execute!(out, MoveTo(0, 0))?;
                let bytes = presenter.present_pixels(
                    Arc::new(frame),
                    Viewport {
                        columns: size.columns,
                        rows: size.rows,
                        width: physical_width.into(),
                        height: physical_height.into(),
                        generation,
                    },
                    &mut out,
                )?;
                write!(
                    out,
                    "\x1b]22;{}\x1b\\",
                    if state.hover.is_some() {
                        "pointer"
                    } else {
                        "default"
                    }
                )?;
                out.flush()?;
                if let Some(trace) = &mut trace {
                    writeln!(
                        trace,
                        "{}",
                        serde_json::json!({"width":physical_width,"height":physical_height,"columns":size.columns,"rows":size.rows,"density":state.density,"theme":state.theme,"theme_notice":theme_notice.is_some(),"classic":state.classic,"bitmap":state.bitmap,"rigid":state.rigid,"playing":state.playing,"shuffle":state.shuffle,"position":state.position,"volume":state.volume,"hover":state.hover,"focus":state.focus,"bytes":bytes,"render_ms":started.elapsed().as_secs_f64()*1000.})
                    )?;
                    trace.flush()?;
                }
                dirty = false;
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('t')
                        if key
                            .modifiers
                            .contains(starkit::crossterm::event::KeyModifiers::ALT) =>
                    {
                        state.theme = (state.theme + 1) % (starkit::theme::BUILTINS.len() + 1);
                        theme_notice = Some(std::time::Instant::now());
                        dirty = true;
                    }
                    KeyCode::Char('c') => {
                        state.classic = !state.classic;
                        dirty = true;
                    }
                    KeyCode::Char('b') => {
                        state.bitmap = !state.bitmap;
                        dirty = true;
                    }
                    KeyCode::Char('r') => {
                        state.rigid = !state.rigid;
                        dirty = true;
                    }
                    KeyCode::Char('d') => {
                        state.density = 3 - state.density;
                        state.hover = None;
                        state.pressed = None;
                        generation += 1;
                        dirty = true;
                    }
                    KeyCode::Char('u') => {
                        state.unicode = !state.unicode;
                        dirty = true;
                    }
                    KeyCode::Char(' ') => {
                        state.playing = !state.playing;
                        dirty = true;
                    }
                    KeyCode::Tab => {
                        state.focus = Some(state.focus.map_or(0, |i| (i + 1) % 9));
                        dirty = true;
                    }
                    KeyCode::BackTab => {
                        state.focus = Some(state.focus.map_or(8, |i| (i + 8) % 9));
                        dirty = true;
                    }
                    KeyCode::Enter => {
                        if let Some(i) = state.focus {
                            activate(&mut state, i);
                            dirty = true;
                        }
                    }
                    KeyCode::Left | KeyCode::Right => {
                        let value = if state.focus == Some(8) {
                            &mut state.volume
                        } else {
                            &mut state.position
                        };
                        *value = if key.code == KeyCode::Left {
                            value.saturating_sub(2)
                        } else {
                            (*value + 2).min(100)
                        };
                        dirty = true;
                    }
                    _ => {}
                },
                Event::Resize(..) => {
                    state.hover = None;
                    state.pressed = None;
                    generation += 1;
                    dirty = true;
                }
                Event::Mouse(mouse) => {
                    let x = (u32::from(mouse.column) * u32::from(physical_width)
                        / u32::from(size.columns)
                        / u32::from(state.density)) as u16;
                    let y = (u32::from(mouse.row) * u32::from(physical_height)
                        / u32::from(size.rows)
                        / u32::from(state.density)) as u16;
                    let hit = regions(width).iter().position(|h| h.rect.contains(x, y));
                    if state.hover != hit {
                        state.hover = hit;
                        dirty = true;
                    }
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            state.pressed = hit;
                            state.focus = hit;
                            if let Some(i) = hit {
                                adjust(&mut state, i, x, width);
                            }
                            dirty = true;
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if let Some(i) = state.pressed {
                                if i >= 7 {
                                    adjust(&mut state, i, x, width);
                                    dirty = true;
                                }
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if let Some(i) = state.pressed {
                                if Some(i) == hit {
                                    activate(&mut state, i);
                                }
                            }
                            state.pressed = None;
                            dirty = true;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();
    presenter.clear(&mut out)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_builtin_themes_preserve_readable_active_controls() {
        let mut proof = Proof::new().unwrap();
        for i in 1..=starkit::theme::BUILTINS.len() {
            proof.prepare_theme(i).unwrap();
            let ink = starkit::theme::color::Rgb::parse_hex(&proof.palette["#10161a"]).unwrap();
            let background =
                starkit::theme::color::Rgb::parse_hex(&proof.palette["#6b7282"]).unwrap();
            assert!(
                ink.contrast(background) >= 4.5,
                "{} has unreadable active controls",
                starkit::theme::BUILTINS[i - 1].id
            );
        }
    }

    #[test]
    fn control_geometry_is_shared_and_scrub_is_bounded() {
        for width in [720, 900, 1352, 1800] {
            let hits = regions(width);
            assert_eq!(hits.len(), 9);
            for hit in &hits {
                assert!(u32::from(hit.rect.x) + u32::from(hit.rect.width) <= u32::from(width));
            }
            let mut state = State::default();
            adjust(&mut state, 7, 0, width);
            assert_eq!(state.position, 0);
            adjust(&mut state, 7, u16::MAX, width);
            assert_eq!(state.position, 100);
            activate(&mut state, 5);
            assert!(state.shuffle);
            activate(&mut state, 1);
            assert!(!state.playing);
        }
    }
}
