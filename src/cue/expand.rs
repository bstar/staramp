//! Flattening a cue sheet into playable virtual tracks.

use std::path::{Path, PathBuf};

use super::model::{CueSheet, Msf};
use super::resolve::Resolution;
use crate::playlist::uri::TrackUri;

/// One playable track carved out of a backing audio file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualTrack {
    /// 1-based, assigned by parse order across the **whole sheet**, continuing
    /// across `FILE` stanzas. This is what MPD's `trackNNNN` counts, and it is
    /// not the same as `number` when a sheet is non-contiguous or multi-FILE.
    pub ordinal: u32,
    /// The `TRACK` number as written.
    pub number: u32,
    /// Which `FILE` stanza this track belongs to. Required: 119 sheets in the
    /// reference library have more than one.
    pub file_index: usize,
    pub backing_path: PathBuf,

    pub start_frame: u64,
    /// `None` means "to end of file".
    pub end_frame: Option<u64>,
    /// Where the pregap starts, when the sheet writes `INDEX 00` separately.
    /// Kept so the boundary policy can be changed without re-indexing.
    pub pregap_start_frame: Option<u64>,

    pub title: Option<String>,
    pub performer: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
}

impl VirtualTrack {
    pub fn duration_frames(&self) -> Option<u64> {
        self.end_frame.map(|e| e.saturating_sub(self.start_frame))
    }
}

/// Where a track ends when the next one begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PregapPolicy {
    /// A track runs to the *next track's* `INDEX 01`, so the pregap belongs to
    /// the track before it.
    ///
    /// This is the default because it makes the virtual tracks a complete,
    /// non-overlapping partition of the file: every sample belongs to exactly
    /// one track, and playing the album is bit-identical to playing the file
    /// straight through. The alternative silently discards audio — the Dream
    /// Theater *Octavarium* sheet has 18 seconds between track 2's `INDEX 00`
    /// and `INDEX 01`, and it is music, not silence.
    #[default]
    Previous,
    /// A track runs to the next track's `INDEX 00` where present. Matches some
    /// other players; loses the pregap audio.
    Next,
}

/// Flatten a resolved sheet into virtual tracks.
///
/// `sample_rates` supplies the rate of each backing file, in `FILE` order. The
/// rate must come from the actual audio file: hi-res cue-split rips exist, and a
/// hardcoded 44100 misplaces every boundary on them.
pub fn expand(
    sheet: &CueSheet,
    resolution: &Resolution,
    sample_rates: &[u32],
    policy: PregapPolicy,
) -> Vec<VirtualTrack> {
    let mut out = Vec::new();
    let mut ordinal = 0u32;

    for (fi, file) in sheet.files.iter().enumerate() {
        // Ordinals count every track in the sheet, including ones in stanzas we
        // could not resolve — otherwise a partially broken multi-FILE sheet
        // would shift the numbering and desynchronise from MPD's playlists.
        let resolved = resolution.files.get(fi).and_then(|r| r.as_ref());
        let rate = sample_rates.get(fi).copied().unwrap_or(44_100);

        for (ti, track) in file.tracks.iter().enumerate() {
            ordinal += 1;
            let Some(resolved) = resolved else { continue };
            if !track.is_audio() {
                continue;
            }
            let Some(start) = track.start() else { continue };

            // The end is the next track's boundary *within the same FILE*; the
            // last track of each stanza runs to EOF.
            let end = file.tracks.get(ti + 1).and_then(|next| match policy {
                PregapPolicy::Previous => next.start(),
                PregapPolicy::Next => next.pregap_start().or_else(|| next.start()),
            });

            out.push(VirtualTrack {
                ordinal,
                number: track.number,
                file_index: fi,
                backing_path: resolved.path.clone(),
                start_frame: start.to_audio_frames(rate),
                end_frame: end.map(|e: Msf| e.to_audio_frames(rate)),
                pregap_start_frame: track.pregap_start().map(|p| p.to_audio_frames(rate)),
                title: track.title.clone(),
                performer: track.performer.clone().or_else(|| sheet.performer.clone()),
                album: sheet.title.clone(),
                album_artist: sheet.performer.clone(),
                genre: sheet.genre.clone(),
                date: sheet.date.clone(),
            });
        }
    }

    out
}

/// The MPD-compatible URI for a virtual track.
pub fn uri_for(cue_rel_path: &str, track: &VirtualTrack) -> TrackUri {
    TrackUri::CueTrack {
        cue_rel_path: cue_rel_path.to_string(),
        ordinal: track.ordinal,
    }
}

/// Path of a cue sheet relative to the library root, for URI construction.
pub fn rel_path(root: &Path, cue_path: &Path) -> Option<String> {
    cue_path
        .strip_prefix(root)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::parser::parse_str;
    use crate::cue::resolve::Disposition;
    use crate::cue::resolve::{MatchKind, Resolution, ResolvedFile};

    fn fake_resolution(n: usize) -> Resolution {
        Resolution {
            disposition: Disposition::Index,
            per_track: false,
            files: (0..n)
                .map(|i| {
                    Some(ResolvedFile {
                        path: PathBuf::from(format!("file{i}.flac")),
                        how: MatchKind::Exact,
                    })
                })
                .collect(),
        }
    }

    #[test]
    fn boundaries_partition_the_file_with_no_gaps() {
        let sheet = parse_str(
            "FILE \"a.flac\" WAVE\n\
             TRACK 01 AUDIO\nINDEX 01 00:00:00\n\
             TRACK 02 AUDIO\nINDEX 00 01:00:00\nINDEX 01 01:02:00\n\
             TRACK 03 AUDIO\nINDEX 01 02:00:00\n",
        );
        let tracks = expand(
            &sheet,
            &fake_resolution(1),
            &[44_100],
            PregapPolicy::Previous,
        );
        assert_eq!(tracks.len(), 3);
        // Track 1 ends exactly where track 2 begins: no sample belongs to
        // neither, and none belongs to both.
        assert_eq!(tracks[0].end_frame, Some(tracks[1].start_frame));
        assert_eq!(tracks[1].end_frame, Some(tracks[2].start_frame));
        assert_eq!(tracks[2].end_frame, None, "last track runs to EOF");
    }

    #[test]
    fn the_pregap_is_not_silently_discarded() {
        // Octavarium has 18 real seconds between INDEX 00 and INDEX 01.
        let sheet = parse_str(
            "FILE \"a.flac\" WAVE\n\
             TRACK 01 AUDIO\nINDEX 01 00:00:00\n\
             TRACK 02 AUDIO\nINDEX 00 08:07:61\nINDEX 01 08:25:43\n",
        );
        let prev = expand(
            &sheet,
            &fake_resolution(1),
            &[44_100],
            PregapPolicy::Previous,
        );
        let next = expand(&sheet, &fake_resolution(1), &[44_100], PregapPolicy::Next);

        // Default keeps the pregap attached to track 1.
        assert_eq!(prev[0].end_frame, Some(prev[1].start_frame));
        // The other policy ends track 1 early, orphaning ~18s of audio.
        let lost = prev[0].end_frame.unwrap() - next[0].end_frame.unwrap();
        let lost_secs = lost as f64 / 44_100.0;
        assert!(
            (17.0..19.0).contains(&lost_secs),
            "expected ~18s of pregap, got {lost_secs}"
        );
    }

    #[test]
    fn ordinals_continue_across_file_stanzas() {
        // Tears For Fears: two .wv files, one sheet. MPD numbers track0005 as
        // the first track of the *second* file.
        let sheet = parse_str(
            "FILE \"d - 1.wv\" WAVE\n\
             TRACK 01 AUDIO\nINDEX 01 00:00:00\n\
             TRACK 02 AUDIO\nINDEX 01 05:00:00\n\
             FILE \"d - 2.wv\" WAVE\n\
             TRACK 03 AUDIO\nINDEX 01 00:00:00\n",
        );
        let t = expand(
            &sheet,
            &fake_resolution(2),
            &[44_100, 44_100],
            PregapPolicy::Previous,
        );
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].ordinal, 1);
        assert_eq!(t[1].ordinal, 2);
        assert_eq!(t[2].ordinal, 3, "ordinal continues into the second FILE");
        assert_eq!(t[2].file_index, 1);
        // The second file's first track starts at 0 *in that file*.
        assert_eq!(t[2].start_frame, 0);
        // ...and the previous file's last track runs to its own EOF.
        assert_eq!(t[1].end_frame, None);
    }

    #[test]
    fn boundaries_scale_with_the_backing_files_rate() {
        let sheet = parse_str(
            "FILE \"a.flac\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n\
             TRACK 02 AUDIO\nINDEX 01 01:00:00\n",
        );
        let at44 = expand(
            &sheet,
            &fake_resolution(1),
            &[44_100],
            PregapPolicy::Previous,
        );
        let at96 = expand(
            &sheet,
            &fake_resolution(1),
            &[96_000],
            PregapPolicy::Previous,
        );
        assert_eq!(at44[1].start_frame, 44_100 * 60);
        assert_eq!(at96[1].start_frame, 96_000 * 60);
    }

    #[test]
    fn uris_match_the_mpd_form() {
        let sheet = parse_str("FILE \"a.flac\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n");
        let t = expand(
            &sheet,
            &fake_resolution(1),
            &[44_100],
            PregapPolicy::Previous,
        );
        let uri = uri_for("Artist/Album/rip.cue", &t[0]);
        assert_eq!(uri.to_string(), "Artist/Album/rip.cue/track0001");
    }
}
