//! Choosing an album's cover by hand.
//!
//! The ranking picks well most of the time, and the archive matches well most
//! of the time, and "most of the time" across thirty thousand tracks is a lot
//! of wrong covers. This is the escape hatch: everything that was considered,
//! in the order it was considered, with the reason it was or was not taken.
//!
//! Two kinds of thing end up in one list. Images already on disk -- the
//! embedded picture, the files beside the audio, the booklet scans -- and
//! releases the Cover Art Archive offered that were not enough like the tagged
//! title to use unasked. Cinderella's `Monster Ballads` matches their `Best
//! Ballads` closely enough that MusicBrainz calls it relevant, and it is the
//! wrong record; that is the judgement this hands back to the user.

use super::overlay::{self, Anchor, Overlay};
use starkit::chrome::rgb;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::widgets::Widget;

use crate::theme::Theme;
use crate::ui::panels::player::truncate;

/// One line in the chooser.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// What it is: a file name, or a release title with its year and country.
    pub label: String,
    /// Where it came from, or how alike the titles are.
    pub note: String,
    /// Releases have to be fetched; local images are already here.
    pub remote: bool,
}

pub struct ChooserView<'a> {
    pub theme: &'a Theme,
    /// The record being chosen for, so the list has a subject.
    pub album: &'a str,
    pub rows: &'a [Row],
    pub cursor: usize,
    pub scroll: usize,
    /// Where this frame's scrollbar is recorded, so a later press or drag can
    /// find it. See `App::bars`.
    pub bars: &'a mut starkit::chrome::scrollbar::Scrollbars<super::Bar>,
}

/// Where the overlay lands, so a click can be tested against it.
pub fn rect(area: Rect, rows: usize) -> Rect {
    overlay::rect(area, (24, 76), rows as u16 + 4, 7, Anchor::Centre)
}

/// The rows themselves, inside the frame and below the heading.
pub fn list_rect(area: Rect, rows: usize) -> Rect {
    let r = rect(area, rows);
    Rect {
        x: r.x + 1,
        y: r.y + 2,
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(3),
    }
}

/// Which row is at a point, given the same `rows` and `scroll` the overlay was
/// drawn with. The inverse of the loop in `render`, beside
/// [`starkit::chrome::settings::hit`] for the same reason.
pub fn hit(area: Rect, rows: usize, scroll: usize, x: u16, y: u16) -> Option<usize> {
    if rows == 0 {
        return None;
    }
    let list = list_rect(area, rows);
    if x < list.x || x >= list.x + list.width || y < list.y || y >= list.y + list.height {
        return None;
    }
    let index = scroll + usize::from(y - list.y);
    (index < rows).then_some(index)
}

/// Keep the cursor visible. One rule, in starkit, for every list that scrolls.
pub use starkit::list::clamp_scroll;

impl<'a> Widget for ChooserView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let r = rect(area, self.rows.len());
        let inner = overlay::render(
            r,
            buf,
            &Overlay {
                theme: t,
                title: "cover",
                detail: None,
                footer: Some("enter choose \u{b7} esc close"),
            },
        );

        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let w = inner.width as usize;

        // The record being chosen for. Without it a list of release titles is
        // just a list of release titles.
        buf.set_string(
            inner.x,
            inner.y,
            truncate(self.album, w),
            Style::default().fg(rgb(t.row_meta_fg)),
        );

        let list = list_rect(area, self.rows.len());
        if self.rows.is_empty() {
            buf.set_string(
                list.x,
                list.y,
                truncate("nothing else to choose from", w),
                Style::default().fg(rgb(t.empty_fg)),
            );
            return;
        }

        let height = list.height as usize;
        for (i, row) in self.rows.iter().enumerate().skip(self.scroll).take(height) {
            let y = list.y + (i - self.scroll) as u16;
            let selected = i == self.cursor;
            let style = if selected {
                Style::default()
                    .fg(rgb(t.row_selected_fg))
                    .bg(rgb(t.row_selected_bg))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(rgb(t.row_fg))
            };

            // The whole width is painted so the selection is a bar rather than
            // a highlight around the text.
            buf.set_string(list.x, y, " ".repeat(list.width as usize), style);

            // The note is right-aligned, which keeps the labels in a column
            // even when they differ wildly in length.
            let note_w = row.note.chars().count().min(list.width as usize / 2);
            let label_w = (list.width as usize).saturating_sub(note_w + 3);
            buf.set_string(list.x + 1, y, truncate(&row.label, label_w), style);
            if note_w > 0 {
                let note = truncate(&row.note, note_w);
                let x = list.x + list.width - note.chars().count() as u16 - 1;
                let note_style = if selected {
                    style
                } else if row.remote {
                    Style::default().fg(rgb(t.accent))
                } else {
                    Style::default().fg(rgb(t.dim))
                };
                buf.set_string(x, y, note, note_style);
            }
        }

        self.bars.draw(
            super::Bar::Chooser,
            super::scrollbar::track(r, list),
            buf,
            t,
            self.rows.len() as u32,
            self.scroll as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;
    use starkit::ratatui::style::Color;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                label: "cover.jpg".into(),
                note: "folder".into(),
                remote: false,
            },
            Row {
                label: "Best Ballads  1996 US".into(),
                note: "50%".into(),
                remote: true,
            },
        ]
    }

    fn draw(rows: &[Row], cursor: usize, w: u16, h: u16) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        ChooserView {
            theme: &theme,
            album: "Monster Ballads",
            rows,
            cursor,
            scroll: 0,
            bars: &mut starkit::chrome::scrollbar::Scrollbars::new(),
        }
        .render(area, &mut buf);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn both_kinds_of_candidate_are_listed_with_the_album_they_are_for() {
        let all = draw(&rows(), 0, 80, 14).join("\n");
        assert!(all.contains("Monster Ballads"), "{all}");
        assert!(all.contains("cover.jpg"), "{all}");
        assert!(all.contains("Best Ballads"), "{all}");
        assert!(all.contains("folder"), "{all}");
        assert!(all.contains("50%"), "{all}");
    }

    #[test]
    fn the_overlay_stays_inside_the_terminal() {
        for (w, h) in [(40u16, 10u16), (80, 24), (200, 60), (24, 8)] {
            let area = Rect::new(0, 0, w, h);
            let r = rect(area, 20);
            assert!(r.x + r.width <= w, "{w}x{h} overflows across");
            assert!(r.y + r.height <= h, "{w}x{h} overflows down");
            let l = list_rect(area, 20);
            assert!(l.y + l.height <= r.y + r.height, "{w}x{h}: rows escape");
        }
    }

    #[test]
    fn an_empty_list_says_so_rather_than_drawing_a_blank_box() {
        let all = draw(&[], 0, 60, 12).join("\n");
        assert!(all.contains("nothing else to choose"), "{all}");
    }

    #[test]
    fn the_selected_row_is_a_bar_across_the_list() {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 80, 14);
        let mut buf = Buffer::empty(area);
        let rows = rows();
        ChooserView {
            theme: &theme,
            album: "Monster Ballads",
            rows: &rows,
            cursor: 1,
            scroll: 0,
            bars: &mut starkit::chrome::scrollbar::Scrollbars::new(),
        }
        .render(area, &mut buf);

        let list = list_rect(area, rows.len());
        let bg = buf[(list.x, list.y + 1)].style().bg;
        assert_eq!(
            bg,
            Some(Color::Rgb(
                theme.row_selected_bg.r,
                theme.row_selected_bg.g,
                theme.row_selected_bg.b
            )),
            "the second row should be the selected one"
        );
        assert_ne!(
            buf[(list.x, list.y)].style().bg,
            bg,
            "and the first should not be"
        );
    }
}
