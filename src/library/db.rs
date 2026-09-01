//! Index database handle.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::schema;

pub struct Db {
    pub conn: Connection,
}

/// What the album panel shows, gathered in one place.
///
/// Deliberately not a row of the `album` table: that table has no track count
/// or duration, is never cleaned up by a rescan, and is not populated at all
/// for cue albums.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumDetail {
    pub album: Option<String>,
    pub artist: Option<String>,
    pub year: Option<i64>,
    pub codec: Option<String>,
    pub track_count: usize,
    pub total_ms: i64,
    /// The directory the track lives in, where its artwork will be.
    pub dir_id: i64,
    /// The audio file itself, library-root-relative -- the cue sheet's backing
    /// file for a virtual track. Where an embedded picture would be.
    pub file_rel: String,
    /// This track's own title and artist, which on a compilation are not the
    /// album's. Needed to ask what record a song originally came from.
    pub track_title: Option<String>,
    pub track_artist: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening index at {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(schema::PRAGMAS)
            .context("applying pragmas")?;
        conn.execute_batch(schema::SCHEMA)
            .context("creating schema")?;

        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            conn.pragma_update(None, "user_version", schema::SCHEMA_VERSION)?;
        } else if version != schema::SCHEMA_VERSION {
            anyhow::bail!(
                "index schema version {version}, expected {}; delete the index to rebuild",
                schema::SCHEMA_VERSION
            );
        }
        Ok(Self { conn })
    }

    /// Read-only handle for the UI, so rendering never contends with a scan.
    /// WAL allows any number of readers alongside the single writer.
    pub fn open_readonly(path: &Path) -> Result<Self> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening index read-only at {}", path.display()))?;
        conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
        Ok(Self { conn })
    }
}

/// One virtual track of a cue album, as the scan recorded it.
///
/// The same fields `cue::expand::VirtualTrack` carries, minus `backing_path`
/// (one per album, not one per track) and `pregap_start_frame`, which `expand`
/// writes but nothing has ever read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueTrackRow {
    pub ordinal: u32,
    pub number: u32,
    pub file_index: usize,
    pub start_frame: u64,
    pub end_frame: Option<u64>,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
}

/// A cue album as the index already knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueAlbumRows {
    /// The sheet, library-root-relative.
    pub cue_rel: String,
    /// The one backing audio file these tracks are carved out of,
    /// library-root-relative.
    pub backing_rel: String,
    pub tracks: Vec<CueTrackRow>,
}

impl Db {
    /// Every virtual track carved out of the same backing file as `uri`.
    ///
    /// One query in place of the whole filesystem dance that playing a cue
    /// track otherwise costs: reading the sheet, guessing its encoding, parsing
    /// it, listing the directory, probing candidate filenames, and opening the
    /// backing audio file *purely* to ask it its sample rate. The scan did all
    /// of that once and wrote the answers down; this reads them back.
    ///
    /// `None` when the URI is not an indexed cue track -- an unindexed sheet, a
    /// plain file, or a path the CLI was handed directly -- and the caller
    /// falls back to the sheet.
    pub fn cue_album_for_uri(&self, uri: &str) -> Result<Option<CueAlbumRows>> {
        let mut stmt = self.conn.prepare(
            // `b` is the backing file's own plain track row, which is where its
            // real sample rate lives. It is load-bearing rather than
            // informational: `scan.rs` computed every frame boundary below from
            // that rate, defaulting to 44100 when it did not know it, while the
            // sheet-based path reads the true rate from the file. Where the two
            // could disagree the fast path must decline, so a NULL here is
            // checked for below and refuses the whole album.
            "WITH me AS (SELECT file_id, cue_file_id FROM track WHERE uri = ?1)
             SELECT cf.rel_path, bf.rel_path, b.sample_rate,
                    t.cue_ordinal, t.cue_track_no, t.cue_file_index,
                    t.start_frame, t.end_frame,
                    t.title, t.artist, t.album, t.album_artist, t.genre, t.date
               FROM track t
               JOIN me      ON t.file_id = me.file_id
                           AND t.cue_file_id IS me.cue_file_id
               JOIN file bf ON bf.id = t.file_id
               JOIN file cf ON cf.id = t.cue_file_id
               LEFT JOIN track b ON b.file_id = t.file_id AND b.cue_ordinal IS NULL
              WHERE me.cue_file_id IS NOT NULL AND t.cue_ordinal IS NOT NULL
              ORDER BY t.cue_ordinal",
        )?;

        let mut cue_rel: Option<String> = None;
        let mut backing_rel: Option<String> = None;
        let mut tracks = Vec::new();
        let mut rows = stmt.query([uri])?;
        while let Some(r) = rows.next()? {
            if r.get::<_, Option<i64>>(2)?.is_none() {
                // The backing file has no indexed sample rate, so the stored
                // frames may have been computed against an assumed 44100.
                return Ok(None);
            }
            cue_rel.get_or_insert_with(|| r.get::<_, String>(0).unwrap_or_default());
            backing_rel.get_or_insert_with(|| r.get::<_, String>(1).unwrap_or_default());
            tracks.push(CueTrackRow {
                ordinal: r.get::<_, i64>(3)? as u32,
                number: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u32,
                file_index: r.get::<_, Option<i64>>(5)?.unwrap_or(0) as usize,
                start_frame: r.get::<_, i64>(6)?.max(0) as u64,
                end_frame: r.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
                title: r.get(8)?,
                performer: r.get(9)?,
                album: r.get(10)?,
                album_artist: r.get(11)?,
                genre: r.get(12)?,
                date: r.get(13)?,
            });
        }

        match (cue_rel, backing_rel) {
            (Some(cue_rel), Some(backing_rel)) if !tracks.is_empty() => Ok(Some(CueAlbumRows {
                cue_rel,
                backing_rel,
                tracks,
            })),
            _ => Ok(None),
        }
    }
}

impl Db {
    /// What the album panel needs about the track at `uri`.
    ///
    /// `None` when the URI is not indexed, which is normal: a playlist can
    /// name a file the scan has never seen.
    pub fn album_for_uri(&self, uri: &str) -> Result<Option<AlbumDetail>> {
        let row = self.conn.query_row(
            // A cue virtual track carries neither a year nor a real codec --
            // its codec is literally "cue" -- so both are taken from the audio
            // file underneath it, which is the same file and the same album.
            // For an ordinary track that join finds the row itself and the
            // coalesce changes nothing.
            "SELECT t.album_id, t.album, COALESCE(t.album_artist, t.artist),
                    COALESCE(t.year, b.year),
                    COALESCE(NULLIF(t.codec, 'cue'), b.codec),
                    f.dir_id, f.rel_path, t.title, t.artist
               FROM track t
               JOIN file f ON f.id = t.file_id
               LEFT JOIN track b ON b.file_id = t.file_id AND b.cue_ordinal IS NULL
              WHERE t.uri = ?1",
            [uri],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            },
        );
        let (album_id, album, artist, year, codec, dir_id, file_rel, track_title, track_artist) =
            match row {
                Ok(v) => v,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e.into()),
            };

        // Cue virtual tracks are never given an `album_id` by the scan, so
        // aggregating on it alone would report every cue album as one track.
        // Fall back to the album's text within the same directory, which is
        // what a cue album is: one sheet, one folder.
        let (track_count, total_ms) = if let Some(id) = album_id {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(duration_ms), 0)
                   FROM track WHERE album_id = ?1 AND hidden = 0",
                [id],
                |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)?)),
            )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(t.duration_ms), 0)
                   FROM track t JOIN file f ON f.id = t.file_id
                  WHERE f.dir_id = ?1 AND t.album IS ?2 AND t.hidden = 0",
                rusqlite::params![dir_id, album],
                |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)?)),
            )?
        };

        Ok(Some(AlbumDetail {
            album,
            artist,
            year,
            codec,
            track_count,
            total_ms,
            dir_id,
            file_rel,
            track_title,
            track_artist,
        }))
    }

    /// Every image file the scan indexed in a directory, as library-relative
    /// paths, cheapest first: this rides `file_dir_idx ON file(dir_id, kind)`.
    pub fn images_in_dir(&self, dir_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path FROM file WHERE dir_id = ?1 AND kind = 2")?;
        let rows = stmt.query_map([dir_id], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    }

    /// Directories one level below `dir_id`, for the `Covers/` and `Scans/`
    /// folders a good many rips put their artwork in.
    pub fn child_dirs(&self, dir_id: i64) -> Result<Vec<(i64, String)>> {
        let parent: String =
            self.conn
                .query_row("SELECT rel_path FROM dir WHERE id = ?1", [dir_id], |r| {
                    r.get(0)
                })?;
        // `rel_path LIKE 'parent/%'` with no further separator is one level.
        let like = if parent.is_empty() {
            "%".to_string()
        } else {
            format!("{parent}/%")
        };
        let mut stmt = self
            .conn
            .prepare("SELECT id, rel_path FROM dir WHERE rel_path LIKE ?1")?;
        let depth = if parent.is_empty() {
            0
        } else {
            parent.matches('/').count() + 1
        };
        let rows = stmt.query_map([like], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows
            .flatten()
            .filter(|(_, p)| p.matches('/').count() == depth)
            .collect())
    }

    pub fn generation(&self) -> Result<i64> {
        let gen: Option<i64> = self
            .conn
            .query_row("SELECT generation FROM scan_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .ok();
        Ok(gen.unwrap_or(0))
    }

    pub fn bump_generation(&self) -> Result<i64> {
        let next = self.generation()? + 1;
        self.conn.execute(
            "INSERT INTO scan_state (id, generation, phase, started_at, files_total, files_done)
             VALUES (1, ?1, 0, ?2, 0, 0)
             ON CONFLICT(id) DO UPDATE SET generation = ?1, phase = 0, started_at = ?2",
            (next, now_secs()),
        )?;
        Ok(next)
    }

    pub fn track_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM track", [], |r| r.get(0))?)
    }

    pub fn file_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM file", [], |r| r.get(0))?)
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_usable_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.track_count().unwrap(), 0);
        assert_eq!(db.generation().unwrap(), 0);
        assert_eq!(db.bump_generation().unwrap(), 1);
        assert_eq!(db.generation().unwrap(), 1);
    }

    /// A directory, its files and its tracks, as the scan would have written
    /// them. Enough to exercise the album queries without a real library.
    fn seed(db: &Db) {
        db.conn
            .execute_batch(
                "INSERT INTO dir (id, rel_path, scan_gen) VALUES
                    (1, 'At Vance/Dragonchaser', 1),
                    (2, 'At Vance/Dragonchaser/Covers', 1),
                    (3, 'Tears/Seeds', 1);

                 INSERT INTO file (id, dir_id, rel_path, size, mtime_ns, kind, scan_gen) VALUES
                    (1, 1, 'At Vance/Dragonchaser/01.flac', 1, 1, 0, 1),
                    (2, 1, 'At Vance/Dragonchaser/02.flac', 1, 1, 0, 1),
                    (3, 1, 'At Vance/Dragonchaser/cover.jpg', 1, 1, 2, 1),
                    (4, 1, 'At Vance/Dragonchaser/back.jpg', 1, 1, 2, 1),
                    (5, 2, 'At Vance/Dragonchaser/Covers/front.png', 1, 1, 2, 1),
                    (6, 3, 'Tears/Seeds/disc.wv', 1, 1, 0, 1);

                 INSERT INTO album (id, name, album_artist, year) VALUES
                    (1, 'Dragonchaser', 'At Vance', 2001);

                 INSERT INTO track (id, uri, file_id, album_id, title, artist,
                                    album_artist, album, year, codec, duration_ms, hidden,
                                    added_at, modified_at, scan_gen) VALUES
                    (1, 'At Vance/Dragonchaser/01.flac', 1, 1, 'Chained', 'At Vance',
                     'At Vance', 'Dragonchaser', 2001, 'flac', 200000, 0, 0, 0, 1),
                    (2, 'At Vance/Dragonchaser/02.flac', 2, 1, 'Dragonchaser', 'At Vance',
                     'At Vance', 'Dragonchaser', 2001, 'flac', 300000, 0, 0, 0, 1);

                 -- A cue album, exactly as the scan writes one: two virtual
                 -- tracks over one backing file, album_id null on both, no
                 -- year, and a codec of literally \"cue\". The backing file
                 -- keeps its own hidden row, which is where the real year and
                 -- codec live.
                 INSERT INTO track (id, uri, file_id, cue_ordinal, album_id, title, artist,
                                    album_artist, album, year, codec, duration_ms, hidden,
                                    added_at, modified_at, scan_gen) VALUES
                    (3, 'Tears/Seeds/x.cue/track0001', 6, 1, NULL, 'Woman', 'Tears',
                     'Tears', 'Seeds', NULL, 'cue', 100000, 0, 0, 0, 1),
                    (4, 'Tears/Seeds/x.cue/track0002', 6, 2, NULL, 'Bad Man', 'Tears',
                     'Tears', 'Seeds', NULL, 'cue', 150000, 0, 0, 0, 1),
                    (5, 'Tears/Seeds/disc.wv', 6, NULL, NULL, NULL, NULL,
                     NULL, NULL, 1989, 'wavpack', 250000, 1, 0, 0, 1);",
            )
            .unwrap();
    }

    #[test]
    fn an_album_is_summarised_from_its_tracks() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let a = db
            .album_for_uri("At Vance/Dragonchaser/01.flac")
            .unwrap()
            .expect("indexed");
        assert_eq!(a.album.as_deref(), Some("Dragonchaser"));
        assert_eq!(a.artist.as_deref(), Some("At Vance"));
        assert_eq!(a.year, Some(2001));
        assert_eq!(a.codec.as_deref(), Some("flac"));
        assert_eq!(a.track_count, 2);
        assert_eq!(a.total_ms, 500_000);
        assert_eq!(a.dir_id, 1);
        assert_eq!(a.file_rel, "At Vance/Dragonchaser/01.flac");
    }

    #[test]
    fn a_cue_album_is_summarised_even_though_it_has_no_album_id() {
        // The scan never sets album_id on cue virtual tracks. Aggregating on
        // it alone would report every cue album as a single track.
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let a = db
            .album_for_uri("Tears/Seeds/x.cue/track0001")
            .unwrap()
            .expect("indexed");
        assert_eq!(a.album.as_deref(), Some("Seeds"));
        assert_eq!(
            a.file_rel, "Tears/Seeds/disc.wv",
            "the audio underneath, not the sheet"
        );
        assert_eq!(a.track_count, 2, "the whole sheet, not one track");
        assert_eq!(a.total_ms, 250_000);
        // Neither of these is on the cue row; both come from the file beneath.
        assert_eq!(a.year, Some(1989), "the year came from the backing file");
        assert_eq!(
            a.codec.as_deref(),
            Some("wavpack"),
            "the codec must be the audio's, not the literal \"cue\""
        );
    }

    #[test]
    fn a_uri_the_scan_never_saw_is_not_an_error() {
        // Playlists routinely name files the index does not have.
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        assert!(db.album_for_uri("nowhere/at/all.flac").unwrap().is_none());
    }

    #[test]
    fn images_come_back_for_the_track_s_own_directory() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let mut imgs = db.images_in_dir(1).unwrap();
        imgs.sort();
        assert_eq!(
            imgs,
            vec![
                "At Vance/Dragonchaser/back.jpg".to_string(),
                "At Vance/Dragonchaser/cover.jpg".to_string()
            ],
            "audio files must not be in here, nor the subdirectory's art"
        );
    }

    #[test]
    fn child_dirs_are_one_level_only() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let kids = db.child_dirs(1).unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].1, "At Vance/Dragonchaser/Covers");
        // And the leaf has none.
        assert!(db.child_dirs(2).unwrap().is_empty());
    }

    #[test]
    fn stats_survive_a_track_row_being_deleted_and_recreated() {
        // This is the whole reason stats are keyed by URI. A rescan drops and
        // reinserts track rows; play counts and ratings must not go with them.
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO track_stat (uri, play_count, rating, loved)
                 VALUES ('A/B/01.flac', 41, 4, 1)",
                [],
            )
            .unwrap();

        db.conn.execute("DELETE FROM track", []).unwrap();

        let (plays, rating, loved): (i64, i64, i64) = db
            .conn
            .query_row(
                "SELECT play_count, rating, loved FROM track_stat WHERE uri = ?1",
                ["A/B/01.flac"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((plays, rating, loved), (41, 4, 1));
    }

    #[test]
    fn playlist_entries_survive_without_a_resolvable_track() {
        // 243 entries in the reference library's playlists do not resolve.
        // They must still round-trip rather than vanishing on write-back.
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute("INSERT INTO playlist (id, name) VALUES (1, 'test')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO playlist_item (playlist_id, pos, uri, track_id, raw_line)
                 VALUES (1, 0, 'Gone/Album/01.flac', NULL, 'Gone/Album/01.flac')",
                [],
            )
            .unwrap();

        let raw: String = db
            .conn
            .query_row(
                "SELECT raw_line FROM playlist_item WHERE playlist_id = 1 AND pos = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, "Gone/Album/01.flac");
    }

    /// A disc-image cue album: one backing file, one sheet, three tracks --
    /// plus a second album in the same directory that must not bleed into it.
    fn seed_cue(db: &Db, backing_rate: &str) {
        db.conn
            .execute_batch(&format!(
                "INSERT INTO dir (id, rel_path, scan_gen) VALUES (10, 'Rips/Album', 1);

                 INSERT INTO file (id, dir_id, rel_path, size, mtime_ns, kind, scan_gen) VALUES
                    (10, 10, 'Rips/Album/rip.wv',    1, 1, 0, 1),
                    (11, 10, 'Rips/Album/rip.cue',   1, 1, 1, 1),
                    (12, 10, 'Rips/Album/other.wv',  1, 1, 0, 1),
                    (13, 10, 'Rips/Album/other.cue', 1, 1, 1, 1);

                 -- The backing files' own rows. The sample rate here is what
                 -- the scan computed every frame boundary below against.
                 INSERT INTO track (id, uri, file_id, codec, sample_rate, hidden,
                                    added_at, modified_at, scan_gen) VALUES
                    (100, 'Rips/Album/rip.wv',   10, 'wavpack', {backing_rate}, 1, 0, 0, 1),
                    (110, 'Rips/Album/other.wv', 12, 'wavpack', 44100,          1, 0, 0, 1);

                 INSERT INTO track (id, uri, file_id, cue_file_id, cue_ordinal,
                                    cue_track_no, cue_file_index,
                                    start_frame, end_frame, title, artist, album,
                                    codec, added_at, modified_at, scan_gen) VALUES
                    (101, 'Rips/Album/rip.cue/track0001', 10, 11, 1, 1, 0,
                          0,       1000000, 'One',   'A', 'Rip', 'cue', 0, 0, 1),
                    (102, 'Rips/Album/rip.cue/track0002', 10, 11, 2, 2, 0,
                          1000000, 2500000, 'Two',   'A', 'Rip', 'cue', 0, 0, 1),
                    (103, 'Rips/Album/rip.cue/track0003', 10, 11, 3, 3, 0,
                          2500000, NULL,    'Three', 'A', 'Rip', 'cue', 0, 0, 1),
                    -- A different sheet over a different file, same folder.
                    (111, 'Rips/Album/other.cue/track0001', 12, 13, 1, 1, 0,
                          0,       500000,  'Other', 'B', 'Oth', 'cue', 0, 0, 1);"
            ))
            .unwrap();
    }

    #[test]
    fn a_cue_track_brings_its_whole_album_back_in_one_query() {
        let db = Db::open_in_memory().unwrap();
        seed_cue(&db, "44100");

        let rows = db
            .cue_album_for_uri("Rips/Album/rip.cue/track0002")
            .unwrap()
            .expect("an indexed cue track has an album");

        assert_eq!(rows.cue_rel, "Rips/Album/rip.cue");
        assert_eq!(rows.backing_rel, "Rips/Album/rip.wv");
        assert_eq!(rows.tracks.len(), 3, "the siblings come too");

        // Ordered by ordinal, so the player can find its window by position.
        let ordinals: Vec<u32> = rows.tracks.iter().map(|t| t.ordinal).collect();
        assert_eq!(ordinals, vec![1, 2, 3]);

        let two = &rows.tracks[1];
        assert_eq!(two.start_frame, 1_000_000);
        assert_eq!(two.end_frame, Some(2_500_000));
        assert_eq!(two.title.as_deref(), Some("Two"));
        assert_eq!(two.performer.as_deref(), Some("A"));

        // NULL end_frame means "to end of file", not zero.
        assert_eq!(rows.tracks[2].end_frame, None);
    }

    /// The other sheet in the same directory is a different album, and asking
    /// for one must not drag in the other's tracks.
    #[test]
    fn a_sibling_sheet_in_the_same_folder_stays_out() {
        let db = Db::open_in_memory().unwrap();
        seed_cue(&db, "44100");

        let rows = db
            .cue_album_for_uri("Rips/Album/other.cue/track0001")
            .unwrap()
            .unwrap();
        assert_eq!(rows.backing_rel, "Rips/Album/other.wv");
        assert_eq!(rows.tracks.len(), 1);
    }

    /// The guard that makes the fast path safe.
    ///
    /// `scan::write_cue_tracks` computes frame boundaries from the backing
    /// file's indexed sample rate and assumes 44100 when it has none, while
    /// reading the sheet gets the real rate from the file. Where the index
    /// cannot vouch for the rate, the two could disagree and place every track
    /// wrongly -- so the album is declined and the caller reads the sheet.
    #[test]
    fn a_backing_file_with_no_indexed_sample_rate_declines() {
        let db = Db::open_in_memory().unwrap();
        seed_cue(&db, "NULL");
        assert_eq!(
            db.cue_album_for_uri("Rips/Album/rip.cue/track0002")
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_plain_file_and_an_unknown_uri_have_no_cue_album() {
        let db = Db::open_in_memory().unwrap();
        seed_cue(&db, "44100");
        assert_eq!(db.cue_album_for_uri("Rips/Album/rip.wv").unwrap(), None);
        assert_eq!(
            db.cue_album_for_uri("Nowhere/x.cue/track0001").unwrap(),
            None
        );
    }
}

/// How much of the real library the cue fast path actually covers.
///
/// The path declines whenever the backing file has no indexed sample rate, so
/// "it compiled" is not evidence it ever runs. This counts.
#[cfg(test)]
mod cue_coverage {
    use super::*;

    #[test]
    #[ignore = "reads the real library"]
    fn how_many_cue_tracks_can_be_opened_from_the_index() {
        let Ok(index) = crate::paths::index_file() else {
            return;
        };
        let Ok(db) = Db::open_readonly(&index) else {
            return;
        };

        let uris: Vec<String> = db
            .conn
            .prepare(
                "SELECT uri FROM track
                  WHERE cue_file_id IS NOT NULL AND hidden = 0
                  ORDER BY uri",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();

        let mut hit = 0usize;
        for u in &uris {
            if db.cue_album_for_uri(u).unwrap().is_some() {
                hit += 1;
            }
        }
        let missing_rate: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track t
                  WHERE t.cue_file_id IS NOT NULL AND t.hidden = 0
                    AND NOT EXISTS (SELECT 1 FROM track b
                                     WHERE b.file_id = t.file_id
                                       AND b.cue_ordinal IS NULL
                                       AND b.sample_rate IS NOT NULL)",
                [],
                |r| r.get(0),
            )
            .unwrap();

        eprintln!(
            "cue tracks {}  from the index {hit}  declined {}  (of which no backing rate {missing_rate})",
            uris.len(),
            uris.len() - hit,
        );
    }
}
