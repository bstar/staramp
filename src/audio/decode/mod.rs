//! Decoding.
//!
//! One trait spans every backend. Everything downstream of this module speaks
//! **interleaved `f32` frames** and nothing else — that is the only sane common
//! currency between symphonia's typed `AudioBufferRef` variants and libavcodec's
//! planar output.

pub mod libav;
pub mod slice;
pub mod symphonia;

use std::path::Path;

use anyhow::Result;

/// What a decoder is producing, discovered when the stream is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSpec {
    pub sample_rate: u32,
    pub channels: u16,
    /// Source bit depth, where the container reports one. Display and the
    /// bit-perfect decision both want it; neither requires it.
    pub bit_depth: Option<u32>,
}

impl StreamSpec {
    /// Samples per frame. Interleaved buffers are `frames * channels` long.
    pub fn samples_per_frame(&self) -> usize {
        self.channels as usize
    }
}

/// A seekable source of interleaved `f32` audio.
///
/// Positions are in **frames**, absolute, and relative to the backing file —
/// `SliceDecoder` is what translates those into cue-virtual-track coordinates,
/// so no implementation of this trait needs to know cue sheets exist.
pub trait Decoder: Send {
    fn spec(&self) -> StreamSpec;

    /// Fill `out` with interleaved `f32`. Returns the number of **frames**
    /// written; `0` means end of stream.
    ///
    /// `out.len()` should be a multiple of `spec().channels`; a partial trailing
    /// frame is not written.
    fn read(&mut self, out: &mut [f32]) -> Result<usize>;

    /// Seek to an absolute frame. Returns the frame actually landed on, which
    /// may precede the request when the container only indexes coarsely.
    fn seek(&mut self, frame: u64) -> Result<u64>;

    /// Current absolute frame position.
    fn position(&self) -> u64;

    /// Total frames, where the container knows. `None` for streams that do not
    /// declare a duration.
    fn total_frames(&self) -> Option<u64>;

    /// Move a cue slice's window to another track of the same backing file.
    ///
    /// Returns false for anything that is not a slice, so the caller falls back
    /// to opening the next track properly. Overridden only by `SliceDecoder`.
    ///
    /// This exists because advancing inside a cue album otherwise means
    /// re-parsing the sheet and reopening the audio file while at most a ring's
    /// worth of audio is left to play -- and the decoder is already sitting at
    /// exactly the right sample.
    fn retarget_slice(&mut self, _start: u64, _end: Option<u64>) -> bool {
        false
    }

    /// Short codec name for display: `flac`, `mp3`, `aac`, `alac`, `wavpack`.
    ///
    /// The *codec*, not the extension. A `.m4a` holds either AAC or ALAC and
    /// the distinction is the whole point of showing it.
    fn codec(&self) -> &str;

    /// Average bitrate over the backing file, in kbps.
    ///
    /// Measured rather than declared -- see [`average_bitrate_kbps`].
    fn bitrate_kbps(&self) -> Option<u32>;
}

/// Average bitrate from file size and duration.
///
/// Containers report a nominal bitrate inconsistently and, for VBR, often
/// wrongly: a LAME V0 MP3 declares whatever its first frame header says. Size
/// over duration is the figure that actually describes the file, it is right
/// for VBR and lossless alike, and it needs nothing from the container beyond
/// the duration we already have.
pub fn average_bitrate_kbps(
    file_size: u64,
    total_frames: Option<u64>,
    sample_rate: u32,
) -> Option<u32> {
    let frames = total_frames?;
    if frames == 0 || sample_rate == 0 || file_size == 0 {
        return None;
    }
    let secs = frames as f64 / sample_rate as f64;
    let kbps = (file_size as f64 * 8.0) / secs / 1000.0;
    (kbps.is_finite() && kbps >= 1.0).then(|| kbps.round() as u32)
}

/// Which implementation should open a given file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Symphonia,
    Libav,
}

/// Extensions symphonia is *permitted* to open.
///
/// This is an allowlist rather than a probe, and that is deliberate. Symphonia's
/// probe scans a window for container magic and will happily claim a file it
/// cannot actually decode:
///
/// > A WavPack file begins with `wvpk`, but WavPack preserves the source WAV
/// > header inside the stream — a `RIFF....WAVE fmt ` block sits at byte 34 of a
/// > typical `.wv`. Symphonia's probe finds it, reads the (entirely correct)
/// > embedded `fmt ` chunk, reports the right sample rate and channel count, and
/// > then decodes the WavPack-compressed payload as though it were raw PCM. The
/// > result is not an error. It is `f32::MAX` noise with plausible metadata.
///
/// A trust-the-probe design therefore produces *silent* corruption on exactly
/// the formats we added a second backend for. Extension decides the backend;
/// the probe only decides the container within symphonia's own territory.
const SYMPHONIA_EXTS: &[&str] = &[
    "flac", "mp3", "ogg", "oga", "wav", "wave", "aif", "aiff", "aifc", "caf",
];

/// Extensions that must go to libav. Symphonia either lacks the codec outright
/// (APE, WavPack, Musepack, DSD, WMA, TTA, TAK, Shorten) or ships only a stub
/// (Opus). Listing them explicitly means a future symphonia release cannot start
/// silently claiming them.
/// Extensions that must go to libav.
///
/// Most are here because symphonia lacks the codec outright (APE, WavPack,
/// Musepack, DSD, WMA, TTA, TAK, Shorten) or ships only a stub (Opus).
///
/// The MP4 family is here for a different and less obvious reason. Symphonia
/// decodes AAC correctly but does not apply the container's encoder-delay
/// trimming, so its output leads ffmpeg's by the priming amount — measured at
/// 1056 samples on a reference file, after which the two decodes agree to
/// 1.05e-08. The audio is right; it just starts ~24 ms early, which is a gapless
/// defect at every AAC track boundary. `FormatOptions::enable_gapless` covers
/// LAME/Xing for MP3 but does not cover this. libav honours the edit list, and
/// we link it regardless, so the whole family goes there.
const LIBAV_EXTS: &[&str] = &[
    "m4a", "m4b", "mp4", "aac", "adts", "alac", "mka", "webm", "ape", "wv", "wvc", "mpc", "mp+",
    "mpp", "dsf", "dff", "opus", "wma", "asf", "tta", "tak", "shn", "ac3", "dts", "mid", "midi",
    "ra", "rm", // Video containers, for their audio stream.
    "mpg", "mpeg", "mov", "mkv", "avi", "m2ts", "ts", "vob", "flv", "wmv", "m4v",
];

/// Choose a backend from the file extension.
///
/// An unknown extension goes to libav: it handles strictly more formats, so an
/// unrecognised file is likelier to be something exotic than something
/// mainstream with an odd name.
pub fn backend_for_path(path: &Path) -> Backend {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match ext.as_deref() {
        Some(e) if SYMPHONIA_EXTS.contains(&e) => Backend::Symphonia,
        Some(e) if LIBAV_EXTS.contains(&e) => Backend::Libav,
        _ => Backend::Libav,
    }
}

/// Open a file with the appropriate backend.
pub fn open(path: &Path) -> Result<Box<dyn Decoder>> {
    match backend_for_path(path) {
        Backend::Symphonia => Ok(Box::new(symphonia::SymphoniaDecoder::open(path)?)),
        Backend::Libav => Ok(Box::new(libav::LibavDecoder::open(path)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn b(s: &str) -> Backend {
        backend_for_path(&PathBuf::from(s))
    }

    #[test]
    fn mainstream_formats_use_symphonia() {
        for p in ["a.flac", "a.mp3", "a.ogg", "a.wav", "a.aif", "a.aiff"] {
            assert_eq!(b(p), Backend::Symphonia, "{p}");
        }
    }

    #[test]
    fn mp4_family_uses_libav_for_encoder_delay_trimming() {
        // Symphonia decodes AAC correctly but does not apply the container's
        // priming trim, leaving ~24ms of extra audio at each track start.
        for p in ["a.m4a", "a.m4b", "a.mp4", "a.aac", "a.alac"] {
            assert_eq!(b(p), Backend::Libav, "{p}");
        }
    }

    #[test]
    fn wavpack_never_reaches_symphonia() {
        // The whole point: symphonia *succeeds* on .wv and returns garbage,
        // because WavPack embeds a real RIFF/WAVE header that the probe finds.
        assert_eq!(b("a.wv"), Backend::Libav);
    }

    #[test]
    fn video_containers_use_libav_for_their_audio_stream() {
        for p in ["a.mpg", "a.mov", "a.mkv", "a.avi", "a.m4v"] {
            assert_eq!(b(p), Backend::Libav, "{p}");
        }
    }

    #[test]
    fn exotic_lossless_uses_libav() {
        for p in ["a.ape", "a.wv", "a.mpc", "a.dsf", "a.dff", "a.tta", "a.tak"] {
            assert_eq!(b(p), Backend::Libav, "{p}");
        }
    }

    #[test]
    fn opus_uses_libav_because_symphonia_only_stubs_it() {
        assert_eq!(b("a.opus"), Backend::Libav);
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert_eq!(b("a.FLAC"), Backend::Symphonia);
        assert_eq!(b("a.WV"), Backend::Libav);
    }

    #[test]
    fn unknown_and_missing_extensions_fall_to_libav() {
        assert_eq!(b("a.zzz"), Backend::Libav);
        assert_eq!(b("noextension"), Backend::Libav);
    }
}
