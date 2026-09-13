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
//!
//! The scan conversion itself is `starkit::graphics::raster`, which knows
//! nothing about transport buttons; what is left here is the five shapes and
//! where they sit on the plate.

use starkit::graphics::raster;

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

// ---------------------------------------------------------------------------
// Rasterising.
// ---------------------------------------------------------------------------

fn rgba(c: Rgb) -> starkit::image::Rgba<u8> {
    starkit::image::Rgba([c.r, c.g, c.b, 255])
}

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
pub fn raster(
    b: Button,
    w: u32,
    h: u32,
    fg: Rgb,
    plate: Rgb,
    bg: Rgb,
) -> starkit::image::RgbaImage {
    let side = w.min(h) as f32;
    let ox = (w as f32 - side) / 2.0;
    let oy = (h as f32 - side) / 2.0;
    let radius = side * 0.12;
    let scale = side / GRID;
    let on_plate = move |x: f32, y: f32| raster::in_rounded_square(x - ox, y - oy, side, radius);

    let mut img = starkit::image::RgbaImage::from_pixel(w.max(1), h.max(1), rgba(bg));
    raster::fill(&mut img, rgba(plate), on_plate);
    // Masked by the plate rather than merely drawn after it: the icon is
    // fitted to the plate's grid, so anything of it that fell outside would be
    // a rounding error rather than part of the shape.
    raster::fill_polygons(&mut img, rgba(fg), polygons(b), |x, y| {
        on_plate(x, y).then(|| ((x - ox) / scale, (y - oy) / scale))
    });
    img
}

/// The play triangle alone, sized to a cell: the playlist's playing-row
/// marker, so the row says the same thing the play button says and looks
/// like it.
///
/// No plate. The triangle is fitted to the cell's width less a pixel each
/// side, and to its height the same way when that is the tighter of the two,
/// then centred, and it is `fg` on `bg` -- the row's own colours, read from
/// the cell the text marker was drawn in, so a cursor bar over the playing
/// row carries through.
pub fn mark(w: u32, h: u32, fg: Rgb, bg: Rgb) -> starkit::image::RgbaImage {
    // The triangle's box on the 24-unit grid.
    let (x0, y0, x1, y1) = (8.0f32, 5.14f32, 19.0f32, 19.14f32);
    let (uw, uh) = (x1 - x0, y1 - y0);
    let (wf, hf) = (w as f32, h as f32);
    let mut scale = (wf - 2.0).max(1.0) / uw;
    if uh * scale > hf - 2.0 {
        scale = (hf - 2.0).max(1.0) / uh;
    }
    let ox = (wf - uw * scale) / 2.0 - x0 * scale;
    let oy = (hf - uh * scale) / 2.0 - y0 * scale;

    let mut img = starkit::image::RgbaImage::from_pixel(w.max(1), h.max(1), rgba(bg));
    raster::fill_polygons(&mut img, rgba(fg), PLAY, |x, y| {
        Some(((x - ox) / scale, (y - oy) / scale))
    });
    img
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
            assert_eq!(img.get_pixel(x, y).0, [0, 0, 0, 255], "corner {x},{y}");
        }
        // The plate is square and vertically centred: rows above and below
        // it are panel, its own rows are plate at the middle column.
        assert_eq!(img.get_pixel(16, 2).0, [0, 0, 0, 255], "above the plate");
        assert_eq!(img.get_pixel(16, 48).0, [0, 0, 0, 255], "below the plate");
        assert_eq!(
            img.get_pixel(1, 25).0,
            [100, 100, 100, 255],
            "plate's left edge"
        );
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
            [255, 255, 255, 255],
            "stop's middle"
        );
        assert_eq!(
            at(Button::Pause, 12.0, 12.0),
            [100, 100, 100, 255],
            "pause's gap"
        );
        assert_eq!(
            at(Button::Pause, 8.0, 12.0),
            [255, 255, 255, 255],
            "pause's left bar"
        );
        assert_eq!(
            at(Button::Play, 10.0, 12.0),
            [255, 255, 255, 255],
            "inside play"
        );
        assert_eq!(
            at(Button::Play, 18.0, 6.0),
            [100, 100, 100, 255],
            "outside play"
        );
        assert_eq!(
            at(Button::Next, 17.0, 12.0),
            [255, 255, 255, 255],
            "next's bar"
        );
        assert_eq!(
            at(Button::Prev, 7.0, 12.0),
            [255, 255, 255, 255],
            "prev's bar"
        );
    }

    #[test]
    fn the_marker_is_a_triangle_filling_the_cell_with_a_pixel_to_spare() {
        let img = mark(8, 17, INK, BG);
        assert_eq!((img.width(), img.height()), (8, 17));
        // A pixel of margin each side, so the tip's column and the base's
        // column are inside the cell, and the corners are background.
        for (x, y) in [(0, 0), (7, 0), (0, 16), (7, 16)] {
            assert_eq!(img.get_pixel(x, y).0, [0, 0, 0, 255], "corner {x},{y}");
        }
        assert_eq!(img.get_pixel(0, 8).0, [0, 0, 0, 255], "the left margin");
        assert_eq!(
            img.get_pixel(2, 8).0,
            [255, 255, 255, 255],
            "inside the base"
        );
        assert_eq!(img.get_pixel(6, 2).0, [0, 0, 0, 255], "above the tip");
        // Centred vertically: the same amount of background above and below.
        let column = |x: u32| {
            (0..17)
                .filter(|&y| img.get_pixel(x, y).0[0] > 0)
                .collect::<Vec<_>>()
        };
        let base = column(1);
        assert_eq!(
            base.first().copied(),
            Some(16 - base.last().copied().unwrap())
        );
    }

    #[test]
    fn a_short_wide_cell_fits_the_marker_to_its_height() {
        let img = mark(20, 6, INK, BG);
        assert_eq!(
            img.get_pixel(10, 0).0,
            [0, 0, 0, 255],
            "the top margin holds"
        );
        assert_eq!(
            img.get_pixel(10, 5).0,
            [0, 0, 0, 255],
            "the bottom margin holds"
        );
        assert!(img.pixels().any(|p| p.0[0] == 255), "some ink");
    }
}
