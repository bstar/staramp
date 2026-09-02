//! Transport button faces that do not depend on the terminal's font.
//!
//! A terminal program cannot ship a font. Every text face -- the block set,
//! the Nerd Font icons, the geometric shapes -- is drawn by whatever typeface
//! the terminal was configured with, at that face's size and to that font's
//! metrics, and two machines set to different fonts draw two different rows
//! of buttons from the same bytes. The two faces here take the font out of
//! it:
//!
//! - [`Kind::Pixel`] draws the shapes from quadrant block elements, two
//!   pixels to a cell each way. Every monospace font carries those, and the
//!   modern terminals draw them themselves without consulting the font at
//!   all, so the result is the same everywhere. It is chunky, because a
//!   three-row plate is six pixels tall.
//! - [`Kind::Image`] rasterises the same shapes at the terminal's real cell
//!   size and puts them there over the graphics protocol the cover art
//!   already uses. It is exact, and it needs a terminal that speaks kitty,
//!   sixel or iTerm2 -- elsewhere it falls back to the block text set.
//!
//! The shapes are the Material Design transport icons, the ones the Nerd
//! Font set names by codepoint: `skip-previous`, `play`, `pause`, `stop`,
//! `skip-next`. They are Apache-2.0, from the Pictogrammers collection, and
//! are carried here as the polygons on their 24-unit grid rather than as
//! glyphs, which is what makes drawing them at any size possible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::theme::color::Rgb;

/// How a set's faces reach the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Characters, drawn by the terminal's font.
    Text,
    /// Quadrant block elements, two pixels to a cell each way.
    Pixel,
    /// A rasterised image per button, over the graphics protocol.
    Image,
}

/// One of the five transport buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
}

impl Button {
    pub const ALL: [Self; 5] = [Self::Prev, Self::Play, Self::Pause, Self::Stop, Self::Next];
}

// ---------------------------------------------------------------------------
// The shapes, on the Material Design 24-unit grid.
//
// `M8,5.14V19.14L19,12.14L8,5.14Z` and its siblings, unwound into vertices.
// Axis-aligned rectangles are given as polygons too, so there is one fill
// routine rather than two.
// ---------------------------------------------------------------------------

type Poly = &'static [(f32, f32)];

const PLAY: &[Poly] = &[&[(8.0, 5.14), (8.0, 19.14), (19.0, 12.14)]];
const PAUSE: &[Poly] = &[
    &[(6.0, 5.0), (10.0, 5.0), (10.0, 19.0), (6.0, 19.0)],
    &[(14.0, 5.0), (18.0, 5.0), (18.0, 19.0), (14.0, 19.0)],
];
const STOP: &[Poly] = &[&[(6.0, 6.0), (18.0, 6.0), (18.0, 18.0), (6.0, 18.0)]];
const NEXT: &[Poly] = &[
    &[(6.0, 6.0), (14.5, 12.0), (6.0, 18.0)],
    &[(16.0, 6.0), (18.0, 6.0), (18.0, 18.0), (16.0, 18.0)],
];
const PREV: &[Poly] = &[
    &[(6.0, 6.0), (8.0, 6.0), (8.0, 18.0), (6.0, 18.0)],
    &[(18.0, 6.0), (18.0, 18.0), (9.5, 12.0)],
];

/// The icon's grid is this many units across.
const GRID: f32 = 24.0;

fn polygons(b: Button) -> &'static [Poly] {
    match b {
        Button::Prev => PREV,
        Button::Play => PLAY,
        Button::Pause => PAUSE,
        Button::Stop => STOP,
        Button::Next => NEXT,
    }
}

/// Even-odd point-in-polygon.
fn inside(poly: Poly, x: f32, y: f32) -> bool {
    let mut hit = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            hit = !hit;
        }
        j = i;
    }
    hit
}

// ---------------------------------------------------------------------------
// Pixel faces.
// ---------------------------------------------------------------------------

/// The plate, in cells. Six by three: at a cell of about 8 by 17 pixels that
/// is 48 by 51, square to within 6%, and it gives the sprite a 12-by-6 grid
/// to be drawn on -- the coarsest at which a triangle still has a slope.
pub const PIXEL_W: u16 = 6;
pub const PIXEL_H: u16 = 3;

const SPRITE_W: usize = PIXEL_W as usize * 2;
const SPRITE_H: usize = PIXEL_H as usize * 2;

/// The faces at pixel resolution, drawn by hand rather than sampled from the
/// polygons: at twelve by six a sampled triangle comes out ragged, and a hand
/// can keep the slope even and the pair of skips exact mirrors.
///
/// A pixel is about 4 by 8.5 screen pixels -- half a cell wide, half a cell
/// tall -- so shapes are drawn twice as wide in pixels as they are tall to
/// come out square. `.` is plate, `#` is ink.
type Sprite = [&'static str; SPRITE_H];

const SPRITE_PLAY: Sprite = [
    "..##........",
    "..#####.....",
    "..########..",
    "..########..",
    "..#####.....",
    "..##........",
];
const SPRITE_PAUSE: Sprite = [
    "...##..##...",
    "...##..##...",
    "...##..##...",
    "...##..##...",
    "...##..##...",
    "...##..##...",
];
const SPRITE_STOP: Sprite = [
    "............",
    "..########..",
    "..########..",
    "..########..",
    "..########..",
    "............",
];
const SPRITE_NEXT: Sprite = [
    ".##......##.",
    ".#####...##.",
    ".#######.##.",
    ".#######.##.",
    ".#####...##.",
    ".##......##.",
];
const SPRITE_PREV: Sprite = [
    ".##......##.",
    ".##...#####.",
    ".##.#######.",
    ".##.#######.",
    ".##...#####.",
    ".##......##.",
];

pub fn sprite(b: Button) -> &'static Sprite {
    match b {
        Button::Prev => &SPRITE_PREV,
        Button::Play => &SPRITE_PLAY,
        Button::Pause => &SPRITE_PAUSE,
        Button::Stop => &SPRITE_STOP,
        Button::Next => &SPRITE_NEXT,
    }
}

/// The quadrant block for a 2-by-2 pattern, indexed by bits
/// `TL << 3 | TR << 2 | BL << 1 | BR`.
///
/// Sixteen patterns, sixteen characters: the block elements carry every
/// combination, which is what makes two colours to a cell enough.
const QUADRANTS: [&str; 16] = [
    " ",        // ....
    "\u{2597}", // ...BR  ▗
    "\u{2596}", // ..BL.  ▖
    "\u{2584}", // ..BLBR ▄
    "\u{259d}", // .TR..  ▝
    "\u{2590}", // .TR.BR ▐
    "\u{259e}", // .TRBL. ▞
    "\u{259f}", // .TRBLBR ▟
    "\u{2598}", // TL...  ▘
    "\u{259a}", // TL..BR ▚
    "\u{258c}", // TL.BL. ▌
    "\u{2599}", // TL.BLBR ▙
    "\u{2580}", // TLTR.. ▀
    "\u{259c}", // TLTR.BR ▜
    "\u{259b}", // TLTRBL. ▛
    "\u{2588}", // TLTRBLBR █
];

fn ink(s: &Sprite, x: usize, y: usize) -> bool {
    s.get(y)
        .and_then(|row| row.as_bytes().get(x))
        .is_some_and(|&b| b == b'#')
}

/// Draw a pixel button: the whole rect is plate, and the sprite is ink on it.
///
/// Two colours to a cell -- the ink in the foreground, the plate behind --
/// and a quadrant character to say which of the cell's four pixels are which.
/// The plate fills every cell, so no cell ever needs a third colour for the
/// panel behind the plate, which is the constraint the design is built
/// around.
pub fn render_pixel(r: Rect, s: &Sprite, fg: Rgb, plate: Rgb, buf: &mut Buffer) {
    let style = Style::default().fg(color(fg)).bg(color(plate));
    for cy in 0..r.height {
        for cx in 0..r.width {
            let (x, y) = (cx as usize * 2, cy as usize * 2);
            let bits = (ink(s, x, y) as usize) << 3
                | (ink(s, x + 1, y) as usize) << 2
                | (ink(s, x, y + 1) as usize) << 1
                | (ink(s, x + 1, y + 1) as usize);
            buf[(r.x + cx, r.y + cy)]
                .set_symbol(QUADRANTS[bits])
                .set_style(style);
        }
    }
}

fn color(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

// ---------------------------------------------------------------------------
// Image faces.
// ---------------------------------------------------------------------------

/// The plate, in cells, when the face is an image. Four by three, the same as
/// the block text set, so switching between the two moves nothing.
pub const IMAGE_W: u16 = 4;

/// Supersamples per pixel edge. Sixteen samples a pixel is enough that a
/// slanted edge shows no steps at cell sizes up to a few dozen pixels.
const SS: u32 = 4;

/// Rasterise one button: panel behind, a square plate on it, the icon on that.
///
/// `w` and `h` are the button's cells in pixels, as the terminal reported
/// them, so the image lands one-to-one with no scaling on its way to the
/// screen. The plate is the largest square that fits, centred; the icon's
/// 24-unit grid is mapped onto the plate, which gives it the margins the
/// Material Design icons are drawn with.
///
/// Opaque, deliberately: what the graphics protocol shows behind a
/// transparent pixel is the cell's background, which is whatever style the
/// placeholder cell happened to keep, and painting the panel colour in
/// ourselves is the only way to be sure of it.
pub fn raster(b: Button, w: u32, h: u32, fg: Rgb, plate: Rgb, bg: Rgb) -> image::RgbImage {
    let side = w.min(h) as f32;
    let ox = (w as f32 - side) / 2.0;
    let oy = (h as f32 - side) / 2.0;
    let radius = side * 0.12;
    let scale = side / GRID;
    let polys = polygons(b);

    let mut img = image::RgbImage::new(w.max(1), h.max(1));
    for (px, py, p) in img.enumerate_pixels_mut() {
        let mut plate_hits = 0u32;
        let mut ink_hits = 0u32;
        for sy in 0..SS {
            for sx in 0..SS {
                let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                if !in_rounded_square(x - ox, y - oy, side, radius) {
                    continue;
                }
                plate_hits += 1;
                let (ux, uy) = ((x - ox) / scale, (y - oy) / scale);
                if polys.iter().any(|poly| inside(poly, ux, uy)) {
                    ink_hits += 1;
                }
            }
        }
        let total = (SS * SS) as f32;
        let c = mix(bg, plate, plate_hits as f32 / total);
        let c = mix(c, fg, ink_hits as f32 / total);
        *p = image::Rgb([c.r, c.g, c.b]);
    }
    img
}

/// Is `(x, y)` inside the `side`-by-`side` square at the origin whose corners
/// are rounded to `radius`?
fn in_rounded_square(x: f32, y: f32, side: f32, radius: f32) -> bool {
    if x < 0.0 || y < 0.0 || x >= side || y >= side {
        return false;
    }
    // Distance from the nearest corner's centre of curvature, when in a
    // corner's square at all.
    let dx = (radius - x).max(x - (side - radius)).max(0.0);
    let dy = (radius - y).max(y - (side - radius)).max(0.0);
    dx * dx + dy * dy <= radius * radius
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb {
        r: f(a.r, b.r),
        g: f(a.g, b.g),
        b: f(a.b, b.b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };
    const PLATE: Rgb = Rgb {
        r: 100,
        g: 100,
        b: 100,
    };
    const BG: Rgb = Rgb { r: 0, g: 0, b: 0 };

    #[test]
    fn every_sprite_fills_its_grid_with_ink_or_plate() {
        for b in Button::ALL {
            let s = sprite(b);
            assert_eq!(s.len(), SPRITE_H, "{b:?} rows");
            for row in s.iter() {
                assert_eq!(row.len(), SPRITE_W, "{b:?} row {row:?}");
                assert!(
                    row.bytes().all(|c| c == b'#' || c == b'.'),
                    "{b:?} row {row:?} has something other than ink or plate"
                );
            }
        }
    }

    #[test]
    fn the_skips_are_mirror_images() {
        for (n, p) in SPRITE_NEXT.iter().zip(SPRITE_PREV.iter()) {
            let mirrored: String = n.chars().rev().collect();
            assert_eq!(&mirrored, p);
        }
    }

    #[test]
    fn play_is_symmetric_top_to_bottom() {
        for i in 0..SPRITE_H / 2 {
            assert_eq!(SPRITE_PLAY[i], SPRITE_PLAY[SPRITE_H - 1 - i], "row {i}");
        }
    }

    /// Ink centred on the plate, so a face never sits to one side of its
    /// button -- the thing the text sets had to be redesigned for.
    #[test]
    fn every_sprite_is_centred_across_the_plate() {
        for b in Button::ALL {
            let s = sprite(b);
            let mut first = SPRITE_W;
            let mut last = 0;
            for row in s.iter() {
                if let Some(a) = row.find('#') {
                    first = first.min(a);
                }
                if let Some(z) = row.rfind('#') {
                    last = last.max(z);
                }
            }
            assert_eq!(first, SPRITE_W - 1 - last, "{b:?} spans {first}..={last}");
        }
    }

    #[test]
    fn the_quadrant_table_is_a_bijection() {
        let mut seen: Vec<&str> = QUADRANTS.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 16);
        // And the bits mean what the comments say they mean.
        assert_eq!(QUADRANTS[0b1000], "\u{2598}", "top left");
        assert_eq!(QUADRANTS[0b0100], "\u{259d}", "top right");
        assert_eq!(QUADRANTS[0b0010], "\u{2596}", "bottom left");
        assert_eq!(QUADRANTS[0b0001], "\u{2597}", "bottom right");
        assert_eq!(QUADRANTS[0b1100], "\u{2580}", "upper half");
        assert_eq!(QUADRANTS[0b1010], "\u{258c}", "left half");
    }

    #[test]
    fn a_pixel_button_is_plate_everywhere_and_ink_where_the_sprite_says() {
        let r = Rect::new(2, 1, PIXEL_W, PIXEL_H);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 6));
        render_pixel(r, sprite(Button::Stop), INK, PLATE, &mut buf);
        for cy in 0..PIXEL_H {
            for cx in 0..PIXEL_W {
                let cell = &buf[(r.x + cx, r.y + cy)];
                assert_eq!(cell.bg, color(PLATE), "plate behind every cell");
            }
        }
        // The stop square is rows 1..=4 of 0..=5, columns 2..=9 of 0..=11:
        // the left column of cells is all plate, the cell at (1, 1) is all
        // ink, and the cell at (1, 0) has ink in its lower half only.
        assert_eq!(buf[(r.x, r.y)].symbol(), " ");
        assert_eq!(buf[(r.x, r.y + 1)].symbol(), " ");
        assert_eq!(buf[(r.x + 1, r.y + 1)].symbol(), "\u{2588}");
        assert_eq!(buf[(r.x + 1, r.y)].symbol(), "\u{2584}");
        // Nothing outside the rect was touched.
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
    }

    #[test]
    fn a_raster_is_the_size_it_was_asked_for_and_the_panel_at_its_corners() {
        let img = raster(Button::Play, 32, 51, INK, PLATE, BG);
        assert_eq!((img.width(), img.height()), (32, 51));
        for (x, y) in [(0, 0), (31, 0), (0, 50), (31, 50)] {
            assert_eq!(img.get_pixel(x, y).0, [0, 0, 0], "corner {x},{y}");
        }
        // The plate is square and vertically centred: rows above and below
        // it are panel, its own rows are plate at the middle column.
        assert_eq!(img.get_pixel(16, 2).0, [0, 0, 0], "above the plate");
        assert_eq!(img.get_pixel(16, 48).0, [0, 0, 0], "below the plate");
        assert_eq!(img.get_pixel(1, 25).0, [100, 100, 100], "plate's left edge");
    }

    #[test]
    fn the_icons_are_where_the_material_grid_puts_them() {
        // A square cell grid, so the mapping is easy to reason about: 48 px
        // a side, 2 px per unit.
        let at = |b: Button, ux: f32, uy: f32| {
            let img = raster(b, 48, 48, INK, PLATE, BG);
            img.get_pixel((ux * 2.0) as u32, (uy * 2.0) as u32).0
        };
        assert_eq!(at(Button::Stop, 12.0, 12.0), [255, 255, 255], "stop's middle");
        assert_eq!(at(Button::Pause, 12.0, 12.0), [100, 100, 100], "pause's gap");
        assert_eq!(at(Button::Pause, 8.0, 12.0), [255, 255, 255], "pause's left bar");
        assert_eq!(at(Button::Play, 10.0, 12.0), [255, 255, 255], "inside play");
        assert_eq!(at(Button::Play, 18.0, 6.0), [100, 100, 100], "outside play");
        assert_eq!(at(Button::Next, 17.0, 12.0), [255, 255, 255], "next's bar");
        assert_eq!(at(Button::Prev, 7.0, 12.0), [255, 255, 255], "prev's bar");
    }

    #[test]
    fn slanted_edges_are_blended_rather_than_stepped() {
        let img = raster(Button::Play, 48, 48, INK, PLATE, BG);
        // Somewhere along the triangle's hypotenuse a pixel is part ink and
        // part plate.
        let partial = img
            .pixels()
            .any(|p| p.0[0] > 100 && p.0[0] < 255);
        assert!(partial, "no anti-aliased pixel anywhere");
    }

    #[test]
    fn the_polygon_test_agrees_with_geometry() {
        let square: Poly = &[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        assert!(inside(square, 1.0, 1.0));
        assert!(!inside(square, 3.0, 1.0));
        let tri: Poly = &[(0.0, 0.0), (0.0, 2.0), (2.0, 1.0)];
        assert!(inside(tri, 0.5, 1.0));
        assert!(!inside(tri, 1.5, 0.2));
    }

    #[test]
    fn rounded_corners_are_cut_and_edges_are_kept() {
        assert!(!in_rounded_square(0.1, 0.1, 10.0, 2.0), "the corner");
        assert!(in_rounded_square(5.0, 0.1, 10.0, 2.0), "the top edge's middle");
        assert!(in_rounded_square(5.0, 5.0, 10.0, 2.0), "the centre");
        assert!(!in_rounded_square(10.0, 5.0, 10.0, 2.0), "just past the right");
    }
}
