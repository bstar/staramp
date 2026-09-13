//! Which words this player's panels offer at the top of their frames.
//!
//! The placement, the hit boxes and the drawing are `starkit::chrome::header`,
//! which is generic over the word list; what is here is the list itself. They
//! are re-exported so that a panel calls `header::render` and `header::hit` as
//! it always did.

pub use starkit::chrome::header::{body, hit, rect, render, slots, Word, ROWS};

/// What a click on the header asked for.
///
/// Which of these a panel offers is the panel's business -- only the playlist
/// has anything to filter -- so every entry point takes the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Filter,
    Gravity,
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

impl Word for Item {
    /// The word as drawn.
    ///
    /// `Cow` rather than `&'static str` because the tagging words carry counts,
    /// and a count that is not on the word is a count nobody can see.
    fn word(self) -> std::borrow::Cow<'static, str> {
        match self {
            Item::Filter => "sorting".into(),
            Item::Gravity => "gravity".into(),
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
}

/// The words a panel that can only be closed and configured offers.
pub const PLAIN: &[Item] = &[Item::Settings, Item::Close];

/// The playlist's, which can also be reordered.
pub const WITH_FILTER: &[Item] = &[Item::Filter, Item::Gravity, Item::Settings, Item::Close];

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Block, Borders, Widget};

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
        assert_eq!(placed.len(), 4, "all four fit at 60 columns");
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
        for w in 0..=50u16 {
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
                assert!(matches!(
                    first,
                    Item::Filter | Item::Gravity | Item::Settings | Item::Close
                ));
            }
            // And nothing is claimed that was not drawn.
            for x in 0..w {
                if let Some(item) = hit(area, WITH_FILTER, x, 1) {
                    assert!(placed.iter().any(|(i, _)| *i == item), "at width {w}");
                }
            }
        }
        assert!(
            seen.contains(&0)
                && seen.contains(&1)
                && seen.contains(&2)
                && seen.contains(&3)
                && seen.contains(&4),
            "every step of the ladder should be reachable: {seen:?}"
        );
    }

    #[test]
    fn tagging_takes_the_header_over_and_gives_it_back() {
        // The words that were there are about the panel; while rows are
        // marked the question is what to do with them.
        assert_eq!(playlist_words(0, 0), WITH_FILTER.to_vec());
        let tagged = playlist_words(3, 0);
        assert!(!tagged.contains(&Item::Filter), "{tagged:?}");
        assert!(!tagged.contains(&Item::Gravity), "{tagged:?}");
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
