//! The spectrum analyzer panel.
//!
//! Two LED rows per terminal row, using the upper-half block: the foreground
//! paints the top LED and the background paints the bottom one, so an eight-row
//! panel renders Winamp's sixteen-row analyzer with sixteen distinct colours.
//! Colouring by *row* rather than by level is what makes it look like Winamp
//! rather than like a bar chart with a gradient.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::theme::resolve::Theme;
use crate::vis::mode::VisMode;

/// How wide the bars are and how much space is between them.
///
/// Configurable because there is no setting that suits every terminal. A gap
/// has to be a whole column: nothing in Unicode is both part-height and
/// part-width, so a bar's tip -- which is a part-height block -- cannot carry
/// a narrower glyph the way its body can. Trying that left the body faintly
/// striped and the tips fused together, which is worse than either choice
/// made honestly. So the gap is a column, and the bar is widened until the gap
/// is a small fraction of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarLayout {
    pub width: u16,
    pub gap: u16,
}

impl Default for BarLayout {
    fn default() -> Self {
        // Three to one: the gap reads as a division rather than as a stripe,
        // and a full-width panel still holds around twenty bars.
        Self { width: 3, gap: 1 }
    }
}

impl BarLayout {
    pub fn sanitised(width: u16, gap: u16) -> Self {
        Self {
            width: width.clamp(1, 16),
            gap: gap.min(8),
        }
    }

    fn pitch(&self) -> u16 {
        self.width + self.gap
    }

    /// How many bars fit across a panel this wide.
    pub fn count(&self, panel: u16) -> usize {
        if panel == 0 {
            return 0;
        }
        (((panel + self.gap) / self.pitch()) as usize).max(1)
    }

    /// The left edge of bar `i`, relative to the panel.
    fn offset(&self, i: usize) -> u16 {
        i as u16 * self.pitch()
    }

    /// Widen or narrow the bars, keeping the gap.
    pub fn resized(&self, by: i16) -> Self {
        Self::sanitised((self.width as i16 + by).max(1) as u16, self.gap)
    }
}

pub fn bar_count(width: u16) -> usize {
    BarLayout::default().count(width)
}

fn rgb(c: crate::theme::color::Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// What a mode needs to draw a frame.
pub struct Frame<'a> {
    /// Smoothed spectrum, one entry per band.
    pub bands: &'a [f32],
    /// Ballistic cap positions, same length as `bands`.
    pub peaks: &'a [f32],
    /// Raw samples, for the trace modes.
    pub wave: &'a [f32],
}

/// Fractional block characters, eighth-height steps.
///
/// Ported from cliamp (MIT, Copyright (c) Bjarne Øverli), along with the
/// `frac_block` shading it feeds. Deciding each cell from the band level
/// against that row's own span is what makes the bars move smoothly instead of
/// jumping a whole character at a time.
const BAR_BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The character for one cell of a bar, given the level and the row's span.
fn frac_block(level: f32, row_bottom: f32, row_top: f32) -> char {
    if level >= row_top {
        return '█';
    }
    if level > row_bottom {
        let frac = (level - row_bottom) / (row_top - row_bottom);
        let idx = (frac * (BAR_BLOCKS.len() - 1) as f32) as usize;
        return BAR_BLOCKS[idx.min(BAR_BLOCKS.len() - 1)];
    }
    ' '
}

/// Ramp colour for a row, by its height up the panel.
///
/// Colouring by *row* rather than by level is what makes it look like Winamp:
/// a given height is always the same colour, so the bars read as an LED ladder.
fn ramp_by_row(theme: &Theme, row_bottom: f32) -> Color {
    let i = (row_bottom.clamp(0.0, 1.0) * 15.0).round() as usize;
    rgb(theme.vis_ramp[i.min(15)])
}

pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    mode: VisMode,
    frame: &Frame,
    bars: BarLayout,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if mode == VisMode::Off {
        fill_bg(area, buf, theme, false);
        return;
    }

    // The dot grid belongs to the LED analyzer; behind a trace it is noise.
    fill_bg(area, buf, theme, mode == VisMode::Leds);

    match mode {
        VisMode::Leds => render_leds(area, buf, theme, frame, bars),
        VisMode::Peaks => render_columns(area, buf, theme, frame, Fill::Frac, true, bars, None),
        VisMode::Bars => render_columns(area, buf, theme, frame, Fill::Frac, false, bars, Some(())),
        VisMode::Dots => render_columns(area, buf, theme, frame, Fill::Braille, false, bars, None),
        VisMode::Wave => render_wave(area, buf, theme, frame),
        VisMode::Scope => render_scope(area, buf, theme, frame),
        VisMode::Cava => render_cava(area, buf, theme, frame, bars),
        VisMode::Off => {}
    }
}

fn fill_bg(area: Rect, buf: &mut Buffer, theme: &Theme, grid: bool) {
    let bg = rgb(theme.vis_bg);
    for y in 0..area.height {
        for x in 0..area.width {
            let dot = grid && x % 2 == 0;
            buf[(area.x + x, area.y + y)]
                .set_char(if dot { '·' } else { ' ' })
                .set_style(Style::default().fg(rgb(theme.vis_grid_fg)).bg(bg));
        }
    }
}

/// How a column is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    /// Fractional blocks: smooth.
    Frac,
    /// Braille stipple.
    Braille,
}

/// Upper half block: two colours in one cell, the top from the foreground and
/// the bottom from the background.
const HALF_UPPER: char = '\u{2580}';

/// Braille dot bits, `[row][col]` in a 4x2 cell.
const BRAILLE_BIT: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

#[allow(clippy::too_many_arguments)]
fn render_columns(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    frame: &Frame,
    fill: Fill,
    caps: bool,
    layout: BarLayout,
    // `Some` for the mode that draws a rung per half-cell rather than sampling
    // the ramp continuously.
    steps: Option<()>,
) {
    if frame.bands.is_empty() {
        return;
    }
    let bg = rgb(theme.vis_bg);
    let bars = layout.count(area.width);
    let height = area.height;
    let dot_rows = height as usize * 4;
    // The bars mode walks a rung per half-cell; the others sample the ramp by
    // height as they always did.
    let rungs = height as usize * 2;
    let colour_at = |v: f32| match steps {
        Some(()) => {
            let rung = ((v.clamp(0.0, 1.0) * rungs as f32) as usize).min(rungs.saturating_sub(1));
            rung_colour(theme, rung, rungs)
        }
        None => ramp_by_row(theme, v),
    };

    for b in 0..bars {
        let x = area.x + layout.offset(b);
        if x >= area.x + area.width {
            break;
        }
        let level = sample(frame.bands, b, bars).clamp(0.0, 1.0);
        let peak = sample(frame.peaks, b, bars).clamp(0.0, 1.0);
        let (cap_row, cap_glyph) = crate::vis::ballistics::cap_position(peak, height);
        let cap_visible = caps && peak > level + 0.01_f32.max(0.5 / dot_rows.max(1) as f32);

        for row in 0..height {
            let row_bottom = (height - 1 - row) as f32 / height as f32;
            let row_top = (height - row) as f32 / height as f32;
            // Each half of the cell is coloured by what is at the middle of
            // it, not at its lower edge. The difference only shows when the
            // bands do not divide evenly into the half-cells -- six bands over
            // a four-row panel is eight halves, so two bands have to be twice
            // the height of the rest. Measuring from the edge put both of them
            // at the bottom, which is where the eye is, and the darkest rung
            // came out visibly taller than every other. Measuring from the
            // middle moves them into the body of the bar where they are a
            // matched pair. Where the geometry does divide evenly the two are
            // the same thing.
            let half = 0.5 / height as f32;
            let lower = row_bottom + half * 0.5;
            let upper = row_bottom + half * 1.5;
            let colour = colour_at(lower);

            let ch = if cap_visible && row == cap_row {
                cap_glyph
            } else {
                match fill {
                    Fill::Frac => frac_block(level, row_bottom, row_top),
                    Fill::Braille => {
                        let mut mask = 0u8;
                        for (dr, bits) in BRAILLE_BIT.iter().enumerate() {
                            let dot_row = row as usize * 4 + dr;
                            let dot_y = (dot_rows - 1 - dot_row) as f32 / dot_rows as f32;
                            if dot_y < level {
                                mask |= bits[0] | bits[1];
                            }
                        }
                        if mask == 0 {
                            ' '
                        } else {
                            char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
                        }
                    }
                }
            };

            if ch == ' ' {
                continue;
            }
            let is_cap = cap_visible && row == cap_row;

            // A row of the panel is one cell, and one cell is one colour --
            // which on the three-row analyzer the main window uses meant three
            // steps of a sixteen-step ramp. A full cell is drawn as an upper
            // half block instead, whose foreground and background are two
            // different colours, so each row carries two: six steps on three
            // rows. The tip of a bar is a part-height block and keeps its
            // single colour, which is where the smoothness comes from.
            let two_tone = !is_cap && ch == BAR_BLOCKS[8];
            let (ch, fg, cell_bg) = if two_tone {
                (HALF_UPPER, colour_at(upper), colour_at(lower))
            } else if is_cap {
                (ch, rgb(theme.vis_peak_fg), bg)
            } else {
                (ch, colour, bg)
            };

            for dx in 0..layout.width {
                if x + dx >= area.x + area.width {
                    break;
                }
                buf[(x + dx, area.y + row)]
                    .set_char(ch)
                    .set_style(Style::default().fg(fg).bg(cell_bg));
            }
        }
    }
}

/// Oscilloscope. Braille gives four vertical dots per cell, so the trace is far
/// smoother than block characters manage.
fn render_wave(area: Rect, buf: &mut Buffer, theme: &Theme, frame: &Frame) {
    let wave = frame.wave;
    if wave.is_empty() {
        return;
    }
    let bg = rgb(theme.vis_bg);
    let w = area.width as usize;
    let dot_rows = area.height as usize * 4;
    let mut cells = vec![0u8; w * area.height as usize];

    for cx in 0..w * 2 {
        let lo = cx * wave.len() / (w * 2);
        let hi = ((cx + 1) * wave.len() / (w * 2))
            .max(lo + 1)
            .min(wave.len());
        // Take the extreme in each slice so a fast transient is not missed.
        let v = wave[lo..hi]
            .iter()
            .copied()
            .fold(0.0f32, |m, s| if s.abs() > m.abs() { s } else { m });

        let y = ((1.0 - v.clamp(-1.0, 1.0)) * 0.5 * (dot_rows - 1) as f32) as usize;
        let (col, row) = (cx / 2, y / 4);
        if col < w && row < area.height as usize {
            cells[row * w + col] |= BRAILLE_BIT[y % 4][cx % 2];
        }
    }

    for (i, mask) in cells.iter().enumerate() {
        if *mask == 0 {
            continue;
        }
        let (x, y) = ((i % w) as u16, (i / w) as u16);
        buf[(area.x + x, area.y + y)]
            .set_char(char::from_u32(0x2800 + *mask as u32).unwrap_or(' '))
            .set_style(Style::default().fg(rgb(theme.vis_osc[0])).bg(bg));
    }
}

/// The waveform as scattered dots, shaded by distance from the centre.
fn render_scope(area: Rect, buf: &mut Buffer, theme: &Theme, frame: &Frame) {
    let wave = frame.wave;
    if wave.is_empty() || area.width == 0 {
        return;
    }
    let bg = rgb(theme.vis_bg);
    let rows = area.height as f32;

    for x in 0..area.width {
        let lo = x as usize * wave.len() / area.width as usize;
        let hi = ((x as usize + 1) * wave.len() / area.width as usize)
            .max(lo + 1)
            .min(wave.len());
        let v = wave[lo..hi]
            .iter()
            .copied()
            .fold(0.0f32, |m, s| if s.abs() > m.abs() { s } else { m });

        let row = ((1.0 - v.clamp(-1.0, 1.0)) * 0.5 * (rows - 1.0)).round() as u16;
        if row >= area.height {
            continue;
        }
        let dist = (v.abs() * (theme.vis_osc.len() - 1) as f32).round() as usize;
        buf[(area.x + x, area.y + row)].set_char('•').set_style(
            Style::default()
                .fg(rgb(theme.vis_osc[dist.min(theme.vis_osc.len() - 1)]))
                .bg(bg),
        );
    }
}

fn render_leds(area: Rect, buf: &mut Buffer, theme: &Theme, frame: &Frame, layout: BarLayout) {
    let (bands, peaks) = (frame.bands, frame.peaks);
    let bg = rgb(theme.vis_bg);
    let led_rows = (area.height * 2) as usize;

    if bands.is_empty() {
        return;
    }
    let bars = layout.count(area.width);

    for b in 0..bars {
        let x = area.x + layout.offset(b);
        if x >= area.x + area.width {
            break;
        }
        // Bands are resampled to the bar count so the panel can be any width.
        let level = sample(bands, b, bars);
        let peak = sample(peaks, b, bars);

        let lit = (level * led_rows as f32).round() as usize;
        let peak_row = (peak * led_rows as f32).round() as usize;

        for row in 0..led_rows {
            // Row 0 is the bottom LED.
            let y = area.y + area.height - 1 - (row / 2) as u16;
            let is_top_half = row % 2 == 1;

            let colour = if row < lit {
                // By row, not by level: LED n is always the same colour, which
                // is exactly what VISCOLOR.TXT describes.
                let idx = (row * 16 / led_rows.max(1)).min(15);
                Some(rgb(theme.vis_ramp[idx]))
            } else if peak_row > 0 && row == peak_row.saturating_sub(1) {
                Some(rgb(theme.vis_peak_fg))
            } else {
                None
            };

            let Some(colour) = colour else { continue };

            for dx in 0..layout.width {
                if x + dx >= area.x + area.width {
                    break;
                }
                let cell = &mut buf[(x + dx, y)];
                if is_top_half {
                    // Upper half block: fg is the top LED, bg keeps whatever the
                    // bottom LED already painted.
                    let existing_bg = cell.style().bg.unwrap_or(bg);
                    cell.set_char('▀')
                        .set_style(Style::default().fg(colour).bg(existing_bg));
                } else {
                    let existing_fg = if cell.symbol() == "▀" {
                        cell.style().fg.unwrap_or(bg)
                    } else {
                        bg
                    };
                    cell.set_char('▀')
                        .set_style(Style::default().fg(existing_fg).bg(colour));
                }
            }
        }
    }
}

/// Colour zones stacked up the panel.
///
/// A handful of flat bands rather than a continuous blend. Over the three rows
/// the main window gives the analyzer a smooth gradient is barely a gradient
/// at all, whereas banding gives the eye a fixed line to read a level against
/// -- which is what the Winamp analyzer and cava's own gradient both rely on.
const COLOUR_BANDS: usize = 4;

/// How much of the theme's ramp the bars mode uses, as a fraction of it.
///
/// Nearly all of it, measured in *lightness* rather than in stops. Walking the
/// stops evenly is what made the bottom rung jump: cosmic's ramp climbs 0.13
/// of Oklab lightness between its first two stops and 0.20 between its third
/// and fourth, then spends eleven stops moving 0.01 at a time. Position along
/// the ramp is not the same thing as how different two colours look, and only
/// the second one is what a ladder should be even in.
const BAR_RAMP: (f32, f32) = (0.0, 1.0);

/// The colour of one rung, `rung` of `rungs` counting from the bottom.
///
/// One rung per half-cell, so the ladder is as fine as the panel can draw and
/// no rung is taller than its neighbours -- six bands across eight half-cells
/// could only ever be shared out unevenly, and the unevenness was visible.
///
/// Interpolated rather than snapped to the ramp's sixteen stops, because a
/// tall panel asks for more rungs than the ramp has stops and rounding them
/// would put the seams back.
fn rung_colour(theme: &Theme, rung: usize, rungs: usize) -> Color {
    let t = if rungs <= 1 {
        0.5
    } else {
        rung as f32 / (rungs - 1) as f32
    };
    let (lo, hi) = BAR_RAMP;
    ramp_at(theme, lo + t * (hi - lo))
}

/// The theme's spectrum ramp sampled `t` of the way along it *by lightness*.
///
/// Follows the ramp's own path -- so a theme that goes green, yellow, red
/// still does -- but paces itself by how far the eye has actually travelled
/// rather than by how many stops have gone by. An evenly spaced ladder over an
/// unevenly spaced ramp is what put a jolt at the bottom of every bar and a
/// flat wash through the middle.
fn ramp_at(theme: &Theme, t: f32) -> Color {
    let ramp = &theme.vis_ramp;
    let last = ramp.len() - 1;

    // How far apart consecutive stops are, in Oklab lightness, and the running
    // total. Absolute, so a ramp that dips somewhere still advances.
    let mut marks = [0.0f64; 16];
    for i in 1..=last {
        let step = (ramp[i].to_oklab().l - ramp[i - 1].to_oklab().l).abs();
        marks[i] = marks[i - 1] + step;
    }
    let total = marks[last];
    if total <= f64::EPSILON {
        // A ramp with no lightness in it at all: fall back to its stops.
        let pos = t.clamp(0.0, 1.0) * last as f32;
        let i = (pos.floor() as usize).min(last);
        return rgb(ramp[i].mix(ramp[(i + 1).min(last)], (pos - i as f32) as f64));
    }

    let want = t.clamp(0.0, 1.0) as f64 * total;
    let i = (1..=last).find(|&i| marks[i] >= want).unwrap_or(last);
    let span = marks[i] - marks[i - 1];
    let into = if span <= f64::EPSILON {
        0.0
    } else {
        (want - marks[i - 1]) / span
    };
    rgb(ramp[i - 1].mix(ramp[i], into))
}

/// The colour for a row, quantised into `bands` zones of the theme's spectrum
/// ramp. A Winamp skin's ramp gives the classic green to red.
///
/// The zones are spread across the whole ramp rather than taken from its foot,
/// so however few there are the top of a full bar is still the ramp's last
/// colour.
fn banded_colour(theme: &Theme, row_bottom: f32, bands: usize) -> Color {
    let last = theme.vis_ramp.len() - 1;
    let f = row_bottom.clamp(0.0, 1.0);
    let band = ((f * bands as f32) as usize).min(bands - 1);
    rgb(theme.vis_ramp[(band * last / (bands - 1).max(1)).min(last)])
}

/// Bars the cava mode draws in a panel this wide.
///
/// Shared with the analysis so it produces exactly the bars the renderer is
/// going to draw rather than a count that has to be resampled.
pub fn cava_bar_count(width: u16) -> usize {
    BarLayout::default().count(width)
}

/// cava's look: narrow bars, eighth-block resolution.
///
/// Deliberately not `render_columns`. That draws two-cell bars and decides
/// each cell from the band level, which is the Winamp analyzer's chunky
/// ladder. This is as many bars as the panel can hold at one cell apiece, so
/// the spectrum reads as a curve. The eighth blocks give eight steps per row,
/// so the three-row panel carries twenty-four levels rather than three.
fn render_cava(area: Rect, buf: &mut Buffer, theme: &Theme, frame: &Frame, layout: BarLayout) {
    if frame.bands.is_empty() || area.height == 0 {
        return;
    }
    let bg = rgb(theme.vis_bg);
    let bars = layout.count(area.width);
    let rows = area.height;
    let full = rows as f32 * 8.0;

    for b in 0..bars {
        let x = area.x + layout.offset(b);
        if x >= area.x + area.width {
            break;
        }
        let level = sample(frame.bands, b, bars).clamp(0.0, 1.0);
        let eighths = (level * full).round() as i32;
        if eighths <= 0 {
            continue;
        }
        for row in 0..rows {
            let from_bottom = (rows - 1 - row) as i32;
            // How much of this cell is filled, in eighths.
            let cell = (eighths - from_bottom * 8).clamp(0, 8) as usize;
            if cell == 0 {
                continue;
            }
            let colour = banded_colour(theme, from_bottom as f32 / rows as f32, COLOUR_BANDS);
            for dx in 0..layout.width {
                if x + dx >= area.x + area.width {
                    break;
                }
                buf[(x + dx, area.y + row)]
                    .set_char(BAR_BLOCKS[cell])
                    .set_style(Style::default().fg(colour).bg(bg));
            }
        }
    }
}

/// Nearest-neighbour resample of `src` to position `i` of `n`.
fn sample(src: &[f32], i: usize, n: usize) -> f32 {
    if src.is_empty() || n == 0 {
        return 0.0;
    }
    let idx = (i * src.len()) / n;
    src[idx.min(src.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_count_scales_with_width_and_never_reaches_zero() {
        assert_eq!(bar_count(0), 0);
        assert!(bar_count(1) >= 1);
        assert!(bar_count(80) > bar_count(20));
    }

    #[test]
    fn resampling_covers_the_source_range() {
        let src = [0.0, 0.25, 0.5, 0.75, 1.0];
        assert_eq!(sample(&src, 0, 5), 0.0);
        assert_eq!(sample(&src, 4, 5), 1.0);
        // Fewer bars than bands still spans the spectrum.
        assert_eq!(sample(&src, 0, 2), 0.0);
        assert_eq!(sample(&src, 1, 2), 0.5);
        // More bars than bands does not index out of bounds.
        for i in 0..20 {
            let _ = sample(&src, i, 20);
        }
    }

    #[test]
    fn an_empty_source_is_silent_rather_than_panicking() {
        assert_eq!(sample(&[], 3, 10), 0.0);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::theme::builtin;
    use ratatui::layout::Rect;

    /// Render a mode into a buffer and return it as plain text, so the shapes
    /// can be asserted on and eyeballed without a terminal.
    fn draw(mode: VisMode, w: u16, h: u16) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);

        // A rising spectrum, so the shape is obvious.
        let n = 24;
        let bands: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                (0.15 + 0.8 * (t * std::f32::consts::PI).sin()).clamp(0.0, 1.0)
            })
            .collect();
        let peaks: Vec<f32> = bands.iter().map(|b| (b + 0.12).min(1.0)).collect();
        let wave: Vec<f32> = (0..256)
            .map(|i| (i as f32 / 256.0 * std::f32::consts::TAU * 3.0).sin() * 0.8)
            .collect();
        render(
            area,
            &mut buf,
            &theme,
            mode,
            &Frame {
                bands: &bands,
                peaks: &peaks,
                wave: &wave,
            },
            BarLayout::default(),
        );

        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn every_mode_draws_something() {
        for m in VisMode::all() {
            let rows = draw(*m, 48, 6);
            let ink: usize = rows
                .iter()
                .map(|r| {
                    r.chars()
                        .filter(|c| !c.is_whitespace() && *c != '·')
                        .count()
                })
                .sum();
            if *m == VisMode::Off {
                assert_eq!(ink, 0, "off must draw nothing");
            } else {
                assert!(
                    ink > 10,
                    "{} drew almost nothing:\n{}",
                    m.name(),
                    rows.join("\n")
                );
            }
        }
    }

    #[test]
    fn a_bar_shows_two_colours_for_every_row_it_fills() {
        // What the half block buys, on a mode that takes the whole ramp. One
        // colour to a cell meant a full-height bar on the four-row analyzer
        // showed four steps of a sixteen-step ramp; two to a cell makes it
        // eight.
        let theme = builtin::load("cosmic").unwrap();
        for rows in [3u16, 4, 6] {
            let area = Rect::new(0, 0, 12, rows);
            let mut buf = Buffer::empty(area);
            let bands = vec![1.0f32; BarLayout::default().count(area.width)];
            render(
                area,
                &mut buf,
                &theme,
                VisMode::Peaks,
                &Frame {
                    bands: &bands,
                    peaks: &bands,
                    wave: &[],
                },
                BarLayout::default(),
            );
            let mut seen = std::collections::BTreeSet::new();
            for y in 0..rows {
                let c = &buf[(0, y)];
                seen.insert(format!("{:?}", c.style().fg));
                seen.insert(format!("{:?}", c.style().bg));
            }
            assert_eq!(
                seen.len(),
                rows as usize * 2,
                "{rows} rows gave {} colours, not {}",
                seen.len(),
                rows * 2
            );
        }
    }

    #[test]
    fn bars_gives_a_rung_to_every_half_cell_it_has() {
        // One rung per half-cell: as fine a ladder as the panel can draw, and
        // every rung the same height as its neighbours. A fixed count could
        // not manage that -- six rungs across the eight half-cells of a
        // four-row panel had to leave two of them double the height, and the
        // unevenness showed.
        let theme = builtin::load("cosmic").unwrap();
        for rows in [3u16, 6, 16] {
            let area = Rect::new(0, 0, 12, rows);
            let mut buf = Buffer::empty(area);
            let bands = vec![1.0f32; BarLayout::default().count(area.width)];
            render(
                area,
                &mut buf,
                &theme,
                VisMode::Bars,
                &Frame {
                    bands: &bands,
                    peaks: &bands,
                    wave: &[],
                },
                BarLayout::default(),
            );
            let mut seen = std::collections::BTreeSet::new();
            for y in 0..rows {
                let c = &buf[(0, y)];
                seen.insert(format!("{:?}", c.style().fg));
                seen.insert(format!("{:?}", c.style().bg));
            }
            assert_eq!(
                seen.len(),
                rows as usize * 2,
                "{rows} rows should give a rung per half-cell"
            );
        }
    }

    /// The height of each colour rung, in half-cells, from the bottom up.
    fn rungs(rows: u16) -> Vec<usize> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 12, rows);
        let mut buf = Buffer::empty(area);
        let bands = vec![1.0f32; BarLayout::default().count(area.width)];
        render(
            area,
            &mut buf,
            &theme,
            VisMode::Bars,
            &Frame {
                bands: &bands,
                peaks: &bands,
                wave: &[],
            },
            BarLayout::default(),
        );
        // Each row is an upper half block: the background is its lower half,
        // the foreground its upper.
        let mut halves = Vec::new();
        for y in (0..rows).rev() {
            let c = &buf[(0, y)];
            halves.push(format!("{:?}", c.style().bg));
            halves.push(format!("{:?}", c.style().fg));
        }
        let mut out: Vec<(String, usize)> = Vec::new();
        for h in halves {
            match out.last_mut() {
                Some((v, n)) if *v == h => *n += 1,
                _ => out.push((h, 1)),
            }
        }
        out.into_iter().map(|(_, n)| n).collect()
    }

    #[test]
    fn no_rung_is_much_taller_than_another_and_the_bottom_is_not_the_fat_one() {
        // Six bands do not divide into the eight half-cells of a four-row
        // panel, so two of them have to be twice the height of the rest.
        // Colouring each half by its lower edge put both at the bottom, and
        // the darkest rung came out a full row tall against neighbours half
        // that -- a visibly fat base. Measuring from the middle of each half
        // spreads them into the body of the bar instead.
        for rows in [3u16, 4, 6, 8, 16] {
            let r = rungs(rows);
            assert_eq!(r.len(), rows as usize * 2, "{rows} rows: {r:?}");
            let (lo, hi) = (r.iter().min().unwrap(), r.iter().max().unwrap());
            assert!(hi - lo <= 1, "{rows} rows: uneven rungs {r:?}");
            assert!(
                r[1..].iter().any(|n| n >= &r[0]),
                "{rows} rows: the bottom rung is the tallest on its own {r:?}"
            );
        }
    }

    #[test]
    fn bars_grow_from_the_bottom() {
        let rows = draw(VisMode::Bars, 48, 6);
        let ink = |r: &String| r.chars().filter(|c| !c.is_whitespace()).count();
        assert!(
            ink(&rows[5]) >= ink(&rows[0]),
            "bottom row should be at least as full as the top:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn peaks_mode_puts_caps_above_the_bars() {
        let rows = draw(VisMode::Peaks, 48, 6);
        let joined = rows.join("");
        assert!(
            crate::vis::ballistics::CAP_GLYPHS
                .iter()
                .any(|g| joined.contains(*g)),
            "no cap glyph drawn:\n{}",
            rows.join("\n")
        );
    }

    /// Render one flat level across the panel and hand back column 0's cells.
    fn cava_column(level: f32, height: u16) -> Vec<(String, Color)> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 12, height);
        let mut buf = Buffer::empty(area);
        let layout = BarLayout::default();
        let bands = vec![level; layout.count(area.width)];
        render(
            area,
            &mut buf,
            &theme,
            VisMode::Cava,
            &Frame {
                bands: &bands,
                peaks: &bands,
                wave: &[],
            },
            layout,
        );
        (0..height)
            .map(|y| {
                let c = &buf[(0, y)];
                (c.symbol().to_string(), c.style().fg.unwrap())
            })
            .collect()
    }

    #[test]
    fn cava_resolves_below_a_whole_row() {
        // The eighth blocks: a four-row panel carries 32 levels, so a level
        // between rows draws a partial block rather than rounding to a row.
        let cells = cava_column(0.55, 4);
        assert!(
            cells
                .iter()
                .any(|(ch, _)| BAR_BLOCKS[1..8].iter().any(|b| ch.starts_with(*b))),
            "no partial block: {:?}",
            cells.iter().map(|c| &c.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_quiet_band_still_draws() {
        // Too small to fill a row must not mean invisible.
        assert!(cava_column(0.05, 3).iter().any(|(ch, _)| ch != " "));
    }

    #[test]
    fn cava_colours_in_bands_rather_than_a_gradient() {
        let mut colours: Vec<Color> = cava_column(1.0, 16).iter().map(|(_, c)| *c).collect();
        colours.dedup();
        assert_eq!(
            colours.len(),
            COLOUR_BANDS,
            "expected flat zones: {colours:?}"
        );
    }

    #[test]
    fn the_colour_bands_span_the_whole_theme_ramp() {
        let theme = builtin::load("cosmic").unwrap();
        let last = theme.vis_ramp.len() - 1;
        assert_eq!(
            banded_colour(&theme, 0.0, COLOUR_BANDS),
            rgb(theme.vis_ramp[0])
        );
        assert_eq!(
            banded_colour(&theme, 0.99, COLOUR_BANDS),
            rgb(theme.vis_ramp[last])
        );
    }

    /// The Oklab lightness of a rendered colour.
    fn lightness(c: Color) -> f64 {
        match c {
            Color::Rgb(r, g, b) => crate::theme::color::Rgb::new(r, g, b).to_oklab().l,
            _ => 0.0,
        }
    }

    #[test]
    fn the_ladder_climbs_the_whole_ramp() {
        // Both ends, so the mode has all the contrast the theme offers.
        let theme = builtin::load("cosmic").unwrap();
        let last = theme.vis_ramp.len() - 1;
        for n in [6usize, 8, 12, 32] {
            assert_eq!(rung_colour(&theme, 0, n), rgb(theme.vis_ramp[0]), "{n}");
            assert_eq!(
                rung_colour(&theme, n - 1, n),
                rgb(theme.vis_ramp[last]),
                "{n}"
            );
        }
    }

    #[test]
    fn every_step_up_the_ladder_is_the_same_size_to_the_eye() {
        // The bug this exists for: the rungs were spaced evenly along the
        // ramp, and the ramp is not evenly spaced. Cosmic climbs 0.13 of
        // lightness between its first two stops and 0.20 between its third and
        // fourth, then eleven stops move 0.01 apiece -- so the bottom of every
        // bar jumped and the middle was a flat wash.
        let theme = builtin::load("cosmic").unwrap();
        for n in [6usize, 8, 12, 32] {
            let steps: Vec<f64> = (1..n)
                .map(|i| {
                    lightness(rung_colour(&theme, i, n)) - lightness(rung_colour(&theme, i - 1, n))
                })
                .collect();
            let want = steps.iter().sum::<f64>() / steps.len() as f64;
            for (i, step) in steps.iter().enumerate() {
                assert!(
                    (step - want).abs() < 0.01,
                    "{n} rungs: step {} is {step:.3}, the rest average {want:.3}",
                    i + 1
                );
                assert!(*step > 0.0, "{n} rungs: step {} does not climb", i + 1);
            }
        }
    }

    #[test]
    fn a_theme_whose_ramp_has_no_lightness_in_it_still_draws() {
        // The pacing divides by the ramp's total lightness travel. A flat one
        // would divide by zero.
        let mut theme = builtin::load("cosmic").unwrap();
        theme.vis_ramp = [crate::theme::color::Rgb::new(80, 80, 80); 16];
        for n in [1usize, 6, 32] {
            for i in 0..n {
                assert_eq!(rung_colour(&theme, i, n), Color::Rgb(80, 80, 80));
            }
        }
    }

    #[test]
    fn every_bar_mode_leaves_a_gap_at_every_height() {
        // Why the gap is a whole column: a bar's tip is a part-height block,
        // and nothing in Unicode is both part-height and part-width, so a
        // sub-cell separator disappeared exactly along the top edge -- the
        // part of the spectrum you look at.
        let layout = BarLayout::default();
        for mode in [VisMode::Bars, VisMode::Peaks, VisMode::Cava, VisMode::Leds] {
            let theme = builtin::load("cosmic").unwrap();
            let area = Rect::new(0, 0, 40, 5);
            let mut buf = Buffer::empty(area);
            let n = layout.count(area.width);
            // Every band a different height, so tips land on different rows.
            let bands: Vec<f32> = (0..n).map(|i| 0.15 + 0.8 * (i % 5) as f32 / 5.0).collect();
            render(
                area,
                &mut buf,
                &theme,
                mode,
                &Frame {
                    bands: &bands,
                    peaks: &bands,
                    wave: &[],
                },
                layout,
            );
            for b in 1..n {
                let x = area.x + layout.offset(b) - 1;
                for y in 0..area.height {
                    let sym = buf[(x, y)].symbol();
                    assert!(
                        sym == " " || sym == "\u{b7}",
                        "{} left ink in the gap at column {x}, row {y}: {sym:?}",
                        mode.name()
                    );
                }
            }
        }
    }

    #[test]
    fn resizing_bars_stops_at_the_ends_rather_than_wrapping() {
        let mut w = BarLayout::sanitised(3, 1);
        for _ in 0..40 {
            w = w.resized(1);
        }
        assert!(w.width <= 16, "widened past the limit: {w:?}");
        let mut n = BarLayout::sanitised(3, 1);
        for _ in 0..40 {
            n = n.resized(-1);
        }
        assert_eq!(n.width, 1, "narrowed to nothing: {n:?}");
        // The gap is not touched by resizing the bar.
        assert_eq!(n.gap, 1);
    }

    #[test]
    fn the_bar_layout_is_configurable_and_bounded() {
        assert_eq!(BarLayout::sanitised(3, 1), BarLayout { width: 3, gap: 1 });
        // A zero width would divide by zero when placing bars.
        assert_eq!(BarLayout::sanitised(0, 0).width, 1);
        assert!(BarLayout::sanitised(999, 999).width <= 16);
        // No gap is a legitimate choice: a solid spectrum.
        assert_eq!(BarLayout::sanitised(2, 0).gap, 0);
        // Wider bars mean fewer of them.
        assert!(BarLayout::sanitised(2, 1).count(80) > BarLayout::sanitised(4, 1).count(80));
    }

    #[test]
    fn a_tiny_panel_does_not_panic() {
        for m in VisMode::all() {
            for (w, h) in [(1, 1), (2, 1), (1, 6), (48, 1)] {
                let _ = draw(*m, w, h);
            }
        }
    }

    /// Not an assertion -- run with `--nocapture` to look at them.
    #[test]
    fn preview_every_mode() {
        for m in VisMode::all() {
            println!("--- {} ---", m.name());
            for line in draw(*m, 56, 6) {
                println!("{line}");
            }
        }
    }
}
