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

use crate::vfs::Media;

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

/// Choose a backend from the extension in a track URI.
///
/// An unknown extension goes to libav: it handles strictly more formats, so an
/// unrecognised file is likelier to be something exotic than something
/// mainstream with an odd name.
///
/// `&str` rather than `&Path` because the URI is the authority. It is always
/// forward-slash separated, the extension is the only part that matters, and
/// building a `Path` just to ask it for the extension is work for nothing --
/// work that a URI whose bytes are not on this machine should not have to do.
pub fn backend_for_uri(uri: &str) -> Backend {
    // `Path::extension` semantics, kept deliberately: a leading dot is a hidden
    // file rather than an extension, so `.flac` has none.
    let name = uri.rsplit('/').next().unwrap_or(uri);
    let ext = name
        .rsplit_once('.')
        .and_then(|(stem, e)| (!stem.is_empty()).then(|| e.to_ascii_lowercase()));

    match ext.as_deref() {
        Some(e) if SYMPHONIA_EXTS.contains(&e) => Backend::Symphonia,
        Some(e) if LIBAV_EXTS.contains(&e) => Backend::Libav,
        _ => Backend::Libav,
    }
}

/// As [`backend_for_uri`], for a path that never was a URI.
pub fn backend_for_path(path: &Path) -> Backend {
    backend_for_uri(&path.to_string_lossy())
}

/// Open `media` with the appropriate backend.
///
/// `name` is the URI. It supplies the extension the backend is chosen by, the
/// container hint, and the text of every error message -- which is why no
/// decoder below this line needs to know whether the bytes were local.
pub fn open(media: Media, name: &str) -> Result<Box<dyn Decoder>> {
    match backend_for_uri(name) {
        Backend::Symphonia => Ok(Box::new(symphonia::SymphoniaDecoder::open(media, name)?)),
        Backend::Libav => Ok(Box::new(libav::LibavDecoder::open(media, name)?)),
    }
}

/// Open a local path directly. The CLI's entry point, and the tests'.
pub fn open_path(path: &Path) -> Result<Box<dyn Decoder>> {
    let name = path.to_string_lossy().into_owned();
    open(Media::Local(path.to_path_buf()), &name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn b(s: &str) -> Backend {
        backend_for_uri(s)
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

    /// Only the last path component decides, so a directory with a dot in it
    /// cannot pull a file to the wrong backend.
    #[test]
    fn only_the_file_name_is_considered() {
        assert_eq!(b("My Albums v1.0/track"), Backend::Libav);
        assert_eq!(b("My Albums v1.0/track.flac"), Backend::Symphonia);
    }

    /// The URI form and the path form must never disagree -- `cmd_probe`
    /// dispatches on the path while playback dispatches on the URI, and a
    /// divergence would report one backend and use another.
    #[test]
    fn the_uri_and_path_forms_agree() {
        for p in [
            "a.flac",
            "a.wv",
            "dir/a.m4a",
            "noextension",
            // A leading dot is a hidden file, not an extension.
            ".flac",
            "a.",
        ] {
            assert_eq!(
                backend_for_uri(p),
                backend_for_path(&PathBuf::from(p)),
                "{p}"
            );
        }
    }
}

/// Decoding a file that is not on this machine.
///
/// The point of these is not the transport, which has its own tests. It is
/// that the *same bytes* decode to the *same audio* whether they arrived from
/// a disk or a pipe -- through both backends, since symphonia and libav reach
/// their input by completely different routes.
#[cfg(test)]
mod remote_decode {
    use super::*;
    use crate::remote::sftp::session::{fake, Session};
    use crate::remote::stream::{RemoteFile, MIN_WINDOW};
    use crate::vfs::Media;
    use std::sync::Arc;

    /// Serve `bytes` as `/f` and open it as a decoder input.
    fn served(bytes: Vec<u8>) -> Media {
        let server = fake::Server::new(&[("/f", bytes)]);
        let (sr, sw) = std::io::pipe().unwrap();
        let (cr, cw) = std::io::pipe().unwrap();
        std::thread::spawn(move || server.serve(sr, cw));
        let session = Session::over(Box::new(sw), Box::new(cr)).unwrap();
        let file = RemoteFile::open(Arc::clone(&session), "/f", MIN_WINDOW).unwrap();
        let len = crate::vfs::RemoteRead::len(&file);
        Media::Stream {
            reader: Box::new(file),
            len,
        }
    }

    fn drain(mut d: Box<dyn Decoder>) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0f32; 4096];
        loop {
            match d.read(&mut buf) {
                Ok(0) => break,
                Ok(frames) => all.extend_from_slice(&buf[..frames * d.spec().samples_per_frame()]),
                Err(e) => panic!("decoding: {e}"),
            }
        }
        all
    }

    /// A 16-bit PCM WAV, which is the shortest route to exercising symphonia.
    fn wav(frames: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..frames)
            .flat_map(|i| {
                let v = ((i as f64 * 0.05).sin() * 12000.0) as i16;
                v.to_le_bytes()
            })
            .collect();
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&44100u32.to_le_bytes());
        w.extend_from_slice(&88200u32.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        w
    }

    #[test]
    fn symphonia_decodes_a_stream_exactly_as_it_decodes_a_file() {
        let bytes = wav(120_000);
        let dir = std::env::temp_dir().join(format!("staramp-rd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        std::fs::write(&path, &bytes).unwrap();

        let local = drain(open(Media::Local(path.clone()), "tone.wav").unwrap());
        let remote = drain(open(served(bytes), "tone.wav").unwrap());

        assert!(!local.is_empty(), "the local decode produced nothing");
        assert_eq!(local.len(), remote.len(), "same number of samples");
        assert_eq!(local, remote, "sample for sample");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// WavPack goes through libav's custom AVIO, which is an entirely
    /// different path into the decoder from symphonia's `MediaSource` -- and
    /// it is the format the project exists to play properly.
    #[test]
    fn libav_decodes_a_stream_exactly_as_it_decodes_a_file() {
        let path = std::path::Path::new("testdata/tone.wv");
        if !path.is_file() {
            eprintln!("testdata/tone.wv missing, skipping");
            return;
        }
        let bytes = std::fs::read(path).unwrap();

        let local = drain(open(Media::Local(path.to_path_buf()), "tone.wv").unwrap());
        let remote = drain(open(served(bytes), "tone.wv").unwrap());

        assert!(!local.is_empty(), "the local decode produced nothing");
        assert_eq!(local.len(), remote.len(), "same number of samples");
        assert_eq!(local, remote, "sample for sample");
    }

    /// Seeking is what a cue slice does on its very first read, so a stream
    /// that cannot seek would break 27% of the reference library's playlists
    /// while looking fine on everything else.
    #[test]
    fn a_streamed_decoder_seeks_to_the_same_place_a_local_one_does() {
        let path = std::path::Path::new("testdata/tone.wv");
        if !path.is_file() {
            return;
        }
        let bytes = std::fs::read(path).unwrap();

        let mut local = open(Media::Local(path.to_path_buf()), "tone.wv").unwrap();
        let mut remote = open(served(bytes), "tone.wv").unwrap();

        for frame in [0u64, 11_025, 44_100, 66_150] {
            let a = local.seek(frame).unwrap();
            let b = remote.seek(frame).unwrap();
            assert_eq!(a, b, "landed on a different frame seeking to {frame}");

            let mut la = vec![0f32; 2048];
            let mut rb = vec![0f32; 2048];
            let na = local.read(&mut la).unwrap();
            let nr = remote.read(&mut rb).unwrap();
            assert_eq!(na, nr, "frames read after seeking to {frame}");
            assert_eq!(la[..na], rb[..nr], "audio after seeking to {frame}");
        }
    }
}
