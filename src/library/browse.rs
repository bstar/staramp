//! The library as three columns: artists, their records, their tracks.
//!
//! Built once from the index and held in memory. The whole model is 29,511
//! tracks and about 9 MB, and building it costs ~150 ms against the reference
//! library -- so there is no paging here, and no query per selection. That is a
//! deliberate choice and it has three reasons, in increasing order of how much
//! they matter:
//!
//! Per-artist queries are not actually cheap: `coalesce(nullif(album_artist,
//! ''), artist) = ?` is not sargable, so every artist selection is a full table
//! scan. The search line cannot be incremental against SQL at all -- in memory
//! it is a filter over a `Vec`, under a millisecond per keystroke. And, the
//! real reason: **album identity is list-relative**. [`group::keys`] decides
//! whether an album's artist matters by looking at what else is in the list, so
//! asking it per artist and asking it over the library give different answers.
//! Computed once, over everything, the browser and the playlist panel cannot
//! disagree about where one record ends.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};

use super::db::Db;
use super::infer::{self, Source};
use crate::playlist::group;
use crate::playlist::queue::QueueItem;
use crate::playlist::uri::TrackUri;

/// One song, one row.
///
/// 1,057 songs in the reference library are addressable twice. A *per-track*
/// cue sheet indexes its virtual tracks and -- correctly, see
/// `cue::resolve::suppresses_backing_files` -- leaves its backing files
/// visible, so the same audio exists as both `.../01 Title.flac` and
/// `.../album.cue/track0001`, both `hidden = 0`. A browser listing both lies
/// about how much music there is, and an "add album" emitting both puts every
/// song into the playlist twice.
///
/// Two rows are the same song when they name the same span of audio:
/// `(file_id, start_frame, end_frame)`. Grouping the 30,774 visible tracks that
/// way gives 1,263 collisions, every one of size exactly two.
///
/// The rank inside the `EXISTS` reads as: a plain row outranks a cue row, and
/// among equals the smaller URI wins. 30,774 - 1,057 - 206 = **29,511**.
///
/// `end_frame IS t.end_frame`, not `=`, is the single most important character
/// here: all 1,057 cue-vs-plain collisions have NULL on *both* sides, and `=`
/// would match none of them while leaving every count looking plausible.
///
/// The URI is the tie-break rather than the row id because URIs survive a
/// rescan and ids do not -- `schema.rs` keys `track_stat` by URI for the same
/// reason -- and because "the first sheet alphabetically wins" is something a
/// person can be told.
///
/// **This is a browser rule.** `stats`, `search`, `query` and the library queue
/// still see every row.
pub const CANONICAL: &str = "\
    t.hidden = 0
    AND NOT EXISTS (SELECT 1 FROM track o
                     WHERE o.hidden = 0
                       AND o.file_id     =  t.file_id
                       AND o.start_frame =  t.start_frame
                       AND o.end_frame   IS t.end_frame
                       AND o.id <> t.id
                       AND (   (o.cue_file_id IS NULL) >  (t.cue_file_id IS NULL)
                            OR ((o.cue_file_id IS NULL) = (t.cue_file_id IS NULL)
                                AND o.uri < t.uri)))";

/// A cue virtual track carries neither a year nor a real codec -- its codec is
/// literally `"cue"` -- so both come from the audio underneath.
///
/// `db::album_for_uri` and `main::META_SELECT` each hold a copy of this join
/// for the same reason. This is the third; there must not be a fourth.
pub const CUE_BACKING_JOIN: &str =
    "LEFT JOIN track b ON b.file_id = t.file_id AND b.cue_ordinal IS NULL";

/// One span of audio. Two rows sharing one are the same recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file_id: i64,
    pub start_frame: i64,
    pub end_frame: Option<i64>,
}

/// A row the browser can show, play, or put in a playlist.
#[derive(Debug, Clone)]
pub struct Track {
    /// Everything the queue needs, in the shape it already uses -- so playing
    /// from the browser is handing rows over, not converting them.
    pub item: QueueItem,
    pub span: Span,
    pub dir_id: i64,
    pub dir: Arc<str>,
    /// The sheet a cue track came from. Multi-disc albums here are one folder
    /// with several sheets, and this is the only thing that keeps their discs
    /// apart -- see [`Model::build`].
    pub sheet: Option<Arc<str>>,
    pub codec: Arc<str>,
    /// Index into [`Model::albums`].
    pub album: u32,
    pub artist_from: Source,
    pub album_from: Source,
    pub year_from: Source,
}

impl Track {
    pub fn uri(&self) -> &TrackUri {
        &self.item.uri
    }
    pub fn title(&self) -> &str {
        self.item.title.as_deref().unwrap_or("")
    }
}

/// Album identity is decided by the one rule the queue already uses.
impl group::Tagged for Track {
    fn album(&self) -> Option<&str> {
        self.item.album.as_deref()
    }
    fn album_artist(&self) -> Option<&str> {
        self.item.album_artist.as_deref()
    }
    fn artist(&self) -> Option<&str> {
        self.item.artist.as_deref()
    }
    fn dir(&self) -> &str {
        &self.dir
    }
}

#[derive(Debug, Clone)]
pub struct Album {
    pub key: group::Key,
    pub title: Arc<str>,
    /// Index into [`Model::artists`].
    pub artist: u32,
    /// The earliest year any of its tracks claims -- `group`'s rule, so a
    /// reissue tagged on one track only does not float the record to the wrong
    /// end of the list.
    pub year: Option<i64>,
    /// Contiguous range into [`Model::tracks`], in disc/sheet/track order.
    pub tracks: Range<u32>,
    pub dir_id: i64,
    pub from: Source,
    pub year_from: Source,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub name: Arc<str>,
    /// `lower(trim(name))`. What the column sorts and searches on.
    pub sort: String,
    /// Contiguous range into [`Model::albums`], in album order.
    pub albums: Range<u32>,
    pub tracks: u32,
    pub from: Source,
}

pub struct Model {
    /// Canonical rows only, grouped by artist then album.
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    /// **Every** URI the index knows, canonical or not, mapped to its span.
    ///
    /// A playlist may perfectly well name the row the browser hides -- the user
    /// has 27 of them and they use both spellings -- so recognising "already
    /// there" depends on knowing the ones we do not show.
    pub by_uri: HashMap<String, Span>,
    /// A span to the canonical track representing it.
    pub by_span: HashMap<Span, u32>,
    /// The scan generation this was built from. A rescan invalidates it.
    pub generation: i64,
}

/// A row as it comes out of SQL, before identity and inference are decided.
struct Raw {
    item: QueueItem,
    span: Span,
    dir_id: i64,
    dir: String,
    sheet: Option<String>,
    codec: String,
    canonical: bool,
    artist_from: Source,
    album_from: Source,
    year_from: Source,
}

impl Model {
    pub fn load(db: &Db) -> Result<Model> {
        let generation = db.generation().unwrap_or(0);
        let raws = Self::read(db)?;
        Ok(Self::build(raws, generation))
    }

    fn read(db: &Db) -> Result<Vec<Raw>> {
        // Every row, not just the canonical ones: `by_uri` has to cover what
        // the browser hides.
        let sql = format!(
            "SELECT t.uri, t.title, t.artist, t.album_artist, t.album,
                    COALESCE(t.year, b.year), t.disc_no, t.track_no, t.duration_ms,
                    t.codec, t.file_id, t.start_frame, t.end_frame,
                    f.dir_id, d.rel_path, c.rel_path,
                    ({CANONICAL}) AS canonical
               FROM track t
               JOIN file f ON f.id = t.file_id
               JOIN dir  d ON d.id = f.dir_id
               LEFT JOIN file c ON c.id = t.cue_file_id
               {CUE_BACKING_JOIN}"
        );
        let mut stmt = db
            .conn
            .prepare(&sql)
            .context("preparing the browse query")?;
        let rows = stmt.query_map([], |r| {
            let uri: String = r.get(0)?;
            let duration_ms: Option<i64> = r.get(8)?;
            Ok(Raw {
                item: QueueItem {
                    uri: TrackUri::parse(&uri),
                    title: r.get(1)?,
                    artist: r.get(2)?,
                    album_artist: r.get(3)?,
                    album: r.get(4)?,
                    year: r.get(5)?,
                    disc_no: r
                        .get::<_, Option<i64>>(6)?
                        .and_then(|v| u32::try_from(v).ok()),
                    track_no: r
                        .get::<_, Option<i64>>(7)?
                        .and_then(|v| u32::try_from(v).ok()),
                    duration_secs: duration_ms.map(|ms| ms / 1000),
                    rg: Default::default(),
                    unplayable: false,
                },
                span: Span {
                    file_id: r.get(10)?,
                    start_frame: r.get(11)?,
                    end_frame: r.get(12)?,
                },
                dir_id: r.get(13)?,
                dir: r.get(14)?,
                sheet: r.get(15)?,
                codec: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                canonical: r.get::<_, i64>(16)? != 0,
                artist_from: Source::Tag,
                album_from: Source::Tag,
                year_from: Source::Tag,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn build(raws: Vec<Raw>, generation: i64) -> Model {
        let mut by_uri = HashMap::with_capacity(raws.len());
        for r in &raws {
            by_uri.insert(r.item.uri.to_string(), r.span);
        }

        let mut raws: Vec<Raw> = raws.into_iter().filter(|r| r.canonical).collect();
        fill_in(&mut raws);

        // Album identity over the whole library at once. Doing this per artist
        // would give a different answer -- that is the point of the module doc.
        let mut tracks: Vec<Track> = raws
            .into_iter()
            .map(|r| Track {
                item: r.item,
                span: r.span,
                dir_id: r.dir_id,
                dir: r.dir.into(),
                sheet: r.sheet.map(Arc::from),
                codec: r.codec.into(),
                album: 0,
                artist_from: r.artist_from,
                album_from: r.album_from,
                year_from: r.year_from,
            })
            .collect();
        let keys = group::keys(&tracks);

        // Group track indices by album key. Untagged tracks each become their
        // own record named by their folder, rather than one anonymous heap.
        let mut groups: Vec<(Option<group::Key>, Vec<usize>)> = Vec::new();
        let mut of_key: HashMap<String, usize> = HashMap::new();
        for (i, key) in keys.iter().enumerate() {
            let handle = match key {
                // The folder as well as the tags: see `infer::album_dir`.
                Some(k) => format!(
                    "k{}\u{0}{}\u{0}{}",
                    k.title(),
                    k.artist(),
                    infer::album_dir(&tracks[i].dir)
                ),
                // Folder, so an untagged record stays one record.
                None => format!("d{}", tracks[i].dir_id),
            };
            match of_key.get(&handle) {
                Some(&g) => groups[g].1.push(i),
                None => {
                    of_key.insert(handle, groups.len());
                    groups.push((key.clone(), vec![i]));
                }
            }
        }

        let mut albums = Vec::with_capacity(groups.len());
        for (key, members) in &groups {
            let (title, from) = album_title(&tracks, members, key.as_ref());
            albums.push(Album {
                key: key.clone().unwrap_or_default(),
                title,
                artist: 0,
                year: members.iter().filter_map(|&i| tracks[i].item.year).min(),
                tracks: 0..0,
                dir_id: modal(members.iter().map(|&i| tracks[i].dir_id)).unwrap_or(0),
                from,
                year_from: best(members, |i| tracks[i].year_from),
            });
            for &i in members {
                tracks[i].album_from = from;
            }
        }

        // Whose record each one is, decided over the whole track set rather
        // than per row -- `coalesce(album_artist, artist)` on each row
        // separately files half a patchy compilation under one name and
        // scatters the rest, which is the failure `group::keys` exists to
        // prevent, one level up.
        let mut by_artist: HashMap<String, (Arc<str>, Source, Vec<usize>)> = HashMap::new();
        for (g, (_, members)) in groups.iter().enumerate() {
            let (name, from) = album_artist(&tracks, members);
            let sort = name.trim().to_lowercase();
            let e = by_artist
                .entry(sort)
                .or_insert_with(|| (name.clone(), from, Vec::new()));
            // One record that really is theirs makes the artist real, however
            // many of their folders had to be guessed at.
            if from.rank() < e.1.rank() {
                e.1 = from;
                e.0 = name.clone();
            }
            e.2.push(g);
            for &i in members {
                tracks[i].artist_from = from;
            }
        }

        let mut order: Vec<(String, Arc<str>, Source, Vec<usize>)> = by_artist
            .into_iter()
            .map(|(sort, (name, from, gs))| (sort, name, from, gs))
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        // Emit everything contiguously so a column is a slice, not a filter.
        let mut out_tracks: Vec<Track> = Vec::with_capacity(tracks.len());
        let mut out_albums: Vec<Album> = Vec::with_capacity(albums.len());
        let mut artists: Vec<Artist> = Vec::with_capacity(order.len());
        let mut taken: Vec<bool> = vec![false; tracks.len()];

        for (sort, name, from, mut gs) in order {
            let first_album = out_albums.len() as u32;
            gs.sort_by(|&x, &y| album_cmp(&albums[x], &albums[y]));
            let mut count = 0u32;
            for g in gs {
                let mut members = groups[g].1.clone();
                members.sort_by(|&x, &y| track_cmp(&tracks[x], &tracks[y]));
                let first_track = out_tracks.len() as u32;
                for i in members {
                    debug_assert!(!taken[i], "a track landed in two records");
                    taken[i] = true;
                    let mut t = tracks[i].clone();
                    t.album = out_albums.len() as u32;
                    out_tracks.push(t);
                }
                let mut a = albums[g].clone();
                a.artist = artists.len() as u32;
                a.tracks = first_track..out_tracks.len() as u32;
                count += a.tracks.len() as u32;
                out_albums.push(a);
            }
            artists.push(Artist {
                name,
                sort,
                albums: first_album..out_albums.len() as u32,
                tracks: count,
                from,
            });
        }

        let by_span = out_tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.span, i as u32))
            .collect();

        Model {
            tracks: out_tracks,
            albums: out_albums,
            artists,
            by_uri,
            by_span,
            generation,
        }
    }

    /// Records whose name or artist we made up. The "needs tagging" filter.
    pub fn inferred_albums(&self) -> impl Iterator<Item = u32> + '_ {
        self.albums
            .iter()
            .enumerate()
            .filter(|(_, a)| a.from.is_guess())
            .map(|(i, _)| i as u32)
    }
}

/// Fill in what the tags do not say, in strict order of trust.
///
/// Tagged siblings beat the folder name every time, and not marginally: in all
/// 15 mixed directories the siblings are the better answer. The folder says
/// `2004 - Metallic Tragedy`; the siblings say `Metallic Tragedy`. The folder
/// says `V- The New Mythology Suite` because `:` is not legal on some
/// filesystems; the siblings say `V: The New Mythology Suite`. One folder
/// misspells the artist that its own files have right.
///
/// Borrowing runs before album identity is decided, so a borrowed title merges
/// with its siblings' record instead of forming a second one of its own.
fn fill_in(raws: &mut [Raw]) {
    // Bucket by directory once. Scanning every row per directory instead is
    // 3,686 x 31,928 against the reference library, which is a second of the
    // startup budget spent on 1.3% of the music.
    let mut in_dir: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, r) in raws.iter().enumerate() {
        in_dir.entry(r.dir_id).or_default().push(i);
    }

    let mut artist_of: HashMap<i64, String> = HashMap::new();
    let mut album_of: HashMap<i64, String> = HashMap::new();
    for (&dir, rows) in &in_dir {
        artist_of.insert(
            dir,
            modal(rows.iter().filter_map(|&i| {
                non_empty(raws[i].item.album_artist.as_deref())
                    .or(non_empty(raws[i].item.artist.as_deref()))
                    .map(str::to_string)
            }))
            .unwrap_or_default(),
        );
        album_of.insert(
            dir,
            modal(
                rows.iter()
                    .filter_map(|&i| non_empty(raws[i].item.album.as_deref()).map(str::to_string)),
            )
            .unwrap_or_default(),
        );
    }

    for r in raws.iter_mut() {
        if non_empty(r.item.artist.as_deref()).is_none()
            && non_empty(r.item.album_artist.as_deref()).is_none()
        {
            let borrowed = artist_of.get(&r.dir_id).filter(|s| !s.is_empty()).cloned();
            r.artist_from = match borrowed {
                Some(a) => {
                    r.item.artist = Some(a);
                    Source::Siblings
                }
                None => match infer::from_path(&r.dir).artist {
                    Some(a) => {
                        r.item.artist = Some(a.to_string());
                        Source::Path
                    }
                    None => Source::Unknown,
                },
            };
        }
        if non_empty(r.item.album.as_deref()).is_none() {
            let borrowed = album_of.get(&r.dir_id).filter(|s| !s.is_empty()).cloned();
            r.album_from = match borrowed {
                Some(a) => {
                    r.item.album = Some(a);
                    Source::Siblings
                }
                None => match infer::from_path(&r.dir).album {
                    Some(a) => {
                        r.item.album = Some(a.to_string());
                        Source::Path
                    }
                    None => Source::Unknown,
                },
            };
        }
        // The year, whatever else the row has. The album column is ordered by
        // it, and 11,974 tracks carry none -- 10,133 of them in a folder that
        // names one. Only consulted when the tag is silent, and marked.
        if r.item.year.is_none() {
            if let Some(y) = infer::year_in_path(&r.dir) {
                r.item.year = Some(y);
                r.year_from = Source::Path;
            } else {
                r.year_from = Source::Unknown;
            }
        }
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// The commonest value, ties broken by the value itself so it is deterministic.
fn modal<T: std::hash::Hash + Eq + Ord + Clone>(it: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: HashMap<T, usize> = HashMap::new();
    for v in it {
        *counts.entry(v).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(v, _)| v)
}

/// The best provenance in a group.
///
/// One track carrying the real tag makes the record a tagged record, even where
/// its siblings borrowed the name from it.
fn best(members: &[usize], of: impl Fn(usize) -> Source) -> Source {
    members
        .iter()
        .map(|&i| of(i))
        .min_by_key(|s| s.rank())
        .unwrap_or(Source::Unknown)
}

fn album_title(
    tracks: &[Track],
    members: &[usize],
    key: Option<&group::Key>,
) -> (Arc<str>, Source) {
    let from = best(members, |i| tracks[i].album_from);
    let spelled = modal(
        members
            .iter()
            .filter_map(|&i| non_empty(tracks[i].item.album.as_deref()).map(str::to_string)),
    );
    match spelled {
        Some(t) => (t.into(), from),
        None => match key {
            Some(k) if !k.title().is_empty() => (k.title().into(), from),
            _ => (
                infer::from_path(&tracks[members[0]].dir)
                    .album
                    .unwrap_or("no album")
                    .into(),
                Source::Path,
            ),
        },
    }
}

/// Whose record this is.
///
/// The commonest `album_artist` if there is one; failing that, the track artist
/// if every track agrees; failing that it is a compilation and says so.
fn album_artist(tracks: &[Track], members: &[usize]) -> (Arc<str>, Source) {
    let from = best(members, |i| tracks[i].artist_from);
    if let Some(a) = modal(
        members
            .iter()
            .filter_map(|&i| non_empty(tracks[i].item.album_artist.as_deref()).map(str::to_string)),
    ) {
        return (a.into(), from);
    }
    let mut artists = members
        .iter()
        .filter_map(|&i| non_empty(tracks[i].item.artist.as_deref()));
    if let Some(first) = artists.next() {
        let first = first.to_string();
        if members
            .iter()
            .filter_map(|&i| non_empty(tracks[i].item.artist.as_deref()))
            .all(|a| a == first)
        {
            return (first.into(), from);
        }
        return ("Various Artists".into(), from);
    }
    match infer::from_path(&tracks[members[0]].dir).artist {
        Some(a) => (a.into(), Source::Path),
        None => ("no artist".into(), Source::Unknown),
    }
}

/// Where a record sits before its own name is considered.
///
/// Undated records go after dated ones, the rule `group` already sets: sorting
/// by year with `None` first would answer "oldest first" with the records whose
/// age is unknown.
fn album_cmp(a: &Album, b: &Album) -> std::cmp::Ordering {
    let bucket = |x: &Album| u8::from(x.year.is_none());
    bucket(a)
        .cmp(&bucket(b))
        .then_with(|| a.year.cmp(&b.year))
        .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
}

/// The order of tracks within one record.
///
/// `disc_no` alone is not enough and cannot be made enough: the cue insert in
/// `scan.rs` omits the column, so **not one of the 10,840 cue rows carries
/// one** -- while 67 folders hold more than one sheet, which is how a multi-disc
/// album is filed here. Ordering by `(disc_no, track_no)` interleaves those
/// discs: track 1 of disc 1, track 1 of disc 2, track 2 of disc 1. Sorting by
/// the sheet keeps each disc whole.
///
/// The URI comes last because it is UNIQUE, which makes the order total --
/// 2,848 `(album, disc, track_no)` triples in the reference library are claimed
/// by more than one row.
fn track_cmp(a: &Track, b: &Track) -> std::cmp::Ordering {
    a.item
        .disc_no
        .unwrap_or(1)
        .cmp(&b.item.disc_no.unwrap_or(1))
        .then_with(|| a.sheet.cmp(&b.sheet))
        .then_with(|| {
            a.item
                .track_no
                .unwrap_or(0)
                .cmp(&b.item.track_no.unwrap_or(0))
        })
        .then_with(|| a.title().to_lowercase().cmp(&b.title().to_lowercase()))
        .then_with(|| a.item.uri.to_string().cmp(&b.item.uri.to_string()))
}

/// A model built from a handful of tuples, for tests in other modules.
///
/// One fixture builder rather than two: the browser panel's navigation has to
/// be tested against a real `Model`, and a second hand-rolled one would drift
/// from what the index actually produces.
#[cfg(test)]
pub(crate) fn fixture(rows: &[(&str, &str, &str, &str, &str)]) -> Model {
    use rusqlite::params;
    let db = Db::open_in_memory().unwrap();
    let mut id = 1i64;
    let mut dirs: HashMap<&str, i64> = HashMap::new();
    for (dir, file, title, artist, album) in rows {
        let dir_id = *dirs.entry(dir).or_insert_with(|| {
            let d = id;
            id += 1;
            db.conn
                .execute(
                    "INSERT INTO dir (id, rel_path, scan_gen) VALUES (?1, ?2, 1)",
                    params![d, dir],
                )
                .unwrap();
            d
        });
        let uri = format!("{dir}/{file}");
        let file_id = id;
        id += 1;
        db.conn
            .execute(
                "INSERT INTO file (id, dir_id, rel_path, size, mtime_ns, kind, scan_gen)
                 VALUES (?1, ?2, ?3, 1, 1, 0, 1)",
                params![file_id, dir_id, uri],
            )
            .unwrap();
        let t = id;
        id += 1;
        db.conn
            .execute(
                "INSERT INTO track (id, uri, file_id, start_frame, hidden, title, artist,
                                    album, codec, added_at, modified_at, scan_gen)
                 VALUES (?1,?2,?3,0,0,?4,?5,?6,'flac',0,0,1)",
                params![t, uri, file_id, title, artist, album],
            )
            .unwrap();
    }
    Model::load(&db).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// A fixture built the way the scanner builds one, because the traps this
    /// module exists for are all in the shapes the scanner produces.
    struct Fixture {
        db: Db,
        next: i64,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture {
                db: Db::open_in_memory().unwrap(),
                next: 1,
            }
        }

        fn dir(&mut self, rel: &str) -> i64 {
            let id = self.next;
            self.next += 1;
            self.db
                .conn
                .execute(
                    "INSERT INTO dir (id, rel_path, scan_gen) VALUES (?1, ?2, 1)",
                    params![id, rel],
                )
                .unwrap();
            id
        }

        fn file(&mut self, dir_id: i64, rel: &str, kind: i64) -> i64 {
            let id = self.next;
            self.next += 1;
            self.db
                .conn
                .execute(
                    "INSERT INTO file (id, dir_id, rel_path, size, mtime_ns, kind, scan_gen)
                     VALUES (?1, ?2, ?3, 1, 1, ?4, 1)",
                    params![id, dir_id, rel, kind],
                )
                .unwrap();
            id
        }

        /// One track row. `cue` is the sheet's file id, `span` its audio extent.
        #[allow(clippy::too_many_arguments)]
        fn track(
            &mut self,
            uri: &str,
            file_id: i64,
            cue: Option<i64>,
            span: (i64, Option<i64>),
            title: Option<&str>,
            artist: Option<&str>,
            album: Option<&str>,
            hidden: i64,
        ) -> i64 {
            let id = self.next;
            self.next += 1;
            self.db
                .conn
                .execute(
                    "INSERT INTO track (id, uri, file_id, cue_file_id, cue_ordinal,
                                        start_frame, end_frame, hidden,
                                        title, artist, album, codec,
                                        added_at, modified_at, scan_gen)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'flac',0,0,1)",
                    params![
                        id,
                        uri,
                        file_id,
                        cue,
                        cue.map(|_| 1),
                        span.0,
                        span.1,
                        hidden,
                        title,
                        artist,
                        album
                    ],
                )
                .unwrap();
            id
        }

        fn model(&self) -> Model {
            Model::load(&self.db).unwrap()
        }
    }

    fn uris(m: &Model) -> Vec<String> {
        m.tracks.iter().map(|t| t.item.uri.to_string()).collect()
    }

    /// A per-track cue sheet leaves its backing files visible, on purpose. Both
    /// spellings of the song are `hidden = 0`, and 1,057 songs in the reference
    /// library are in this state.
    fn per_track_cue(f: &mut Fixture, title_on_cue: Option<&str>) -> Model {
        let d = f.dir("Brothers of Metal/2017 - Prophecy of Ragnarok");
        let audio = f.file(
            d,
            "Brothers of Metal/2017 - Prophecy of Ragnarok/01.flac",
            0,
        );
        let sheet = f.file(d, "Brothers of Metal/2017 - Prophecy of Ragnarok/x.cue", 1);
        f.track(
            "Brothers of Metal/2017 - Prophecy of Ragnarok/01.flac",
            audio,
            None,
            (0, None),
            Some("Death of the God of Light"),
            Some("Brothers of Metal"),
            Some("Prophecy of Ragnarok"),
            0,
        );
        f.track(
            "Brothers of Metal/2017 - Prophecy of Ragnarok/x.cue/track0001",
            audio,
            Some(sheet),
            (0, None),
            title_on_cue,
            Some("Brothers of Metal"),
            Some("Prophecy of Ragnarok"),
            0,
        );
        f.model()
    }

    #[test]
    fn a_per_track_cue_and_its_backing_file_are_one_song() {
        let mut f = Fixture::new();
        let m = per_track_cue(&mut f, Some("Death of the God of Light"));
        assert_eq!(m.tracks.len(), 1, "{:#?}", uris(&m));
        assert!(!m.tracks[0].item.uri.is_cue(), "the plain row should win");
    }

    #[test]
    fn the_plain_row_wins_because_it_is_never_the_poorer_of_the_two() {
        // In 57 of the 1,057 real collisions the cue row's title is NULL and
        // the plain row's is not. Never the other way round -- so preferring
        // the plain row cannot lose a title.
        let mut f = Fixture::new();
        let m = per_track_cue(&mut f, None);
        assert_eq!(m.tracks.len(), 1);
        assert_eq!(m.tracks[0].title(), "Death of the God of Light");
    }

    /// The single most important character in the module.
    ///
    /// Every one of the 1,057 collisions has `end_frame` NULL on *both* sides.
    /// Written `=` instead of `IS`, the predicate matches nothing, the whole
    /// feature quietly does nothing, and every other test here still passes.
    #[test]
    fn a_null_end_frame_still_counts_as_the_same_span() {
        let mut f = Fixture::new();
        let m = per_track_cue(&mut f, Some("x"));
        assert_eq!(m.tracks.len(), 1, "NULL = NULL is never true in SQL");
    }

    #[test]
    fn a_disc_image_cue_keeps_every_one_of_its_tracks() {
        // The other shape: one file, many tracks, each a different extent of
        // it. They share a `file_id` but not a span and must all survive.
        let mut f = Fixture::new();
        let d = f.dir("Running Wild/1995 - Masquerade");
        let audio = f.file(d, "Running Wild/1995 - Masquerade/rip.flac", 0);
        let sheet = f.file(d, "Running Wild/1995 - Masquerade/rip.cue", 1);
        for (i, (start, end)) in [(0, Some(1000)), (1000, Some(2000)), (2000, None)]
            .into_iter()
            .enumerate()
        {
            f.track(
                &format!("Running Wild/1995 - Masquerade/rip.cue/track{:04}", i + 1),
                audio,
                Some(sheet),
                (start, end),
                Some(&format!("Track {}", i + 1)),
                Some("Running Wild"),
                Some("Masquerade"),
                0,
            );
        }
        // The backing file is hidden by the scanner in this case.
        f.track(
            "Running Wild/1995 - Masquerade/rip.flac",
            audio,
            None,
            (0, None),
            None,
            None,
            None,
            1,
        );
        let m = f.model();
        assert_eq!(m.tracks.len(), 3, "{:#?}", uris(&m));
    }

    #[test]
    fn two_sheets_over_one_recording_pick_the_same_one_every_time() {
        // 206 spans in the reference library are covered by two sheets in one
        // folder. Their titles are identical, so there is no better/worse to
        // fall back on -- only determinism matters.
        let build = || {
            let mut f = Fixture::new();
            let d = f.dir("Tears For Fears/vinyl/Songs from the Big Chair");
            let audio = f.file(d, "Tears For Fears/vinyl/a.flac", 0);
            let one = f.file(d, "Tears For Fears/vinyl/b.cue", 1);
            let two = f.file(d, "Tears For Fears/vinyl/a.cue", 1);
            for sheet in [one, two] {
                let name = if sheet == one { "b" } else { "a" };
                f.track(
                    &format!("Tears For Fears/vinyl/{name}.cue/track0001"),
                    audio,
                    Some(sheet),
                    (0, None),
                    Some("Shout"),
                    Some("Tears For Fears"),
                    Some("Songs from the Big Chair"),
                    0,
                );
            }
            f.model()
        };
        let first = uris(&build());
        assert_eq!(first.len(), 1);
        assert_eq!(first, uris(&build()), "the choice must not wobble");
    }

    #[test]
    fn hiding_a_row_from_the_browser_does_not_hide_it_from_the_playlists() {
        // A playlist may name the row the browser does not show -- the
        // reference library's own playlists use both spellings. Recognising
        // "already there" depends on knowing the ones we hide.
        let mut f = Fixture::new();
        let m = per_track_cue(&mut f, Some("x"));
        assert_eq!(m.tracks.len(), 1);
        for uri in [
            "Brothers of Metal/2017 - Prophecy of Ragnarok/01.flac",
            "Brothers of Metal/2017 - Prophecy of Ragnarok/x.cue/track0001",
        ] {
            assert!(m.by_uri.contains_key(uri), "{uri} is not addressable");
        }
        // And both name the same audio, which is what makes them duplicates.
        assert_eq!(
            m.by_uri["Brothers of Metal/2017 - Prophecy of Ragnarok/01.flac"],
            m.by_uri["Brothers of Metal/2017 - Prophecy of Ragnarok/x.cue/track0001"]
        );
    }

    #[test]
    fn tagged_siblings_name_a_record_better_than_its_folder_does() {
        // The folder says `2004 - Metallic Tragedy`; the one tagged sibling
        // says `Metallic Tragedy`. All 15 mixed folders are like this.
        let mut f = Fixture::new();
        let d = f.dir("Magic Kingdom/2004 - Metallic Tragedy");
        let a = f.file(d, "Magic Kingdom/2004 - Metallic Tragedy/01.flac", 0);
        let b = f.file(d, "Magic Kingdom/2004 - Metallic Tragedy/02.flac", 0);
        f.track(
            "Magic Kingdom/2004 - Metallic Tragedy/01.flac",
            a,
            None,
            (0, None),
            Some("One"),
            Some("Magic Kingdom"),
            Some("Metallic Tragedy"),
            0,
        );
        f.track(
            "Magic Kingdom/2004 - Metallic Tragedy/02.flac",
            b,
            None,
            (0, None),
            Some("Two"),
            None,
            None,
            0,
        );
        let m = f.model();
        assert_eq!(
            m.albums.len(),
            1,
            "a borrowed title must not split the record"
        );
        assert_eq!(&*m.albums[0].title, "Metallic Tragedy");
        assert_eq!(m.albums[0].tracks.len(), 2);
    }

    #[test]
    fn a_wholly_untagged_folder_is_still_a_record_with_a_name() {
        let mut f = Fixture::new();
        let d = f.dir("Avantasia/2001 - The Metal Opera");
        for i in 1..=2 {
            let file = f.file(d, &format!("Avantasia/2001 - The Metal Opera/0{i}.flac"), 0);
            f.track(
                &format!("Avantasia/2001 - The Metal Opera/0{i}.flac"),
                file,
                None,
                (0, None),
                None,
                None,
                None,
                0,
            );
        }
        let m = f.model();
        assert_eq!(m.artists.len(), 1);
        assert_eq!(&*m.artists[0].name, "Avantasia");
        assert_eq!(m.albums.len(), 1);
        assert_eq!(&*m.albums[0].title, "2001 - The Metal Opera");
        // The folder carries the year the tags do not.
        assert_eq!(m.albums[0].year, Some(2001));
        // And it says it was a guess, so the panel can mark it.
        assert!(m.albums[0].from.is_guess());
        assert_eq!(m.inferred_albums().count(), 1);
    }

    #[test]
    fn a_multi_disc_cue_album_keeps_its_discs_whole() {
        // Not one of the 10,840 cue rows in the reference library carries a
        // `disc_no`, and 67 folders hold more than one sheet. Ordering on
        // `(disc_no, track_no)` alone interleaves the discs.
        let mut f = Fixture::new();
        let d = f.dir("Pagan's Mind/2007 - God's Equation");
        let a1 = f.file(d, "Pagan's Mind/2007 - God's Equation/d1.flac", 0);
        let a2 = f.file(d, "Pagan's Mind/2007 - God's Equation/d2.flac", 0);
        let s1 = f.file(d, "Pagan's Mind/2007 - God's Equation/disc1.cue", 1);
        let s2 = f.file(d, "Pagan's Mind/2007 - God's Equation/disc2.cue", 1);
        for (sheet, audio, disc) in [(s1, a1, 1), (s2, a2, 2)] {
            for n in 1..=2i64 {
                f.track(
                    &format!(
                        "Pagan's Mind/2007 - God's Equation/disc{disc}.cue/track{:04}",
                        n
                    ),
                    audio,
                    Some(sheet),
                    (n * 1000, Some(n * 1000 + 999)),
                    Some(&format!("d{disc}t{n}")),
                    Some("Pagan's Mind"),
                    Some("God's Equation"),
                    0,
                );
            }
        }
        let m = f.model();
        assert_eq!(m.tracks.len(), 4);
        let titles: Vec<&str> = m.tracks.iter().map(|t| t.title()).collect();
        assert_eq!(
            titles,
            ["d1t1", "d1t2", "d2t1", "d2t2"],
            "the discs interleaved"
        );
    }

    #[test]
    fn two_rips_of_one_record_are_two_records() {
        // Eight folders hold a rip of Black Sabbath's *Paranoid* in the
        // reference library. Keyed on tags alone they become one 64-track
        // record; the folders are what say they are eight releases.
        let mut f = Fixture::new();
        for rip in ["FLAC", "vinyl rip", "1974 remaster"] {
            let d = f.dir(&format!("Black Sabbath/{rip}/Paranoid"));
            for n in 1..=2i64 {
                let file = f.file(d, &format!("Black Sabbath/{rip}/Paranoid/0{n}.flac"), 0);
                f.track(
                    &format!("Black Sabbath/{rip}/Paranoid/0{n}.flac"),
                    file,
                    None,
                    (0, None),
                    Some("War Pigs"),
                    Some("Black Sabbath"),
                    Some("Paranoid"),
                    0,
                );
            }
        }
        let m = f.model();
        assert_eq!(m.albums.len(), 3, "the rips merged into one record");
        assert!(m.albums.iter().all(|a| a.tracks.len() == 2));
        assert_eq!(m.artists.len(), 1, "and they are all the same band");
    }

    #[test]
    fn a_two_disc_set_is_still_one_record() {
        // The other side of the same rule: disc folders fold up, or every
        // deluxe edition in the library splits in three.
        let mut f = Fixture::new();
        for disc in ["CD1", "CD2"] {
            let d = f.dir(&format!("Powerwolf/Wake Up The Wicked (Deluxe)/{disc}"));
            let file = f.file(
                d,
                &format!("Powerwolf/Wake Up The Wicked (Deluxe)/{disc}/01.flac"),
                0,
            );
            f.track(
                &format!("Powerwolf/Wake Up The Wicked (Deluxe)/{disc}/01.flac"),
                file,
                None,
                (0, None),
                Some("Bete du Gevaudan"),
                Some("Powerwolf"),
                Some("Wake Up The Wicked"),
                0,
            );
        }
        let m = f.model();
        assert_eq!(m.albums.len(), 1, "the discs split into separate records");
        assert_eq!(m.albums[0].tracks.len(), 2);
    }

    #[test]
    fn records_are_ordered_by_year_with_the_undated_last() {
        let mut f = Fixture::new();
        for (album, year) in [
            ("Later", Some(2010)),
            ("Undated", None),
            ("Early", Some(1990)),
        ] {
            let d = f.dir(&format!("A/{album}"));
            let file = f.file(d, &format!("A/{album}/01.flac"), 0);
            let id = f.track(
                &format!("A/{album}/01.flac"),
                file,
                None,
                (0, None),
                Some("t"),
                Some("A"),
                Some(album),
                0,
            );
            if let Some(y) = year {
                f.db.conn
                    .execute("UPDATE track SET year = ?1 WHERE id = ?2", params![y, id])
                    .unwrap();
            }
        }
        let m = f.model();
        let names: Vec<&str> = m.albums.iter().map(|a| &*a.title).collect();
        assert_eq!(names, ["Early", "Later", "Undated"]);
    }

    #[test]
    fn every_track_belongs_to_exactly_one_record_and_one_artist() {
        // The ranges are slices, so an off-by-one here silently hides music.
        let mut f = Fixture::new();
        let m = per_track_cue(&mut f, Some("x"));
        let mut seen = vec![false; m.tracks.len()];
        for artist in &m.artists {
            for a in artist.albums.clone() {
                let album = &m.albums[a as usize];
                assert_eq!(album.artist, m.albums[a as usize].artist);
                for t in album.tracks.clone() {
                    assert!(!seen[t as usize], "track {t} is in two records");
                    seen[t as usize] = true;
                    assert_eq!(m.tracks[t as usize].album, a);
                }
            }
        }
        assert!(seen.iter().all(|&s| s), "a track belongs to no record");
    }

    /// The numbers this design was measured against.
    ///
    /// A personal regression guard, not a test a contributor can run: the
    /// counts are of one specific library, and it skips when that index is
    /// absent, so it can only ever fail for the person it was written by. It
    /// is kept because those counts are the whole argument for the predicate
    /// above, and a silent drift in them is exactly the failure that would
    /// otherwise go unnoticed. Ignored by default; `cargo test -- --ignored`
    /// on any other machine will simply skip it.
    #[test]
    #[ignore = "reads the real index"]
    fn the_reference_library_still_measures_what_the_design_says() {
        let Ok(path) = crate::paths::index_file() else {
            return;
        };
        if !path.exists() {
            return;
        }
        let db = Db::open_readonly(&path).unwrap();
        let visible: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM track WHERE hidden = 0", [], |r| {
                r.get(0)
            })
            .unwrap();
        let canonical: i64 = db
            .conn
            .query_row(
                &format!("SELECT COUNT(*) FROM track t WHERE {CANONICAL}"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(visible, 30_774, "the library changed under the test");
        assert_eq!(canonical, 29_511, "1,057 cue-vs-plain + 206 cue-vs-cue");

        let m = Model::load(&db).unwrap();
        assert_eq!(m.tracks.len() as i64, canonical);
        assert_eq!(m.by_uri.len(), 31_928, "every URI stays addressable");
        assert!(!m.artists.is_empty() && !m.albums.is_empty());
    }
}
