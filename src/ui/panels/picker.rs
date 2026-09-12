//! The playlist picker.
//!
//! Winamp's playlist list, and what `staramp` opens on by default: a curated
//! playlist is almost always a better starting point than thirty thousand
//! tracks in album order.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::panels::player::truncate;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// One entry in the picker.
#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub tracks: usize,
    /// Entries that do not resolve against the index. Shown rather than hidden:
    /// a playlist that is quietly half-broken is worse than one that says so.
    pub missing: usize,
}

pub struct PickerView<'a> {
    pub theme: &'a Theme,
    pub entries: &'a [PlaylistEntry],
    pub cursor: usize,
    pub scroll: usize,
    /// Shown when there is nothing to pick.
    pub empty_hint: &'a str,
}

/// Where the overlay lands, so a click can be tested against it.
pub fn rect(area: Rect, entries: usize) -> Rect {
    let w = area.width.saturating_sub(4).clamp(20, 72);
    let h = area.height.saturating_sub(4).min(entries as u16 + 4).max(6);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// The list area inside the overlay's border.
pub fn list_rect(area: Rect, entries: usize) -> Rect {
    Block::default()
        .borders(Borders::ALL)
        .inner(rect(area, entries))
}

impl<'a> Widget for PickerView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;

        let rect = rect(area, self.entries.len());
        Clear.render(rect, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(t.border_focused)))
            .title(Span::styled(
                " PLAYLISTS ",
                Style::default()
                    .fg(rgb(t.header_fg))
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(
                Line::from(Span::styled(
                    " enter load · esc close ",
                    Style::default().fg(rgb(t.dim)),
                ))
                .right_aligned(),
            )
            .style(Style::default().bg(rgb(t.panel_bg)));

        let inner = block.inner(rect);
        block.render(rect, buf);
        if inner.height == 0 {
            return;
        }

        if self.entries.is_empty() {
            buf.set_string(
                inner.x + 1,
                inner.y,
                truncate(self.empty_hint, inner.width.saturating_sub(2) as usize),
                Style::default().fg(rgb(t.empty_fg)),
            );
            return;
        }

        let height = inner.height as usize;
        for row in 0..height {
            let i = self.scroll + row;
            if i >= self.entries.len() {
                break;
            }
            let e = &self.entries[i];
            let y = inner.y + row as u16;
            let selected = i == self.cursor;

            let style = if selected {
                Style::default()
                    .fg(rgb(t.row_selected_fg))
                    .bg(rgb(t.row_selected_bg))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(rgb(t.row_fg))
            };

            let count = if e.missing > 0 {
                format!("{} tracks, {} missing", e.tracks, e.missing)
            } else {
                format!("{} tracks", e.tracks)
            };
            let avail = inner.width as usize;
            let name_w = avail.saturating_sub(count.len() + 3);
            let line = format!(
                "{}{:<name_w$}  {count}",
                if selected { '>' } else { ' ' },
                truncate(&e.name, name_w),
                name_w = name_w
            );
            buf.set_string(inner.x, y, truncate(&line, avail), style);

            // Colour the "missing" tail separately so a damaged playlist reads
            // at a glance without having to parse the number.
            if e.missing > 0 && !selected && avail > count.len() + 2 {
                buf.set_string(
                    inner.x + (avail - count.len()) as u16,
                    y,
                    &count,
                    Style::default().fg(rgb(t.row_missing_fg)),
                );
            }
        }
    }
}

/// Keep the cursor visible.
pub fn clamp_scroll(cursor: usize, scroll: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if cursor < scroll {
        cursor
    } else if cursor >= scroll + height {
        cursor + 1 - height
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_follows_the_cursor() {
        assert_eq!(clamp_scroll(0, 0, 8), 0);
        assert_eq!(clamp_scroll(9, 0, 8), 2);
        assert_eq!(clamp_scroll(1, 5, 8), 1);
        assert_eq!(clamp_scroll(3, 0, 0), 0);
    }
}
