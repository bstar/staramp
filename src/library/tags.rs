//! Reading tags and technical properties.
//!
//! lofty rather than symphonia's metadata, for three reasons. symphonia only
//! surfaces metadata for containers it can demux, so it cannot read APE tags on
//! `.ape`/`.wv`/`.mpc` at all and 148 files in the reference library would have
//! none. lofty parses ReplayGain uniformly across ID3v2 `TXXX`, Vorbis comments
//! and APEv2 rather than needing per-format special casing. And it writes, which
//! the "store computed ReplayGain back to the file" option needs.

use std::path::Path;

use anyhow::{Context, Result};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;

#[derive(Debug, Clone, Default)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub track_no: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_no: Option<u32>,
    pub disc_total: Option<u32>,
    pub year: Option<u32>,
    pub date: Option<String>,

    pub rg_track_gain: Option<f32>,
    pub rg_track_peak: Option<f32>,
    pub rg_album_gain: Option<f32>,
    pub rg_album_peak: Option<f32>,

    pub mb_recording_id: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_artist_id: Option<String>,

    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate_kbps: Option<u32>,
}

pub fn read(path: &Path) -> Result<TrackTags> {
    let tagged = Probe::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .read()
        .with_context(|| format!("reading tags from {}", path.display()))?;

    let props = tagged.properties();
    let mut t = TrackTags {
        duration_ms: Some(props.duration().as_millis() as u64),
        sample_rate: props.sample_rate(),
        bit_depth: props.bit_depth(),
        channels: props.channels(),
        bitrate_kbps: props.audio_bitrate(),
        ..Default::default()
    };

    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        // No tags at all is normal for plenty of rips; the file is still
        // playable and the scanner falls back to the filename.
        return Ok(t);
    };

    t.title = tag.title().map(|s| s.to_string());
    t.artist = tag.artist().map(|s| s.to_string());
    t.album = tag.album().map(|s| s.to_string());
    t.genre = tag.genre().map(|s| s.to_string());
    t.track_no = tag.track();
    t.track_total = tag.track_total();
    t.disc_no = tag.disk();
    t.disc_total = tag.disk_total();
    t.year = tag.year();

    let s = |key: &ItemKey| tag.get_string(key).map(|v| v.to_string());
    t.album_artist = s(&ItemKey::AlbumArtist);
    t.composer = s(&ItemKey::Composer);
    t.date = s(&ItemKey::RecordingDate).or_else(|| t.year.map(|y| y.to_string()));

    // ReplayGain values are written as "-7.86 dB"; take the leading number.
    let gain = |key: &ItemKey| -> Option<f32> {
        tag.get_string(key)?.split_whitespace().next()?.parse().ok()
    };
    t.rg_track_gain = gain(&ItemKey::ReplayGainTrackGain);
    t.rg_album_gain = gain(&ItemKey::ReplayGainAlbumGain);
    t.rg_track_peak = tag
        .get_string(&ItemKey::ReplayGainTrackPeak)
        .and_then(|v| v.parse().ok());
    t.rg_album_peak = tag
        .get_string(&ItemKey::ReplayGainAlbumPeak)
        .and_then(|v| v.parse().ok());

    t.mb_recording_id = s(&ItemKey::MusicBrainzRecordingId);
    t.mb_release_id = s(&ItemKey::MusicBrainzReleaseId);
    t.mb_artist_id = s(&ItemKey::MusicBrainzArtistId);

    Ok(t)
}

/// Best-effort artist and title from a filename, for files with no usable tags.
///
/// Conservative on purpose: it is better to show a filename than to invent an
/// artist by splitting on the wrong hyphen.
pub fn from_filename(path: &Path) -> (Option<String>, Option<String>) {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return (None, None);
    };
    // Strip a leading track number: "01 - ", "01. ", "01_".
    let stem = stem
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches([' ', '-', '.', '_']);

    match stem.split_once(" - ") {
        Some((artist, title)) if !artist.is_empty() && !title.is_empty() => (
            Some(artist.trim().to_string()),
            Some(title.trim().to_string()),
        ),
        _ => (None, Some(stem.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn filename_fallback_splits_artist_and_title() {
        let (a, t) = from_filename(&PathBuf::from("01 - Angra - Nova Era.flac"));
        assert_eq!(a.as_deref(), Some("Angra"));
        assert_eq!(t.as_deref(), Some("Nova Era"));
    }

    #[test]
    fn filename_fallback_strips_leading_track_numbers() {
        let (a, t) = from_filename(&PathBuf::from("07. Son of a Wolf.flac"));
        assert_eq!(a, None);
        assert_eq!(t.as_deref(), Some("Son of a Wolf"));
    }

    #[test]
    fn filename_fallback_does_not_invent_an_artist() {
        // A hyphen without surrounding spaces is part of the title, not a
        // separator. Guessing here produces wrong artists at scale.
        let (a, t) = from_filename(&PathBuf::from("Die-Die-Crucified.flac"));
        assert_eq!(a, None);
        assert_eq!(t.as_deref(), Some("Die-Die-Crucified"));
    }
}
