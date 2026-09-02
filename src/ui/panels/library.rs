//! The library, as three columns.
//!
//! Artists, their records, their tracks -- a shape borrowed from every music
//! library that has ever worked, because the alternative for 29,511 tracks is a
//! list nobody can find anything in.
//!
//! This draws the whole window rather than docking under the player. Three
//! columns need the width, and the player carries on behind it: the browser is
//! a view, not a modal, which is why (unlike every overlay next door) clicking
//! outside it does nothing and only `esc` closes it.
//!
//! [`layout`] is the single source of where everything sits. The renderer draws
//! from it and the mouse handler tests against it, for the reason the playlist
//! panel gives about its own offset: deriving the geometry twice is how a click
//! comes to select the row above the one it landed on.

// A few accessors exist for a caller that has not been written: the browser
// is opened one way today, and `new` and the model getters are what a second
// entry point would need.
#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::panels::player::truncate;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Which column is which, everywhere.
pub const ARTISTS: usize = 0;
pub const ALBUMS: usize = 1;
pub const TRACKS: usize = 2;

/// Below this much room inside the border, three columns stop being columns.
const THREE: u16 = 62;
/// And below this, two do.
const TWO: u16 = 42;

/// Where everything sits. Pure, and the only place that decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    /// The search line, across the top inside the border.
    pub search: Rect,
    /// Right-aligned on the search row: a count, or the trail back to here.
    pub summary: Rect,
    pub heads: [Rect; 3],
    pub bodies: [Rect; 3],
    /// Where a divider is drawn, between each pair of shown columns.
    pub rules: [Option<u16>; 2],
    pub footer: Rect,
    /// Which columns are on screen, left to right.
    pub shown: Vec<usize>,
}

/// What a click landed on.
///
/// `row` is an offset into the body; the caller adds its own scroll, the way
/// the playlist and the picker already do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Search,
    Head(usize),
    Row { column: usize, row: u16 },
    Footer,
    Nothing,
}

/// The three column rects, given the room inside the border.
///
/// Widths come from the names they hold: artist names have a median of 11
/// characters and a 95th percentile of 18, so a column of 12 to 28 truncates
/// almost nothing. Album titles reach 103 characters and track titles 161, so
/// those two take what is left and truncate.
fn widths(w: u16, columns: usize) -> Vec<u16> {
    match columns {
        1 => vec![w],
        2 => {
            let left = (w * 40 / 100).clamp(14, 34).min(w.saturating_sub(10));
            vec![left, w.saturating_sub(left + 1)]
        }
        _ => {
            let a = (w * 22 / 100).clamp(12, 28);
            let b = (w * 32 / 100).clamp(16, 40);
            vec![a, b, w.saturating_sub(a + b + 2)]
        }
    }
}

/// Which columns survive, and in what order.
///
/// The window slides rather than truncating: with the focus on TRACKS at two
/// columns you keep ALBUMS, and with it on ARTISTS you keep ALBUMS. **The
/// parent is never the one that goes** -- a column you cannot see is still
/// there, and moving left or right slides the window rather than stopping at
/// the edge of what is drawn.
fn shown(inner_w: u16, focus: usize) -> Vec<usize> {
    if inner_w >= THREE {
        vec![ARTISTS, ALBUMS, TRACKS]
    } else if inner_w >= TWO {
        let first = focus.saturating_sub(1).min(1);
        vec![first, first + 1]
    } else {
        vec![focus.min(TRACKS)]
    }
}

pub fn layout(area: Rect, focus: usize) -> Layout {
    let frame = area;
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let bottom = area.y.saturating_add(area.height);

    let shown = shown(inner.width, focus);
    let ws = widths(inner.width, shown.len());

    // The key hints sit on the bottom border, as they do in the settings and
    // picker overlays, so nothing inside is spent on them.
    let head_h = u16::from(inner.height >= 3);
    let search = Rect {
        height: head_h.min(inner.height),
        ..inner
    };
    let summary = Rect {
        x: inner.x + inner.width.saturating_sub(inner.width / 3),
        width: inner.width / 3,
        ..search
    };
    let footer = Rect {
        y: bottom.saturating_sub(1),
        height: 0,
        ..inner
    };

    let top = inner.y.saturating_add(search.height);
    let body_h = inner
        .height
        .saturating_sub(search.height)
        .saturating_sub(head_h);

    let mut heads = [Rect::ZERO; 3];
    let mut bodies = [Rect::ZERO; 3];
    let mut rules = [None; 2];
    let mut x = inner.x;
    for (slot, &col) in shown.iter().enumerate() {
        let w = ws[slot];
        heads[col] = Rect {
            x,
            y: top,
            width: w,
            height: head_h,
        };
        bodies[col] = Rect {
            x,
            y: top.saturating_add(head_h),
            width: w,
            height: body_h,
        };
        x = x.saturating_add(w);
        if slot + 1 < shown.len() {
            rules[slot] = Some(x);
            x = x.saturating_add(1);
        }
    }

    Layout {
        frame,
        search,
        summary,
        heads,
        bodies,
        rules,
        footer,
        shown,
    }
}

fn within(r: Rect, x: u16, y: u16) -> bool {
    r.width > 0 && r.height > 0 && x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
}

/// What is at `(x, y)`.
///
/// Never reports a column that was not drawn: a column out of [`Layout::shown`]
/// has a zero rect, and nothing is inside one of those.
pub fn hit(l: &Layout, x: u16, y: u16) -> Hit {
    if within(l.search, x, y) {
        return Hit::Search;
    }
    if within(l.footer, x, y) {
        return Hit::Footer;
    }
    for c in 0..3 {
        if within(l.heads[c], x, y) {
            return Hit::Head(c);
        }
        if within(l.bodies[c], x, y) {
            return Hit::Row {
                column: c,
                row: y - l.bodies[c].y,
            };
        }
    }
    Hit::Nothing
}

/// One row, in any of the three columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// A count, or a duration. Right-aligned, and the first thing dropped.
    pub meta: String,
    /// The track number. `None` in the artist and album columns.
    pub lead: Option<String>,
    /// Named from its folder rather than from a tag.
    pub inferred: bool,
    /// The search matched this row itself.
    pub matched: bool,
}

impl Entry {
    pub fn new(label: impl Into<String>, meta: impl Into<String>) -> Entry {
        Entry {
            label: label.into(),
            meta: meta.into(),
            ..Entry::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Column<'a> {
    pub head: &'a str,
    pub rows: &'a [Entry],
    pub cursor: usize,
    pub scroll: usize,
    /// What to say when there is nothing here, rather than drawing a blank.
    pub empty: &'a str,
}

pub struct LibraryView<'a> {
    pub theme: &'a Theme,
    pub search: &'a str,
    /// Draws the caret, and tells the reader their keys are going into the box.
    pub typing: bool,
    pub columns: [Column<'a>; 3],
    pub focus: usize,
    pub summary: &'a str,
    pub keys: &'a str,
}

pub fn clamp_scroll(cursor: usize, scroll: usize, height: usize) -> usize {
    super::picker::clamp_scroll(cursor, scroll, height)
}

/// What is drawn when the row is too narrow for all of it.
///
/// In order: the meta column, then the track number, then nothing more -- the
/// label is the row. Monotonic, so a window that grows never shows less.
fn parts(width: usize, e: &Entry) -> (bool, bool) {
    let meta = !e.meta.is_empty() && width >= 16;
    let lead = e.lead.is_some() && width >= 12;
    (lead, meta)
}

impl<'a> Widget for LibraryView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let l = layout(area, self.focus);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(t.border_focused)))
            .title(Span::styled(
                format!("{}LIBRARY ", super::frame::TITLE_LEAD),
                Style::default().fg(rgb(t.header_fg)),
            ))
            .title_bottom(
                Line::from(Span::styled(
                    format!(" {} ", self.keys),
                    Style::default().fg(rgb(t.dim)),
                ))
                .right_aligned(),
            )
            .style(Style::default().bg(rgb(t.panel_bg)));
        block.render(l.frame, buf);
        super::frame::render_corners(l.frame, buf, t);

        if l.search.height > 0 {
            let caret = if self.typing { "\u{2582}" } else { "" };
            let text = if self.search.is_empty() && !self.typing {
                "search: /".to_string()
            } else {
                format!("search: {}{caret}", self.search)
            };
            let style = if self.search.is_empty() && !self.typing {
                Style::default().fg(rgb(t.empty_fg))
            } else if self.typing {
                Style::default().fg(rgb(t.accent))
            } else {
                Style::default().fg(rgb(t.dim))
            };
            let room = l.search.width.saturating_sub(l.summary.width + 1) as usize;
            buf.set_string(l.search.x, l.search.y, truncate(&text, room), style);
            if !self.summary.is_empty() && l.summary.width > 0 {
                let s = truncate(self.summary, l.summary.width as usize);
                let x = l.summary.right().saturating_sub(s.chars().count() as u16);
                buf.set_string(x, l.summary.y, s, Style::default().fg(rgb(t.row_meta_fg)));
            }
        }

        for &c in &l.shown {
            let col = self.columns[c];
            let head = l.heads[c];
            if head.height > 0 && head.width > 0 {
                buf.set_string(
                    head.x,
                    head.y,
                    truncate(col.head, head.width as usize),
                    Style::default()
                        .fg(rgb(t.row_meta_fg))
                        .add_modifier(Modifier::BOLD),
                );
            }
            render_column(l.bodies[c], &col, c == self.focus, t, buf);
        }

        // Dividers last, so no column's padding paints over them.
        for x in l.rules.iter().flatten() {
            let first = l.shown[0];
            for y in l.heads[first].y..l.bodies[first].bottom() {
                buf[(*x, y)]
                    .set_char('\u{2502}')
                    .set_style(Style::default().fg(rgb(t.border)).bg(rgb(t.panel_bg)));
            }
        }
    }
}

fn render_column(area: Rect, col: &Column<'_>, focused: bool, t: &Theme, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    if col.rows.is_empty() {
        buf.set_string(
            area.x,
            area.y,
            truncate(col.empty, w),
            Style::default().fg(rgb(t.empty_fg)),
        );
        return;
    }

    let height = area.height as usize;
    let scroll = clamp_scroll(col.cursor, col.scroll, height);
    for (i, e) in col.rows.iter().enumerate().skip(scroll).take(height) {
        let y = area.y + (i - scroll) as u16;
        let selected = i == col.cursor;

        // Only the focused column carries the bright bar. Three lit rows at
        // once and there is no telling which one the keys are going to.
        let style = match (selected, focused) {
            (true, true) => Style::default()
                .fg(rgb(t.row_selected_fg))
                .bg(rgb(t.row_selected_bg))
                .add_modifier(Modifier::BOLD),
            (true, false) => Style::default().fg(rgb(t.row_fg)).bg(rgb(t.row_cursor_bg)),
            _ if e.inferred => Style::default().fg(rgb(t.row_meta_fg)),
            _ if e.matched => Style::default().fg(rgb(t.accent)),
            _ => Style::default().fg(rgb(t.row_fg)),
        };

        // The whole width first, so a selection is a bar rather than a
        // highlight around the text.
        buf.set_string(area.x, y, " ".repeat(w), style);

        let (want_lead, want_meta) = parts(w, e);
        let meta = if want_meta { e.meta.as_str() } else { "" };
        let meta_w = meta.chars().count();
        let lead = match (want_lead, &e.lead) {
            (true, Some(l)) => format!("{l:>3} "),
            _ => String::new(),
        };

        let label_w = w
            .saturating_sub(lead.chars().count())
            .saturating_sub(if meta_w > 0 { meta_w + 1 } else { 0 });
        buf.set_string(area.x, y, &lead, style);
        buf.set_string(
            area.x + lead.chars().count() as u16,
            y,
            truncate(&e.label, label_w),
            style,
        );
        if meta_w > 0 {
            let x = area.x + area.width - meta_w as u16;
            let meta_style = if selected {
                style
            } else {
                Style::default().fg(rgb(t.dim))
            };
            buf.set_string(x, y, meta, meta_style);
        }
    }
}

/// The browser's own state: what is selected, what is typed, where it scrolled.
///
/// Kept here rather than in `App` because it is behaviour and not just state --
/// the cascade from one column to the next, remembering the way back, filtering
/// on the search. `App` holds the `Option` and wires it to keys, mouse and
/// draw, and everything below is testable without an `App` at all.
///
/// Per window, deliberately. What a terminal is showing of the *index* is not
/// what the shared view describes -- two windows browsing are two people at two
/// filing cabinets, not one session -- and sharing a half-typed search would
/// republish the view on every keystroke.
pub struct Library {
    model: std::sync::Arc<crate::library::browse::Model>,
    pub focus: usize,
    cursor: [usize; 3],
    scroll: [usize; 3],
    pub search: String,
    pub typing: bool,
    /// Indices into the model, after the search. The columns are these.
    shown: [Vec<u32>; 3],
    /// The record last open under each artist, and the track under each record.
    ///
    /// By name, not by index: stepping back left and right again has to land
    /// where you left, and an index means something different the moment the
    /// list is filtered. The same reason the shared view keys its cursor on a
    /// URI rather than a position.
    album_of: std::collections::HashMap<String, String>,
    track_of: std::collections::HashMap<String, String>,
}

impl Library {
    pub fn new(model: std::sync::Arc<crate::library::browse::Model>) -> Library {
        let mut lib = Library {
            model,
            focus: ARTISTS,
            cursor: [0; 3],
            scroll: [0; 3],
            search: String::new(),
            typing: false,
            shown: [Vec::new(), Vec::new(), Vec::new()],
            album_of: Default::default(),
            track_of: Default::default(),
        };
        lib.refilter();
        lib
    }

    pub fn model(&self) -> &crate::library::browse::Model {
        &self.model
    }

    /// Which artists survive the search, then cascade into albums and tracks.
    ///
    /// The search filters the *left* column only. Typing `reptile` narrows the
    /// artists to Helloween and the track column still shows all eighteen
    /// tracks of Master of the Rings with Reptile lit -- because seeing what
    /// you found *in its record* is the whole reason for three columns rather
    /// than a flat list of results.
    pub fn refilter(&mut self) {
        let needle = self.search.trim().to_lowercase();
        let m = &self.model;
        let mut hits: Vec<u32> = (0..m.artists.len() as u32)
            .filter(|&i| needle.is_empty() || self.artist_matches(i, &needle))
            .collect();
        // An artist whose own name matches comes first. Typing `helloween`
        // and landing on Armory -- which merely covers one of their songs --
        // is technically a match and practically the wrong answer.
        if !needle.is_empty() {
            hits.sort_by_key(|&i| !m.artists[i as usize].sort.contains(&needle));
        }
        self.shown[ARTISTS] = hits;
        self.cursor[ARTISTS] =
            self.cursor[ARTISTS].min(self.shown[ARTISTS].len().saturating_sub(1));
        self.cascade();
    }

    fn artist_matches(&self, i: u32, needle: &str) -> bool {
        let m = &self.model;
        let a = &m.artists[i as usize];
        if a.sort.contains(needle) {
            return true;
        }
        a.albums.clone().any(|ai| {
            let al = &m.albums[ai as usize];
            al.title.to_lowercase().contains(needle)
                || al.tracks.clone().any(|ti| {
                    m.tracks[ti as usize]
                        .title()
                        .to_lowercase()
                        .contains(needle)
                })
        })
    }

    /// Rebuild both child columns. For a change of artist, or of the search.
    fn cascade(&mut self) {
        self.rebuild_albums();
        self.rebuild_tracks();
    }

    /// The records under the selected artist, cursor back where it last was.
    fn rebuild_albums(&mut self) {
        let m = self.model.clone();
        self.shown[ALBUMS] = match self.selected_artist() {
            Some(a) => m.artists[a as usize].albums.clone().collect(),
            None => Vec::new(),
        };
        if let Some(a) = self.selected_artist() {
            let want = self.album_of.get(&*m.artists[a as usize].name).cloned();
            self.cursor[ALBUMS] = want
                .and_then(|w| {
                    self.shown[ALBUMS]
                        .iter()
                        .position(|&i| m.albums[i as usize].title.as_ref() == w)
                })
                .unwrap_or(0);
        } else {
            self.cursor[ALBUMS] = 0;
        }
    }

    /// The tracks of the selected record.
    fn rebuild_tracks(&mut self) {
        let m = self.model.clone();
        self.shown[TRACKS] = match self.selected_album() {
            Some(a) => m.albums[a as usize].tracks.clone().collect(),
            None => Vec::new(),
        };
        if let Some(a) = self.selected_album() {
            let want = self.track_of.get(&*m.albums[a as usize].title).cloned();
            self.cursor[TRACKS] = want
                .and_then(|w| {
                    self.shown[TRACKS]
                        .iter()
                        .position(|&i| m.tracks[i as usize].title() == w)
                })
                .unwrap_or(0);
        } else {
            self.cursor[TRACKS] = 0;
        }
    }

    /// Remember where we were, so coming back lands here.
    fn remember(&mut self) {
        let m = self.model.clone();
        if let (Some(a), Some(al)) = (self.selected_artist(), self.selected_album()) {
            self.album_of.insert(
                m.artists[a as usize].name.to_string(),
                m.albums[al as usize].title.to_string(),
            );
            if let Some(t) = self.selected_track() {
                self.track_of.insert(
                    m.albums[al as usize].title.to_string(),
                    m.tracks[t as usize].title().to_string(),
                );
            }
        }
    }

    pub fn selected_artist(&self) -> Option<u32> {
        self.shown[ARTISTS].get(self.cursor[ARTISTS]).copied()
    }
    pub fn selected_album(&self) -> Option<u32> {
        self.shown[ALBUMS].get(self.cursor[ALBUMS]).copied()
    }
    pub fn selected_track(&self) -> Option<u32> {
        self.shown[TRACKS].get(self.cursor[TRACKS]).copied()
    }

    pub fn cursor(&self, column: usize) -> usize {
        self.cursor[column]
    }
    pub fn scroll(&self, column: usize) -> usize {
        self.scroll[column]
    }
    pub fn set_scroll(&mut self, column: usize, v: usize) {
        self.scroll[column] = v;
    }
    pub fn len(&self, column: usize) -> usize {
        self.shown[column].len()
    }

    /// Move the focused column's cursor, cascading if it feeds another.
    pub fn step(&mut self, delta: i32) {
        let n = self.shown[self.focus].len();
        if n == 0 {
            return;
        }
        let cur = self.cursor[self.focus] as i32;
        self.cursor[self.focus] = (cur + delta).clamp(0, n as i32 - 1) as usize;
        self.after_move();
    }

    pub fn jump(&mut self, end: bool) {
        let n = self.shown[self.focus].len();
        self.cursor[self.focus] = if end { n.saturating_sub(1) } else { 0 };
        self.after_move();
    }

    /// Rebuild only what sits to the *right* of the column that moved.
    ///
    /// Rebuilding the moved column too would put its cursor straight back
    /// where it was remembered from -- stepping down the album list would
    /// snap to the same record every time.
    fn after_move(&mut self) {
        match self.focus {
            ARTISTS => self.cascade(),
            ALBUMS => self.rebuild_tracks(),
            _ => {}
        }
        self.remember();
    }

    /// Move between columns. Never wraps: a full-screen jump from the track
    /// list back to the artists is disorienting rather than quick.
    pub fn shift(&mut self, delta: i32) {
        self.focus = (self.focus as i32 + delta).clamp(0, TRACKS as i32) as usize;
    }

    pub fn select(&mut self, column: usize, row: usize) {
        if row < self.shown[column].len() {
            self.focus = column;
            self.cursor[column] = row;
            self.after_move();
        }
    }

    /// What `space` would add: the focused column's selection, as queue items.
    ///
    /// An artist means every one of their records in album order, an album
    /// means the record, a track means the track. Drawn from the model, which
    /// is one row per song already -- so a record with a per-track cue sheet
    /// yields fourteen tracks and not twenty-eight.
    pub fn selection(&self, whole_album: bool) -> (String, Vec<crate::playlist::queue::QueueItem>) {
        let m = &self.model;
        let take = |range: std::ops::Range<u32>| -> Vec<_> {
            range.map(|t| m.tracks[t as usize].item.clone()).collect()
        };
        let column = if whole_album {
            ALBUMS.min(self.focus.max(ALBUMS))
        } else {
            self.focus
        };
        match column {
            ARTISTS => match self.selected_artist() {
                Some(a) => {
                    let ar = &m.artists[a as usize];
                    let items: Vec<_> = ar
                        .albums
                        .clone()
                        .flat_map(|al| take(m.albums[al as usize].tracks.clone()))
                        .collect();
                    (
                        format!("{} \u{2014} {} tracks", ar.name, items.len()),
                        items,
                    )
                }
                None => (String::new(), Vec::new()),
            },
            ALBUMS => match self.selected_album() {
                Some(a) => {
                    let al = &m.albums[a as usize];
                    let items = take(al.tracks.clone());
                    (
                        format!("{} \u{2014} {} tracks", al.title, items.len()),
                        items,
                    )
                }
                None => (String::new(), Vec::new()),
            },
            _ => match self.selected_track() {
                Some(t) => {
                    let tr = &m.tracks[t as usize];
                    (tr.title().to_string(), vec![tr.item.clone()])
                }
                None => (String::new(), Vec::new()),
            },
        }
    }

    /// What the search line says on the right.
    pub fn summary(&self) -> String {
        let n = self.shown[ARTISTS].len();
        if self.search.trim().is_empty() {
            format!("{n} artists")
        } else if n == 1 {
            format!("1 of {} artists", self.model.artists.len())
        } else {
            format!("{n} of {} artists", self.model.artists.len())
        }
    }

    /// The rows of one column, ready to draw.
    pub fn rows(&self, column: usize) -> Vec<Entry> {
        let m = &self.model;
        let needle = self.search.trim().to_lowercase();
        let lit = |s: &str| !needle.is_empty() && s.to_lowercase().contains(&needle);
        self.shown[column]
            .iter()
            .map(|&i| match column {
                ARTISTS => {
                    let a = &m.artists[i as usize];
                    Entry {
                        label: a.name.to_string(),
                        meta: a.tracks.to_string(),
                        lead: None,
                        inferred: a.from.is_guess(),
                        matched: lit(&a.name),
                    }
                }
                ALBUMS => {
                    let al = &m.albums[i as usize];
                    Entry {
                        label: match al.year {
                            Some(y) => format!("{y} {}", al.title),
                            None => al.title.to_string(),
                        },
                        meta: al.tracks.len().to_string(),
                        lead: None,
                        inferred: al.from.is_guess(),
                        matched: lit(&al.title),
                    }
                }
                _ => {
                    let t = &m.tracks[i as usize];
                    Entry {
                        label: t.title().to_string(),
                        meta: t
                            .item
                            .duration_secs
                            .map(|s| format!("{}:{:02}", s / 60, s % 60))
                            .unwrap_or_default(),
                        lead: t.item.track_no.map(|n| n.to_string()),
                        inferred: false,
                        matched: lit(t.title()),
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;

    fn entries(n: usize) -> Vec<Entry> {
        (0..n)
            .map(|i| Entry::new(format!("row {i}"), format!("{i}")))
            .collect()
    }

    fn a_library() -> Library {
        // Two artists, one with two records, so both cascades have somewhere
        // to go.
        let m = crate::library::browse::fixture(&[
            (
                "Helloween/1994 - Master of the Rings",
                "01.flac",
                "Irritation",
                "Helloween",
                "Master of the Rings",
            ),
            (
                "Helloween/1994 - Master of the Rings",
                "02.flac",
                "Sole Survivor",
                "Helloween",
                "Master of the Rings",
            ),
            (
                "Helloween/1996 - The Time of the Oath",
                "01.flac",
                "We Burn",
                "Helloween",
                "The Time of the Oath",
            ),
            (
                "Angra/1993 - Angels Cry",
                "01.flac",
                "Unfinished Allegro",
                "Angra",
                "Angels Cry",
            ),
        ]);
        Library::new(std::sync::Arc::new(m))
    }

    #[test]
    fn stepping_through_the_records_actually_moves() {
        // The cascade rebuilds the columns to the *right* of the one that
        // moved. Rebuilding the moved column as well puts its cursor straight
        // back where it was remembered from, and the album list becomes
        // impossible to walk.
        let mut lib = a_library();
        lib.step(1); // Angra sorts first; Helloween is the one with two.
        lib.shift(1);
        assert_eq!(lib.focus, ALBUMS);
        assert_eq!(lib.len(ALBUMS), 2, "the artist should have two records");
        let first = lib.selected_album();
        lib.step(1);
        assert_ne!(lib.selected_album(), first, "the album cursor snapped back");
        assert_eq!(lib.cursor(ALBUMS), 1);
    }

    #[test]
    fn choosing_a_record_changes_the_tracks_beside_it() {
        let mut lib = a_library();
        lib.step(1);
        lib.shift(1);
        assert_eq!(lib.len(TRACKS), 2, "Master of the Rings has two tracks");
        lib.step(1);
        assert_eq!(lib.len(TRACKS), 1, "The Time of the Oath has one");
    }

    #[test]
    fn stepping_back_left_and_right_again_lands_where_it_was_left() {
        let mut lib = a_library();
        lib.step(1);
        lib.shift(1);
        lib.step(1);
        let want = lib.selected_album();
        lib.shift(-1); // back to artists
        lib.shift(1); // and in again
        assert_eq!(lib.selected_album(), want, "it forgot where we were");
    }

    #[test]
    fn space_on_an_artist_offers_every_one_of_their_tracks() {
        let mut lib = a_library();
        // Artists are sorted, so Angra is first and Helloween second.
        lib.step(1);
        let (what, items) = lib.selection(false);
        assert_eq!(items.len(), 3, "{what}");
        assert!(what.starts_with("Helloween"), "{what}");
        // And on a record, only that record.
        lib.shift(1);
        let (_, items) = lib.selection(false);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn the_search_narrows_the_artists_and_clearing_it_restores_them() {
        let mut lib = a_library();
        assert_eq!(lib.len(ARTISTS), 2);
        lib.search = "angra".into();
        lib.refilter();
        assert_eq!(lib.len(ARTISTS), 1);
        lib.search.clear();
        lib.refilter();
        assert_eq!(lib.len(ARTISTS), 2);
    }

    #[test]
    fn an_artist_whose_own_name_matches_comes_first() {
        // Searching a track title should not bury the artist you named.
        let mut lib = a_library();
        lib.search = "helloween".into();
        lib.refilter();
        assert_eq!(lib.len(ARTISTS), 1);
        let rows = lib.rows(ARTISTS);
        assert_eq!(rows[0].label, "Helloween");
    }

    #[test]
    fn the_three_columns_never_overlap_or_escape_the_frame() {
        for w in 40..200u16 {
            for h in 8..40u16 {
                for focus in 0..3 {
                    let area = Rect::new(0, 0, w, h);
                    let l = layout(area, focus);
                    let mut last_right = area.x;
                    for &c in &l.shown {
                        let b = l.bodies[c];
                        assert!(b.x >= last_right, "{w}x{h}: column {c} overlaps");
                        assert!(b.right() <= area.right(), "{w}x{h}: column {c} escapes");
                        assert!(b.bottom() <= area.bottom(), "{w}x{h}: column {c} overflows");
                        last_right = b.right();
                    }
                    assert!(l.footer.bottom() <= area.bottom(), "{w}x{h}: footer");
                    assert!(l.search.bottom() <= area.bottom(), "{w}x{h}: search");
                }
            }
        }
    }

    /// The doctrine test: what was drawn at a cell is what a click there finds.
    #[test]
    fn each_row_is_where_its_hit_box_says_it_is() {
        for w in [50u16, 80, 120, 200] {
            let area = Rect::new(0, 0, w, 20);
            let l = layout(area, ARTISTS);
            for &c in &l.shown {
                let b = l.bodies[c];
                for row in 0..b.height {
                    assert_eq!(
                        hit(&l, b.x, b.y + row),
                        Hit::Row { column: c, row },
                        "width {w}, column {c}, row {row}"
                    );
                }
            }
        }
    }

    #[test]
    fn nothing_that_was_not_drawn_can_be_clicked() {
        for w in 40..120u16 {
            let area = Rect::new(0, 0, w, 20);
            let l = layout(area, TRACKS);
            for c in 0..3 {
                if l.shown.contains(&c) {
                    continue;
                }
                // A column that is not on screen has no cells at all, so no
                // click can land on one.
                assert_eq!(l.bodies[c], Rect::ZERO, "width {w}: column {c} was drawn");
                assert_eq!(l.heads[c], Rect::ZERO, "width {w}: head {c} was drawn");
            }
        }
    }

    #[test]
    fn a_narrow_window_slides_the_pane_rather_than_squeezing_three_columns() {
        let mut widths_seen = Vec::new();
        for w in 40..80u16 {
            let n = layout(Rect::new(0, 0, w, 20), TRACKS).shown.len();
            widths_seen.push(n);
        }
        // Never loses a column as the window grows.
        for pair in widths_seen.windows(2) {
            assert!(pair[1] >= pair[0], "a wider window showed fewer columns");
        }
        assert!(widths_seen.contains(&1) && widths_seen.contains(&2) && widths_seen.contains(&3));
    }

    #[test]
    fn the_focused_column_always_keeps_its_parent_on_screen() {
        // Two columns, focus on tracks: albums must survive, because a track
        // list with no idea which record it belongs to is not navigable.
        let l = layout(Rect::new(0, 0, 50, 20), TRACKS);
        assert_eq!(l.shown, vec![ALBUMS, TRACKS]);
        let l = layout(Rect::new(0, 0, 50, 20), ALBUMS);
        assert_eq!(l.shown, vec![ARTISTS, ALBUMS]);
        let l = layout(Rect::new(0, 0, 50, 20), ARTISTS);
        assert_eq!(l.shown, vec![ARTISTS, ALBUMS]);
    }

    fn draw(w: u16, h: u16, focus: usize, cursor: usize) -> (Buffer, Vec<Entry>) {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let rows = entries(6);
        let col = Column {
            head: "ARTISTS",
            rows: &rows,
            cursor,
            scroll: 0,
            empty: "nothing",
        };
        LibraryView {
            theme: &theme,
            search: "",
            typing: false,
            columns: [
                col,
                Column {
                    head: "ALBUMS",
                    ..col
                },
                Column {
                    head: "TRACKS",
                    ..col
                },
            ],
            focus,
            summary: "862 artists",
            keys: "esc back",
        }
        .render(area, &mut buf);
        (buf, rows)
    }

    fn text(buf: &Buffer, area: Rect) -> String {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(area.x + x, area.y + y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn all_three_columns_are_drawn_with_their_headings() {
        let (buf, _) = draw(100, 16, ARTISTS, 0);
        let all = text(&buf, Rect::new(0, 0, 100, 16));
        for head in ["ARTISTS", "ALBUMS", "TRACKS"] {
            assert!(all.contains(head), "{head} missing:\n{all}");
        }
        assert!(all.contains("LIBRARY"), "{all}");
        assert!(all.contains("862 artists"), "{all}");
    }

    /// The classic three-column bug: a selection bar that runs across the
    /// whole window instead of stopping at its own column.
    #[test]
    fn the_selected_row_is_a_bar_across_its_own_column_and_no_further() {
        let theme = builtin::load("cosmic").unwrap();
        let (buf, _) = draw(100, 16, ALBUMS, 1);
        let l = layout(Rect::new(0, 0, 100, 16), ALBUMS);
        let bar = Some(Color::Rgb(
            theme.row_selected_bg.r,
            theme.row_selected_bg.g,
            theme.row_selected_bg.b,
        ));
        let y = l.bodies[ALBUMS].y + 1;
        assert_eq!(buf[(l.bodies[ALBUMS].x, y)].style().bg, bar);
        assert_eq!(buf[(l.bodies[ALBUMS].right() - 1, y)].style().bg, bar);
        // The same row in the neighbours is not lit.
        assert_ne!(buf[(l.bodies[ARTISTS].x, y)].style().bg, bar);
        assert_ne!(buf[(l.bodies[TRACKS].x, y)].style().bg, bar);
    }

    #[test]
    fn only_the_focused_column_draws_the_bright_selection() {
        let theme = builtin::load("cosmic").unwrap();
        let (buf, _) = draw(100, 16, ARTISTS, 0);
        let l = layout(Rect::new(0, 0, 100, 16), ARTISTS);
        let bright = Some(Color::Rgb(
            theme.row_selected_bg.r,
            theme.row_selected_bg.g,
            theme.row_selected_bg.b,
        ));
        assert_eq!(
            buf[(l.bodies[ARTISTS].x, l.bodies[ARTISTS].y)].style().bg,
            bright
        );
        assert_ne!(
            buf[(l.bodies[ALBUMS].x, l.bodies[ALBUMS].y)].style().bg,
            bright
        );
    }

    #[test]
    fn a_column_with_nothing_in_it_says_so_rather_than_drawing_a_blank() {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 100, 16);
        let mut buf = Buffer::empty(area);
        let none: Vec<Entry> = Vec::new();
        let col = Column {
            head: "TRACKS",
            rows: &none,
            cursor: 0,
            scroll: 0,
            empty: "no albums here",
        };
        LibraryView {
            theme: &theme,
            search: "",
            typing: false,
            columns: [col, col, col],
            focus: 0,
            summary: "",
            keys: "",
        }
        .render(area, &mut buf);
        assert!(text(&buf, area).contains("no albums here"));
    }

    #[test]
    fn a_row_loses_its_count_before_it_loses_its_name() {
        let e = Entry {
            label: "Helloween".into(),
            meta: "486".into(),
            lead: Some("3".into()),
            ..Entry::default()
        };
        // Monotonic: nothing comes back as the column narrows.
        let mut seen = (true, true);
        for w in (4..40usize).rev() {
            let now = parts(w, &e);
            assert!(!(now.0 && !seen.0), "the lead came back at {w}");
            assert!(!(now.1 && !seen.1), "the meta came back at {w}");
            seen = now;
        }
        assert_eq!(parts(30, &e), (true, true));
        assert_eq!(parts(6, &e), (false, false));
    }

    #[test]
    fn the_search_line_shows_what_was_typed_and_that_it_is_being_typed() {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 100, 16);
        let rows = entries(2);
        let col = Column {
            head: "ARTISTS",
            rows: &rows,
            cursor: 0,
            scroll: 0,
            empty: "",
        };
        let mut buf = Buffer::empty(area);
        LibraryView {
            theme: &theme,
            search: "helloween",
            typing: true,
            columns: [col, col, col],
            focus: 0,
            summary: "",
            keys: "",
        }
        .render(area, &mut buf);
        let all = text(&buf, area);
        assert!(all.contains("search: helloween"), "{all}");
        assert!(all.contains('\u{2582}'), "no caret while typing:\n{all}");
    }

    #[test]
    fn a_name_taken_from_a_folder_is_drawn_differently_from_a_tagged_one() {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 100, 16);
        let rows = vec![
            Entry::new("Tagged", "1"),
            Entry {
                label: "Guessed".into(),
                inferred: true,
                ..Entry::default()
            },
        ];
        let col = Column {
            head: "ARTISTS",
            rows: &rows,
            cursor: 5,
            scroll: 0,
            empty: "",
        };
        let mut buf = Buffer::empty(area);
        LibraryView {
            theme: &theme,
            search: "",
            typing: false,
            columns: [col, col, col],
            focus: 0,
            summary: "",
            keys: "",
        }
        .render(area, &mut buf);
        let l = layout(area, 0);
        let plain = buf[(l.bodies[ARTISTS].x, l.bodies[ARTISTS].y)].style().fg;
        let guess = buf[(l.bodies[ARTISTS].x, l.bodies[ARTISTS].y + 1)]
            .style()
            .fg;
        assert_ne!(plain, guess, "a guess looks exactly like a fact");
    }
}
