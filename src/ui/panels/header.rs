//! The row of actions at the top of a docked panel.
//!
//! Two words, `settings` and `close`, right-aligned on the first row inside the
//! border. They replace the single `X` that used to sit on the border itself,
//! which took several attempts to get right: a lone glyph is at the mercy of
//! whichever font the terminal falls back to for it, at whatever size and
//! weight that font happens to have. Words are drawn in the same face as the
//! text beside them and cannot go wrong that way.
//!
//! Same doctrine as the rest of the panel chrome: one function decides where
//! the words are, the renderer draws from it and the mouse tests against it.
//! A hit box computed separately from what it points at is the bug this
//! arrangement makes impossible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};

use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;

fn rgb(c: Rgb) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(c.r, c.g, c.b)
}

/// What a click on the header asked for.
///
/// Which of these a panel offers is the panel's business -- only the playlist
/// has anything to filter -- so every entry point takes the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Filter,
    Settings,
    Close,
    /// How many rows are tagged. Not clickable; it is the mode's name.
    Tagged(usize),
    Copy,
    Put(usize),
    Move,
    Remove,
    Untag,
}

impl Item {
    /// The word as drawn.
    ///
    /// `Cow` rather than `&'static str` because the tagging words carry counts,
    /// and a count that is not on the word is a count nobody can see.
    pub fn word(self) -> std::borrow::Cow<'static, str> {
        match self {
            Item::Filter => "filter".into(),
            Item::Settings => "settings".into(),
            Item::Close => "close".into(),
            Item::Tagged(n) => format!("{n} tagged").into(),
            Item::Copy => "y copy".into(),
            Item::Put(n) => format!("u put {n}").into(),
            Item::Move => "m move".into(),
            Item::Remove => "D remove".into(),
            Item::Untag => "T untag".into(),
        }
    }

    fn width(self) -> u16 {
        self.word().chars().count() as u16
    }
}

/// The words a panel that can only be closed and configured offers.
pub const PLAIN: &[Item] = &[Item::Settings, Item::Close];

/// The playlist's, which can also be reordered.
pub const WITH_FILTER: &[Item] = &[Item::Filter, Item::Settings, Item::Close];

/// The playlist's header words, given what is tagged and what was copied.
///
/// Tagging takes the row over. The words that were there are about the panel,
/// and while rows are marked the question is what to do with them -- so the
/// header says so, at the top, where it does not scroll away under the list it
/// is talking about. Clearing the tags gives the panel back.
///
/// One function because the renderer and the mouse handler must be looking at
/// the same list: the counts change the widths, and a slice built twice puts
/// the hit boxes somewhere the words are not.
pub fn playlist_words(tagged: usize, copied: usize) -> Vec<Item> {
    if tagged == 0 {
        return WITH_FILTER.to_vec();
    }
    // Leftmost goes first when the panel narrows, so the count -- which is the
    // one thing repeated in the status note -- leads, and `D remove` and
    // `T untag`, the way back out, are the last to go.
    let mut out = vec![Item::Tagged(tagged), Item::Copy];
    if copied > 0 {
        out.push(Item::Put(copied));
    }
    out.push(Item::Move);
    out.push(Item::Remove);
    out.push(Item::Untag);
    out
}

/// Rows the header costs a panel.
pub const ROWS: u16 = 1;

/// Blank columns between words.
const GAP: u16 = 2;

/// Blank columns kept to the right, matching the playlist's own right padding
/// so the words line up with the durations under them.
const RIGHT_PAD: u16 = 1;

/// The header row: the first row inside the border.
pub fn rect(area: Rect) -> Rect {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    Rect {
        height: inner.height.min(ROWS),
        ..inner
    }
}

/// What the panel has left for its own content.
///
/// Every panel takes its content area from here rather than from
/// `block.inner(area)`, so the one-row offset lives in one place. Three
/// separate derivations of the playlist's offset is exactly how a click comes
/// to select the row above the one it landed on.
pub fn body(area: Rect) -> Rect {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    // A panel with no room for a body gets an empty rect inside itself rather
    // than one starting past its own bottom edge. `Block::inner` moves the
    // corner down whether or not there was anything to move it into.
    let bottom = area.y.saturating_add(area.height);
    Rect {
        y: inner.y.saturating_add(ROWS).min(bottom),
        height: inner.height.saturating_sub(ROWS),
        ..inner
    }
}

/// Columns a run of words needs, including the gaps between them.
fn width_of(items: &[Item]) -> u16 {
    let words: u16 = items.iter().map(|i| i.width()).sum();
    words + GAP * items.len().saturating_sub(1) as u16
}

/// Where each word sits, right to left; empty when none of them fit.
///
/// Words are dropped from the left until the rest fit, rather than the header
/// vanishing whole. A panel too narrow for `filter settings close` still has
/// room for `close`, and losing the way to close a panel because it got narrow
/// would be a worse answer than losing the way to reorder it.
///
/// The renderer and the mouse handler both come through here, so a word that
/// was never drawn cannot be clicked.
pub fn slots(area: Rect, items: &[Item]) -> Vec<(Item, Rect)> {
    let row = rect(area);
    if row.height == 0 {
        return Vec::new();
    }
    // A leading column as well, so the words never run into the left border.
    let mut kept = items;
    while !kept.is_empty() && row.width < 1 + width_of(kept) + RIGHT_PAD {
        kept = &kept[1..];
    }
    if kept.is_empty() {
        // Nothing fits, and the walk below would step off the left edge of a
        // panel too narrow to hold even the padding.
        return Vec::new();
    }

    let mut out = Vec::with_capacity(kept.len());
    let mut right = row.x + row.width - RIGHT_PAD;
    for item in kept.iter().rev() {
        right -= item.width();
        out.push((
            *item,
            Rect {
                x: right,
                y: row.y,
                width: item.width(),
                height: 1,
            },
        ));
        right = right.saturating_sub(GAP);
    }
    out.reverse();
    out
}

/// Which word a click landed on, if any.
pub fn hit(area: Rect, items: &[Item], x: u16, y: u16) -> Option<Item> {
    slots(area, items)
        .into_iter()
        .find(|(_, r)| y == r.y && x >= r.x && x < r.x + r.width)
        .map(|(item, _)| item)
}

/// Draw the header.
///
/// In `t.dim`, the same weight as the track count that used to sit beside the
/// close mark. Chrome should read as chrome.
pub fn render(area: Rect, items: &[Item], buf: &mut Buffer, t: &Theme) {
    let style = Style::default().fg(rgb(t.dim));
    for (item, r) in slots(area, items) {
        buf.set_string(r.x, r.y, item.word().as_ref(), style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;
    use ratatui::widgets::Widget;

    /// Draw a panel with a header and return its rows as text.
    fn draw(w: u16, h: u16, items: &[Item]) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        Block::default()
            .borders(Borders::ALL)
            .render(area, &mut buf);
        render(area, items, &mut buf, &theme);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn under(rows: &[String], r: Rect) -> String {
        rows[r.y as usize]
            .chars()
            .skip(r.x as usize)
            .take(r.width as usize)
            .collect()
    }

    #[test]
    fn each_word_is_where_its_hit_box_says_it_is() {
        // The whole reason both come from `slots`. A click that misses what it
        // is pointing at is worse than no click at all.
        for items in [PLAIN, WITH_FILTER] {
            for (w, h) in [(40u16, 9u16), (60, 9), (100, 24), (24, 9)] {
                let area = Rect::new(0, 0, w, h);
                let rows = draw(w, h, items);
                for (item, r) in slots(area, items) {
                    assert_eq!(under(&rows, r), item.word(), "{items:?} at {w}x{h}");
                }
            }
        }
    }

    #[test]
    fn a_click_on_a_word_reports_that_word() {
        let area = Rect::new(0, 0, 60, 9);
        let placed = slots(area, WITH_FILTER);
        assert_eq!(placed.len(), 3, "all three fit at 60 columns");
        for (item, r) in &placed {
            assert_eq!(hit(area, WITH_FILTER, r.x, r.y), Some(*item));
            assert_eq!(hit(area, WITH_FILTER, r.x + r.width - 1, r.y), Some(*item));
            // The gap in front of it belongs to nothing.
            assert_eq!(hit(area, WITH_FILTER, r.x - 1, r.y), None);
            // Nor does the row below.
            assert_eq!(hit(area, WITH_FILTER, r.x, r.y + 1), None);
        }
        assert_eq!(hit(area, WITH_FILTER, area.x + 1, placed[0].1.y), None);
        // And a panel that does not offer `filter` does not answer for it.
        assert_ne!(
            hit(area, PLAIN, placed[0].1.x, placed[0].1.y),
            Some(Item::Filter)
        );
    }

    #[test]
    fn a_narrow_panel_keeps_close_and_drops_the_rest() {
        // The header used to vanish whole below its full width. Losing the way
        // to close a panel because it got narrow is a worse answer than losing
        // the way to reorder it.
        let mut seen: Vec<usize> = Vec::new();
        for w in 0..=30u16 {
            let area = Rect::new(0, 0, w, 9);
            let placed = slots(area, WITH_FILTER);
            seen.push(placed.len());
            let rows = draw(w, 9, WITH_FILTER);
            let all = rows.join("");
            for item in WITH_FILTER {
                let drawn = placed.iter().any(|(i, _)| i == item);
                assert_eq!(
                    all.contains(item.word().as_ref()),
                    drawn,
                    "{:?} at width {w}",
                    item.word()
                );
            }
            // Whatever survives, `close` is in it.
            if let Some((first, _)) = placed.first() {
                assert_eq!(placed.last().unwrap().0, Item::Close, "at width {w}");
                assert!(matches!(first, Item::Filter | Item::Settings | Item::Close));
            }
            // And nothing is claimed that was not drawn.
            for x in 0..w {
                if let Some(item) = hit(area, WITH_FILTER, x, 1) {
                    assert!(placed.iter().any(|(i, _)| *i == item), "at width {w}");
                }
            }
        }
        assert!(
            seen.contains(&0) && seen.contains(&1) && seen.contains(&2) && seen.contains(&3),
            "every step of the ladder should be reachable: {seen:?}"
        );
    }

    #[test]
    fn the_body_starts_below_the_header() {
        let area = Rect::new(0, 0, 60, 9);
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let b = body(area);
        assert_eq!(b.y, inner.y + ROWS);
        assert_eq!(b.height, inner.height - ROWS);
        assert_eq!(b.x, inner.x);
        assert_eq!(b.width, inner.width);
        // And the header sits exactly where the body is not.
        assert_eq!(rect(area).y + rect(area).height, b.y);
    }

    #[test]
    fn a_panel_with_no_room_has_no_body_rather_than_a_wrapped_one() {
        for h in 0..=3u16 {
            let area = Rect::new(0, 0, 60, h);
            let b = body(area);
            let inner = Block::default().borders(Borders::ALL).inner(area);
            assert!(
                b.height <= inner.height,
                "body taller than the panel at {h}"
            );
            assert!(
                b.y + b.height <= area.y + area.height,
                "body escapes at {h}"
            );
        }
    }
    #[test]
    fn tagging_takes_the_header_over_and_gives_it_back() {
        // The words that were there are about the panel; while rows are
        // marked the question is what to do with them.
        assert_eq!(playlist_words(0, 0), WITH_FILTER.to_vec());
        let tagged = playlist_words(3, 0);
        assert!(!tagged.contains(&Item::Filter), "{tagged:?}");
        assert_eq!(tagged[0], Item::Tagged(3));
        assert!(tagged.contains(&Item::Remove) && tagged.contains(&Item::Untag));
    }

    #[test]
    fn the_put_word_only_appears_once_something_is_copied() {
        assert!(!playlist_words(2, 0)
            .iter()
            .any(|i| matches!(i, Item::Put(_))));
        assert!(playlist_words(2, 5).contains(&Item::Put(5)));
    }

    #[test]
    fn the_words_carry_their_counts() {
        assert_eq!(Item::Tagged(1).word(), "1 tagged");
        assert_eq!(Item::Tagged(793).word(), "793 tagged");
        assert_eq!(Item::Put(6).word(), "u put 6");
    }

    #[test]
    fn every_tagging_word_is_where_its_hit_box_says_it_is() {
        // The counts change the widths, so this is the arrangement most able
        // to put a hit box beside its word rather than on it.
        let theme = builtin::load("cosmic").unwrap();
        for tagged in [1usize, 9, 42, 793] {
            let words = playlist_words(tagged, 3);
            let area = Rect::new(0, 0, 100, 6);
            let mut buf = Buffer::empty(area);
            render(area, &words, &mut buf, &theme);
            for (item, r) in slots(area, &words) {
                let drawn: String = (0..r.width)
                    .map(|dx| buf[(r.x + dx, r.y)].symbol().to_string())
                    .collect();
                assert_eq!(drawn, item.word().as_ref(), "{item:?} at {tagged} tagged");
                assert_eq!(hit(area, &words, r.x, r.y), Some(item));
            }
        }
    }

    #[test]
    fn a_narrow_panel_keeps_the_way_back_out() {
        // `slots` drops from the left, so the count goes first and the two
        // words that end tagging -- remove and untag -- are the last to go.
        let words = playlist_words(3, 0);
        for w in 12..60u16 {
            let kept: Vec<Item> = slots(Rect::new(0, 0, w, 6), &words)
                .into_iter()
                .map(|(i, _)| i)
                .collect();
            if kept.len() == 1 {
                assert_eq!(kept[0], Item::Untag, "at {w} columns");
            }
            if !kept.is_empty() {
                assert_eq!(*kept.last().unwrap(), Item::Untag, "at {w} columns");
            }
        }
    }
}
