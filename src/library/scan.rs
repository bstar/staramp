//! Library scanning.
//!
//! Resumable and incremental. A full walk of the reference library is ~60k
//! inodes across 1.1 TB on a removable disk, so the expensive part — reading
//! tags — must happen only for files whose `(size, mtime)` actually changed.
//! That is the difference between a warm rescan taking seconds and taking
//! fifteen minutes.
//!
//! Concurrency follows what SQLite actually allows: many readers, exactly one
//! writer. Tag extraction fans out across rayon, results funnel through a single
//! writer, and writes are batched into transactions. Fighting that model
//! produces `SQLITE_BUSY` under load rather than speed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;
use rusqlite::params;

use super::db::{now_secs, Db};
use super::tags;
use crate::cue;
use crate::playlist::uri::TrackUri;

/// Rows per transaction. Large enough that commit overhead disappears, small
/// enough that an interrupted scan loses little work.
const BATCH: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Audio = 0,
    Cue = 1,
    Image = 2,
    Playlist = 3,
}

/// Extensions worth indexing. Everything else — the `.log`, `.accurip`, `.sfv`,
/// `.nfo`, `.md5` litter that EAC leaves in every rip directory — is discarded
/// during the walk rather than stored and filtered later.
fn classify(path: &Path) -> Option<FileKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    const AUDIO: &[&str] = &[
        "flac", "mp3", "ogg", "oga", "opus", "m4a", "m4b", "mp4", "aac", "alac", "wav", "wave",
        "aif", "aiff", "aifc", "ape", "wv", "mpc", "mp+", "dsf", "dff", "wma", "tta", "tak", "shn",
        "mka",
        // Video containers carry audio streams, and album folders in the
        // reference library hold bonus video tracks that the playlists
        // reference. libav plays their audio; refusing to index them would
        // leave those playlist entries permanently unresolved.
        "mpg", "mpeg", "mov", "mkv", "avi", "webm", "m2ts", "ts", "vob", "flv", "wmv", "m4v",
    ];
    const IMAGE: &[&str] = &["jpg", "jpeg", "png", "bmp", "gif", "webp", "tif", "tiff"];
    if AUDIO.contains(&ext.as_str()) {
        Some(FileKind::Audio)
    } else if ext == "cue" {
        Some(FileKind::Cue)
    } else if IMAGE.contains(&ext.as_str()) {
        Some(FileKind::Image)
    } else if ext == "m3u" || ext == "m3u8" || ext == "pls" {
        Some(FileKind::Playlist)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct FoundFile {
    rel_path: String,
    abs_path: PathBuf,
    size: i64,
    mtime_ns: i64,
    kind: FileKind,
}

#[derive(Debug, Default, Clone)]
pub struct ScanStats {
    pub files_seen: usize,
    pub audio_files: usize,
    pub cue_files: usize,
    pub image_files: usize,
    pub unchanged: usize,
    pub tagged: usize,
    pub tracks_inserted: usize,
    pub cue_tracks: usize,
    pub hidden_backing: usize,
    pub removed: usize,
    pub tag_errors: usize,
    pub elapsed_secs: f64,
}

#[derive(Default)]
pub struct ScanOptions {
    /// Re-read tags for every file, ignoring change detection.
    pub force: bool,
}

pub fn scan(db: &mut Db, root: &Path, opts: &ScanOptions) -> Result<ScanStats> {
    let began = Instant::now();
    let mut stats = ScanStats::default();

    // Phase 0: is the library actually there? A removable mount that is absent
    // must not be mistaken for a library whose every file was deleted.
    if !root.is_dir() {
        anyhow::bail!("library root {} is not a directory", root.display());
    }
    if std::fs::read_dir(root)?.next().is_none() {
        anyhow::bail!(
            "library root {} is empty — refusing to scan, in case the disk is not mounted",
            root.display()
        );
    }

    let generation = db.bump_generation()?;

    // Phase 1: walk.
    let found = walk(root);
    stats.files_seen = found.len();
    for f in &found {
        match f.kind {
            FileKind::Audio => stats.audio_files += 1,
            FileKind::Cue => stats.cue_files += 1,
            FileKind::Image => stats.image_files += 1,
            FileKind::Playlist => {}
        }
    }

    // Phase 2: change detection against what is already indexed.
    let existing = load_existing(db)?;
    let mut changed: Vec<&FoundFile> = Vec::new();
    for f in &found {
        match existing.get(&f.rel_path) {
            Some(&(size, mtime)) if !opts.force && size == f.size && mtime == f.mtime_ns => {
                stats.unchanged += 1;
            }
            _ => changed.push(f),
        }
    }

    // Every file seen keeps its generation current, changed or not; what is left
    // behind at an older generation is what has been deleted.
    touch_generation(db, &found, generation)?;

    // Phase 3: tags, in parallel, for changed audio only.
    let audio_changed: Vec<&FoundFile> = changed
        .iter()
        .copied()
        .filter(|f| f.kind == FileKind::Audio)
        .collect();

    let tagged: Vec<(&FoundFile, Option<tags::TrackTags>)> = audio_changed
        .par_iter()
        .map(|f| {
            let t = tags::read(&f.abs_path)
                .map_err(|e| {
                    tracing::debug!("tags for {}: {e}", f.rel_path);
                })
                .ok();
            (*f, t)
        })
        .collect();

    stats.tag_errors = tagged.iter().filter(|(_, t)| t.is_none()).count();
    stats.tagged = tagged.len();

    // Phase 4: write, batched, single writer.
    write_files(db, &found, generation)?;
    stats.tracks_inserted = write_tracks(db, &tagged, generation)?;

    // Phase 5: cue sheets.
    let (cue_tracks, hidden) = write_cue_tracks(db, root, &found, generation)?;
    stats.cue_tracks = cue_tracks;
    stats.hidden_backing = hidden;

    // Phase 6: drop what is gone, and rebuild search.
    stats.removed = remove_stale(db, generation)?;
    rebuild_fts(db)?;

    stats.elapsed_secs = began.elapsed().as_secs_f64();
    Ok(stats)
}

/// How long a cue track is, in milliseconds.
///
/// A sheet gives each track's *start*, so a track's length is the next one's
/// start minus its own -- except the last of each `FILE` stanza, which the
/// sheet describes only as "to the end of the file". That one has no length at
/// all unless the backing file's own length is brought in, and the symptom is
/// unmistakable: every album's last track showing `-:--`, and every track of a
/// sheet with one `FILE` per track, since each of those is the last of its
/// stanza.
fn cue_duration_ms(
    t: &cue::expand::VirtualTrack,
    rate: u32,
    backing_ms: Option<i64>,
) -> Option<i64> {
    let rate = rate.max(1) as u64;
    if let Some(frames) = t.duration_frames() {
        return Some((frames * 1000 / rate) as i64);
    }
    // Runs to the end of the file, so its length is what is left of the file.
    // A start past the end means a sheet that disagrees with its audio, and a
    // negative duration would be worse than none.
    let start_ms = (t.start_frame * 1000 / rate) as i64;
    backing_ms.map(|total| (total - start_ms).max(0))
}

/// The year in a date, when there is one to find.
///
/// A cue sheet's `REM DATE` is a free-text line: every one of the 10,269 in
/// the reference library is a bare year, but the field is not defined to be,
/// and `2005-06-12` should not become nothing. The first four-digit run in a
/// plausible range wins.
fn year_in(date: &str) -> Option<i64> {
    let digits: Vec<char> = date.chars().collect();
    digits
        .windows(4)
        .filter(|w| w.iter().all(char::is_ascii_digit))
        .find_map(|w| {
            let y: i64 = w.iter().collect::<String>().parse().ok()?;
            (1000..=2999).contains(&y).then_some(y)
        })
}

fn walk(root: &Path) -> Vec<FoundFile> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(kind) = classify(path) else { continue };
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let Some(rel_path) = rel.to_str() else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };

        out.push(FoundFile {
            rel_path: rel_path.to_string(),
            abs_path: path.to_path_buf(),
            size: meta.len() as i64,
            mtime_ns: mtime_ns(&meta),
            kind,
        });
    }
    out
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn load_existing(db: &Db) -> Result<HashMap<String, (i64, i64)>> {
    let mut stmt = db
        .conn
        .prepare("SELECT rel_path, size, mtime_ns FROM file")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?;
    Ok(rows.flatten().collect())
}

/// Ensure a `dir` row exists, returning its id.
fn dir_id(tx: &rusqlite::Transaction, rel_path: &str, generation: i64) -> Result<i64> {
    let dir = Path::new(rel_path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    tx.execute(
        "INSERT INTO dir (rel_path, scan_gen) VALUES (?1, ?2)
         ON CONFLICT(rel_path) DO UPDATE SET scan_gen = ?2",
        params![dir, generation],
    )?;
    Ok(
        tx.query_row("SELECT id FROM dir WHERE rel_path = ?1", [dir], |r| {
            r.get(0)
        })?,
    )
}

fn write_files(db: &mut Db, found: &[FoundFile], generation: i64) -> Result<()> {
    for chunk in found.chunks(BATCH) {
        let tx = db.conn.transaction()?;
        for f in chunk {
            let did = dir_id(&tx, &f.rel_path, generation)?;
            tx.execute(
                "INSERT INTO file (dir_id, rel_path, size, mtime_ns, kind, scan_gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(rel_path) DO UPDATE SET
                   size = ?3, mtime_ns = ?4, kind = ?5, scan_gen = ?6",
                params![
                    did,
                    f.rel_path,
                    f.size,
                    f.mtime_ns,
                    f.kind as i64,
                    generation
                ],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

/// Mark every file we just saw as current for this generation — and the tracks
/// that hang off it too.
///
/// Touching only `file` here was a real bug: a warm rescan re-reads no tags, so
/// unchanged files write no `track` rows, so every one of their tracks kept an
/// old `scan_gen` and `remove_stale` deleted the lot. The first rescan of the
/// reference library dropped all 21,053 plain tracks and left only the cue
/// virtual ones.
fn touch_generation(db: &mut Db, found: &[FoundFile], generation: i64) -> Result<()> {
    for chunk in found.chunks(BATCH) {
        let tx = db.conn.transaction()?;
        for f in chunk {
            tx.execute(
                "UPDATE file SET scan_gen = ?2 WHERE rel_path = ?1",
                params![f.rel_path, generation],
            )?;
            tx.execute(
                "UPDATE track SET scan_gen = ?2
                 WHERE file_id = (SELECT id FROM file WHERE rel_path = ?1)
                    OR cue_file_id = (SELECT id FROM file WHERE rel_path = ?1)",
                params![f.rel_path, generation],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

fn write_tracks(
    db: &mut Db,
    tagged: &[(&FoundFile, Option<tags::TrackTags>)],
    generation: i64,
) -> Result<usize> {
    let now = now_secs();
    let mut n = 0;

    for chunk in tagged.chunks(BATCH) {
        let tx = db.conn.transaction()?;
        for (f, t) in chunk {
            let file_id: i64 = tx.query_row(
                "SELECT id FROM file WHERE rel_path = ?1",
                [&f.rel_path],
                |r| r.get(0),
            )?;

            let t = t.clone().unwrap_or_default();
            let (fn_artist, fn_title) = tags::from_filename(&f.abs_path);
            let title = t.title.clone().or(fn_title);
            let artist = t.artist.clone().or(fn_artist);
            let codec = f
                .abs_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let lossless = matches!(
                codec.as_str(),
                "flac"
                    | "ape"
                    | "wv"
                    | "alac"
                    | "wav"
                    | "aiff"
                    | "aif"
                    | "tta"
                    | "tak"
                    | "dsf"
                    | "dff"
                    | "shn"
            );

            let album_id = upsert_album(
                &tx,
                t.album.as_deref(),
                t.album_artist.as_deref().or(artist.as_deref()),
                t.year,
            )?;

            tx.execute(
                "INSERT INTO track (
                    uri, file_id, title, artist, album_artist, album, album_id,
                    composer, genre, track_no, track_total, disc_no, disc_total,
                    year, date, codec, duration_ms, sample_rate, bit_depth,
                    channels, bitrate_kbps, is_lossless, file_size,
                    rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak,
                    rg_source, mb_recording_id, mb_release_id, mb_artist_id,
                    added_at, modified_at, scan_gen
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                    ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34
                 )
                 ON CONFLICT(uri) DO UPDATE SET
                    file_id=?2, title=?3, artist=?4, album_artist=?5, album=?6,
                    album_id=?7, composer=?8, genre=?9, track_no=?10,
                    track_total=?11, disc_no=?12, disc_total=?13, year=?14,
                    date=?15, codec=?16, duration_ms=?17, sample_rate=?18,
                    bit_depth=?19, channels=?20, bitrate_kbps=?21,
                    is_lossless=?22, file_size=?23, rg_track_gain=?24,
                    rg_track_peak=?25, rg_album_gain=?26, rg_album_peak=?27,
                    rg_source=?28, mb_recording_id=?29, mb_release_id=?30,
                    mb_artist_id=?31, modified_at=?33, scan_gen=?34",
                params![
                    f.rel_path,
                    file_id,
                    title,
                    artist,
                    t.album_artist,
                    t.album,
                    album_id,
                    t.composer,
                    t.genre,
                    t.track_no,
                    t.track_total,
                    t.disc_no,
                    t.disc_total,
                    t.year,
                    t.date,
                    codec,
                    t.duration_ms.map(|d| d as i64),
                    t.sample_rate,
                    t.bit_depth,
                    t.channels,
                    t.bitrate_kbps,
                    lossless as i64,
                    f.size,
                    t.rg_track_gain,
                    t.rg_track_peak,
                    t.rg_album_gain,
                    t.rg_album_peak,
                    // 1 = from tags. Only ~1 in 60 files here actually has them,
                    // which is why the EBU R128 scanner is a real subsystem.
                    if t.rg_track_gain.is_some() {
                        1i64
                    } else {
                        0i64
                    },
                    t.mb_recording_id,
                    t.mb_release_id,
                    t.mb_artist_id,
                    now,
                    now,
                    generation,
                ],
            )?;
            n += 1;
        }
        tx.commit()?;
    }
    Ok(n)
}

fn upsert_album(
    tx: &rusqlite::Transaction,
    name: Option<&str>,
    album_artist: Option<&str>,
    year: Option<u32>,
) -> Result<Option<i64>> {
    let Some(name) = name else { return Ok(None) };
    tx.execute(
        "INSERT INTO album (name, album_artist, year) VALUES (?1, ?2, ?3)
         ON CONFLICT(name, album_artist, year) DO NOTHING",
        params![name, album_artist, year],
    )?;
    let id = tx
        .query_row(
            "SELECT id FROM album WHERE name IS ?1 AND album_artist IS ?2 AND year IS ?3",
            params![name, album_artist, year],
            |r| r.get(0),
        )
        .ok();
    Ok(id)
}

/// Expand every cue sheet into virtual tracks.
///
/// Returns `(virtual tracks written, backing files hidden)`.
fn write_cue_tracks(
    db: &mut Db,
    root: &Path,
    found: &[FoundFile],
    generation: i64,
) -> Result<(usize, usize)> {
    let cues: Vec<&FoundFile> = found.iter().filter(|f| f.kind == FileKind::Cue).collect();
    if cues.is_empty() {
        return Ok((0, 0));
    }

    // Sample rates and lengths come from what the scanner already read, so
    // expanding a cue does not reopen every backing file. The length is needed
    // for the same reason the rate is: see `cue_duration_ms`.
    let backing_files: HashMap<String, (u32, Option<i64>)> = {
        let mut stmt = db.conn.prepare(
            "SELECT uri, sample_rate, duration_ms FROM track WHERE sample_rate IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, u32>(1)?, r.get::<_, Option<i64>>(2)?),
            ))
        })?;
        rows.flatten().collect()
    };

    // Parse and resolve in parallel; write serially.
    let expanded: Vec<(String, Vec<cue::expand::VirtualTrack>, bool, Vec<String>)> = cues
        .par_iter()
        .filter_map(|f| {
            let sheet = cue::parser::parse_file(&f.abs_path).ok()?;
            let res = cue::resolve::resolve(&sheet, &f.abs_path);
            if !res.is_indexable() {
                return None;
            }
            let file_rates: Vec<u32> = res
                .files
                .iter()
                .map(|rf| {
                    rf.as_ref()
                        .and_then(|rf| rf.path.strip_prefix(root).ok())
                        .and_then(|p| p.to_str())
                        .and_then(|p| backing_files.get(p))
                        .map(|(rate, _)| *rate)
                        .unwrap_or(44_100)
                })
                .collect();
            let tracks = cue::expand::expand(
                &sheet,
                &res,
                &file_rates,
                cue::expand::PregapPolicy::default(),
            );
            let backing: Vec<String> = res
                .files
                .iter()
                .flatten()
                .filter_map(|rf| rf.path.strip_prefix(root).ok())
                .filter_map(|p| p.to_str().map(|s| s.to_string()))
                .collect();
            Some((
                f.rel_path.clone(),
                tracks,
                res.suppresses_backing_files(),
                backing,
            ))
        })
        .collect();

    let now = now_secs();
    let mut written = 0usize;
    let mut hidden = 0usize;

    for chunk in expanded.chunks(64) {
        let tx = db.conn.transaction()?;
        for (cue_rel, tracks, suppress, backing) in chunk {
            let cue_file_id: i64 =
                tx.query_row("SELECT id FROM file WHERE rel_path = ?1", [cue_rel], |r| {
                    r.get(0)
                })?;

            for t in tracks {
                let Ok(backing_rel) = t.backing_path.strip_prefix(root) else {
                    continue;
                };
                let Some(backing_rel) = backing_rel.to_str() else {
                    continue;
                };
                let Ok(file_id) = tx.query_row::<i64, _, _>(
                    "SELECT id FROM file WHERE rel_path = ?1",
                    [backing_rel],
                    |r| r.get(0),
                ) else {
                    continue;
                };

                let uri = TrackUri::CueTrack {
                    cue_rel_path: cue_rel.clone(),
                    ordinal: t.ordinal,
                }
                .to_string();

                let (rate, backing_ms) = backing_files
                    .get(backing_rel)
                    .copied()
                    .unwrap_or((44_100, None));
                let duration_ms = cue_duration_ms(t, rate, backing_ms);
                let year = t.date.as_deref().and_then(year_in);

                tx.execute(
                    "INSERT INTO track (
                        uri, file_id, cue_file_id, cue_ordinal, cue_track_no,
                        cue_file_index, start_frame, end_frame,
                        title, artist, album_artist, album, genre, track_no, date,
                        year, codec, duration_ms, added_at, modified_at, scan_gen
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                        ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
                     )
                     ON CONFLICT(uri) DO UPDATE SET
                        file_id=?2, cue_file_id=?3, cue_ordinal=?4, cue_track_no=?5,
                        cue_file_index=?6, start_frame=?7, end_frame=?8, title=?9,
                        artist=?10, album_artist=?11, album=?12, genre=?13,
                        track_no=?14, date=?15, year=?16, codec=?17, duration_ms=?18,
                        modified_at=?20, scan_gen=?21",
                    params![
                        uri,
                        file_id,
                        cue_file_id,
                        t.ordinal,
                        t.number,
                        t.file_index as i64,
                        t.start_frame as i64,
                        t.end_frame.map(|e| e as i64),
                        t.title,
                        t.performer,
                        t.album_artist,
                        t.album,
                        t.genre,
                        t.number,
                        t.date,
                        year,
                        "cue",
                        duration_ms,
                        now,
                        now,
                        generation,
                    ],
                )?;
                written += 1;
            }

            // A disc image's backing file is hidden so one 70-minute track does
            // not appear alongside the thirteen carved out of it. A per-track
            // cue's backing files are the tracks and must stay visible.
            if *suppress {
                for b in backing {
                    hidden += tx.execute("UPDATE track SET hidden = 1 WHERE uri = ?1", [b])?;
                }
            }
        }
        tx.commit()?;
    }

    Ok((written, hidden))
}

/// Drop what is no longer on disk.
///
/// Files are the authority: deleting a stale `file` row cascades to its tracks,
/// which is both correct and impossible to get out of step with. Tracks are
/// additionally swept by generation to catch cue virtual tracks whose sheet
/// still exists but no longer lists them.
fn remove_stale(db: &mut Db, generation: i64) -> Result<usize> {
    let before: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM track", [], |r| r.get(0))?;

    let tx = db.conn.transaction()?;
    tx.execute("DELETE FROM file WHERE scan_gen < ?1", [generation])?;
    tx.execute("DELETE FROM track WHERE scan_gen < ?1", [generation])?;
    tx.execute("DELETE FROM dir WHERE scan_gen < ?1", [generation])?;
    tx.commit()?;

    let after: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM track", [], |r| r.get(0))?;
    Ok((before - after).max(0) as usize)
}

fn rebuild_fts(db: &Db) -> Result<()> {
    db.conn
        .execute("INSERT INTO track_fts(track_fts) VALUES('rebuild')", [])
        .context("rebuilding full-text index")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classifies_audio_cue_and_image_and_ignores_rip_litter() {
        let k = |s: &str| classify(&PathBuf::from(s));
        assert_eq!(k("a.flac"), Some(FileKind::Audio));
        assert_eq!(k("a.APE"), Some(FileKind::Audio));
        assert_eq!(k("a.cue"), Some(FileKind::Cue));
        assert_eq!(k("cover.jpg"), Some(FileKind::Image));
        assert_eq!(k("x.m3u"), Some(FileKind::Playlist));
        // EAC leaves these in every rip directory; indexing them is pure cost.
        for litter in ["a.log", "a.accurip", "a.sfv", "a.nfo", "a.md5", "a.txt"] {
            assert_eq!(k(litter), None, "{litter}");
        }
    }

    /// A cue track starting at `start_sec`, ending at `end_sec` if it has an
    /// end at all.
    fn vtrack(start_sec: u64, end_sec: Option<u64>) -> cue::expand::VirtualTrack {
        cue::expand::VirtualTrack {
            ordinal: 1,
            number: 1,
            file_index: 0,
            backing_path: PathBuf::from("a.flac"),
            start_frame: start_sec * 44_100,
            end_frame: end_sec.map(|e| e * 44_100),
            pregap_start_frame: None,
            title: None,
            performer: None,
            album: None,
            album_artist: None,
            genre: None,
            date: None,
        }
    }

    #[test]
    fn a_year_is_found_in_whatever_shape_the_date_takes() {
        assert_eq!(year_in("2005"), Some(2005));
        assert_eq!(year_in("2005-06-12"), Some(2005));
        assert_eq!(year_in("12/06/2005"), Some(2005));
        assert_eq!(year_in("June 2005"), Some(2005));
        assert_eq!(year_in("1999"), Some(1999));
    }

    #[test]
    fn a_date_with_no_year_in_it_gives_none_rather_than_a_guess() {
        assert_eq!(year_in(""), None);
        assert_eq!(year_in("05"), None, "two digits is not a year");
        assert_eq!(year_in("unknown"), None);
        assert_eq!(year_in("0999"), None, "before recorded music");
        assert_eq!(
            year_in("MICP-10504"),
            Some(1050),
            "a catalogue number reads as one"
        );
    }

    #[test]
    fn a_cue_track_with_a_next_one_is_measured_between_them() {
        let t = vtrack(60, Some(240));
        assert_eq!(cue_duration_ms(&t, 44_100, Some(3_600_000)), Some(180_000));
        // And the backing length is not needed when the sheet answers.
        assert_eq!(cue_duration_ms(&t, 44_100, None), Some(180_000));
    }

    #[test]
    fn the_last_track_of_a_stanza_runs_to_the_end_of_the_file() {
        // The bug this exists for: every album's last track showed `-:--`,
        // because a sheet says where a track starts and only the audio file
        // says where the last one stops.
        let t = vtrack(2_700, None);
        assert_eq!(cue_duration_ms(&t, 44_100, Some(3_000_000)), Some(300_000));
    }

    #[test]
    fn an_unmeasurable_last_track_stays_unmeasured() {
        // No length for the backing file either: better to say nothing than to
        // invent a number.
        assert_eq!(cue_duration_ms(&vtrack(60, None), 44_100, None), None);
    }

    #[test]
    fn a_sheet_that_runs_past_its_audio_gives_nothing_rather_than_a_negative() {
        let t = vtrack(4_000, None);
        assert_eq!(cue_duration_ms(&t, 44_100, Some(3_000_000)), Some(0));
    }

    #[test]
    fn a_hi_res_rip_is_measured_at_its_own_rate() {
        // Frames, not seconds: a 96 kHz rip measured at 44.1 would come out
        // more than twice as long as it is.
        let t = cue::expand::VirtualTrack {
            start_frame: 60 * 96_000,
            end_frame: Some(240 * 96_000),
            ..vtrack(0, None)
        };
        assert_eq!(cue_duration_ms(&t, 96_000, None), Some(180_000));
    }
}
