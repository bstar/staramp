//! Making a decoded stream fit what the device will actually accept.
//!
//! The policy in [`crate::audio::output`] is follow-the-file: open the device at
//! the file's own rate and channel count so the signal reaches the DAC
//! untouched. Where the device can do that, this module is not used at all and
//! playback stays bit-perfect.
//!
//! Some devices cannot. A Bluetooth headset advertises 44.1 kHz and nothing
//! else, a CoreAudio device offers only the channel counts it physically has,
//! and an ALSA `hw:` device with no `plug` in front of it behaves the same way.
//! Before this existed the mismatch was *detected* -- `RateMode::Resampled` is
//! older than this file -- and then ignored: the ring was filled at the file's
//! rate and drained at the device's, so a 48 kHz track on a 44.1 kHz-only
//! device played 8.4% slow. That is what this fixes.
//!
//! It wraps any [`Decoder`] and presents one whose `spec()` is the *device's*
//! shape, so every layer above -- the ring, the output, the progress bar,
//! seeking -- keeps working in a single consistent domain and needs to know
//! nothing about the conversion.
//!
//! swresample rather than a Rust resampler: it is already linked, already a
//! dependency, and `libav.rs` already uses it for exactly this. It also does
//! the channel conversion in the same pass, so mono into a stereo-only device
//! costs no second stage.

use anyhow::{anyhow, Context, Result};
use ff::format::sample::{Sample, Type as SampleType};
use ffmpeg_next as ff;

use super::{Decoder, StreamSpec};

/// Frames pulled from the inner decoder per conversion pass. Working space
/// only, unrelated to the ring: large enough that the per-call overhead of
/// building an ffmpeg frame is amortised, small enough to stay responsive to
/// a stop request.
const CHUNK_FRAMES: usize = 4096;

/// A decoder whose output has been converted to the device's rate and channel
/// count.
pub struct Adapting {
    inner: Box<dyn Decoder>,
    swr: ff::software::resampling::Context,
    in_spec: StreamSpec,
    out_spec: StreamSpec,
    /// Copy a mono source across the output's channels here, rather than
    /// letting swresample's rematrix do it.
    ///
    /// Its up-mix matrix is power-preserving: mono to stereo comes out at
    /// 1/sqrt(2), a hair over 3 dB down. That is defensible acoustics and the
    /// wrong answer here, because every other layer a listener meets -- ALSA's
    /// `plug`, PulseAudio, CoreAudio's own up-mix -- duplicates at unity. Left
    /// to swresample, the same file would play 3 dB quieter on macOS, where a
    /// device offers no mono, than on Linux, where `plug` handles it. Only the
    /// up-mix is taken over: swresample's *down*-mix matrix is standard and is
    /// what a 5.1 file on a stereo device still goes through.
    expand_mono: bool,
    /// Frames read from `inner`, before any channel expansion.
    raw: Vec<f32>,
    /// Interleaved input as swresample sees it.
    scratch: Vec<f32>,
    /// Converted output not yet handed to the caller. swresample buffers
    /// internally as well, so this only holds what a `read` did not take.
    pending: std::collections::VecDeque<f32>,
    /// The inner decoder has returned 0 and the resampler has been drained.
    eos: bool,
    /// Position in **output** frames, in whatever domain the inner decoder
    /// reports -- track-relative behind a `SliceDecoder`, absolute otherwise.
    ///
    /// Counted rather than derived from the inner position: the resampler and
    /// `pending` both hold audio the inner decoder has already passed over, so
    /// scaling `inner.position()` would run ahead of what has been emitted.
    pos: u64,
}

impl Adapting {
    /// Wrap `inner` so it produces `out` instead of its own shape.
    pub fn new(inner: Box<dyn Decoder>, out: StreamSpec) -> Result<Self> {
        let in_spec = inner.spec();
        if in_spec.sample_rate == 0 || in_spec.channels == 0 {
            return Err(anyhow!(
                "cannot convert a stream of {} Hz, {} channels",
                in_spec.sample_rate,
                in_spec.channels
            ));
        }
        let expand_mono = in_spec.channels == 1 && out.channels > 1;
        let swr = Self::build(&Self::mid_spec(&in_spec, &out), &out)?;
        Ok(Self {
            inner,
            swr,
            expand_mono,
            raw: Vec::new(),
            in_spec,
            // The source's bit depth is not the device's, but it is what the
            // file *was*, which is what the UI is reporting.
            out_spec: StreamSpec {
                bit_depth: in_spec.bit_depth,
                ..out
            },
            scratch: Vec::new(),
            pending: std::collections::VecDeque::new(),
            eos: false,
            pos: 0,
        })
    }

    /// The shape handed to swresample: the source, except that a mono source
    /// bound for a multi-channel device has already been widened.
    fn mid_spec(in_spec: &StreamSpec, out: &StreamSpec) -> StreamSpec {
        StreamSpec {
            channels: if in_spec.channels == 1 && out.channels > 1 {
                out.channels
            } else {
                in_spec.channels
            },
            ..*in_spec
        }
    }

    fn build(in_spec: &StreamSpec, out: &StreamSpec) -> Result<ff::software::resampling::Context> {
        let packed = Sample::F32(SampleType::Packed);
        ff::software::resampler(
            (
                packed,
                ff::ChannelLayout::default(in_spec.channels as i32),
                in_spec.sample_rate,
            ),
            (
                packed,
                ff::ChannelLayout::default(out.channels as i32),
                out.sample_rate,
            ),
        )
        .with_context(|| {
            format!(
                "building a resampler for {} Hz {}ch -> {} Hz {}ch",
                in_spec.sample_rate, in_spec.channels, out.sample_rate, out.channels
            )
        })
    }

    /// Output frames a given number of input frames turns into, rounded up.
    fn to_out(&self, frames: u64) -> u64 {
        scale(frames, self.out_spec.sample_rate, self.in_spec.sample_rate)
    }

    /// Input frames a given number of output frames came from.
    fn to_in(&self, frames: u64) -> u64 {
        scale(frames, self.in_spec.sample_rate, self.out_spec.sample_rate)
    }

    /// Push one chunk of input through the resampler into `pending`.
    ///
    /// Returns false at end of stream, once the resampler has also been
    /// drained -- swresample holds samples back, and dropping them would clip
    /// the tail off every track.
    fn pump(&mut self) -> Result<bool> {
        if self.eos {
            return Ok(false);
        }
        let in_ch = self.in_spec.channels as usize;
        self.raw.resize(CHUNK_FRAMES * in_ch, 0.0);
        let got = self.inner.read(&mut self.raw)?;
        if got == 0 {
            self.eos = true;
            self.drain()?;
            return Ok(false);
        }

        let mid_ch = Self::mid_spec(&self.in_spec, &self.out_spec).channels as usize;
        self.scratch.resize(got * mid_ch, 0.0);
        if self.expand_mono {
            for (f, frame) in self.scratch.chunks_exact_mut(mid_ch).enumerate() {
                frame.fill(self.raw[f]);
            }
        } else {
            self.scratch.copy_from_slice(&self.raw[..got * mid_ch]);
        }

        let mut input = ff::frame::Audio::new(
            Sample::F32(SampleType::Packed),
            got,
            ff::ChannelLayout::default(mid_ch as i32),
        );
        input.set_rate(self.in_spec.sample_rate);
        // Packed layout keeps every channel interleaved in plane 0. The plane
        // is padded to libav's alignment, so only `got` frames of it are real.
        input.plane_mut::<f32>(0)[..got * mid_ch].copy_from_slice(&self.scratch[..got * mid_ch]);

        let mut out = self.out_frame(self.to_out(got as u64) as usize);
        self.swr
            .run(&input, &mut out)
            .map_err(|e| anyhow!("resampling: {e}"))?;
        self.absorb(&out);
        Ok(true)
    }

    /// Drain whatever the resampler is still holding.
    fn drain(&mut self) -> Result<()> {
        loop {
            let mut out = self.out_frame(CHUNK_FRAMES);
            self.swr
                .flush(&mut out)
                .map_err(|e| anyhow!("flushing the resampler: {e}"))?;
            if out.samples() == 0 {
                return Ok(());
            }
            self.absorb(&out);
        }
    }

    /// An output frame with room for `frames`, plus a margin.
    ///
    /// Sized rather than left empty: `run` allocates an empty output frame to
    /// the *input* frame count, which is short whenever the conversion is
    /// upward. Anything that still does not fit stays in swresample's own FIFO
    /// and arrives on the next call, so a tight estimate costs throughput
    /// rather than samples -- but there is no reason to be tight.
    fn out_frame(&self, frames: usize) -> ff::frame::Audio {
        let mut f = ff::frame::Audio::new(
            Sample::F32(SampleType::Packed),
            frames + 64,
            ff::ChannelLayout::default(self.out_spec.channels as i32),
        );
        f.set_rate(self.out_spec.sample_rate);
        f
    }

    fn absorb(&mut self, frame: &ff::frame::Audio) {
        let frames = frame.samples();
        if frames == 0 {
            return;
        }
        let ch = self.out_spec.channels as usize;
        let plane: &[f32] = frame.plane(0);
        let want = (frames * ch).min(plane.len());
        self.pending.extend(&plane[..want]);
    }

    /// Throw away every converted sample and start the resampler again.
    ///
    /// A seek is a discontinuity by definition, so the filter history is not
    /// worth preserving -- and carrying it across would bleed the old position
    /// into the first samples of the new one.
    fn reset(&mut self) -> Result<()> {
        self.swr = Self::build(
            &Self::mid_spec(&self.in_spec, &self.out_spec),
            &self.out_spec,
        )?;
        self.pending.clear();
        self.eos = false;
        Ok(())
    }
}

/// `frames * num / den`, rounded up, without overflowing on long files.
fn scale(frames: u64, num: u32, den: u32) -> u64 {
    if den == 0 {
        return frames;
    }
    let (num, den) = (num as u128, den as u128);
    let scaled = (frames as u128 * num).div_ceil(den);
    scaled.min(u64::MAX as u128) as u64
}

impl Decoder for Adapting {
    fn spec(&self) -> StreamSpec {
        self.out_spec
    }

    fn codec(&self) -> &str {
        self.inner.codec()
    }

    fn bitrate_kbps(&self) -> Option<u32> {
        self.inner.bitrate_kbps()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = self.out_spec.channels as usize;
        if ch == 0 {
            return Ok(0);
        }
        let want = out.len() / ch * ch;
        while self.pending.len() < want {
            if !self.pump()? {
                break;
            }
        }
        let take = want.min(self.pending.len() / ch * ch);
        for slot in out[..take].iter_mut() {
            // Length checked above, so this cannot be None.
            *slot = self.pending.pop_front().unwrap_or(0.0);
        }
        let frames = take / ch;
        self.pos += frames as u64;
        Ok(frames)
    }

    /// Seek to an absolute **output** frame.
    fn seek(&mut self, frame: u64) -> Result<u64> {
        let landed = self.inner.seek(self.to_in(frame))?;
        self.reset()?;
        self.pos = self.to_out(landed);
        Ok(self.pos)
    }

    fn position(&self) -> u64 {
        self.pos
    }

    fn total_frames(&self) -> Option<u64> {
        self.inner.total_frames().map(|f| self.to_out(f))
    }

    /// `start` and `end` address the backing file, not this stream, so they
    /// are passed through in the inner decoder's own frames.
    ///
    /// The resampler is deliberately *not* reset: retargeting is how a cue
    /// album advances from one virtual track to the next without a seek, the
    /// audio either side of the boundary is continuous, and flushing the
    /// filter history there would put a discontinuity into the one place the
    /// design works hardest to keep clean.
    fn retarget_slice(&mut self, start: u64, end: Option<u64>) -> bool {
        if !self.inner.retarget_slice(start, end) {
            return false;
        }
        self.pos = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoder producing a fixed number of frames of a constant value per
    /// channel, so both the frame count and the channel mapping are checkable.
    struct Tone {
        spec: StreamSpec,
        pos: u64,
        len: u64,
    }

    impl Tone {
        fn boxed(sample_rate: u32, channels: u16, len: u64) -> Box<dyn Decoder> {
            Box::new(Self {
                spec: StreamSpec {
                    sample_rate,
                    channels,
                    bit_depth: Some(16),
                },
                pos: 0,
                len,
            })
        }
    }

    impl Decoder for Tone {
        fn spec(&self) -> StreamSpec {
            self.spec
        }
        fn codec(&self) -> &str {
            "tone"
        }
        fn bitrate_kbps(&self) -> Option<u32> {
            None
        }
        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let ch = self.spec.channels as usize;
            let want = (out.len() / ch).min(self.len.saturating_sub(self.pos) as usize);
            for f in 0..want {
                for c in 0..ch {
                    // A constant per channel: DC, so a resampler reproduces it
                    // exactly rather than approximately.
                    out[f * ch + c] = 0.5;
                }
            }
            self.pos += want as u64;
            Ok(want)
        }
        fn seek(&mut self, frame: u64) -> Result<u64> {
            self.pos = frame.min(self.len);
            Ok(self.pos)
        }
        fn position(&self) -> u64 {
            self.pos
        }
        fn total_frames(&self) -> Option<u64> {
            Some(self.len)
        }
    }

    fn drain(d: &mut dyn Decoder) -> Vec<f32> {
        let ch = d.spec().channels as usize;
        let mut all = Vec::new();
        let mut buf = vec![0f32; 1024 * ch];
        loop {
            let n = d.read(&mut buf).expect("read");
            if n == 0 {
                return all;
            }
            all.extend_from_slice(&buf[..n * ch]);
        }
    }

    fn out(sample_rate: u32, channels: u16) -> StreamSpec {
        StreamSpec {
            sample_rate,
            channels,
            bit_depth: None,
        }
    }

    #[test]
    fn scaling_rounds_up_and_survives_a_long_file() {
        assert_eq!(scale(48_000, 44_100, 48_000), 44_100);
        assert_eq!(scale(44_100, 48_000, 44_100), 48_000);
        // A ten-hour 192 kHz file is ~6.9e9 frames; the intermediate product
        // overflows u64 and must not.
        assert_eq!(scale(6_912_000_000, 48_000, 192_000), 1_728_000_000);
        assert_eq!(scale(100, 44_100, 44_100), 100);
    }

    /// The bug this module exists for: one second in has to be one second out.
    #[test]
    fn a_rate_the_device_refused_comes_out_the_right_length() {
        let mut a =
            Adapting::new(Tone::boxed(48_000, 2, 48_000), out(44_100, 2)).expect("adapting");
        assert_eq!(a.spec().sample_rate, 44_100);

        let frames = drain(&mut a).len() / 2;
        // One second of audio, however the resampler distributes its filter
        // delay across the ends.
        let drift = (frames as i64 - 44_100).abs();
        assert!(drift <= 64, "44100 expected, got {frames}");
    }

    #[test]
    fn upward_conversion_is_the_same_the_other_way() {
        let mut a =
            Adapting::new(Tone::boxed(44_100, 2, 44_100), out(48_000, 2)).expect("adapting");
        let frames = drain(&mut a).len() / 2;
        let drift = (frames as i64 - 48_000).abs();
        assert!(drift <= 64, "48000 expected, got {frames}");
    }

    /// A CoreAudio device offers only the channel counts the hardware has, so
    /// every mono file on a Mac arrives here.
    #[test]
    fn mono_reaches_a_stereo_only_device_on_both_channels() {
        let mut a = Adapting::new(Tone::boxed(44_100, 1, 4_410), out(44_100, 2)).expect("adapting");
        assert_eq!(a.spec().channels, 2);

        let samples = drain(&mut a);
        assert_eq!(samples.len(), 4_410 * 2, "one frame in, one frame out");
        // Rate is unchanged, so the samples are exact rather than filtered.
        for pair in samples.as_chunks::<2>().0.iter().skip(1) {
            assert_eq!(pair[0], pair[1], "the same signal in both channels");
            assert!(
                (pair[0] - 0.5).abs() < 1e-6,
                "amplitude preserved, not power-normalised"
            );
        }
    }

    #[test]
    fn the_duration_is_reported_in_the_frames_the_position_is_counted_in() {
        let a = Adapting::new(Tone::boxed(48_000, 2, 96_000), out(44_100, 2)).expect("adapting");
        // Two seconds, in the output's frames.
        assert_eq!(a.total_frames(), Some(88_200));
    }

    /// Seeking is asked for in output frames and has to land in the same
    /// domain, or the progress bar and the clock disagree after every seek.
    #[test]
    fn a_seek_is_asked_for_and_answered_in_output_frames() {
        let mut a =
            Adapting::new(Tone::boxed(48_000, 2, 480_000), out(44_100, 2)).expect("adapting");
        // Five seconds in, counted at 44.1 kHz.
        let landed = a.seek(220_500).expect("seek");
        let drift = (landed as i64 - 220_500).abs();
        assert!(drift <= 2, "220500 expected, got {landed}");
        assert_eq!(a.position(), landed, "position follows the seek");
    }

    #[test]
    fn nothing_is_converted_when_the_shape_already_matches() {
        // Not a case the callers construct -- they check `needs_adapting`
        // first -- but the wrapper must still be transparent if they do.
        let mut a = Adapting::new(Tone::boxed(44_100, 2, 1_000), out(44_100, 2)).expect("adapting");
        let samples = drain(&mut a);
        assert_eq!(samples.len(), 2_000);
        assert!(samples.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }
}
