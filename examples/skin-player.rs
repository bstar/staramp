//! Offline visual proof. Does not change the installed AMP interface.
use anyhow::Result;
use starkit::{
    image::{self, Rgba, RgbaImage},
    native_surface::{
        skin::{BitmapFont, Insets, NineSlice, Repeat},
        PixelRect, Primitive, Surface,
    },
    terminal_graphics::renderer::render_surface_overlay,
};
use std::path::Path;
fn color(s: &str) -> Rgba<u8> {
    Rgba([
        u8::from_str_radix(&s[1..3], 16).unwrap(),
        u8::from_str_radix(&s[3..5], 16).unwrap(),
        u8::from_str_radix(&s[5..7], 16).unwrap(),
        255,
    ])
}
fn rect(im: &mut RgbaImage, x: u16, y: u16, w: u16, h: u16, c: &str) {
    for yy in y..y + h {
        for xx in x..x + w {
            im.put_pixel(xx.into(), yy.into(), color(c));
        }
    }
}
fn slice(im: &mut RgbaImage, root: &Path, name: &str, r: PixelRect, n: u16) -> Result<()> {
    let source = image::open(root.join(format!("1x/{name}.png")))?.to_rgba8();
    NineSlice {
        insets: Insets {
            left: n,
            top: n,
            right: n,
            bottom: n,
        },
        horizontal: Repeat::Tile,
        vertical: Repeat::Tile,
    }
    .paint(&source, im, r)
}
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
fn player(width: u16, bitmap: bool) -> Result<RgbaImage> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/skins/classic");
    let mut im = RgbaImage::from_pixel(width.into(), 230, color("#171820"));
    slice(&mut im, &root, "panel", PixelRect::new(0, 0, width, 230), 6)?;
    rect(&mut im, 7, 7, width - 14, 24, "#242733");
    slice(
        &mut im,
        &root,
        "well",
        PixelRect::new(14, 43, width - 28, 110),
        3,
    )?;
    let mut s = Surface::new(width, 230, "#171820".into());
    text(&mut s, 16, 24, 250, "S T A R / A M P", 12, "#d7d9cf", true);
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
    text(&mut s, 28, 138, 145, "PLAY · STEREO", 11, "#a4d791", false);
    for i in 0..56 {
        let count = (3.
            + 12. * (i as f32 * 0.11 + 1.).sin().abs() * (0.7 + 0.3 * (i as f32 * 0.37).cos()))
            as u16;
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
        "Terra Atlantica — Through the Water and the Waves",
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
        ((width - 156) as f32 * 0.23) as u16,
        3,
        "#a4d791",
    );
    for i in 0..5 {
        let x = 16 + i * 35;
        slice(
            &mut im,
            &root,
            if i == 1 {
                "button-active"
            } else {
                "button-normal"
            },
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
    let font: BitmapFont = serde_json::from_slice(&std::fs::read(root.join("control-font.json"))?)?;
    let atlas = image::open(root.join("1x/control-font.png"))?.to_rgba8();
    for (x, label, on) in [(237, "SHUFFLE", false), (346, "REP ALL", true)] {
        slice(
            &mut im,
            &root,
            if on { "button-active" } else { "button-normal" },
            PixelRect::new(x, 187, 100, 29),
            4,
        )?;
        if bitmap {
            let mut labelim = RgbaImage::new(font.measure(label)?, 7);
            let lw = labelim.width() as u16;
            font.paint(
                &atlas,
                &mut labelim,
                PixelRect::new(0, 0, lw, 7),
                label,
                if on { [16, 22, 26] } else { [224, 226, 213] },
            )?;
            let scaled = image::imageops::resize(
                &labelim,
                lw as u32 * 2,
                14,
                image::imageops::FilterType::Nearest,
            );
            image::imageops::overlay(&mut im, &scaled, i64::from(x + (100 - lw * 2) / 2), 195);
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
    rect(&mut im, width - 180, 200, 104, 4, "#a4d791");
    text(&mut s, width - 31, 207, 25, "80", 11, "#d7d9cf", false);
    render_surface_overlay(&im, &s)
}
fn main() -> Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/staramp-skin-proof".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out)?;
    for w in [900, 1352, 1800] {
        for bitmap in [false, true] {
            player(w, bitmap)?.save(out.join(format!(
                "player-{w}-{}.png",
                if bitmap { "bitmap" } else { "outline" }
            )))?;
        }
    }
    let reference = image::open(out.join("approved-player.png"))?.to_rgba8();
    let rendered = player(1352, false)?;
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
