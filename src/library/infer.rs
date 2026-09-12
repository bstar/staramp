//! What a directory path says about the music inside it.
//!
//! 408 tracks in the reference library carry no artist tag and no album artist,
//! and 363 of those have no album either. They are not junk -- they are whole
//! records, filed in folders that name them perfectly well: `Avantasia/2001 -
//! The Metal Opera`, `Hammerfall/FLAC/2002 - Crimson Thunder`. Left to the tags
//! alone they would sit in one anonymous heap at the bottom of the artist
//! column, which is exactly the material the user cannot currently find.
//!
//! The library is filed `Artist/.../Album`, and that is not an assumption: the
//! top-level component matches the artist tag exactly for 92.2% of *tagged*
//! tracks, and for 95.5% once either name is allowed to contain the other
//! (`Helloween` under `Helloween/FLAC`). A rule right nine times in ten is worth
//! applying where the alternative is a blank column.
//!
//! Nothing here is ever written back to the index or to a file's tags. It
//! decides what to *show*, and every value it produces is marked so the browser
//! can draw it differently. A guess presented as a fact is worse than a blank.

/// Where a displayed value came from.
///
/// Per field rather than per row: 45 tracks have a real artist tag and no album
/// at all, and telling someone their artist is a guess would be a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The file's own tag.
    #[default]
    Tag,
    /// Borrowed from tagged siblings in the same directory.
    Siblings,
    /// Read off `dir.rel_path`.
    Path,
    /// Nothing to go on.
    Unknown,
}

impl Source {
    /// Did we make this up? What the panel marks.
    pub fn is_guess(self) -> bool {
        matches!(self, Source::Path | Source::Unknown)
    }

    /// How far to be trusted, lower being better.
    ///
    /// A record whose name one track actually carries is a tagged record, even
    /// if its other tracks borrowed the name. The best answer in the group is
    /// the one that decided it.
    pub fn rank(self) -> u8 {
        match self {
            Source::Tag => 0,
            Source::Siblings => 1,
            Source::Path => 2,
            Source::Unknown => 3,
        }
    }
}

/// What a path suggests. Borrowed from the input; nothing is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FromPath<'a> {
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
}

/// Components that are shelving rather than records.
///
/// Every one of these is in the reference library, and each would otherwise
/// become an album title: `Stratovarius/FLAC/2000 - Infinite/.../Video`,
/// `Black Sabbath/.../DATA`.
const NOT_AN_ALBUM: &[&str] = &[
    "video", "videos", "scans", "covers", "cover", "artwork", "art", "data", "bonus", "extras",
];

/// Format and media folders that sit between the artist and the record.
///
/// Deliberately *not* used to find the album -- the album is found by taking
/// the directory the audio is actually in, which needs no list. This is only
/// consulted when that directory is itself one of these, which happens when a
/// stray file sits directly under `Artist/FLAC`.
const NOT_AN_ALBUM_EITHER: &[&str] = &["flac", "mp3", "vinyl", "cd", "lossless", "music"];

fn is_disc_folder(c: &str) -> bool {
    let c = c.trim().to_ascii_lowercase();
    for prefix in ["disc", "disk", "cd", "part"] {
        let Some(rest) = c.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start_matches([' ', '-', '_', '.']);
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            continue;
        }
        // A disc may be titled as well as numbered -- the reference library has
        // `Disc 1 - Out In The Streets` -- so what follows the number only has
        // to be a separator rather than more of the name. `Discovery` is not a
        // disc, and neither is `CD Single`.
        let after = &rest[digits..];
        if after.is_empty() || after.starts_with([' ', '-', '_', '.', ')', ']']) {
            return true;
        }
    }
    false
}

fn is_shelf(c: &str) -> bool {
    let lower = c.trim().to_ascii_lowercase();
    NOT_AN_ALBUM.contains(&lower.as_str())
        || NOT_AN_ALBUM_EITHER.contains(&lower.as_str())
        || is_disc_folder(c)
}

/// Read a directory path as artist and album.
///
/// The artist is the first component -- the only one whose meaning is fixed,
/// since album directories in this library sit at depths 1 through 8 and a
/// depth rule breaks at both ends.
///
/// The album is the directory the audio is in, walking up past shelving. That
/// needs no list of format folders in the *middle* of a path: `Hammerfall/FLAC/
/// 2002 - Crimson Thunder` gives `2002 - Crimson Thunder` because that is where
/// the files are, not because `FLAC` was recognised.
pub fn from_path(rel_path: &str) -> FromPath<'_> {
    let parts: Vec<&str> = rel_path
        .split('/')
        .filter(|p| !p.trim().is_empty())
        .collect();
    let Some(&artist) = parts.first() else {
        // The library root. Two tracks sit here, and they are nobody's.
        return FromPath::default();
    };

    // Walk up from the deepest component, past disc and media folders. Bounded
    // so a pathological tree cannot walk out of its own artist.
    let mut album = None;
    for &c in parts.iter().skip(1).rev().take(3) {
        if !is_shelf(c) {
            album = Some(c);
            break;
        }
    }

    FromPath {
        artist: Some(artist),
        album,
    }
}

/// The directory the record lives in, with any disc or media folder folded up.
///
/// What tells two rips of one album apart. `group::keys` decides album identity
/// from tags alone, which is right for a queue and wrong for a whole library:
/// eight rips of *Paranoid* in eight folders are eight releases, and merging
/// them into one 64-track record is the very duplication a browser exists to
/// avoid. The filesystem already separates them, and folding disc folders up
/// keeps a two-disc set together.
pub fn album_dir(rel_path: &str) -> &str {
    let mut end = rel_path.trim_end_matches('/');
    // Only ever folds *up*, never past the artist, so a record filed directly
    // under its artist keeps its own folder.
    for _ in 0..3 {
        let Some((parent, last)) = end.rsplit_once('/') else {
            break;
        };
        if is_shelf(last) {
            end = parent;
        } else {
            break;
        }
    }
    end
}

/// The first plausible release year in a folder name.
///
/// The untagged material has no year either, and its folders very often carry
/// one -- `2002 - Crimson Thunder`, `Stratovarius - 1997 - Visions`. Recovering
/// it puts these records in their place in the album column instead of dumping
/// them all in the undated bucket at the end.
///
/// The year is *not* stripped from the displayed name. The folder name is the
/// honest thing to show, and a title edited by a regex is how you end up
/// showing `01.15 Ozzy Osbourne` for a folder called `1982.01.15 Ozzy
/// Osbourne`.
pub fn year_in(name: &str) -> Option<i64> {
    let b = name.as_bytes();
    for i in 0..b.len().saturating_sub(3) {
        // Not part of a longer run of digits: `19999` is not a year, and
        // neither is the `2016` inside a catalogue number like `NB20164`.
        if i > 0 && b[i - 1].is_ascii_digit() {
            continue;
        }
        if i + 4 < b.len() && b[i + 4].is_ascii_digit() {
            continue;
        }
        let run = &b[i..i + 4];
        if !run.iter().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let y: i64 = std::str::from_utf8(run).ok()?.parse().ok()?;
        if (1900..=2099).contains(&y) {
            return Some(y);
        }
    }
    None
}

/// The year named anywhere in a directory path, deepest component first.
///
/// 10,133 of the 11,974 undated tracks in the reference library sit in a folder
/// that names a year, and the album column is ordered by year -- so without
/// this a third of the library piles up in the undated bucket at the end.
///
/// Trustworthy, measured: where a track has both, the folder agrees with the
/// tag for 12,849 of 13,911 tracks (92.4%). The 7.6% that disagree are not
/// errors so much as a different question -- `Ancient Curse - The Landing
/// (Remastered) (2025)` is tagged 1997 for the recording and named 2025 for the
/// release, and `Blondie - Against The Odds - 1974-1982 (2022)` is both. So it
/// is only ever consulted when the tag is silent, and what it produces is
/// marked [`Source::Path`] rather than passed off as a tag.
pub fn year_in_path(rel_path: &str) -> Option<i64> {
    rel_path.rsplit('/').find_map(year_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these is a real directory in the reference library.
    #[test]
    fn the_artist_is_the_top_level_folder() {
        for (path, artist) in [
            ("Avantasia/2001 - The Metal Opera", "Avantasia"),
            ("Hammerfall/FLAC/2002 - Crimson Thunder", "Hammerfall"),
            ("Matthew S. Burns/Last Call BBS", "Matthew S. Burns"),
            ("Voice Memos", "Voice Memos"),
        ] {
            assert_eq!(from_path(path).artist, Some(artist), "{path}");
        }
    }

    #[test]
    fn a_format_folder_is_not_an_album() {
        // The album is where the audio is, so `FLAC` is skipped without ever
        // being recognised as a format.
        assert_eq!(
            from_path("Hammerfall/FLAC/2002 - Crimson Thunder").album,
            Some("2002 - Crimson Thunder")
        );
        assert_eq!(
            from_path("Tom Petty/vinyl/Tom Petty - Full Moon Fever").album,
            Some("Tom Petty - Full Moon Fever")
        );
    }

    #[test]
    fn a_disc_folder_is_not_an_album() {
        for path in [
            "Pagan's Mind/FLAC/2007 - God's Equation/Disc 1",
            "Pagan's Mind/FLAC/2007 - God's Equation/CD2",
            "Pagan's Mind/FLAC/2007 - God's Equation/disc-2",
        ] {
            assert_eq!(
                from_path(path).album,
                Some("2007 - God's Equation"),
                "{path}"
            );
        }
        // Both discs of a set therefore land on one record, which is the point.
        assert_eq!(
            from_path("A/2007 - X/Disc 1").album,
            from_path("A/2007 - X/Disc 2").album
        );
    }

    #[test]
    fn a_media_folder_is_not_an_album() {
        assert_eq!(
            from_path("Stratovarius/FLAC/2000 - Infinite/Video").album,
            Some("2000 - Infinite")
        );
        assert_eq!(
            from_path("Black Sabbath/1970 - Paranoid/DATA").album,
            Some("1970 - Paranoid")
        );
    }

    #[test]
    fn a_folder_that_only_looks_like_a_disc_is_still_an_album() {
        // `CD` and a number is shelving; a word after it is a record.
        assert!(is_disc_folder("CD1"));
        assert!(is_disc_folder("Disc 2"));
        assert!(!is_disc_folder("CD Single"));
        assert!(!is_disc_folder("Discovery"));
        assert_eq!(from_path("Daft Punk/Discovery").album, Some("Discovery"));
    }

    #[test]
    fn an_album_is_found_at_every_depth_the_library_actually_uses() {
        // Depths 1 through 8 all occur; a fixed-depth rule breaks at both ends.
        assert_eq!(from_path("Voice Memos").album, None);
        assert_eq!(from_path("A/Album").album, Some("Album"));
        assert_eq!(from_path("A/b/c/d/e/f/g/Album").album, Some("Album"));
    }

    #[test]
    fn the_library_root_infers_nothing_and_does_not_panic() {
        assert_eq!(from_path(""), FromPath::default());
        assert_eq!(from_path("/"), FromPath::default());
        assert_eq!(from_path("   "), FromPath::default());
    }

    #[test]
    fn a_year_in_the_folder_name_is_found_wherever_it_sits() {
        assert_eq!(year_in("2002 - Crimson Thunder"), Some(2002));
        assert_eq!(year_in("Stratovarius - 1997 - Visions"), Some(1997));
        assert_eq!(year_in("1999 - Theater Of Salvation [FLAC]"), Some(1999));
        assert_eq!(year_in("Boston  1976(Original Pressing,LP)"), Some(1976));
        assert_eq!(year_in("Rubber Soul"), None);
    }

    #[test]
    fn a_longer_run_of_digits_is_not_a_year() {
        // Catalogue numbers are everywhere in this library's folder names.
        assert_eq!(year_in("NB20164"), None);
        assert_eq!(year_in("19999"), None);
        assert_eq!(year_in("SCCD-15"), None);
        // But a real year beside one is still found.
        assert_eq!(year_in("1999 - No Escape (SCCD-15)"), Some(1999));
    }

    #[test]
    fn a_year_is_never_stripped_from_the_name_it_was_found_in() {
        // The folder name is what gets shown. Editing it with a regex is how
        // `1982.01.15 Ozzy Osbourne` becomes `01.15 Ozzy Osbourne`.
        let name = "2001 - The Metal Opera";
        assert_eq!(year_in(name), Some(2001));
        assert_eq!(
            from_path("Avantasia/2001 - The Metal Opera").album,
            Some(name)
        );
    }

    #[test]
    fn the_year_is_found_at_whatever_depth_names_it() {
        assert_eq!(
            year_in_path("Hammerfall/FLAC/2002 - Crimson Thunder"),
            Some(2002)
        );
        // The deepest wins: a disc folder inside a dated album folder.
        assert_eq!(year_in_path("A/2007 - X/Disc 1"), Some(2007));
        assert_eq!(year_in_path("The Beatles/Rubber Soul"), None);
    }

    #[test]
    fn the_album_folder_folds_discs_up_but_not_the_record_itself() {
        assert_eq!(
            album_dir("Powerwolf/Wake Up The Wicked (Deluxe)/CD1"),
            "Powerwolf/Wake Up The Wicked (Deluxe)"
        );
        // A disc may be titled as well as numbered.
        assert_eq!(
            album_dir("Blondie/Against The Odds/Disc 1 - Out In The Streets"),
            "Blondie/Against The Odds"
        );
        // And a record filed straight under its artist keeps its own folder.
        assert_eq!(
            album_dir("Helloween/1994 - Master of the Rings"),
            "Helloween/1994 - Master of the Rings"
        );
        assert_eq!(album_dir("Voice Memos"), "Voice Memos");
    }

    #[test]
    fn two_rips_of_one_record_have_different_album_folders() {
        // What stops eight rips of Paranoid becoming one 64-track record.
        assert_ne!(
            album_dir("Black Sabbath/FLAC/Paranoid"),
            album_dir("Black Sabbath/vinyl/Paranoid")
        );
    }

    #[test]
    fn a_guess_knows_that_it_is_one() {
        assert!(!Source::Tag.is_guess());
        assert!(!Source::Siblings.is_guess());
        assert!(Source::Path.is_guess());
        assert!(Source::Unknown.is_guess());
    }
}
