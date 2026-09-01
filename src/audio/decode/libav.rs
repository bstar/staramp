//! The universal decode path: libavformat + libavcodec, linked in-process.
//!
//! Handles everything symphonia cannot or should not: APE, WavPack, Musepack,
//! DSD, Opus, WMA, TTA, TAK, Shorten, and the MP4/AAC family (where libav
//! applies the container's encoder-delay trim and symphonia does not).
//!
//! In-process rather than a subprocess, which buys real seeking instead of
//! kill-and-respawn with `-ss`, no process spawn per track, and no runtime
//! dependency on an `ffmpeg` binary existing on `PATH`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use anyhow::{anyhow, Context, Result};
use ff::format::sample::{Sample, Type as SampleType};
use ffmpeg_next as ff;

use super::{Decoder, StreamSpec};
use crate::vfs::Media;

static FFMPEG_INIT: Once = Once::new();

fn init_ffmpeg() -> Result<()> {
    let mut err = None;
    FFMPEG_INIT.call_once(|| {
        if let Err(e) = ff::init() {
            err = Some(e);
        }
        // libav's default log level writes codec chatter to stderr, which would
        // land on top of the TUI. Route it away.
        ff::util::log::set_level(ff::util::log::Level::Fatal);
    });
    match err {
        Some(e) => Err(anyhow!("initialising libav: {e}")),
        None => Ok(()),
    }
}

pub struct LibavDecoder {
    input: ff::format::context::Input,
    decoder: ff::decoder::Audio,
    resampler: ff::software::resampling::Context,
    stream_index: usize,
    /// Stream time base, needed to convert PTS to and from frames.
    time_base: ff::Rational,

    spec: StreamSpec,
    /// Codec short name, for display.
    codec: String,
    /// Backing file size, for the average-bitrate calculation.
    file_size: u64,
    total_frames: Option<u64>,
    /// Set to abandon a read that is blocked on a link that has gone away.
    /// `None` for a local file, which cannot block indefinitely.
    cancel: Option<Arc<AtomicBool>>,

    /// Converted interleaved samples not yet handed to the caller.
    buf: Vec<f32>,
    buf_pos: usize,

    pos: u64,
    eos: bool,
}

impl LibavDecoder {
    /// Open `media`, named `name` for every error message.
    pub fn open(media: Media, name: &str) -> Result<Self> {
        init_ffmpeg()?;

        let mut cancel = None;
        let (input, file_size) = match media {
            // The local arm keeps `avformat_open_input(path)` deliberately, so
            // libavformat's own `file:` protocol and read-ahead stay in play.
            Media::Local(ref p) => {
                let n = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                let input = ff::format::input(p).with_context(|| format!("opening {name}"))?;
                (input, n)
            }
            Media::Stream { reader, len } => {
                // 256 KiB rather than the 32 KiB default: on a link where a
                // round trip costs milliseconds, the buffer size is
                // effectively the request size.
                let io =
                    ff::format::context::StreamIo::from_read_seek_with_capacity(reader, 1 << 18)
                        .map_err(|e| anyhow!("{name}: custom io: {e}"))?;

                // Not optional. Without an interrupt callback a stalled read
                // wedges the decode thread with no way out, and errors on a
                // custom AVIO are sticky -- `fill_buffer` latches whatever the
                // callback returned and no further I/O happens, so there is no
                // recovering a context that has hung once.
                let token = Arc::new(AtomicBool::new(false));
                cancel = Some(Arc::clone(&token));
                let watch = Arc::clone(&token);
                let input = ff::format::input_from_stream_with_interrupt(
                    io,
                    // A hint for probing only. libavformat does no I/O on it
                    // when `pb` is already set, and it is what keeps exotic
                    // containers probing as cheaply as a local file does.
                    Some(name),
                    None,
                    move || watch.load(Ordering::Relaxed),
                )
                .map_err(|e| anyhow!("opening {name}: {e}"))?;
                (input, len)
            }
        };

        let stream = input
            .streams()
            .best(ff::media::Type::Audio)
            .ok_or_else(|| anyhow!("{name}: no audio stream"))?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let stream_duration = stream.duration();
        let container_duration = input.duration();

        let ctx = ff::codec::context::Context::from_parameters(stream.parameters())
            .with_context(|| format!("{name}: codec parameters"))?;
        let decoder = ctx
            .decoder()
            .audio()
            .with_context(|| format!("{name}: opening audio decoder"))?;

        let codec = decoder
            .codec()
            .map(|c| c.name().to_string())
            .unwrap_or_else(|| "audio".to_string());

        let in_rate = decoder.rate();
        let layout = decoder.channel_layout();
        let channels = layout.channels() as u16;
        if in_rate == 0 || channels == 0 {
            return Err(anyhow!(
                "{name}: decoder reports rate {in_rate}, {channels} channels"
            ));
        }

        // DSD decodes to PCM at a rate far above anything a sound device will
        // accept (the reference .dsf files come out at 705600 Hz). Bring those
        // down here, to a rate that is in PipeWire's allowed-rates list, rather
        // than pushing an unplayable rate downstream and resampling twice.
        let out_rate = normalise_rate(in_rate);
        if out_rate != in_rate {
            tracing::info!("{name}: resampling {in_rate} Hz -> {out_rate} Hz at the decoder");
        }

        // Everything downstream speaks packed f32. libav hands out planar for
        // most of these codecs, so this conversion is not optional.
        let resampler = ff::software::resampler(
            (decoder.format(), layout, in_rate),
            (Sample::F32(SampleType::Packed), layout, out_rate),
        )
        .with_context(|| format!("{name}: building resampler"))?;

        // The stream's duration, in its own time base.
        //
        // Not `stream.frames()`: libav's `nb_frames` counts *packets*, not
        // sample frames, and rescaling it as though it were a timestamp is
        // wrong by the codec's block size -- 1024x for AAC, 73728x for APE.
        // A 75-minute APE came out as 0.06 seconds, which broke the seek bar
        // and truncated the last track of every cue-split album on this path.
        let total_frames = if stream_duration > 0 {
            Some(rescale_to_frames(stream_duration, time_base, out_rate))
        } else if container_duration > 0 {
            // Container duration is in AV_TIME_BASE units, i.e. microseconds.
            Some((container_duration as f64 / 1_000_000.0 * out_rate as f64) as u64)
        } else {
            None
        };

        Ok(Self {
            spec: StreamSpec {
                sample_rate: out_rate,
                channels,
                bit_depth: bit_depth_of(&decoder),
            },
            codec,
            file_size,
            total_frames,
            cancel,
            input,
            decoder,
            resampler,
            stream_index,
            time_base,
            buf: Vec::new(),
            buf_pos: 0,
            pos: 0,
            eos: false,
        })
    }

    /// Pull packets until converted samples are buffered, or the file ends.
    fn fill(&mut self) -> Result<bool> {
        loop {
            if self.buf_pos < self.buf.len() {
                return Ok(true);
            }
            if self.eos {
                return Ok(false);
            }

            self.buf.clear();
            self.buf_pos = 0;

            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {
                    if packet.stream() != self.stream_index {
                        continue;
                    }
                    if let Err(e) = self.decoder.send_packet(&packet) {
                        // A corrupt packet is not a corrupt file.
                        tracing::debug!("skipping packet: {e}");
                        continue;
                    }
                }
                Err(ff::Error::Eof) => {
                    // Flush whatever the decoder still holds before stopping.
                    let _ = self.decoder.send_eof();
                    self.eos = true;
                    self.drain_frames()?;
                    return Ok(self.buf_pos < self.buf.len());
                }
                Err(e) => return Err(anyhow!("reading packet: {e}")),
            }

            self.drain_frames()?;
        }
    }

    /// Move every frame the decoder currently holds into `buf`.
    fn drain_frames(&mut self) -> Result<()> {
        let mut decoded = ff::frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            let mut converted = ff::frame::Audio::empty();
            self.resampler
                .run(&decoded, &mut converted)
                .map_err(|e| anyhow!("resampling: {e}"))?;
            self.append(&converted);
        }
        Ok(())
    }

    /// Append a packed-f32 frame's samples.
    fn append(&mut self, frame: &ff::frame::Audio) {
        let frames = frame.samples();
        if frames == 0 {
            return;
        }
        let ch = self.spec.channels as usize;
        // Packed layout keeps every channel interleaved in plane 0, but the
        // plane is padded to libav's alignment, so the frame count -- not the
        // plane length -- decides how much is real audio.
        let plane: &[f32] = frame.plane(0);
        let want = frames * ch;
        self.buf.extend_from_slice(&plane[..want.min(plane.len())]);
    }
}

impl Decoder for LibavDecoder {
    fn spec(&self) -> StreamSpec {
        self.spec
    }

    fn codec(&self) -> &str {
        &self.codec
    }

    fn bitrate_kbps(&self) -> Option<u32> {
        super::average_bitrate_kbps(self.file_size, self.total_frames, self.spec.sample_rate)
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = self.spec.samples_per_frame();
        if ch == 0 {
            return Ok(0);
        }
        let want_frames = out.len() / ch;
        let mut done = 0usize;

        while done < want_frames {
            if !self.fill()? {
                break;
            }
            let avail = (self.buf.len() - self.buf_pos) / ch;
            let take = avail.min(want_frames - done);
            if take == 0 {
                break;
            }
            out[done * ch..(done + take) * ch]
                .copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + take * ch]);
            self.buf_pos += take * ch;
            self.pos += take as u64;
            done += take;
        }

        Ok(done)
    }

    fn seek(&mut self, frame: u64) -> Result<u64> {
        // libav seeks in AV_TIME_BASE units regardless of the stream's own base.
        let secs = frame as f64 / self.spec.sample_rate as f64;
        let ts = (secs * f64::from(ff::ffi::AV_TIME_BASE)) as i64;

        // `..ts` sets max_ts without a lower bound, so libav lands at or before
        // the target and we decode forward from there. Seeking past the target
        // would make a seek silently skip audio.
        self.input
            .seek(ts, ..ts)
            .map_err(|e| anyhow!("seeking: {e}"))?;

        self.decoder.flush();
        self.buf.clear();
        self.buf_pos = 0;
        self.eos = false;
        self.pos = frame;
        Ok(frame)
    }

    fn position(&self) -> u64 {
        self.pos
    }

    fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }
}

/// Map a decoder output rate onto something a sound device will actually take.
///
/// Only DSD needs this in practice: its PCM output lands in the hundreds of
/// kHz. Everything else passes through untouched so the bit-perfect path stays
/// bit-perfect.
fn normalise_rate(rate: u32) -> u32 {
    const ALLOWED: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];
    if rate <= 192_000 {
        return rate;
    }
    // Stay in the same family: 44.1k-derived rates halve down to 176400,
    // 48k-derived ones to 192000.
    let target = if rate.is_multiple_of(44_100) {
        176_400
    } else {
        192_000
    };
    debug_assert!(ALLOWED.contains(&target));
    target
}

fn rescale_to_frames(value: i64, time_base: ff::Rational, rate: u32) -> u64 {
    let secs = value as f64 * f64::from(time_base.numerator()) / f64::from(time_base.denominator());
    (secs * rate as f64).max(0.0) as u64
}

fn bit_depth_of(decoder: &ff::decoder::Audio) -> Option<u32> {
    match decoder.format() {
        Sample::U8(_) => Some(8),
        Sample::I16(_) => Some(16),
        Sample::I32(_) => Some(32),
        Sample::F32(_) => None, // float: no meaningful integer depth
        Sample::F64(_) => None,
        Sample::I64(_) => Some(64),
        Sample::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::normalise_rate;

    #[test]
    fn ordinary_rates_pass_through_untouched() {
        for r in [44100, 48000, 88200, 96000, 176400, 192000, 22050, 32000] {
            assert_eq!(normalise_rate(r), r);
        }
    }

    #[test]
    fn dsd_rates_come_down_into_the_allowed_set() {
        // The reference .dsf files decode to 705600 Hz, which no device takes.
        assert_eq!(normalise_rate(705_600), 176_400);
        assert_eq!(normalise_rate(2_822_400), 176_400);
        assert_eq!(normalise_rate(384_000), 192_000);
    }
}
