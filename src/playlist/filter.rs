//! Narrowing the playlist to the rows that match some words.
//!
//! Plain words, not the query language: `/` is for finding the record you can
//! half remember while the music plays, and `black sab 1970` should do it.
//! Every word has to appear somewhere in the row -- artist, title, album,
//! year or file path -- in any order and any case.

use super::queue::QueueItem;

/// Does the item match every word of `query`? An empty query matches all.
pub fn matches(item: &QueueItem, query: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return true;
    }
    let hay = haystack(item);
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// One flag per item: whether it matches.
pub fn mask(items: &[QueueItem], query: &str) -> Vec<bool> {
    if query.split_whitespace().next().is_none() {
        return vec![true; items.len()];
    }
    items.iter().map(|i| matches(i, query)).collect()
}

/// Everything a row can be found by, lower-cased and run together with
/// spaces, so a word cannot match across two fields by accident.
fn haystack(item: &QueueItem) -> String {
    let mut s = String::new();
    for part in [
        item.artist.as_deref(),
        item.album_artist.as_deref(),
        item.title.as_deref(),
        item.album.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        s.push_str(&part.to_lowercase());
        s.push(' ');
    }
    if let Some(y) = item.year {
        s.push_str(&y.to_string());
        s.push(' ');
    }
    s.push_str(&item.uri.to_string().to_lowercase());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::uri::TrackUri;

    fn item(artist: &str, title: &str, album: &str, year: i64, path: &str) -> QueueItem {
        let mut q = QueueItem::new(TrackUri::File {
            rel_path: path.into(),
        });
        q.artist = Some(artist.into());
        q.title = Some(title.into());
        q.album = Some(album.into());
        q.year = Some(year);
        q
    }

    #[test]
    fn every_word_has_to_be_somewhere_in_the_row() {
        let q = item(
            "Black Sabbath",
            "The Wizard",
            "Black Sabbath",
            1970,
            "Black Sabbath/x.flac",
        );
        assert!(matches(&q, "wizard"));
        assert!(matches(&q, "BLACK wiz"));
        assert!(matches(&q, "1970 sabbath"));
        assert!(!matches(&q, "wizard paranoid"));
        assert!(matches(&q, ""), "no words, no narrowing");
        assert!(matches(&q, "   "));
    }

    #[test]
    fn the_file_path_counts() {
        let mut q = item("", "", "", 0, "Rips/1987 Germany Vertigo/01.flac");
        q.artist = None;
        q.title = None;
        q.album = None;
        q.year = None;
        assert!(matches(&q, "germany"));
        assert!(!matches(&q, "japan"));
    }

    #[test]
    fn a_word_cannot_match_across_two_fields() {
        let q = item("Ozzy", "Crazy", "Blizzard", 1980, "o/c.flac");
        assert!(!matches(&q, "ozzycrazy"));
    }

    #[test]
    fn the_mask_is_one_flag_per_item() {
        let items = vec![
            item("Angra", "Nova Era", "Rebirth", 2001, "a/1.flac"),
            item("Angra", "Carry On", "Angels Cry", 1993, "a/2.flac"),
        ];
        assert_eq!(mask(&items, "rebirth"), [true, false]);
        assert_eq!(mask(&items, ""), [true, true]);
    }
}
