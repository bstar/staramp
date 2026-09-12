//! Opening a track URI as a decoder.
//!
//! This is where cue virtual tracks stop being special. Above this line
//! everything deals in a `Box<dyn Decoder>` with track-relative positions.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use super::decode::{self, slice::SliceDecoder, Decoder};
use crate::cue::{
    expand::{expand, PregapPolicy, VirtualTrack},
    parser, resolve,
};
use crate::library::db::{CueAlbumRows, Db};
use crate::playlist::uri::TrackUri;
use crate::vfs::Vfs;

/// What was opened, for display.
pub struct OpenedTrack {
    pub decoder: Box<dyn Decoder>,
    /// Populated for cue virtual tracks.
    pub virtual_track: Option<VirtualTrack>,
    /// The audio file actually being read.
    pub backing_path: PathBuf,
    /// The rest of this track's stanza, when it came from a cue sheet.
    pub album: Option<CueAlbum>,
}

/// The other tracks carved out of the *same backing file* as the one opened.
///
/// Held by the player so that advancing inside a cue album costs nothing: the
/// decoder is already at the right sample, and all that is needed is to move
/// the window. Without it, every track change inside an album re-reads and
/// re-parses the sheet and reopens the audio file, with at most a ring's worth
/// of audio left to cover it.
///
/// Deliberately one stanza, not the whole sheet. Only this stanza's sample rate
/// was read, so only its tracks have trustworthy frame boundaries -- and a
/// track in another stanza is in another file, which has to be opened anyway.
#[derive(Clone)]
pub struct CueAlbum {
    /// The sheet, spelled exactly as the URIs that name it do.
    pub cue_rel: String,
    /// The one backing audio file these tracks are carved out of.
    pub backing_rel: String,
    pub tracks: Vec<VirtualTrack>,
}

impl CueAlbum {
    pub fn track(&self, ordinal: u32) -> Option<&VirtualTrack> {
        self.tracks
            .iter()
            .find(|t| t.ordinal == ordinal)
            .or_else(|| self.tracks.iter().find(|t| t.number == ordinal))
    }

    /// The track `uri` names, if it is another window over the file already
    /// open -- in which case advancing to it needs no disk at all.
    ///
    /// `None` for anything else, and the caller opens the track properly.
    ///
    /// Compared as URI text rather than as resolved paths. URIs are preserved
    /// byte for byte on the way in and out of the index, so two spellings of
    /// one sheet is not a case that arises -- and text needs no library root,
    /// which is what lets this work when the file is not on this machine.
    pub fn window_onto(&self, uri: &TrackUri) -> Option<&VirtualTrack> {
        let TrackUri::CueTrack {
            cue_rel_path,
            ordinal,
        } = uri
        else {
            return None;
        };
        if *cue_rel_path != self.cue_rel {
            return None;
        }
        self.track(*ordinal)
    }
}

/// Which `FILE` stanza holds `ordinal`, from the sheet alone.
///
/// Ordinals are assigned by parse order across the whole sheet and do not
/// depend on any sample rate, so this needs no disk -- which is the point. It
/// is what lets the open below touch one audio file instead of all of them.
fn stanza_of(sheet: &crate::cue::model::CueSheet, ordinal: u32) -> Option<usize> {
    let mut seen = 0u32;
    for (fi, file) in sheet.files.iter().enumerate() {
        let n = file.tracks.len() as u32;
        if ordinal > seen && ordinal <= seen + n {
            return Some(fi);
        }
        seen += n;
    }
    None
}

/// Open a URI, resolving cue virtual tracks against their sheet.
///
/// `vfs` is what root-relative URIs are resolved against. An absolute path in
/// the URI is used as-is, which is what the CLI passes.
pub fn open(vfs: &Vfs, index: Option<&Db>, uri: &TrackUri) -> Result<OpenedTrack> {
    match uri {
        TrackUri::File { rel_path } => {
            let media = vfs.media(rel_path)?;
            let backing_path = vfs
                .local_path(rel_path)
                .unwrap_or_else(|| PathBuf::from(rel_path));
            let decoder = decode::open(media, rel_path)?;
            Ok(OpenedTrack {
                decoder,
                virtual_track: None,
                backing_path,
                album: None,
            })
        }
        TrackUri::CueTrack {
            cue_rel_path,
            ordinal,
        } => {
            // The index already holds every boundary the sheet would be read
            // for. Where it does, that is the whole open.
            if let Some(db) = index {
                match db.cue_album_for_uri(&uri.to_string()) {
                    Ok(Some(rows)) => {
                        return open_cue_from_index(vfs, rows, cue_rel_path, *ordinal)
                    }
                    Ok(None) => {}
                    // An unreadable index is not a reason not to play: the
                    // sheet is still there and still authoritative.
                    Err(e) => tracing::debug!("{uri}: index lookup failed, reading the sheet: {e}"),
                }
            }
            let cue_path = vfs
                .local_path(cue_rel_path)
                .ok_or_else(|| anyhow!("{cue_rel_path}: no local path for a cue sheet"))?;
            open_cue_track(&cue_path, cue_rel_path, *ordinal)
        }
    }
}

/// Open a cue virtual track from what the scan already recorded.
///
/// The sheet is not read, the directory is not listed, and the backing file is
/// opened exactly once -- against the sheet-based path below, which reads and
/// character-set-guesses the sheet, lists the directory to match its `FILE`
/// references, and opens the backing file a first time purely to ask its
/// sample rate.
fn open_cue_from_index(
    vfs: &Vfs,
    rows: CueAlbumRows,
    cue_rel: &str,
    ordinal: u32,
) -> Result<OpenedTrack> {
    let backing_path = vfs
        .local_path(&rows.backing_rel)
        .unwrap_or_else(|| PathBuf::from(&rows.backing_rel));

    let tracks: Vec<VirtualTrack> = rows
        .tracks
        .iter()
        .map(|r| VirtualTrack {
            ordinal: r.ordinal,
            number: r.number,
            file_index: r.file_index,
            backing_path: backing_path.clone(),
            start_frame: r.start_frame,
            end_frame: r.end_frame,
            // `expand` writes this and nothing has ever read it. Recomputing
            // the boundary policy would need a re-index anyway -- see the note
            // in `scan::write_cue_tracks`.
            pregap_start_frame: None,
            title: r.title.clone(),
            performer: r.performer.clone(),
            album: r.album.clone(),
            album_artist: r.album_artist.clone(),
            genre: r.genre.clone(),
            date: r.date.clone(),
        })
        .collect();

    let track = tracks
        .iter()
        .find(|t| t.ordinal == ordinal)
        // As in the sheet path: a non-contiguous sheet can still be addressed
        // by the number written in it.
        .or_else(|| tracks.iter().find(|t| t.number == ordinal))
        .ok_or_else(|| {
            anyhow!(
                "{cue_rel}: no track {ordinal} (the index has {})",
                tracks.len()
            )
        })?
        .clone();

    let media = vfs.media(&rows.backing_rel)?;
    let backing = decode::open(media, &rows.backing_rel)
        .with_context(|| format!("opening backing file {}", rows.backing_rel))?;
    let decoder = Box::new(SliceDecoder::new(
        backing,
        track.start_frame,
        track.end_frame,
    ));

    Ok(OpenedTrack {
        backing_path: track.backing_path.clone(),
        virtual_track: Some(track),
        decoder,
        album: Some(CueAlbum {
            cue_rel: cue_rel.to_string(),
            backing_rel: rows.backing_rel,
            tracks,
        }),
    })
}

/// Open a cue virtual track by reading its sheet.
///
/// The fallback, for a sheet the scan has never seen: `staramp ui
/// /path/to/album`, or a playlist naming something outside the library.
/// `cue_rel` is how the URI spelled the sheet, which is what the album is keyed
/// on afterwards.
pub fn open_cue_track(cue_path: &Path, cue_rel: &str, ordinal: u32) -> Result<OpenedTrack> {
    let sheet = parser::parse_file(cue_path)?;
    let resolution = resolve::resolve(&sheet, cue_path);
    if !resolution.is_indexable() {
        return Err(anyhow!(
            "{}: not indexable ({:?})",
            cue_path.display(),
            resolution.disposition
        ));
    }

    // The backing file's own sample rate decides where every boundary falls, so
    // it has to be read from the file rather than assumed. Hi-res cue-split rips
    // exist in real libraries and a hardcoded 44100 misplaces every track on them.
    //
    // Only the stanza this track belongs to is opened. Opening all of them cost
    // one decoder per `FILE` -- twenty-two of them on the worst sheet in the
    // reference library -- every time a track changed, inline, while the ring
    // drained. The rate of a stanza only affects tracks *in* that stanza
    // (`expand` reads it per `FILE`), so the others can be left unread; the
    // tracks that depend on them are dropped below rather than returned wrong.
    let want = stanza_of(&sheet, ordinal);
    let mut rates = vec![44_100u32; sheet.files.len()];
    let mut opened: Option<Box<dyn Decoder>> = None;
    let mut opened_path: Option<PathBuf> = None;
    for (fi, f) in resolution.files.iter().enumerate() {
        // When the ordinal is not in any stanza -- a sheet addressed by its
        // written TRACK number rather than by position -- fall back to reading
        // every rate, because any of them might be the one.
        if want.is_some() && want != Some(fi) {
            continue;
        }
        let Some(rf) = f else { continue };
        let d = decode::open_path(&rf.path)
            .with_context(|| format!("opening backing file {}", rf.path.display()))?;
        rates[fi] = d.spec().sample_rate;
        if want == Some(fi) {
            opened_path = Some(rf.path.clone());
            opened = Some(d);
        }
    }

    let tracks = expand(&sheet, &resolution, &rates, PregapPolicy::default());
    let track = tracks
        .iter()
        .find(|t| t.ordinal == ordinal)
        // A sheet with non-contiguous TRACK numbers can still be addressed by
        // the number written in it, so fall back to that before giving up.
        .or_else(|| tracks.iter().find(|t| t.number == ordinal))
        .ok_or_else(|| {
            anyhow!(
                "{}: no track {ordinal} (sheet has {} playable tracks)",
                cue_path.display(),
                tracks.len()
            )
        })?
        .clone();

    // Reuse the decoder opened for the rate rather than opening the same file a
    // second time -- it is at frame zero either way, and the slice positions
    // itself on first read.
    let backing = match opened {
        Some(d) if opened_path.as_deref() == Some(track.backing_path.as_path()) => d,
        _ => decode::open_path(&track.backing_path)?,
    };
    let decoder = Box::new(SliceDecoder::new(
        backing,
        track.start_frame,
        track.end_frame,
    ));

    // Only this stanza's tracks: the others' rates were never read, so their
    // boundaries are not to be trusted.
    let album = CueAlbum {
        cue_rel: cue_rel.to_string(),
        backing_rel: track.backing_path.to_string_lossy().into_owned(),
        tracks: tracks
            .iter()
            .filter(|t| t.backing_path == track.backing_path)
            .cloned()
            .collect(),
    };

    Ok(OpenedTrack {
        backing_path: track.backing_path.clone(),
        virtual_track: Some(track),
        decoder,
        album: Some(album),
    })
}

#[cfg(test)]
mod open_timing {
    use super::*;
    use std::time::Instant;

    /// How long a track change actually blocks the decode loop.
    ///
    /// The ring holds 200 ms. Anything slower than that is an underrun the
    /// listener hears as a gap or a click.
    #[test]
    #[ignore = "reads the real library"]
    fn how_long_does_opening_a_track_take() {
        let Ok(cfg) = crate::config::Config::load() else {
            return;
        };
        let Some(root) = cfg.library_root.clone() else {
            return;
        };
        if !root.is_dir() {
            eprintln!("library not mounted, skipping");
            return;
        }
        let Ok(index) = crate::paths::index_file() else {
            return;
        };
        let db = crate::library::db::Db::open_readonly(&index).unwrap();
        let vfs = Vfs::local(root);

        for (label, sql) in [
            (
                "plain file",
                "SELECT uri FROM track WHERE cue_file_id IS NULL AND hidden = 0 LIMIT 6",
            ),
            (
                "cue, disc image",
                "SELECT t.uri FROM track t JOIN file f ON f.id = t.cue_file_id
                 WHERE t.cue_file_id IS NOT NULL AND t.hidden = 0
                   AND (SELECT COUNT(DISTINCT file_id) FROM track o
                        WHERE o.cue_file_id = t.cue_file_id) = 1
                 LIMIT 6",
            ),
            (
                "cue, per-track sheet",
                "SELECT t.uri FROM track t
                 WHERE t.cue_file_id IS NOT NULL AND t.hidden = 0
                   AND (SELECT COUNT(DISTINCT file_id) FROM track o
                        WHERE o.cue_file_id = t.cue_file_id) > 8
                 LIMIT 6",
            ),
        ] {
            let uris: Vec<String> = db
                .conn
                .prepare(sql)
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .flatten()
                .collect();
            // Both ways round, so the cost the index removes is visible
            // rather than asserted. `None` is the path that reads the sheet,
            // lists the directory and opens the backing file for its rate.
            let timed = |index: Option<&crate::library::db::Db>| -> Vec<f64> {
                let mut times = Vec::new();
                for u in &uris {
                    let uri = crate::playlist::uri::TrackUri::parse(u);
                    let t0 = Instant::now();
                    if super::open(&vfs, index, &uri).is_ok() {
                        times.push(t0.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                times
            };
            let from_sheet = timed(None);
            let from_index = timed(Some(&db));

            if from_index.is_empty() {
                eprintln!("{label:24} no samples");
                continue;
            }
            let median = |t: &[f64]| t.get(t.len() / 2).copied().unwrap_or(f64::NAN);
            eprintln!(
                "{label:24} n={:2}  index {:6.1} ms   sheet {:6.1} ms  (medians)",
                from_index.len(),
                median(&from_index),
                median(&from_sheet),
            );
            assert!(
                *from_index.last().unwrap() < crate::audio::ring::RING_MS as f64,
                "{label}: the slowest open must fit inside the ring, got {:.1} ms",
                from_index.last().unwrap()
            );
        }
        eprintln!("\nthe ring holds {} ms", crate::audio::ring::RING_MS);
    }

    /// The index and the sheet must place every boundary identically.
    ///
    /// This is the one thing that makes reading boundaries out of the index
    /// safe. `scan::write_cue_tracks` computed them from the backing file's
    /// indexed sample rate; `open_cue_track` reads the rate from the file
    /// itself. Where those two rates differ, every track on the record starts
    /// in the wrong place -- silently, and with a plausible-looking duration.
    /// So compare the two paths on real records rather than trust the guard.
    #[test]
    #[ignore = "reads the real library"]
    fn the_index_and_the_sheet_agree_on_every_boundary() {
        let Ok(cfg) = crate::config::Config::load() else {
            return;
        };
        let Some(root) = cfg.library_root.clone() else {
            return;
        };
        if !root.is_dir() {
            eprintln!("library not mounted, skipping");
            return;
        }
        let Ok(index) = crate::paths::index_file() else {
            return;
        };
        let db = crate::library::db::Db::open_readonly(&index).unwrap();
        let vfs = Vfs::local(root);

        // Spread across the library rather than the first N, which would all
        // come from one artist and one ripping habit.
        let uris: Vec<String> = db
            .conn
            .prepare(
                "SELECT uri FROM track
                  WHERE cue_file_id IS NOT NULL AND hidden = 0
                  ORDER BY uri
                  LIMIT 400",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();

        let (mut compared, mut skipped) = (0usize, 0usize);
        for u in &uris {
            let uri = TrackUri::parse(u);
            let (Ok(from_index), Ok(from_sheet)) =
                (open(&vfs, Some(&db), &uri), open(&vfs, None, &uri))
            else {
                skipped += 1;
                continue;
            };
            let (Some(a), Some(b)) = (&from_index.virtual_track, &from_sheet.virtual_track) else {
                skipped += 1;
                continue;
            };
            assert_eq!(a.start_frame, b.start_frame, "{u}: start frame");
            assert_eq!(a.end_frame, b.end_frame, "{u}: end frame");
            assert_eq!(
                from_index.backing_path, from_sheet.backing_path,
                "{u}: backing file"
            );
            compared += 1;
        }
        eprintln!("compared {compared} cue tracks both ways, skipped {skipped}");
        assert!(compared > 0, "nothing was actually compared");
    }

    /// The whole point of the album cache: the next track of a cue album is a
    /// window onto the file already open, so advancing needs no disk.
    #[test]
    #[ignore = "reads the real library"]
    fn the_next_track_of_a_cue_album_needs_no_disk() {
        let Ok(cfg) = crate::config::Config::load() else {
            return;
        };
        let Some(root) = cfg.library_root.clone() else {
            return;
        };
        if !root.is_dir() {
            return;
        }
        let Ok(index) = crate::paths::index_file() else {
            return;
        };
        let db = crate::library::db::Db::open_readonly(&index).unwrap();
        // A disc-image sheet: many tracks, one backing file.
        let uri: String = db
            .conn
            .query_row(
                "SELECT t.uri FROM track t
                   WHERE t.cue_file_id IS NOT NULL AND t.hidden = 0 AND t.cue_ordinal = 1
                     AND (SELECT COUNT(DISTINCT file_id) FROM track o
                          WHERE o.cue_file_id = t.cue_file_id) = 1
                     AND (SELECT COUNT(*) FROM track o
                          WHERE o.cue_file_id = t.cue_file_id) > 3
                   LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let first = TrackUri::parse(&uri);
        let vfs = Vfs::local(root);
        let opened = open(&vfs, Some(&db), &first).expect("opening the first track");
        let album = opened.album.expect("a cue track must carry its album");
        assert!(
            album.tracks.len() > 3,
            "the album should hold its siblings, got {}",
            album.tracks.len()
        );

        // Track two of the same sheet is a window onto the same file.
        let TrackUri::CueTrack { cue_rel_path, .. } = &first else {
            unreachable!()
        };
        let second = TrackUri::CueTrack {
            cue_rel_path: cue_rel_path.clone(),
            ordinal: 2,
        };
        let window = album
            .window_onto(&second)
            .expect("track two is in the same file");
        assert!(window.start_frame > 0, "track two starts after track one");

        // And a different sheet is not, so the caller opens it properly.
        let elsewhere = TrackUri::CueTrack {
            cue_rel_path: "Somewhere Else/other.cue".into(),
            ordinal: 2,
        };
        assert!(album.window_onto(&elsewhere).is_none());

        // The decoder really does move rather than reopen.
        let mut d = opened.decoder;
        assert!(
            d.retarget_slice(window.start_frame, window.end_frame),
            "a cue slice must be retargetable"
        );
        assert_eq!(d.position(), 0, "the new track starts at zero");
    }
}
