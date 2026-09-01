//! The native decode path: FLAC, MP3, Vorbis, AAC, ALAC, WAV, AIFF.
//!
//! Covers 20,890 of the 22,048 files in the reference library. Which files reach
//! this module is decided by the extension allowlist in the parent module, not
//! by probing — see the note on WavPack there for why probing is not safe.

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder as SymDecoder, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::{Decoder, StreamSpec};

pub struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn SymDecoder>,
    track_id: u32,
    spec: StreamSpec,
    /// Codec short name, for display. Symphonia's registry knows it; the
    /// extension does not (`.m4a` is AAC or ALAC).
    codec: &'static str,
    /// Backing file size, for the average-bitrate calculation.
    file_size: u64,
    /// False until the sample rate and channel count are known. Some containers
    /// (notably ADTS/MP4 AAC) do not declare them, so they are learned from the
    /// first decoded packet instead.
    spec_known: bool,
    total_frames: Option<u64>,

    /// Decoded interleaved samples not yet handed to the caller. Packets decode
    /// to a variable number of frames, so a leftover buffer is unavoidable.
    buf: Option<SampleBuffer<f32>>,
    /// Read cursor into `buf`, in samples.
    buf_pos: usize,
    /// Valid samples in `buf`.
    buf_len: usize,

    /// Absolute frame position of the next frame `read` will return.
    pos: u64,
    eos: bool,
}

impl SymphoniaDecoder {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let fmt_opts = FormatOptions {
            // Applies LAME/Xing encoder delay and padding trimming. This is the
            // difference between real and fake MP3 gapless.
            enable_gapless: true,
            ..Default::default()
        };

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
            .with_context(|| format!("probing {}", path.display()))?;
        let format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| anyhow!("{}: no default track", path.display()))?;
        let track_id = track.id;
        let params = &track.codec_params;

        let codec = symphonia::default::get_codecs()
            .get_codec(params.codec)
            .map(|d| d.short_name)
            .unwrap_or("audio");

        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .with_context(|| format!("no decoder for {}", path.display()))?;

        // AAC in MP4 routinely omits both of these at the container level; they
        // only become known once a packet has been decoded.
        let declared_rate = params.sample_rate;
        let declared_channels = params.channels.map(|c| c.count() as u16);
        let spec_known = declared_rate.is_some() && declared_channels.is_some();

        let mut this = Self {
            spec: StreamSpec {
                sample_rate: declared_rate.unwrap_or(0),
                channels: declared_channels.unwrap_or(0),
                bit_depth: params.bits_per_sample,
            },
            codec,
            file_size,
            spec_known,
            total_frames: params.n_frames,
            format,
            decoder,
            track_id,
            buf: None,
            buf_pos: 0,
            buf_len: 0,
            pos: 0,
            eos: false,
        };

        if !this.spec_known {
            // Decode one packet to learn the format. The audio is kept, not
            // discarded — `fill` leaves it buffered for the first `read`.
            this.fill().with_context(|| {
                format!("{}: priming to discover stream format", path.display())
            })?;
            if !this.spec_known {
                return Err(anyhow!(
                    "{}: no audio decoded, cannot determine format",
                    path.display()
                ));
            }
        }

        Ok(this)
    }

    /// Decode packets until `buf` holds samples, or the stream ends.
    ///
    /// Returns `false` at end of stream.
    fn fill(&mut self) -> Result<bool> {
        loop {
            if self.buf_pos < self.buf_len {
                return Ok(true);
            }
            if self.eos {
                return Ok(false);
            }

            let packet = match self.format.next_packet() {
                Ok(p) => p,
                // Symphonia signals end of stream as an unexpected EOF.
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eos = true;
                    return Ok(false);
                }
                Err(SymError::ResetRequired) => {
                    // A chained stream changed parameters mid-file. Rather than
                    // silently continue with the wrong parameters, stop cleanly.
                    self.eos = true;
                    return Ok(false);
                }
                Err(e) => return Err(e).context("reading packet"),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let ts = packet.ts();
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let frames = decoded.frames();
                    if frames == 0 {
                        continue;
                    }
                    let sig = *decoded.spec();

                    if !self.spec_known {
                        self.spec.sample_rate = sig.rate;
                        self.spec.channels = sig.channels.count() as u16;
                        self.spec_known = true;
                    }

                    let need = frames * sig.channels.count();
                    // Reallocate only when a packet is larger than any seen so
                    // far. In practice this settles after the first packet.
                    let grow = match &self.buf {
                        Some(b) => b.capacity() < need,
                        None => true,
                    };
                    if grow {
                        self.buf = Some(SampleBuffer::<f32>::new(frames as u64, sig));
                    }
                    let buf = self.buf.as_mut().expect("just allocated");
                    buf.copy_interleaved_ref(decoded);

                    self.buf_len = buf.len();
                    self.buf_pos = 0;
                    // Trust the container's timestamp over our own tally: it is
                    // correct across seeks and gapless trimming, and ours is not.
                    self.pos = ts;
                }
                // A corrupt packet is not a corrupt file. Skip it and carry on,
                // which is what every other player does and what users expect.
                Err(SymError::DecodeError(e)) => {
                    tracing::debug!("skipping undecodable packet: {e}");
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.eos = true;
                    return Ok(false);
                }
                Err(e) => return Err(e).context("decoding packet"),
            }
        }
    }
}

impl Decoder for SymphoniaDecoder {
    fn spec(&self) -> StreamSpec {
        self.spec
    }

    fn codec(&self) -> &str {
        self.codec
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
        let mut done_frames = 0usize;

        while done_frames < want_frames {
            if !self.fill()? {
                break;
            }
            let buf = self.buf.as_ref().expect("fill guarantees a buffer");
            let avail_frames = (self.buf_len - self.buf_pos) / ch;
            let take = avail_frames.min(want_frames - done_frames);
            if take == 0 {
                break;
            }

            let src = &buf.samples()[self.buf_pos..self.buf_pos + take * ch];
            out[done_frames * ch..(done_frames + take) * ch].copy_from_slice(src);

            self.buf_pos += take * ch;
            self.pos += take as u64;
            done_frames += take;
        }

        Ok(done_frames)
    }

    fn seek(&mut self, frame: u64) -> Result<u64> {
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::TimeStamp {
                    ts: frame,
                    track_id: self.track_id,
                },
            )
            .context("seeking")?;

        // Decoders carry inter-packet state; after a seek that state describes
        // the wrong part of the file.
        self.decoder.reset();
        self.buf_pos = 0;
        self.buf_len = 0;
        self.eos = false;
        self.pos = seeked.actual_ts;
        Ok(seeked.actual_ts)
    }

    fn position(&self) -> u64 {
        self.pos
    }

    fn total_frames(&self) -> Option<u64> {
        self.total_frames
    }
}
