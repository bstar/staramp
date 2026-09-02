//! The equalizer window.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::audio::dsp::eq::{BAND_LABELS, MAX_GAIN_DB};
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

pub struct EqView<'a> {
    pub theme: &'a Theme,
    pub gains: [f32; 10],
    pub preamp: f32,
    pub enabled: bool,
    pub preset: &'a str,
    pub focused_band: usize,
    pub focused: bool,
}

/// The panel's interactive geometry.
///
/// Same contract as the player panel's: one pure function that the renderer
/// places things with and the mouse handler hit-tests against.
pub struct Geometry {
    pub inner: Rect,
    /// The `[ON ]` / `[OFF]` switch.
    pub toggle: Rect,
    pub preset_prev: Rect,
    pub preset_next: Rect,
    /// The rows the sliders occupy, all ten bands.
    pub sliders: Rect,
    /// Cells per band column.
    pub col_w: u16,
    /// Rows from the top of `sliders` to the zero line.
    pub mid: u16,
}

/// Rows the panel occupies: border, header row, and six of body -- the status
/// line, four slider rows and the band labels.
pub const PANEL_ROWS: u16 = super::header::ROWS + 8;

/// `None` when the panel is too small to draw its controls.
pub fn geometry(area: Rect, preset: &str) -> Option<Geometry> {
    let inner = super::header::body(area);
    if inner.height < 3 || inner.width < 40 {
        return None;
    }
    let slider_rows = inner.height.saturating_sub(2);
    let name = preset.chars().count() as u16;
    Some(Geometry {
        inner,
        toggle: Rect::new(inner.x + 1, inner.y, 5, 1),
        preset_prev: Rect::new(inner.x + 7, inner.y, 1, 1),
        // `‹ name ›`: the closing chevron sits one past the name.
        preset_next: Rect::new(inner.x + 7 + 2 + name + 1, inner.y, 1, 1),
        sliders: Rect::new(inner.x, inner.y + 1, inner.width, slider_rows),
        col_w: (inner.width / 10).max(3),
        mid: slider_rows / 2,
    })
}

impl Geometry {
    /// Which band column a given x falls in.
    ///
    /// The whole column is the target, not just the one cell the slider is
    /// drawn on -- a one-cell hit box is unusable with a mouse.
    pub fn band_at(&self, x: u16) -> Option<usize> {
        if x < self.inner.x {
            return None;
        }
        let b = ((x - self.inner.x) / self.col_w) as usize;
        (b < BAND_LABELS.len()).then_some(b)
    }

    /// The gain a click at row `y` means, inverting the renderer's mapping.
    pub fn gain_at(&self, y: u16) -> f32 {
        if self.mid == 0 {
            return 0.0;
        }
        let r = y.saturating_sub(self.sliders.y) as i32;
        let rel = self.mid as i32 - r;
        (rel as f32 / self.mid as f32).clamp(-1.0, 1.0) * MAX_GAIN_DB
    }
}

impl<'a> Widget for EqView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(if self.focused {
                t.border_focused
            } else {
                t.border
            })))
            .title(Span::styled(
                "\u{2550} EQUALIZER ",
                Style::default().fg(rgb(t.header_fg)),
            ))
            .style(Style::default().bg(rgb(t.bg)));

        let inner = super::header::body(area);
        block.render(area, buf);
        // Over the border the block just drew: the corners give the panel its
        // colour, and the header carries what it can be asked to do.
        super::frame::render_corners(area, buf, t, self.focused);
        super::header::render(area, super::header::PLAIN, buf, t);
        if inner.height == 0 || inner.width < 40 {
            return;
        }
        let g = geometry(area, self.preset);

        // Status line: on/off and the current preset.
        buf.set_string(
            inner.x + 1,
            inner.y,
            if self.enabled { "[ON ]" } else { "[OFF]" },
            Style::default()
                .fg(rgb(if self.enabled {
                    t.eq_enabled_fg
                } else {
                    t.eq_disabled_fg
                }))
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            inner.x + 7,
            inner.y,
            format!("‹ {} ›", self.preset),
            Style::default().fg(rgb(t.eq_band_value)),
        );
        let pre = format!("PREAMP {:+.0}", self.preamp);
        if inner.width > pre.len() as u16 + 10 {
            buf.set_string(
                inner.x + inner.width - pre.len() as u16 - 1,
                inner.y,
                &pre,
                Style::default().fg(rgb(t.eq_preamp_fg)),
            );
        }

        if inner.height < 3 {
            return;
        }

        // Sliders. Each band gets a column; the zero line is drawn across.
        let Some(g) = g else { return };
        let slider_rows = g.sliders.height;
        let col_w = g.col_w;
        let mid = g.mid;

        for (b, label) in BAND_LABELS.iter().enumerate() {
            let x = inner.x + b as u16 * col_w + col_w / 2;
            if x >= inner.x + inner.width {
                break;
            }
            let gain = self.gains[b].clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
            let extent = ((gain / MAX_GAIN_DB) * mid as f32).round() as i32;

            for r in 0..slider_rows {
                let y = inner.y + 1 + r;
                let rel = mid as i32 - r as i32; // positive above the middle
                let (ch, colour) = if rel == 0 {
                    ('═', t.eq_zero_line)
                } else if extent > 0 && rel > 0 && rel <= extent {
                    ('█', t.eq_slider_fill_pos)
                } else if extent < 0 && rel < 0 && rel >= extent {
                    ('█', t.eq_slider_fill_neg)
                } else {
                    ('│', t.eq_slider_track)
                };
                buf[(x, y)]
                    .set_char(ch)
                    .set_style(Style::default().fg(rgb(colour)).bg(rgb(t.bg)));
            }

            // Band label, with the focused one highlighted.
            let lx = x.saturating_sub(label.len() as u16 / 2);
            let style = if b == self.focused_band && self.focused {
                Style::default()
                    .fg(rgb(t.eq_band_focused))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(rgb(t.eq_band_label))
            };
            buf.set_string(lx, inner.y + inner.height - 1, label, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;

    fn draw(width: u16, height: u16, gains: [f32; 10]) -> (Buffer, Geometry) {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        EqView {
            theme: &theme,
            gains,
            preamp: 0.0,
            enabled: true,
            preset: "flat",
            focused_band: 0,
            focused: true,
        }
        .render(area, &mut buf);
        (buf, geometry(area, "flat").unwrap())
    }

    fn at(buf: &Buffer, r: Rect) -> String {
        (0..r.width.max(1))
            .map(|i| buf[(r.x + i, r.y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn the_hit_rects_sit_on_the_controls_the_renderer_draws() {
        let (buf, g) = draw(60, PANEL_ROWS, [0.0; 10]);
        assert_eq!(at(&buf, g.toggle), "[ON ]");
        assert_eq!(at(&buf, g.preset_prev), "‹");
        assert_eq!(at(&buf, g.preset_next), "›");
    }

    #[test]
    fn a_click_maps_back_to_the_gain_it_looks_like() {
        let (_, g) = draw(60, PANEL_ROWS, [0.0; 10]);
        // The zero line, the top of a column, and the bottom.
        assert_eq!(g.gain_at(g.sliders.y + g.mid), 0.0);
        assert_eq!(g.gain_at(g.sliders.y), MAX_GAIN_DB);
        assert_eq!(g.gain_at(g.sliders.y + g.mid * 2), -MAX_GAIN_DB);
    }

    #[test]
    fn setting_a_band_from_a_click_draws_it_back_at_that_height() {
        // The round trip that matters: click a row, and the bar the renderer
        // draws reaches exactly that row.
        let (_, g) = draw(60, PANEL_ROWS, [0.0; 10]);
        let target = g.sliders.y + 1;
        let mut gains = [0.0f32; 10];
        gains[3] = g.gain_at(target);
        let (buf, g) = draw(60, PANEL_ROWS, gains);
        let x = g.inner.x + 3 * g.col_w + g.col_w / 2;
        assert_eq!(
            buf[(x, target)].symbol(),
            "█",
            "the click row is not filled"
        );
    }

    #[test]
    fn every_column_is_reachable_and_none_overlap() {
        let (_, g) = draw(60, PANEL_ROWS, [0.0; 10]);
        let mut seen = vec![];
        for x in g.inner.x..g.inner.x + g.inner.width {
            if let Some(b) = g.band_at(x) {
                seen.push(b);
            }
        }
        seen.dedup();
        assert_eq!(
            seen,
            (0..10).collect::<Vec<_>>(),
            "columns are out of order"
        );
    }
}
