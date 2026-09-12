//! CUE sheet data model.

use std::collections::BTreeMap;

/// A CD timestamp: minutes, seconds, and frames at 75 frames per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Msf {
    pub minutes: u32,
    pub seconds: u32,
    pub frames: u32,
}

impl Msf {
    pub fn new(minutes: u32, seconds: u32, frames: u32) -> Self {
        Self {
            minutes,
            seconds,
            frames,
        }
    }

    /// Total CD frames (1/75 s units) from the start of the file.
    pub fn cd_frames(&self) -> u64 {
        (self.minutes as u64 * 60 + self.seconds as u64) * 75 + self.frames as u64
    }

    /// Convert to audio frames at a given sample rate.
    ///
    /// The rate argument is load-bearing and must come from the **backing audio
    /// file**, not a 44100 constant. Hi-res vinyl rips in the reference library
    /// are 24-bit/96 kHz *and* cue-split; a hardcoded 588-samples-per-CD-frame
    /// conversion silently misplaces every boundary on exactly those albums.
    pub fn to_audio_frames(self, sample_rate: u32) -> u64 {
        // Rounded rather than truncated: at 75 Hz the truncation error
        // accumulates visibly across a 70-minute disc image.
        (self.cd_frames() * sample_rate as u64 + 37) / 75
    }
}

impl std::fmt::Display for Msf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02}:{:02}:{:02}",
            self.minutes, self.seconds, self.frames
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CueTrack {
    /// The number as written in the sheet. Not necessarily contiguous, not
    /// necessarily 1-based, and *not* what MPD's `trackNNNN` ordinal uses.
    pub number: u32,
    /// `AUDIO`, `MODE1/2352`, and so on. Only `AUDIO` is playable.
    pub kind: String,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub songwriter: Option<String>,
    pub isrc: Option<String>,
    pub flags: Vec<String>,
    /// `INDEX 00` is the pregap start, `INDEX 01` the track proper. Higher
    /// indices are legal and rare.
    pub indices: BTreeMap<u32, Msf>,
    pub pregap: Option<Msf>,
    pub postgap: Option<Msf>,
}

impl CueTrack {
    pub fn is_audio(&self) -> bool {
        self.kind.eq_ignore_ascii_case("AUDIO")
    }

    /// Where the track proper begins.
    pub fn start(&self) -> Option<Msf> {
        self.indices
            .get(&1)
            .copied()
            .or_else(|| self.indices.get(&0).copied())
    }

    /// Where the pregap begins, when one is written separately.
    pub fn pregap_start(&self) -> Option<Msf> {
        self.indices.get(&0).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueFile {
    /// Exactly as written in the `FILE` line, before any resolution.
    pub name: String,
    /// `WAVE`, `APE`, `BINARY`, `MP3`...
    pub kind: String,
    pub tracks: Vec<CueTrack>,
}

// No `Eq`: the ReplayGain fields are floats.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueSheet {
    pub title: Option<String>,
    pub performer: Option<String>,
    pub songwriter: Option<String>,
    pub catalog: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub comment: Option<String>,
    /// ReplayGain occasionally appears as `REM REPLAYGAIN_ALBUM_GAIN`.
    pub replaygain_album_gain: Option<f32>,
    pub replaygain_album_peak: Option<f32>,
    pub files: Vec<CueFile>,
    /// The encoding the sheet was decoded from, for diagnostics.
    pub encoding: String,
    /// Non-fatal problems. A sheet is never rejected for one bad line.
    pub warnings: Vec<String>,
}

impl CueSheet {
    /// Total audio tracks across every `FILE` stanza.
    pub fn track_count(&self) -> usize {
        self.files.iter().map(|f| f.tracks.len()).sum()
    }

    pub fn is_multi_file(&self) -> bool {
        self.files.len() > 1
    }
}
