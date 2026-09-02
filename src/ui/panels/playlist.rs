//! The playlist editor window.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use std::collections::HashSet;

use crate::playlist::queue::QueueItem;
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::digits;
use crate::ui::panels::player::truncate;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Blank columns kept to the right of the duration.
const RIGHT_PAD: u16 = 1;

/// Where the tracks go: the panel less its border and its header row.
///
/// Pure, and the single source of the offset. The renderer draws from it, the
/// scrollbar measures against it, and the mouse handler turns a click into a
/// row with it. Deriving that offset separately in each of those places is how
/// a click comes to select the row above the one it landed on.
pub fn list_rect(area: Rect) -> Rect {
    super::header::body(area)
}

/// The cells carrying a playing marker, for the picture that goes over it.
///
/// The playing track's row when it is on screen, and any record heading that
/// advertises the playing track inside it -- the same rows the renderer puts
/// its text marker on, derived the same way, so the picture lands where the
/// chevron was and nowhere else.
pub fn marker_cells(
    area: Rect,
    rows: &Rows,
    scroll: usize,
    playing: Option<usize>,
) -> Vec<(u16, u16)> {
    let Some(playing) = playing else {
        return Vec::new();
    };
    let inner = list_rect(area);
    let mut out = Vec::new();
    for row in 0..inner.height as usize {
        let Some(line) = rows.rows().get(scroll + row) else {
            break;
        };
        let marked = match line {
            Row::Track(i) => *i == playing,
            Row::Section { playing, .. } => *playing,
        };
        if marked && inner.width > 0 {
            out.push((inner.x, inner.y + row as u16));
        }
    }
    out
}

/// One line of the list.
///
/// A grouped playlist draws more lines than it has tracks, so "the third row"
/// and "the third track" stop being the same thing. Everything that maps
/// between them goes through [`Rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// The divider introducing a record.
    Section {
        /// The album title, lower case: what a fold is remembered by, so a
        /// click can find the same record again after the list has been
        /// reordered, reloaded, or put away and resumed tomorrow.
        fold: String,
        label: String,
        tracks: usize,
        /// Its tracks are not drawn.
        folded: bool,
        /// Folded, and the track playing is one of the ones it is hiding.
        /// Only then: an open record's own track already carries the marker,
        /// and two of them in a column reads as two things playing.
        playing: bool,
    },
    /// A track, by its position in the queue's view.
    Track(usize),
}

/// The lines the panel draws, and the map back to the tracks under them.
///
/// The single source of the row/track mapping, in the same spirit as
/// [`list_rect`]: the renderer draws from it, the scrollbar measures against
/// it and the mouse handler turns a click into a track with it. Deriving that
/// mapping separately in each place is how a click comes to select the row
/// above the one it landed on -- and with dividers in the way it would not
/// even be off by a consistent amount.
#[derive(Debug, Clone, Default)]
pub struct Rows {
    rows: Vec<Row>,
    /// Which row each track sits on, or `None` when its record is folded.
    row_of: Vec<Option<usize>>,
    /// The tracks that are actually drawn, in the order they are drawn.
    ///
    /// What the cursor walks. Stepping through the tracks themselves would
    /// have it disappear into a folded record and come out somewhere else.
    shown: Vec<usize>,
    /// The row of each track's heading, so a folded track still has somewhere
    /// on screen to point at.
    section_of: Vec<Option<usize>>,
}

impl Rows {
    /// No dividers: one row per track, which is what an ungrouped queue gets.
    pub fn flat(tracks: usize) -> Rows {
        Rows {
            rows: (0..tracks).map(Row::Track).collect(),
            row_of: (0..tracks).map(Some).collect(),
            shown: (0..tracks).collect(),
            section_of: vec![None; tracks],
        }
    }

    /// A divider wherever the record changes, folding away the ones in
    /// `folded` and marking the one that holds `playing`.
    ///
    /// Sorts nothing. It breaks on runs of the same album, so it is only
    /// meaningful over an order that is already grouped -- which is why the
    /// queue does the ordering and the panel only draws the seams.
    pub fn grouped(items: &[QueueItem], folded: &HashSet<String>, playing: Option<usize>) -> Rows {
        let keys = crate::playlist::group::keys(items);
        let mut rows: Vec<Row> = Vec::with_capacity(items.len() + 8);
        let mut row_of = vec![None; items.len()];
        let mut section_of = vec![None; items.len()];
        let mut shown = Vec::with_capacity(items.len());

        let mut i = 0;
        while i < items.len() {
            // The run of tracks belonging to this record, and the earliest
            // year any of them claims -- the same rule the ordering used, so
            // the heading agrees with the position.
            let mut end = i;
            let mut year: Option<i64> = None;
            while end < items.len() && keys[end] == keys[i] {
                year = match (year, items[end].year) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                end += 1;
            }
            // Blank counts as absent, the same way the grouping counts it, or
            // the untagged run gets a heading with nothing in it.
            let album = items[i]
                .album
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty());
            // The title alone, not the whole grouping key: the artist half of
            // that key exists only to separate two records of the same name
            // inside one queue, and a fold has to mean the same thing in the
            // next queue, where the namesake may not be there.
            let fold = keys[i]
                .as_ref()
                .map(|k| k.title().to_string())
                .unwrap_or_default();
            let shut = folded.contains(&fold);
            for t in section_of.iter_mut().take(end).skip(i) {
                *t = Some(rows.len());
            }
            rows.push(Row::Section {
                fold,
                label: heading(album, year),
                tracks: end - i,
                folded: shut,
                playing: shut && playing.is_some_and(|p| (i..end).contains(&p)),
            });
            if !shut {
                for (track, at) in row_of.iter_mut().enumerate().take(end).skip(i) {
                    *at = Some(rows.len());
                    shown.push(track);
                    rows.push(Row::Track(track));
                }
            }
            i = end;
        }
        Rows {
            rows,
            row_of,
            shown,
            section_of,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Where a track is drawn, or `None` when its record is folded away.
    pub fn row_of_track(&self, track: usize) -> Option<usize> {
        self.row_of.get(track).copied().flatten()
    }

    /// The first track at or after `row`.
    pub fn track_at_or_after(&self, row: usize) -> Option<usize> {
        self.rows.get(row..)?.iter().find_map(|r| match r {
            Row::Track(t) => Some(*t),
            Row::Section { .. } => None,
        })
    }

    /// The visible track `delta` steps from `track`, stopping at the ends.
    ///
    /// Folded records are stepped over rather than through: an arrow key that
    /// moved the cursor into one would look like it had done nothing, several
    /// times, and then jumped.
    pub fn step(&self, track: usize, delta: i32) -> usize {
        if self.shown.is_empty() {
            return track;
        }
        // A cursor on a track that has just been folded away steps from where
        // that record sits rather than from nowhere.
        let at = match self.shown.binary_search(&track) {
            Ok(i) => i as i32,
            Err(i) => i as i32 - if delta < 0 { 0 } else { 1 },
        };
        let last = self.shown.len() as i32 - 1;
        self.shown[(at + delta).clamp(0, last) as usize]
    }

    /// The first and last tracks on show.
    pub fn ends(&self) -> Option<(usize, usize)> {
        Some((*self.shown.first()?, *self.shown.last()?))
    }

    /// The row of the heading a track sits under, if the list has headings.
    pub fn section_row(&self, track: usize) -> Option<usize> {
        self.section_of.get(track).copied().flatten()
    }

    /// The nearest visible track, for a cursor whose own has been folded away.
    pub fn nearest_shown(&self, track: usize) -> Option<usize> {
        if self.row_of_track(track).is_some() {
            return Some(track);
        }
        match self.shown.binary_search(&track) {
            Ok(i) => self.shown.get(i).copied(),
            Err(i) => self.shown.get(i).or_else(|| self.shown.last()).copied(),
        }
    }

    /// The row to scroll to for `track`: its heading, when it opens a record.
    ///
    /// Arriving at the first track of an album with the album's name just off
    /// the top of the panel is the obvious annoyance, and this is the cure.
    pub fn anchor_row(&self, track: usize) -> usize {
        let Some(row) = self.row_of_track(track) else {
            // Folded away: its heading is the nearest thing to it on screen.
            return self.section_row(track).unwrap_or(0);
        };
        match row.checked_sub(1).and_then(|r| self.rows.get(r)) {
            Some(Row::Section { .. }) => row - 1,
            _ => row,
        }
    }
}

/// What a divider says: the record, and when it came out.
fn heading(album: Option<&str>, year: Option<i64>) -> String {
    match (album, year) {
        (None, _) => "no album".into(),
        (Some(a), Some(y)) => format!("{y} \u{b7} {a}"),
        (Some(a), None) => a.into(),
    }
}

/// Draw one divider: a rule, the record it introduces, and how long it is.
///
/// The rule is what makes a heading read as a seam rather than as a track with
/// odd text in it, and it is the first thing to go when the panel is narrow:
/// the name of the record is the part that carries meaning.
#[allow(clippy::too_many_arguments)]
fn render_section(
    x: u16,
    y: u16,
    width: u16,
    label: &str,
    tracks: usize,
    folded: bool,
    playing: bool,
    buf: &mut Buffer,
    t: &Theme,
) {
    let avail = width as usize;
    if avail == 0 {
        return;
    }
    let rule = Style::default().fg(rgb(t.divider));
    let name_style = Style::default().fg(rgb(t.row_meta_fg));
    let count_style = Style::default().fg(rgb(t.dim));

    buf.set_string(x, y, " ".repeat(avail), Style::default());

    // The playing marker sits in the same column the track rows put theirs in,
    // which is the whole reason a folded record advertises what is inside it:
    // the one row you most want to see is the one folding hides.
    let lead = if playing {
        "> \u{2500} "
    } else {
        "\u{2500}\u{2500} "
    };
    let lead_w = lead.chars().count();
    if avail <= lead_w + 1 {
        buf.set_string(x, y, truncate(label, avail), name_style);
        return;
    }
    buf.set_string(
        x,
        y,
        lead,
        if playing {
            Style::default().fg(rgb(t.row_playing_fg))
        } else {
            rule
        },
    );

    // No glyph for the fold: a lone symbol is at the mercy of whichever font
    // the terminal falls back to for it, which is the lesson the close mark
    // taught. Words are drawn in the same face as the text beside them, and
    // the folded wording is the shorter of the two because it is the one that
    // has to survive a narrow panel.
    let count = match (tracks, folded) {
        (1, false) => "1 track".to_string(),
        (n, false) => format!("{n} tracks"),
        (n, true) => format!("{n} hidden"),
    };
    let count_w = count.chars().count();

    // A folded record must say so even when there is barely room, so its count
    // is reserved out of the label's width rather than taking what is left.
    // An open one says how long it is, which is worth having but not worth
    // truncating the record's name for.
    let room = avail - lead_w;
    let name_w = if folded {
        room.saturating_sub(count_w + 2).max(1)
    } else {
        room
    };
    let name = truncate(label, name_w);
    let name_w = name.chars().count();
    buf.set_string(x + lead_w as u16, y, &name, name_style);

    // A gap either side of the rule, and at least a rule worth drawing: below
    // that the count is not worth the room it costs.
    let used = lead_w + name_w;
    if avail < used + count_w + 4 {
        // No room for a rule between them, but a folded record still has to
        // say it is folded: push the count hard right instead.
        if folded && avail > used + count_w {
            buf.set_string(x + (avail - count_w) as u16, y, &count, count_style);
        }
        return;
    }
    let count_x = avail - count_w;
    buf.set_string(
        x + used as u16 + 1,
        y,
        "\u{2500}".repeat(count_x - used - 2),
        rule,
    );
    buf.set_string(x + count_x as u16, y, &count, count_style);
}

pub struct PlaylistView<'a> {
    pub theme: &'a Theme,
    pub name: &'a str,
    pub items: &'a [QueueItem],
    /// The lines to draw, from [`Rows`].
    pub rows: &'a Rows,
    /// A position in the queue's view, not a row.
    pub cursor: usize,
    /// Likewise a position in the view.
    pub playing: Option<usize>,
    /// A row, because the dividers between here and the cursor take space.
    pub scroll: usize,
    pub focused: bool,
    /// Rows marked for a bulk action, by position in the view.
    pub tagged: &'a std::collections::HashSet<usize>,
    /// The transport's own faces, so the cursor row is marked with the same
    /// play glyph the play button carries.
    pub glyphs: super::player::Glyphs,
    /// The header words this panel offers, which is one fewer while mirroring:
    /// the leader owns the queue and there is nothing here to reorder.
    pub header_items: &'a [super::header::Item],
}

impl<'a> PlaylistView<'a> {
    /// Keep the cursor on screen, returning the scroll offset to use.
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
}

impl<'a> Widget for PlaylistView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let total = self.items.len();
        let title = format!("{}PLAYLIST — {} ", super::frame::TITLE_LEAD, self.name);
        // Trailing space for the close mark, which is drawn over the border
        // afterwards: a right-aligned title reaches the same cells, and the
        // two would fight for them.
        let count = if total > 0 {
            format!(
                " {}/{}{}",
                self.cursor + 1,
                total,
                super::frame::TITLE_TRAIL
            )
        } else {
            format!(" empty{}", super::frame::TITLE_TRAIL)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(if self.focused {
                t.border_focused
            } else {
                t.border
            })))
            .title(Span::styled(title, Style::default().fg(rgb(t.header_fg))))
            .title_top(
                ratatui::text::Line::from(Span::styled(count, Style::default().fg(rgb(t.dim))))
                    .right_aligned(),
            )
            .style(Style::default().bg(rgb(t.panel_bg)));

        let inner = list_rect(area);
        block.render(area, buf);
        // Over the border the block just drew: the corners give the panel its
        // colour, and the mark is what closes it.
        super::frame::render_corners(area, buf, t);
        super::header::render(area, self.header_items, buf, t);
        if inner.height == 0 {
            return;
        }

        if total == 0 {
            buf.set_string(
                inner.x + 2,
                inner.y + inner.height / 2,
                "nothing queued — pass a directory or playlist on the command line",
                Style::default().fg(rgb(t.empty_fg)),
            );
            return;
        }

        let height = inner.height as usize;
        // Dividers take rows, so what has to fit is the rows, not the tracks.
        let lines = self.rows.len();
        // Reserve the scrollbar column up front rather than drawing over the
        // rows afterwards, which was clipping the last digit of every duration.
        // A column of padding goes with it, so the duration never sits flush
        // against the border or against the scroll marker.
        let has_scrollbar = lines > height && inner.width > 2;
        let content_w = inner
            .width
            .saturating_sub(if has_scrollbar { 1 } else { 0 })
            .saturating_sub(RIGHT_PAD);

        for row in 0..height {
            let y = inner.y + row as u16;
            let i = match self.rows.rows().get(self.scroll + row) {
                None => break,
                Some(Row::Section {
                    label,
                    tracks,
                    folded,
                    playing,
                    ..
                }) => {
                    render_section(
                        inner.x, y, content_w, label, *tracks, *folded, *playing, buf, t,
                    );
                    continue;
                }
                Some(&Row::Track(i)) => i,
            };
            let Some(item) = self.items.get(i) else {
                continue;
            };
            let is_cursor = i == self.cursor;
            let is_playing = Some(i) == self.playing;
            let is_tagged = self.tagged.contains(&i);

            let (fg, bg) = if is_cursor && is_playing {
                (t.row_selected_fg, Some(t.row_selected_bg))
            } else if is_cursor {
                (
                    if self.focused {
                        t.row_selected_fg
                    } else {
                        t.row_cursor_fg
                    },
                    Some(if self.focused {
                        t.row_selected_bg
                    } else {
                        t.row_cursor_bg
                    }),
                )
            } else if is_playing {
                (t.row_playing_fg, t.row_playing_bg)
            } else if item.unplayable {
                (t.row_missing_fg, None)
            } else if item.uri.is_cue() {
                (t.row_virtual_fg, None)
            } else if is_tagged {
                (t.row_marked_fg, None)
            } else {
                (t.row_fg, None)
            };

            let mut style = Style::default().fg(rgb(fg));
            if let Some(b) = bg {
                style = style.bg(rgb(b));
            }
            if is_playing {
                style = style.add_modifier(Modifier::BOLD);
            }

            // A fixed one-column marker, so state changes never shift the
            // text. The playing row carries the transport's own play face
            // rather than an ASCII stand-in for it: it is the same statement
            // the play button is making, and it should look like it. The
            // cursor is shown by its bar, not by a mark.
            let marker = if is_playing {
                self.glyphs.play_mark()
            } else if item.unplayable {
                "!"
            } else {
                " "
            };

            let dur = item
                .duration_secs
                .map(|d| digits::clock(d as f64))
                .unwrap_or_else(|| "-:--".into());
            // The tag rides the index's full stop rather than the marker
            // column: same width, so nothing shifts; ASCII, so no font can
            // lose it the way `render_section` warns a lone symbol can; and it
            // leaves `!` and the play face the one column they have.
            let idx = format!("{:>4}{}", i + 1, if is_tagged { '+' } else { '.' });
            let name = match (&item.artist, &item.title) {
                (Some(a), Some(ti)) => format!("{a} — {ti}"),
                (None, Some(ti)) => ti.clone(),
                _ => item.uri.to_string(),
            };

            let avail = content_w as usize;
            let fixed = 1 + idx.len() + 1 + dur.len() + 1;
            let name_w = avail.saturating_sub(fixed);

            // Drawn in three pieces rather than as one string, so the index
            // and the duration can carry their own colours. The widths are
            // the same either way: the row is still marker, index, a padded
            // title, and the duration, filling `avail` exactly.
            let head = format!("{marker}{idx} ");
            let body = format!("{:<name_w$}", truncate(&name, name_w), name_w = name_w);
            let tail = format!(" {dur}");

            // A highlighted row keeps one colour throughout: two accents
            // inside a selection bar fight the selection rather than reading
            // as detail.
            let plain = !is_cursor && !is_playing;
            let part = |c: Rgb| {
                let mut st = Style::default().fg(rgb(if plain { c } else { fg }));
                if let Some(b) = bg {
                    st = st.bg(rgb(b));
                }
                if is_playing {
                    st = st.add_modifier(Modifier::BOLD);
                }
                st
            };

            // Paint the row's background across its whole width first, so a
            // selection bar does not stop where the text does.
            buf.set_string(inner.x, y, " ".repeat(avail), style);
            let mut x = inner.x;
            buf.set_string(
                x,
                y,
                &head,
                part(if is_tagged {
                    t.row_marked_fg
                } else {
                    t.row_index_fg
                }),
            );
            x += head.chars().count() as u16;
            buf.set_string(x, y, &body, style);
            x += name_w as u16;
            buf.set_string(x, y, &tail, part(t.row_duration_fg));
        }

        // Scrollbar, when it is worth having.
        //
        // The thumb only. A full-height track drew a second vertical line
        // right inside the panel border, which read as a doubled border rather
        // than as a position indicator. The column is still reserved either
        // way, so nothing shifts as the marker moves.
        //
        // Solid, and the colour of the border it sits against, so it reads as
        // a bead running down that border rather than as a separate widget.
        // It follows the border into focus for the same reason.
        if has_scrollbar {
            let x = inner.x + inner.width - 1;
            let bar = if self.focused {
                t.border_focused
            } else {
                t.border
            };
            // Position over the *scrollable* range, not over the track count.
            // `scroll / total` never reaches the bottom -- with 40 tracks in a
            // 28-row panel the marker stopped eight rows down at the end of
            // the list, which the full-height track used to disguise.
            let max_scroll = lines - height;
            let thumb = (self.scroll * (height - 1))
                .checked_div(max_scroll)
                .unwrap_or(0)
                .min(height - 1);
            buf[(x, inner.y + thumb as u16)]
                .set_char('█')
                .set_style(Style::default().fg(rgb(bar)));
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::playlist::queue::QueueItem;
    use crate::playlist::uri::TrackUri;
    use crate::theme::builtin;

    fn draw(total: usize, scroll: usize, height: u16) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let items: Vec<QueueItem> = (0..total)
            .map(|i| {
                let mut q = QueueItem::new(TrackUri::File {
                    rel_path: format!("track{i}.flac"),
                });
                q.title = Some(format!("Track {i}"));
                q.duration_secs = Some(90);
                q
            })
            .collect();
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        PlaylistView {
            theme: &theme,
            name: "test",
            items: &items,
            rows: &Rows::flat(items.len()),
            cursor: scroll,
            playing: None,
            scroll,
            focused: true,
            tagged: &Default::default(),
            glyphs: crate::ui::panels::player::Glyphs::default(),
            header_items: crate::ui::panels::header::WITH_FILTER,
        }
        .render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// The scrollbar column, top to bottom, inside the border.
    /// The first row of the list, and how many there are -- taken from
    /// `list_rect` rather than assumed, so a change to the panel's chrome
    /// moves the tests with it instead of breaking them.
    fn list_of(height: u16) -> Rect {
        list_rect(Rect::new(0, 0, 40, height))
    }

    fn marker_column(rows: &[String]) -> String {
        let x = rows[0].chars().count() - 2;
        let list = list_of(rows.len() as u16);
        rows[list.y as usize..(list.y + list.height) as usize]
            .iter()
            .map(|r| r.chars().nth(x).unwrap_or(' '))
            .collect()
    }

    /// The foreground colour of a cell, addressed by list row rather than by
    /// screen row.
    fn cell_fg(rows_of: usize, scroll: usize, height: u16, row: u16, x: u16) -> Color {
        let theme = builtin::load("cosmic").unwrap();
        let items: Vec<QueueItem> = (0..rows_of)
            .map(|i| {
                let mut q = QueueItem::new(TrackUri::File {
                    rel_path: format!("t{i}.flac"),
                });
                q.title = Some(format!("Track {i}"));
                q.duration_secs = Some(90);
                q
            })
            .collect();
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        PlaylistView {
            theme: &theme,
            name: "test",
            items: &items,
            rows: &Rows::flat(items.len()),
            cursor: 0,
            playing: None,
            scroll,
            focused: true,
            tagged: &Default::default(),
            glyphs: crate::ui::panels::player::Glyphs::default(),
            header_items: crate::ui::panels::header::WITH_FILTER,
        }
        .render(area, &mut buf);
        buf[(x, list_of(height).y + row)].style().fg.unwrap()
    }

    /// Nothing folded, nothing playing -- what most of these tests want.
    fn open(items: &[QueueItem]) -> Rows {
        Rows::grouped(items, &HashSet::new(), None)
    }

    /// Two records, already in album order, as the queue would hand them over.
    fn grouped_items() -> Vec<QueueItem> {
        let mut items = Vec::new();
        for (album, year, tracks) in [("Holy Land", 1996, 3), ("Chained", 2005, 2)] {
            for n in 1..=tracks {
                let mut q = QueueItem::new(TrackUri::File {
                    rel_path: format!("{album}/{n}.flac"),
                });
                q.title = Some(format!("{album} {n}"));
                q.album = Some(album.into());
                q.year = Some(year);
                q.track_no = Some(n);
                q.duration_secs = Some(90);
                items.push(q);
            }
        }
        items
    }

    fn grouped_draw(items: &[QueueItem], rows: &Rows, scroll: usize, height: u16) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        PlaylistView {
            theme: &theme,
            name: "test",
            items,
            rows,
            cursor: 0,
            playing: None,
            scroll,
            focused: true,
            tagged: &Default::default(),
            glyphs: crate::ui::panels::player::Glyphs::default(),
            header_items: crate::ui::panels::header::WITH_FILTER,
        }
        .render(area, &mut buf);
        (0..height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_divider_appears_at_each_change_of_record_and_nowhere_else() {
        let items = grouped_items();
        let rows = open(&items);
        assert_eq!(rows.len(), items.len() + 2, "one heading per record");
        let sections: Vec<&Row> = rows
            .rows()
            .iter()
            .filter(|r| matches!(r, Row::Section { .. }))
            .collect();
        assert_eq!(sections.len(), 2);
        let Row::Section { label, tracks, .. } = sections[0] else {
            unreachable!()
        };
        assert_eq!(label, "1996 \u{b7} Holy Land");
        assert_eq!(*tracks, 3);
        assert!(matches!(rows.rows()[0], Row::Section { .. }));
        assert_eq!(rows.rows()[1], Row::Track(0));
        assert!(matches!(rows.rows()[4], Row::Section { .. }));
    }

    #[test]
    fn a_divider_says_which_record_it_opens_and_how_long_it_is() {
        let items = grouped_items();
        let rows = open(&items);
        let drawn = grouped_draw(&items, &rows, 0, 12);
        let list = list_of(12);
        let head = &drawn[list.y as usize];
        assert!(head.contains("1996 \u{b7} Holy Land"), "{head:?}");
        assert!(head.contains("3 tracks"), "{head:?}");
        assert!(head.contains('\u{2500}'), "no rule drawn: {head:?}");
        // And the record with one track does not say "1 tracks".
        let mut one = grouped_items();
        one.truncate(4);
        let rows = open(&one);
        let drawn = grouped_draw(&one, &rows, 0, 12);
        let last = &drawn[(list.y + 4) as usize];
        assert!(
            last.contains("1 track") && !last.contains("1 tracks"),
            "{last:?}"
        );
    }

    #[test]
    fn a_click_lands_on_the_track_it_points_at_past_a_divider() {
        // The grouped version of the test below it, and the reason `Rows`
        // exists: with headings in the way, a row and a track are different
        // numbers, and the renderer and the mouse handler have to agree on
        // which is which.
        let items = grouped_items();
        let rows = open(&items);
        for scroll in 0..3 {
            let drawn = grouped_draw(&items, &rows, scroll, 9);
            let list = list_of(9);
            for row in 0..list.height as usize {
                let Some(line) = drawn.get(list.y as usize + row) else {
                    break;
                };
                let Some(track) = rows.track_at_or_after(scroll + row) else {
                    continue;
                };
                // A click on this screen row selects `track`; the row the
                // renderer drew there must be that track, or a heading above
                // it -- never a different track.
                match &rows.rows()[scroll + row] {
                    Row::Track(t) => {
                        assert_eq!(*t, track);
                        let n = t + 1;
                        assert!(
                            line.contains(&format!("{n}.")),
                            "scroll {scroll} row {row}: {line:?}"
                        );
                    }
                    Row::Section { label, .. } => {
                        assert!(line.contains(label.as_str()), "{line:?}");
                    }
                }
            }
        }
    }

    /// What the `n`th heading in a grouped list is folded by.
    fn fold_of(rows: &Rows, n: usize) -> String {
        rows.rows()
            .iter()
            .filter_map(|r| match r {
                Row::Section { fold, .. } => Some(fold.clone()),
                Row::Track(_) => None,
            })
            .nth(n)
            .unwrap()
    }

    #[test]
    fn a_folded_record_keeps_its_heading_and_loses_its_tracks() {
        let items = grouped_items();
        let all = open(&items);
        let folded = HashSet::from([fold_of(&all, 0)]);
        let rows = Rows::grouped(&items, &folded, None);

        assert_eq!(rows.len(), all.len() - 3, "the first record's three tracks");
        assert!(
            matches!(rows.rows()[0], Row::Section { folded: true, .. }),
            "{:?}",
            rows.rows()[0]
        );
        assert!(
            matches!(rows.rows()[1], Row::Section { folded: false, .. }),
            "the next heading should follow straight on"
        );
        for track in 0..3 {
            assert_eq!(
                rows.row_of_track(track),
                None,
                "track {track} is folded away"
            );
        }
        // And its tracks still know where their heading is, so the panel has
        // somewhere to scroll to for them.
        assert_eq!(rows.section_row(0), Some(0));
    }

    #[test]
    fn a_folded_record_says_how_much_it_is_hiding_and_whether_it_is_playing() {
        let items = grouped_items();
        let folded = HashSet::from([fold_of(&open(&items), 0)]);
        let rows = Rows::grouped(&items, &folded, Some(1));
        let drawn = grouped_draw(&items, &rows, 0, 12);
        let head = &drawn[list_of(12).y as usize];
        assert!(head.contains("3 hidden"), "{head:?}");
        assert!(
            head.contains("1996 \u{b7} Holy Land"),
            "the record is still named: {head:?}"
        );
        // In the marker column the track rows use, one cell inside the border.
        assert_eq!(
            head.chars().nth(1),
            Some('>'),
            "a folded record hides the playing track, so it says so: {head:?}"
        );

        // Open, the track carries the marker and the heading must not: two in
        // a column reads as two things playing.
        let open_rows = Rows::grouped(&items, &HashSet::new(), Some(1));
        let drawn = grouped_draw(&items, &open_rows, 0, 12);
        let head = &drawn[list_of(12).y as usize];
        assert_eq!(head.chars().nth(1), Some('\u{2500}'), "{head:?}");
    }

    #[test]
    fn the_cursor_steps_over_a_folded_record_rather_than_into_it() {
        // Otherwise an arrow key looks like it did nothing, three times, and
        // then jumped.
        let items = grouped_items();
        let all = open(&items);
        let folded = HashSet::from([fold_of(&all, 0)]);
        let rows = Rows::grouped(&items, &folded, None);

        // Tracks 0..3 are hidden; 3 and 4 are what is left.
        assert_eq!(rows.ends(), Some((3, 4)));
        assert_eq!(rows.step(3, 1), 4);
        assert_eq!(rows.step(4, 1), 4, "stops at the end");
        assert_eq!(rows.step(3, -1), 3, "and at the start");
        // A cursor left inside the folded record comes back out.
        assert_eq!(rows.nearest_shown(1), Some(3));
        assert_eq!(rows.nearest_shown(4), Some(4), "one already on show stays");
    }

    #[test]
    fn folding_everything_leaves_a_list_of_records() {
        let items = grouped_items();
        let all = open(&items);
        let folded = HashSet::from([fold_of(&all, 0), fold_of(&all, 1)]);
        let rows = Rows::grouped(&items, &folded, None);
        assert_eq!(rows.len(), 2, "two headings and nothing else");
        assert!(rows.rows().iter().all(|r| matches!(r, Row::Section { .. })));
        assert_eq!(rows.ends(), None, "nothing to put a cursor on");
        assert_eq!(
            rows.step(2, 1),
            2,
            "and stepping is a no-op rather than a panic"
        );
        assert_eq!(rows.nearest_shown(2), None);
    }

    #[test]
    fn an_ungrouped_list_folds_nothing_and_steps_plainly() {
        let rows = Rows::flat(5);
        assert_eq!(rows.ends(), Some((0, 4)));
        assert_eq!(rows.step(0, 3), 3);
        assert_eq!(rows.step(0, 99), 4);
        assert_eq!(rows.step(4, -99), 0);
        assert_eq!(rows.section_row(2), None);
    }

    #[test]
    fn every_track_maps_to_a_row_and_back() {
        let items = grouped_items();
        for rows in [open(&items), Rows::flat(items.len())] {
            for track in 0..items.len() {
                let row = rows.row_of_track(track).unwrap();
                assert_eq!(rows.track_at_or_after(row), Some(track));
                assert_eq!(rows.rows()[row], Row::Track(track));
            }
            assert_eq!(rows.track_at_or_after(rows.len()), None, "past the end");
        }
    }

    #[test]
    fn a_click_on_a_divider_takes_the_record_under_it() {
        let items = grouped_items();
        let rows = open(&items);
        assert_eq!(rows.track_at_or_after(0), Some(0));
        assert_eq!(rows.track_at_or_after(4), Some(3), "the second heading");
    }

    #[test]
    fn scrolling_to_the_first_track_of_a_record_brings_its_name_along() {
        let items = grouped_items();
        let rows = open(&items);
        // Track 3 opens the second record, whose heading is the row above it.
        assert_eq!(rows.row_of_track(3), Some(5));
        assert_eq!(rows.anchor_row(3), 4);
        // A track in the middle of a record anchors on itself.
        assert_eq!(rows.anchor_row(1), rows.row_of_track(1).unwrap());
    }

    #[test]
    fn the_scrollbar_measures_the_rows_it_has_to_show() {
        // Five tracks fit in the list; five tracks and two headings do not.
        let items = grouped_items();
        let height = 9u16;
        assert_eq!(
            list_of(height).height as usize,
            6,
            "the list this test needs"
        );
        let flat = grouped_draw(&items, &Rows::flat(items.len()), 0, height);
        let grouped = grouped_draw(&items, &open(&items), 0, height);
        let bar = |rows: &[String]| {
            let list = list_of(height);
            (list.y..list.y + list.height)
                .filter(|y| rows[*y as usize].contains('\u{2588}'))
                .count()
        };
        assert_eq!(bar(&flat), 0, "everything fits, so no marker");
        assert_eq!(bar(&grouped), 1, "the headings push it over");
    }

    #[test]
    fn the_count_ends_clear_of_the_corner() {
        // A border character between the count and the corner, matching the
        // one between the corner and the title at the other end. The close
        // mark that used to be reserved a slot here is gone; the actions live
        // on the header row now.
        let rows = draw(3, 0, 9);
        let top = &rows[0];
        assert!(top.contains("1/3"), "the count is gone: {top:?}");
        assert!(
            top.trim_end().ends_with("\u{2550}\u{2557}"),
            "no buffer before the right corner: {top:?}"
        );
        assert!(!top.contains('X'), "the close mark should be gone: {top:?}");
    }

    #[test]
    fn a_click_lands_on_the_track_it_points_at() {
        // The renderer and the mouse handler both take the list area from
        // `list_rect`. If they ever stopped agreeing, a click would select the
        // row above the one under the pointer.
        let area = Rect::new(0, 0, 40, 9);
        let list = list_rect(area);
        let rows = draw(6, 0, 9);
        for row in 0..list.height {
            let drawn = &rows[(list.y + row) as usize];
            let n = row + 1;
            assert!(
                drawn.contains(&format!("{n}.")),
                "row {row} of the list should be track {n}: {drawn:?}"
            );
        }
    }

    #[test]
    fn the_playing_row_is_marked_with_the_transport_own_play_face() {
        // The same statement the play button is making, so it looks like it --
        // and it follows `[ui] glyphs` rather than being an ASCII stand-in
        // that disagrees with the button two panels up.
        let items = grouped_items();
        let rows = Rows::flat(items.len());
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 40, 12);
        let list = list_of(12);

        let glyphs = crate::ui::panels::player::Glyphs::default();
        let mut buf = Buffer::empty(area);
        PlaylistView {
            theme: &theme,
            name: "test",
            items: &items,
            rows: &rows,
            cursor: 2,
            playing: Some(0),
            scroll: 0,
            focused: true,
            tagged: &Default::default(),
            glyphs,
            header_items: crate::ui::panels::header::WITH_FILTER,
        }
        .render(area, &mut buf);

        let at = |row: u16| buf[(list.x, list.y + row)].symbol().to_string();
        assert_eq!(at(0), glyphs.play_mark(), "the playing row");
        // One column wide however wide the button it came from is, so
        // nothing in the row shifts.
        assert_eq!(glyphs.play_mark().chars().count(), 1);
        // The cursor is shown by its bar, not by a mark of its own.
        assert_eq!(at(2), " ", "the cursor row");
        assert_eq!(at(1), " ", "an ordinary row");
    }

    #[test]
    fn the_cursor_row_is_a_bar_darker_than_the_panel_it_sits_in() {
        // Dark enough to read as the current row rather than as a hole cut in
        // the list, and still plainly a bar.
        let theme = builtin::load("cosmic").unwrap();
        assert_ne!(theme.row_selected_bg, theme.panel_bg);
        let lift = |c: crate::theme::color::Rgb| c.to_oklab().l;
        assert!(
            lift(theme.row_selected_bg) < lift(theme.row_selected_fg),
            "the bar should be darker than the text on it"
        );
    }

    #[test]
    fn the_index_and_duration_carry_their_own_colour() {
        let theme = builtin::load("cosmic").unwrap();
        // The second row of the list is an ordinary one; the first is the cursor.
        let index = cell_fg(6, 0, 9, 1, 3);
        let title = cell_fg(6, 0, 9, 1, 10);
        let duration = cell_fg(6, 0, 8, 2, 36);
        assert_eq!(index, rgb(theme.row_index_fg), "the index is not its own");
        assert_eq!(duration, rgb(theme.row_duration_fg), "so is the duration");
        assert_ne!(index, title, "index and title should differ");
    }

    #[test]
    fn a_selected_row_keeps_one_colour_throughout() {
        // Two accents inside a selection bar fight the selection rather than
        // reading as detail.
        let index = cell_fg(6, 0, 9, 0, 3);
        let title = cell_fg(6, 0, 9, 0, 10);
        let duration = cell_fg(6, 0, 9, 0, 36);
        assert_eq!(index, title, "the cursor row broke into pieces");
        assert_eq!(duration, title);
    }

    #[test]
    fn the_duration_does_not_touch_the_border() {
        // Short enough not to scroll, so the only thing to the right of the
        // duration is the panel border.
        let rows = draw(3, 0, 9);
        let row = &rows[list_of(9).y as usize];
        let chars: Vec<char> = row.chars().collect();
        let n = chars.len();
        assert_eq!(
            chars[n - 1],
            '\u{2551}',
            "expected the border last: {row:?}"
        );
        assert_eq!(
            chars[n - 2],
            ' ',
            "the duration is flush to the border: {row:?}"
        );
        assert!(
            chars[n - 3].is_ascii_digit(),
            "the duration should sit one column in: {row:?}"
        );
    }

    #[test]
    fn the_duration_does_not_touch_the_scroll_marker() {
        let rows = draw(200, 0, 9);
        // The first row of the list carries the marker at this scroll position.
        let first = list_of(9).y as usize;
        let chars: Vec<char> = rows[first].chars().collect();
        let n = chars.len();
        assert_eq!(
            chars[n - 2],
            '\u{2588}',
            "expected the marker: {:?}",
            rows[first]
        );
        assert_eq!(
            chars[n - 3],
            ' ',
            "the duration is flush to the marker: {:?}",
            rows[first]
        );
    }

    #[test]
    fn the_scrollbar_is_a_marker_not_a_second_border() {
        let rows = draw(200, 0, 10);
        let col = marker_column(&rows);
        assert_eq!(col.matches('█').count(), 1, "expected one marker: {col:?}");
        assert!(
            !col.contains('│'),
            "the track line is back, and reads as a doubled border: {col:?}"
        );
        assert!(
            col.chars().filter(|c| *c != '█').all(|c| c == ' '),
            "the column should be otherwise empty: {col:?}"
        );
    }

    #[test]
    fn the_marker_follows_the_scroll_position() {
        let top = marker_column(&draw(200, 0, 12));
        let bottom = marker_column(&draw(200, 190, 12));
        assert_eq!(top.find('█'), Some(0), "at the top: {top:?}");
        assert!(
            bottom.find('█').unwrap() > top.find('█').unwrap(),
            "scrolling down should move the marker down: {bottom:?}"
        );
    }

    #[test]
    fn the_marker_reaches_the_bottom_at_the_end_of_the_list() {
        // 40 tracks in a 28-row viewport: the last scroll position shows the
        // final track, so the marker belongs on the last row.
        let height = 30u16;
        let visible = list_of(height).height as usize;
        let total = 40;
        let col = marker_column(&draw(total, total - visible, height));
        assert_eq!(
            col.find('█'),
            Some(visible - 1),
            "the marker never reaches the bottom: {col:?}"
        );
    }

    #[test]
    fn a_list_that_fits_has_no_scrollbar_at_all() {
        let col = marker_column(&draw(3, 0, 10));
        assert!(!col.contains('█'), "nothing to scroll: {col:?}");
    }
    #[test]
    fn the_marker_cell_is_the_playing_row_when_it_is_on_screen() {
        let rows = Rows::flat(20);
        let area = Rect::new(0, 0, 40, 12);
        let list = list_of(12);
        assert_eq!(
            marker_cells(area, &rows, 0, Some(2)),
            vec![(list.x, list.y + 2)]
        );
        // Scrolled past it: nothing to mark.
        assert!(marker_cells(area, &rows, 5, Some(2)).is_empty());
        // Scrolled to it: relative to the top of the list.
        assert_eq!(
            marker_cells(area, &rows, 2, Some(2)),
            vec![(list.x, list.y)]
        );
        assert!(marker_cells(area, &rows, 0, None).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::PlaylistView as V;

    #[test]
    fn scrolling_follows_the_cursor_down() {
        assert_eq!(V::clamp_scroll(0, 0, 10), 0);
        assert_eq!(V::clamp_scroll(5, 0, 10), 0, "still visible, do not move");
        assert_eq!(V::clamp_scroll(10, 0, 10), 1, "one past the bottom");
        assert_eq!(V::clamp_scroll(50, 0, 10), 41);
    }

    #[test]
    fn scrolling_follows_the_cursor_up() {
        assert_eq!(V::clamp_scroll(3, 10, 10), 3);
        assert_eq!(V::clamp_scroll(0, 30, 10), 0);
    }

    #[test]
    fn a_zero_height_pane_does_not_panic() {
        assert_eq!(V::clamp_scroll(5, 2, 0), 0);
    }
    /// The mark, and the three ways it must not be lost.
    mod tagging {
        use super::super::*;
        use crate::playlist::queue::QueueItem;
        use crate::playlist::uri::TrackUri;
        use crate::theme::resolve::Theme;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;
        use ratatui::widgets::Widget;
        use std::collections::HashSet;

        fn draw_tagged(
            tagged: &HashSet<usize>,
            cursor: usize,
            playing: Option<usize>,
        ) -> (Buffer, Rect, Theme) {
            let theme = crate::theme::builtin::load("cosmic").unwrap();
            let items: Vec<QueueItem> = (0..4)
                .map(|i| {
                    let mut q = QueueItem::new(TrackUri::File {
                        rel_path: format!("t{i}.flac"),
                    });
                    q.title = Some(format!("Track {i}"));
                    q.duration_secs = Some(90);
                    q
                })
                .collect();
            let area = Rect::new(0, 0, 44, 10);
            let mut buf = Buffer::empty(area);
            PlaylistView {
                theme: &theme,
                name: "test",
                items: &items,
                rows: &Rows::flat(items.len()),
                cursor,
                playing,
                scroll: 0,
                focused: true,
                tagged,
                glyphs: crate::ui::panels::player::Glyphs::default(),
                header_items: crate::ui::panels::header::WITH_FILTER,
            }
            .render(area, &mut buf);
            (buf, list_rect(area), theme)
        }

        /// The mark rides the index's full stop, not the marker column.
        #[test]
        fn a_tagged_row_says_so_where_the_full_stop_goes() {
            let tagged: HashSet<usize> = [1].into_iter().collect();
            let (buf, list, theme) = draw_tagged(&tagged, 3, None);
            // Row 1 tagged, row 0 not: `+` against `.`, same width.
            assert_eq!(buf[(list.x + 5, list.y + 1)].symbol(), "+");
            assert_eq!(buf[(list.x + 5, list.y)].symbol(), ".");
            // And the marker column is left to `!` and the play face.
            assert_eq!(buf[(list.x, list.y + 1)].symbol(), " ");
            // `row_marked_fg` gets its first reader.
            assert_eq!(
                buf[(list.x + 3, list.y + 1)].style().fg,
                Some(Color::Rgb(
                    theme.row_marked_fg.r,
                    theme.row_marked_fg.g,
                    theme.row_marked_fg.b
                ))
            );
        }

        /// A file that is missing says so whatever you have marked.
        ///
        /// Intent must not outrank state: `!` and `row_missing_fg` are facts
        /// about the file you cannot recover any other way, and a tag is your
        /// own doing, which you already know about.
        #[test]
        fn a_tagged_unplayable_row_keeps_its_warning() {
            let theme = crate::theme::builtin::load("cosmic").unwrap();
            let mut items: Vec<QueueItem> = (0..2)
                .map(|i| {
                    let mut q = QueueItem::new(TrackUri::File {
                        rel_path: format!("t{i}.flac"),
                    });
                    q.title = Some(format!("Track {i}"));
                    q
                })
                .collect();
            items[0].unplayable = true;
            let area = Rect::new(0, 0, 44, 8);
            let mut buf = Buffer::empty(area);
            let tagged: HashSet<usize> = [0].into_iter().collect();
            PlaylistView {
                theme: &theme,
                name: "test",
                items: &items,
                rows: &Rows::flat(items.len()),
                cursor: 1,
                playing: None,
                scroll: 0,
                focused: true,
                tagged: &tagged,
                glyphs: crate::ui::panels::player::Glyphs::default(),
                header_items: crate::ui::panels::header::WITH_FILTER,
            }
            .render(area, &mut buf);
            let list = list_rect(area);
            assert_eq!(buf[(list.x, list.y)].symbol(), "!", "the warning was lost");
            assert_eq!(
                buf[(list.x + 8, list.y)].style().fg,
                Some(Color::Rgb(
                    theme.row_missing_fg.r,
                    theme.row_missing_fg.g,
                    theme.row_missing_fg.b
                )),
                "a tag repainted a broken file as healthy"
            );
            // It still says it is tagged, in the column that is free.
            assert_eq!(buf[(list.x + 5, list.y)].symbol(), "+");
        }

        /// Both claims on one row, in two different places.
        #[test]
        fn a_tagged_playing_row_shows_the_play_face_and_the_tag() {
            let tagged: HashSet<usize> = [0].into_iter().collect();
            let (buf, list, _) = draw_tagged(&tagged, 3, Some(0));
            let play = crate::ui::panels::player::Glyphs::default().play_mark();
            assert_eq!(buf[(list.x, list.y)].symbol(), play);
            assert_eq!(
                buf[(list.x + 5, list.y)].symbol(),
                "+",
                "the tag was invisible on the playing row"
            );
        }

        /// Three claims, three channels, none lost.
        #[test]
        fn a_tagged_row_under_the_cursor_keeps_one_colour_and_still_shows_the_mark() {
            let tagged: HashSet<usize> = [2].into_iter().collect();
            let (buf, list, theme) = draw_tagged(&tagged, 2, None);
            assert_eq!(buf[(list.x + 5, list.y + 2)].symbol(), "+");
            let bar = Some(Color::Rgb(
                theme.row_selected_bg.r,
                theme.row_selected_bg.g,
                theme.row_selected_bg.b,
            ));
            assert_eq!(buf[(list.x, list.y + 2)].style().bg, bar);
            // One colour throughout, as the selection rule demands: the index
            // does not keep the marked accent inside a bar.
            let at = |dx: u16| buf[(list.x + dx, list.y + 2)].style().fg;
            assert_eq!(at(3), at(8), "two accents inside the selection bar");
        }

        /// ASCII, so no font can lose it -- the lesson `render_section` records.
        #[test]
        fn the_tag_survives_every_glyph_set() {
            let glyphs = crate::ui::panels::player::Glyphs::default();
            let theme = crate::theme::builtin::load("cosmic").unwrap();
            let items: Vec<QueueItem> = (0..2)
                .map(|i| {
                    QueueItem::new(TrackUri::File {
                        rel_path: format!("t{i}.flac"),
                    })
                })
                .collect();
            let area = Rect::new(0, 0, 44, 8);
            let mut buf = Buffer::empty(area);
            let tagged: HashSet<usize> = [0].into_iter().collect();
            PlaylistView {
                theme: &theme,
                name: "test",
                items: &items,
                rows: &Rows::flat(items.len()),
                cursor: 1,
                playing: None,
                scroll: 0,
                focused: true,
                tagged: &tagged,
                glyphs,
                header_items: crate::ui::panels::header::WITH_FILTER,
            }
            .render(area, &mut buf);
            let list = list_rect(area);
            assert_eq!(buf[(list.x + 5, list.y)].symbol(), "+");
        }

        #[test]
        fn nothing_tagged_draws_exactly_what_it_always_did() {
            let none = HashSet::new();
            let (buf, list, _) = draw_tagged(&none, 1, None);
            for row in 0..4u16 {
                assert_eq!(buf[(list.x, list.y + row)].symbol(), " ");
                assert_eq!(buf[(list.x + 5, list.y + row)].symbol(), ".");
            }
        }
    }
}
