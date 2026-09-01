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
use crate::playlist::uri::TrackUri;

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
    pub cue_path: PathBuf,
    pub backing_path: PathBuf,
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
    pub fn window_onto(&self, root: &Path, uri: &TrackUri) -> Option<&VirtualTrack> {
        let TrackUri::CueTrack {
            cue_rel_path,
            ordinal,
        } = uri
        else {
            return None;
        };
        if absolutise(root, cue_rel_path) != self.cue_path {
            return None;
        }
        let t = self.track(*ordinal)?;
        (t.backing_path == self.backing_path).then_some(t)
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
/// `root` is the library root that relative URIs are resolved against. An
/// absolute path in the URI is used as-is, which is what the CLI passes.
pub fn open(root: &Path, uri: &TrackUri) -> Result<OpenedTrack> {
    match uri {
        TrackUri::File { rel_path } => {
            let path = absolutise(root, rel_path);
            let decoder = decode::open(&path)?;
            Ok(OpenedTrack {
                decoder,
                virtual_track: None,
                backing_path: path,
                album: None,
            })
        }
        TrackUri::CueTrack {
            cue_rel_path,
            ordinal,
        } => open_cue_track(&absolutise(root, cue_rel_path), *ordinal),
    }
}

pub fn open_cue_track(cue_path: &Path, ordinal: u32) -> Result<OpenedTrack> {
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
        let d = decode::open(&rf.path)
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
        _ => decode::open(&track.backing_path)?,
    };
    let decoder = Box::new(SliceDecoder::new(
        backing,
        track.start_frame,
        track.end_frame,
    ));

    // Only this stanza's tracks: the others' rates were never read, so their
    // boundaries are not to be trusted.
    let album = CueAlbum {
        cue_path: cue_path.to_path_buf(),
        backing_path: track.backing_path.clone(),
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

fn absolutise(root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
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
            let mut times = Vec::new();
            for u in &uris {
                let uri = crate::playlist::uri::TrackUri::parse(u);
                let t0 = Instant::now();
                if super::open(&root, &uri).is_ok() {
                    times.push(t0.elapsed().as_secs_f64() * 1000.0);
                }
            }
            if times.is_empty() {
                eprintln!("{label:24} no samples");
                continue;
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            eprintln!(
                "{label:24} n={} min {:6.1} ms  median {:6.1} ms  max {:6.1} ms",
                times.len(),
                times[0],
                times[times.len() / 2],
                times[times.len() - 1]
            );
        }
        eprintln!("\nthe ring holds {} ms", crate::audio::ring::RING_MS);
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
        let opened = open(&root, &first).expect("opening the first track");
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
            .window_onto(&root, &second)
            .expect("track two is in the same file");
        assert!(window.start_frame > 0, "track two starts after track one");

        // And a different sheet is not, so the caller opens it properly.
        let elsewhere = TrackUri::CueTrack {
            cue_rel_path: "Somewhere Else/other.cue".into(),
            ordinal: 2,
        };
        assert!(album.window_onto(&root, &elsewhere).is_none());

        // The decoder really does move rather than reopen.
        let mut d = opened.decoder;
        assert!(
            d.retarget_slice(window.start_frame, window.end_frame),
            "a cue slice must be retargetable"
        );
        assert_eq!(d.position(), 0, "the new track starts at zero");
    }
}
