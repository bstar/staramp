//! The playlist picker.
//!
//! Winamp's playlist list, and what `staramp` opens on by default: a curated
//! playlist is almost always a better starting point than thirty thousand
//! tracks in album order.

use super::overlay::{self, Anchor, Overlay};
use starkit::chrome::rgb;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::widgets::Widget;

use crate::theme::Theme;
use crate::ui::panels::player::truncate;

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
    overlay::rect(area, (20, 72), entries as u16 + 4, 6, Anchor::Centre)
}

/// The list area inside the overlay's border.
pub fn list_rect(area: Rect, entries: usize) -> Rect {
    overlay::inner(rect(area, entries))
}

/// Which entry is at a point, given the same `entries` and `scroll` the
/// overlay was drawn with. Beside [`starkit::chrome::settings::hit`] for the
/// same reason.
pub fn hit(area: Rect, entries: usize, scroll: usize, x: u16, y: u16) -> Option<usize> {
    if entries == 0 {
        return None;
    }
    let list = list_rect(area, entries);
    if x < list.x || x >= list.x + list.width || y < list.y || y >= list.y + list.height {
        return None;
    }
    let index = scroll + usize::from(y - list.y);
    (index < entries).then_some(index)
}

impl<'a> Widget for PickerView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;

        let rect = rect(area, self.entries.len());
        let inner = overlay::render(
            rect,
            buf,
            &Overlay {
                theme: t,
                title: "playlists",
                detail: None,
                footer: Some("enter load \u{b7} esc close"),
            },
        );
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

/// Keep the cursor visible. Moved to starkit, where every list that scrolls
/// needs it.
pub use starkit::list::clamp_scroll;
