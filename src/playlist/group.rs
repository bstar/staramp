//! Putting a queue in album order.
//!
//! Pure: a list of items in, a permutation of `0..n` out. It knows nothing
//! about the queue that will install it or the panel that will draw dividers
//! along it, and both of those come through [`keys`] so they cannot disagree
//! about where one record ends and the next begins.
//!
//! The permutation is the contract. `Queue` installs the result as its whole
//! playback order, so an ordering that dropped an index would not merely hide
//! a track — it would make it unreachable.

use super::queue::QueueItem;

/// What decides whether two items belong to the same record.
///
/// The album title carries it, the artist breaks ties, and the folder breaks
/// the ties that are left: the same title from the same artist in two
/// directories is two rips of one record, and merging them made a 48-track
/// *Black Sabbath* out of seven copies, interleaved by track number. The
/// folder is only consulted when it has to be, so a record that exists once
/// is keyed exactly as before. Disc folders fold up first -- see
/// `infer::album_dir` -- so a two-disc set stays one record.
///
/// Keying on
/// `(album_artist, album)` outright looks more careful and is worse in
/// practice: a compilation whose `album_artist` is tagged on some tracks and
/// not others splits into pieces, and that is far commoner in a real library
/// than two different records sharing a title.
///
/// The tie-break falls back to the track artist, because the case that
/// actually turns up is one record ripped twice -- an mp3 folder with no
/// `album_artist` and a cue rip with one -- sitting alongside a different
/// record of the same name. The fallback puts the two rips together and still
/// keeps the two records apart. It cannot scatter a patchy compilation,
/// because a compilation only reaches this branch if its title is claimed by
/// two different album artists, and its own is one value on every row that
/// has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    title: String,
    /// Empty unless this title genuinely belongs to more than one artist.
    artist: String,
    /// The record's folder, empty unless this title and artist are found in
    /// more than one -- that is, unless there is more than one rip of it.
    version: String,
    /// The shortest tail of `version` that no other rip of this record
    /// shares: what a heading shows. Empty when `version` is.
    label: String,
}

impl Key {
    /// The album title, normalised.
    ///
    /// How a record is named on its own, without the artist that only exists
    /// to separate it from a namesake in the same queue. That makes it the
    /// right handle for anything that has to survive the queue changing --
    /// folding a record away, say, which is remembered across playlists and
    /// across launches, where the namesake may not be there at all.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The artist, empty unless this title is claimed by more than one.
    ///
    /// Only meaningful beside [`Key::title`]: on its own it says nothing,
    /// because most records leave it blank on purpose.
    pub fn artist(&self) -> &str {
        &self.artist
    }

    /// The folder that tells this rip from the others, empty when there is
    /// only one rip.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// What to show for the version: as little of the folder path as tells
    /// this rip from the others, from the end -- `1970 - Black Sabbath
    /// [Castle]` when the last folder differs, `FLAC/Paranoid` against
    /// `vinyl/Paranoid` when it is the one above that does. A box set's rip
    /// four folders deep is named by the one folder that is its own.
    pub fn version_label(&self) -> Option<&str> {
        (!self.label.is_empty()).then_some(self.label.as_str())
    }
}

/// The last `n` components of a path.
fn tail(path: &str, n: usize) -> &str {
    let mut cut = path.len();
    for _ in 0..n {
        match path[..cut].rfind('/') {
            Some(i) => cut = i,
            None => return path,
        }
    }
    &path[cut + 1..]
}

/// The shortest tail of `dir` that none of `others` ends with.
fn distinguishing_tail<'a>(dir: &'a str, others: &[String]) -> &'a str {
    let depth = dir.split('/').count();
    for n in 1..=depth {
        let t = tail(dir, n);
        let shared = others.iter().any(|o| {
            o != dir
                && (o.ends_with(t)
                    && (o.len() == t.len() || o.as_bytes()[o.len() - t.len() - 1] == b'/'))
        });
        if !shared {
            return t;
        }
    }
    dir
}

/// The same normalisation the keys use, for a title from somewhere else --
/// a config file, say.
pub fn normalise_title(s: &str) -> String {
    normalise(s)
}

fn normalise(s: &str) -> String {
    s.trim().to_lowercase()
}

fn tag(s: Option<&str>) -> Option<String> {
    let t = normalise(s?);
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// The three tags album identity is decided from, and the folder that
/// separates rips.
///
/// A trait rather than the concrete queue item, because the library browser
/// holds a different row and the two must not end up with different rules for
/// where one record ends and the next begins.
pub trait Tagged {
    fn album(&self) -> Option<&str>;
    fn album_artist(&self) -> Option<&str>;
    fn artist(&self) -> Option<&str>;
    /// The directory the track's file is in, relative to the library root.
    fn dir(&self) -> &str;
}

impl Tagged for QueueItem {
    fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }
    fn album_artist(&self) -> Option<&str> {
        self.album_artist.as_deref()
    }
    fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }
    fn dir(&self) -> &str {
        self.uri
            .backing_path()
            .rsplit_once('/')
            .map(|(d, _)| d)
            .unwrap_or("")
    }
}

/// The album each item belongs to, one entry per item, `None` for untagged.
///
/// Resolved over the whole list rather than item by item, because whether the
/// album artist matters depends on what else is in the list.
pub fn keys<T: Tagged>(items: &[T]) -> Vec<Option<Key>> {
    use std::collections::{HashMap, HashSet};

    // Which artists claim each title. One or none means the title is enough.
    let mut claims: HashMap<String, HashSet<String>> = HashMap::new();
    for item in items {
        if let Some(title) = tag(item.album()) {
            let e = claims.entry(title).or_default();
            if let Some(a) = tag(item.album_artist()) {
                e.insert(a);
            }
        }
    }

    let mut keys: Vec<Option<Key>> = items
        .iter()
        .map(|item| {
            let title = tag(item.album())?;
            let shared = claims.get(&title).is_some_and(|a| a.len() > 1);
            let artist = if shared {
                tag(item.album_artist())
                    .or_else(|| tag(item.artist()))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            Some(Key {
                title,
                artist,
                version: String::new(),
                label: String::new(),
            })
        })
        .collect();

    // Which folders hold each record. More than one is more than one rip,
    // and only then does the folder join the key.
    let folder = |item: &T| crate::library::infer::album_dir(item.dir()).to_string();
    let mut rips: HashMap<Key, HashSet<String>> = HashMap::new();
    for (item, key) in items.iter().zip(&keys) {
        if let Some(k) = key {
            rips.entry(k.clone()).or_default().insert(folder(item));
        }
    }
    for (item, key) in items.iter().zip(keys.iter_mut()) {
        if let Some(k) = key {
            if let Some(dirs) = rips.get(k).filter(|d| d.len() > 1) {
                let dir = folder(item);
                let others: Vec<String> = dirs.iter().cloned().collect();
                k.label = distinguishing_tail(&dir, &others).to_string();
                k.version = dir;
            }
        }
    }
    keys
}

/// Where a group sits before its own key is considered.
///
/// Undated records go after dated ones and untagged tracks after everything,
/// in *both* directions. Reversing those along with the years would answer
/// "newest first" by opening on the tracks whose age is unknown.
const DATED: u8 = 0;
const UNDATED: u8 = 1;
const UNTAGGED: u8 = 2;

/// The album titles in the order the records appear, one entry per record.
///
/// What a hand-made order is written down as: a list of records rather than a
/// list of positions, so it still means something after the playlist has been
/// edited or the arrangement carried to another one.
pub fn titles_in_order(items: &[QueueItem]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<Option<Key>> = None;
    for key in keys(items) {
        if last.as_ref() == Some(&key) {
            continue;
        }
        out.push(key.as_ref().map(|k| k.title.clone()).unwrap_or_default());
        last = Some(key);
    }
    out
}

/// A permutation of `0..items.len()`, albums in year order — or in `manual`
/// order, for the records it names.
///
/// A record the hand-made order does not mention keeps its place in the year
/// order, after everything that is named: loading a playlist full of records
/// nobody has arranged should not scatter them.
///
/// Within a record: disc, then track number, then the file name for anything
/// without one -- so a rip with no track numbers comes out in the order its
/// files are named, which is nearly always the order of the record -- and
/// last of all the order it arrived in.
pub fn album_order(items: &[QueueItem], descending: bool, manual: &[String]) -> Vec<usize> {
    let keys = keys(items);

    // First appearance decides the grouping, so an order is stable against
    // anything the sort below cannot distinguish.
    let mut groups: Vec<Group> = Vec::new();
    let mut seen: std::collections::HashMap<Option<Key>, usize> = std::collections::HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        let at = *seen.entry(key.clone()).or_insert_with(|| {
            groups.push(Group {
                key: key.clone(),
                year: None,
                members: Vec::new(),
            });
            groups.len() - 1
        });
        let g = &mut groups[at];
        // The earliest year any of its tracks claims. Rips disagree with
        // themselves often enough that picking one arbitrarily is not stable.
        g.year = match (g.year, items[i].year) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        g.members.push(i);
    }

    for g in &mut groups {
        g.members.sort_by_cached_key(|&i| {
            let it = &items[i];
            // Numbered tracks first, in number order; the unnumbered follow,
            // by name. The name is the whole URI, lower-cased: within one
            // record that differs only in the file name, and for a cue rip in
            // the zero-padded track ordinal, which sorts as it counts.
            (
                it.disc_no.unwrap_or(0),
                it.track_no.unwrap_or(u32::MAX),
                it.uri.to_string().to_lowercase(),
                i,
            )
        });
    }

    // Where the hand-made order puts a record, if it names it at all.
    let placed = |g: &Group| {
        manual
            .iter()
            .position(|t| t == g.title())
            .unwrap_or(usize::MAX)
    };

    groups.sort_by(|a, b| {
        // Anything arranged by hand comes first, in that arrangement.
        let (pa, pb) = (placed(a), placed(b));
        if pa != usize::MAX || pb != usize::MAX {
            return pa.cmp(&pb);
        }
        a.bucket().cmp(&b.bucket()).then_with(|| {
            if a.bucket() != DATED {
                // Alphabetical is alphabetical; only years have a direction.
                return a.title().cmp(b.title());
            }
            let ord = a.year.cmp(&b.year).then_with(|| a.title().cmp(b.title()));
            if descending {
                ord.reverse()
            } else {
                ord
            }
        })
    });

    let order: Vec<usize> = groups.into_iter().flat_map(|g| g.members).collect();
    debug_assert!(
        is_permutation(&order, items.len()),
        "album_order lost a track"
    );
    order
}

struct Group {
    key: Option<Key>,
    year: Option<i64>,
    members: Vec<usize>,
}

impl Group {
    fn bucket(&self) -> u8 {
        match (&self.key, self.year) {
            (None, _) => UNTAGGED,
            (Some(_), None) => UNDATED,
            (Some(_), Some(_)) => DATED,
        }
    }

    fn title(&self) -> &str {
        self.key.as_ref().map(|k| k.title.as_str()).unwrap_or("")
    }
}

/// Every index of `0..n`, exactly once.
pub fn is_permutation(order: &[usize], n: usize) -> bool {
    if order.len() != n {
        return false;
    }
    let mut seen = vec![false; n];
    for &i in order {
        match seen.get_mut(i) {
            Some(s) if !*s => *s = true,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::uri::TrackUri;

    /// `album, album_artist, year, disc, track`
    fn item(album: &str, artist: &str, year: Option<i64>, disc: u32, track: u32) -> QueueItem {
        let mut q = QueueItem::new(TrackUri::File {
            rel_path: format!("{album}-{disc}-{track}.flac"),
        });
        q.album = (!album.is_empty()).then(|| album.to_string());
        q.album_artist = (!artist.is_empty()).then(|| artist.to_string());
        q.year = year;
        q.disc_no = Some(disc);
        q.track_no = Some(track);
        q
    }

    /// `item`, filed in a folder.
    fn filed(dir: &str, album: &str, artist: &str, track: u32) -> QueueItem {
        let mut q = item(album, artist, Some(1970), 1, track);
        q.uri = TrackUri::File {
            rel_path: format!("{dir}/{track:02}.flac"),
        };
        q
    }

    #[test]
    fn two_rips_of_one_record_are_two_records() {
        let items = vec![
            filed(
                "Black Sabbath/1970 - Black Sabbath [Castle]",
                "Black Sabbath",
                "Black Sabbath",
                1,
            ),
            filed(
                "Black Sabbath/1970 - Black Sabbath [Castle]",
                "Black Sabbath",
                "Black Sabbath",
                2,
            ),
            filed(
                "Black Sabbath/Black Sabbath (2009 Remaster)",
                "Black Sabbath",
                "Black Sabbath",
                1,
            ),
            filed(
                "Black Sabbath/Black Sabbath (2009 Remaster)",
                "Black Sabbath",
                "Black Sabbath",
                2,
            ),
        ];
        let keys = keys(&items);
        assert_eq!(keys[0], keys[1]);
        assert_eq!(keys[2], keys[3]);
        assert_ne!(keys[0], keys[2], "two folders, two records");
        assert_eq!(
            keys[0].as_ref().unwrap().version_label(),
            Some("1970 - Black Sabbath [Castle]")
        );
        // And the order keeps each rip together rather than interleaving by
        // track number.
        let order = album_order(&items, false, &[]);
        let dirs: Vec<&str> = order.iter().map(|&i| items[i].dir()).collect();
        assert!(
            dirs[0] == dirs[1] && dirs[2] == dirs[3] && dirs[1] != dirs[2],
            "{dirs:?}"
        );
    }

    #[test]
    fn a_rip_is_named_by_as_little_of_its_path_as_tells_it_apart() {
        let items = vec![
            filed(
                "Black Sabbath/FLAC/Paranoid",
                "Paranoid",
                "Black Sabbath",
                1,
            ),
            filed(
                "Black Sabbath/vinyl/Paranoid",
                "Paranoid",
                "Black Sabbath",
                1,
            ),
            filed(
                "Black Sabbath/Box/2004 Black Box/1970 Paranoid (Rhino)",
                "Paranoid",
                "Black Sabbath",
                1,
            ),
        ];
        let labels: Vec<Option<String>> = keys(&items)
            .iter()
            .map(|k| {
                k.as_ref()
                    .and_then(|k| k.version_label())
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(
            labels,
            [
                Some("FLAC/Paranoid".into()),
                Some("vinyl/Paranoid".into()),
                Some("1970 Paranoid (Rhino)".into()),
            ]
        );
        assert_eq!(tail("a/b/c", 1), "c");
        assert_eq!(tail("a/b/c", 2), "b/c");
        assert_eq!(tail("a/b/c", 9), "a/b/c");
    }

    #[test]
    fn a_record_that_exists_once_is_keyed_without_its_folder() {
        let items = vec![
            filed("Angra/Holy Land", "Holy Land", "Angra", 1),
            filed("Angra/Holy Land", "Holy Land", "Angra", 2),
            filed("Angra/Fireworks", "Fireworks", "Angra", 1),
        ];
        for k in keys(&items).into_iter().flatten() {
            assert_eq!(k.version(), "", "{k:?}");
            assert_eq!(k.version_label(), None);
        }
    }

    #[test]
    fn disc_folders_do_not_split_a_set_into_rips() {
        let items = vec![
            filed(
                "Blondie/Against The Odds/Disc 1",
                "Against The Odds",
                "Blondie",
                1,
            ),
            filed(
                "Blondie/Against The Odds/CD2",
                "Against The Odds",
                "Blondie",
                1,
            ),
        ];
        let keys = keys(&items);
        assert_eq!(keys[0], keys[1]);
        assert_eq!(keys[0].as_ref().unwrap().version(), "");
    }

    #[test]
    fn unnumbered_tracks_follow_the_numbered_ones_by_file_name() {
        let mut a = filed("X/Y", "Y", "X", 0);
        a.track_no = None;
        a.uri = TrackUri::File {
            rel_path: "X/Y/b side.flac".into(),
        };
        let mut b = filed("X/Y", "Y", "X", 0);
        b.track_no = None;
        b.uri = TrackUri::File {
            rel_path: "X/Y/a side.flac".into(),
        };
        let c = filed("X/Y", "Y", "X", 2);
        let d = filed("X/Y", "Y", "X", 1);
        let items = vec![a, b, c, d];
        let order = album_order(&items, false, &[]);
        let names: Vec<String> = order.iter().map(|&i| items[i].uri.to_string()).collect();
        assert_eq!(
            names,
            [
                "X/Y/01.flac",
                "X/Y/02.flac",
                "X/Y/a side.flac",
                "X/Y/b side.flac"
            ]
        );
    }

    /// The album title of each item, in the order `album_order` gives them.
    fn titles(items: &[QueueItem], descending: bool) -> Vec<String> {
        album_order(items, descending, &[])
            .into_iter()
            .map(|i| items[i].album.clone().unwrap_or_else(|| "-".into()))
            .collect()
    }

    fn dedup(v: Vec<String>) -> Vec<String> {
        let mut v = v;
        v.dedup();
        v
    }

    #[test]
    fn albums_come_out_in_year_order_both_ways() {
        let items = vec![
            item("Chained", "At Vance", Some(2005), 1, 1),
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("Fireworks", "Angra", Some(1998), 1, 1),
        ];
        assert_eq!(
            dedup(titles(&items, false)),
            ["Holy Land", "Fireworks", "Chained"]
        );
        assert_eq!(
            dedup(titles(&items, true)),
            ["Chained", "Fireworks", "Holy Land"]
        );
    }

    #[test]
    fn a_hand_made_order_beats_the_years() {
        let items = vec![
            item("Chained", "At Vance", Some(2005), 1, 1),
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("Fireworks", "Angra", Some(1998), 1, 1),
        ];
        assert_eq!(
            titles_in_order(&items),
            ["chained", "holy land", "fireworks"],
            "the order as it stands, which is what an arrangement starts from"
        );

        let by_hand = vec![
            "fireworks".to_string(),
            "chained".into(),
            "holy land".into(),
        ];
        let out: Vec<String> = album_order(&items, false, &by_hand)
            .into_iter()
            .map(|i| items[i].album.clone().unwrap())
            .collect();
        assert_eq!(out, ["Fireworks", "Chained", "Holy Land"]);
        // And the direction no longer applies to what was arranged.
        let flipped: Vec<String> = album_order(&items, true, &by_hand)
            .into_iter()
            .map(|i| items[i].album.clone().unwrap())
            .collect();
        assert_eq!(flipped, out);
    }

    #[test]
    fn a_record_the_arrangement_does_not_name_keeps_its_place_after_it() {
        // Loading a playlist full of records nobody has arranged should not
        // scatter them.
        let items = vec![
            item("Chained", "At Vance", Some(2005), 1, 1),
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("Fireworks", "Angra", Some(1998), 1, 1),
        ];
        let by_hand = vec!["chained".to_string()];
        let out: Vec<String> = album_order(&items, false, &by_hand)
            .into_iter()
            .map(|i| items[i].album.clone().unwrap())
            .collect();
        assert_eq!(out, ["Chained", "Holy Land", "Fireworks"]);
    }

    #[test]
    fn an_arrangement_naming_records_that_are_not_here_changes_nothing() {
        let items = vec![
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("Fireworks", "Angra", Some(1998), 1, 1),
        ];
        let by_hand = vec!["something else".to_string()];
        assert_eq!(
            album_order(&items, false, &by_hand),
            album_order(&items, false, &[])
        );
    }

    #[test]
    fn the_untagged_group_is_last_in_both_directions() {
        let items = vec![
            item("", "", None, 1, 1),
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("", "", None, 1, 2),
            item("Chained", "At Vance", Some(2005), 1, 1),
        ];
        for descending in [false, true] {
            let out = dedup(titles(&items, descending));
            assert_eq!(out.last().unwrap(), "-", "{descending}: {out:?}");
            assert_eq!(out.len(), 3, "the untagged tracks should be one group");
        }
    }

    #[test]
    fn undated_albums_sit_after_the_dated_ones_whichever_way_round() {
        let items = vec![
            item("Bootlegs", "Angra", None, 1, 1),
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("Chained", "At Vance", Some(2005), 1, 1),
            item("", "", None, 1, 1),
        ];
        for descending in [false, true] {
            let out = dedup(titles(&items, descending));
            assert_eq!(out[2], "Bootlegs", "{descending}: {out:?}");
            assert_eq!(out[3], "-", "{descending}: {out:?}");
        }
    }

    #[test]
    fn an_album_takes_the_earliest_year_its_tracks_claim() {
        // A reissue tagged on one track only must not float the whole record
        // to the wrong end of the list.
        let items = vec![
            item("Reissued", "A", Some(2011), 1, 2),
            item("Reissued", "A", Some(1984), 1, 1),
            item("Later", "B", Some(1990), 1, 1),
        ];
        assert_eq!(dedup(titles(&items, false)), ["Reissued", "Later"]);
    }

    #[test]
    fn tracks_within_an_album_go_by_disc_then_number() {
        let items = vec![
            item("A", "X", Some(2000), 2, 1),
            item("A", "X", Some(2000), 1, 2),
            item("A", "X", Some(2000), 1, 1),
        ];
        assert_eq!(album_order(&items, false, &[]), [2, 1, 0]);
    }

    #[test]
    fn an_album_with_no_track_numbers_keeps_the_order_it_arrived_in() {
        let mut items = vec![
            item("A", "X", Some(2000), 1, 1),
            item("A", "X", Some(2000), 1, 1),
            item("A", "X", Some(2000), 1, 1),
        ];
        for it in &mut items {
            it.track_no = None;
            it.disc_no = None;
        }
        assert_eq!(album_order(&items, false, &[]), [0, 1, 2]);
    }

    #[test]
    fn a_title_two_artists_share_is_split_but_a_patchy_compilation_is_not() {
        // Two different records called the same thing: separate.
        let two = vec![
            item("Greatest Hits", "Queen", Some(1981), 1, 1),
            item("Greatest Hits", "Abba", Some(1975), 1, 1),
        ];
        let k = keys(&two);
        assert_ne!(k[0], k[1], "two artists, two records");

        // One compilation, tagged on some rows and not others: together.
        let patchy = vec![
            item("Monster Ballads", "Various Artists", Some(1996), 1, 1),
            item("Monster Ballads", "", Some(1996), 1, 2),
        ];
        let k = keys(&patchy);
        assert_eq!(k[0], k[1], "one record, patchily tagged");
    }

    #[test]
    fn one_record_ripped_twice_stays_one_record() {
        // Straight out of the reference library: `Chained` is an At Vance
        // record and a Crystal Eyes record, and the Crystal Eyes one is there
        // twice -- an mp3 folder with no `album_artist` and a cue rip with
        // one. Three groups would be wrong; so would one.
        let mut mp3 = item("Chained", "", Some(2008), 1, 1);
        mp3.artist = Some("Crystal Eyes".into());
        let mut cue = item("Chained", "Crystal Eyes", None, 1, 1);
        cue.artist = Some("Crystal Eyes".into());
        let mut other = item("Chained", "At Vance", Some(2005), 1, 1);
        other.artist = Some("At Vance".into());

        let k = keys(&[mp3, cue, other]);
        assert_eq!(k[0], k[1], "the same record ripped twice");
        assert_ne!(k[0], k[2], "a different record of the same name");
    }

    #[test]
    fn the_album_title_is_matched_past_case_and_padding() {
        let items = vec![
            item("Holy Land", "Angra", Some(1996), 1, 1),
            item("  holy land ", "Angra", Some(1996), 1, 2),
        ];
        let k = keys(&items);
        assert_eq!(k[0], k[1], "{k:?}");
    }

    #[test]
    fn nothing_is_ever_lost_however_little_there_is_to_sort_on() {
        // The property the queue depends on: an order that dropped an index
        // would make that track unreachable, not merely invisible.
        let bare: Vec<QueueItem> = (0..7)
            .map(|i| {
                QueueItem::new(TrackUri::File {
                    rel_path: format!("{i}.flac"),
                })
            })
            .collect();
        for descending in [false, true] {
            let order = album_order(&bare, descending, &[]);
            assert!(is_permutation(&order, bare.len()), "{order:?}");
        }
        assert!(is_permutation(&album_order(&[], false, &[]), 0));
    }

    #[test]
    fn a_permutation_is_checked_honestly() {
        assert!(is_permutation(&[2, 0, 1], 3));
        assert!(!is_permutation(&[0, 0, 1], 3), "a repeat is not one");
        assert!(!is_permutation(&[0, 1], 3), "nor is a short one");
        assert!(!is_permutation(&[0, 1, 3], 3), "nor one out of range");
    }
}
