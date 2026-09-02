//! Transport buttons that do not go through the terminal's font.
//!
//! A terminal program cannot ship a font. A text face -- a Nerd Font icon, a
//! geometric shape, a letter -- is drawn by whatever typeface the terminal
//! was configured with, at that face's size and to that font's metrics, and
//! two machines set to different fonts draw two different rows of buttons
//! from the same bytes. So the buttons are not text. Each is rasterised here
//! at the terminal's real cell size and put on the screen over the graphics
//! protocol the cover art already uses: exact, and identical on every
//! terminal that speaks kitty, sixel or iTerm2. Where none does, the player
//! draws the ASCII faces in `player::Glyphs`, which every font can manage.
//!
//! The shapes are the Material Design transport icons, the ones the Nerd
//! Font set names by codepoint: `skip-previous`, `play`, `pause`, `stop`,
//! `skip-next`. They are Apache-2.0, from the Pictogrammers collection, and
//! are carried here as the polygons on their 24-unit grid rather than as
//! glyphs, which is what makes drawing them at any size possible.

use crate::theme::color::Rgb;

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
// Rasterising.
// ---------------------------------------------------------------------------

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
        assert_eq!(
            at(Button::Stop, 12.0, 12.0),
            [255, 255, 255],
            "stop's middle"
        );
        assert_eq!(
            at(Button::Pause, 12.0, 12.0),
            [100, 100, 100],
            "pause's gap"
        );
        assert_eq!(
            at(Button::Pause, 8.0, 12.0),
            [255, 255, 255],
            "pause's left bar"
        );
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
        let partial = img.pixels().any(|p| p.0[0] > 100 && p.0[0] < 255);
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
        assert!(
            in_rounded_square(5.0, 0.1, 10.0, 2.0),
            "the top edge's middle"
        );
        assert!(in_rounded_square(5.0, 5.0, 10.0, 2.0), "the centre");
        assert!(
            !in_rounded_square(10.0, 5.0, 10.0, 2.0),
            "just past the right"
        );
    }
}
