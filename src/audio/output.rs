//! cpal output and the sample-rate policy.
//!
//! The policy is **follow the file**: open the device at the file's own rate so
//! the signal reaches the DAC untouched. PipeWire is normally configured with an
//! `allowed-rates` list precisely so the graph can renegotiate to match a
//! client, and reaching it through pipewire-alsa is what triggers that.
//!
//! When the device refuses the rate we fall back rather than fail, but we say so
//! — a "bit-perfect" indicator that lies is worse than none.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, StreamConfig, SupportedBufferSize};
use rtrb::Consumer;

use super::tap::Tap;

/// How the output rate was chosen, so the UI can be honest about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateMode {
    /// Device opened at the file's own rate. Bit-perfect, if nothing else in the
    /// chain touches the samples.
    Native,
    /// Device refused the file's rate; samples are being converted.
    Resampled { from: u32, to: u32 },
}

impl RateMode {
    pub fn is_bit_perfect(&self) -> bool {
        matches!(self, RateMode::Native)
    }
}

/// The device, and the shape it agreed to take.
///
/// Decided *before* the ring is built rather than inside [`Output::open`],
/// because everything upstream depends on the answer: the ring is sized in the
/// device's frames, and a stream the device would not take at its own rate has
/// to be converted on the way in. Asking the device last -- which is what this
/// used to do -- meant the answer arrived after every decision that needed it,
/// and a rate the device had refused was simply played at the wrong speed.
pub struct Plan {
    device: cpal::Device,
    pub device_name: String,
    /// What the device will actually run at.
    pub sample_rate: u32,
    pub channels: u16,
    pub rate_mode: RateMode,
    /// The source's channel count, when the device would not take it. A
    /// stereo-only device is the normal cause; CoreAudio offers only the
    /// counts the hardware physically has, so a mono file has nowhere to go.
    pub remixed_from: Option<u16>,
}

impl Plan {
    /// The shape a decoder has to produce for this device.
    pub fn out_spec(
        &self,
        src: &crate::audio::decode::StreamSpec,
    ) -> crate::audio::decode::StreamSpec {
        crate::audio::decode::StreamSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            bit_depth: src.bit_depth,
        }
    }

    /// True when the samples reach the device exactly as the file holds them.
    ///
    /// Both halves matter: a rate the device took is not bit-perfect if the
    /// channels had to be remixed to get there.
    pub fn is_bit_perfect(&self) -> bool {
        self.rate_mode.is_bit_perfect() && self.remixed_from.is_none()
    }

    /// Whether a decoder producing `src` needs converting first.
    pub fn needs_adapting(&self, src: &crate::audio::decode::StreamSpec) -> bool {
        src.sample_rate != self.sample_rate || src.channels != self.channels
    }
}

/// Choose the device and the format it will run at, for a stream of `spec`.
///
/// `fixed_rate` is `[output] mode = "fixed"`: pin the device to one rate and
/// convert everything to it, instead of following each file. It costs
/// bit-perfect playback and buys never rebuilding the stream at a track
/// boundary -- worth having where the device only offers one rate anyway.
pub fn plan(spec: &crate::audio::decode::StreamSpec, fixed_rate: Option<u32>) -> Result<Plan> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default output device"))?;
    let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());

    let wanted = fixed_rate.unwrap_or(spec.sample_rate);
    let (sample_rate, channels) = choose_format(&device, wanted, spec.channels)?;

    // Measured against the *file*, not against what was asked for: pinning the
    // device to 48 kHz and getting it is still a conversion for a 44.1 kHz
    // track, and the indicator has to say so.
    let rate_mode = if sample_rate == spec.sample_rate {
        RateMode::Native
    } else {
        RateMode::Resampled {
            from: spec.sample_rate,
            to: sample_rate,
        }
    };

    Ok(Plan {
        device,
        device_name,
        sample_rate,
        channels,
        rate_mode,
        remixed_from: (channels != spec.channels).then_some(spec.channels),
    })
}

/// State the callback publishes and everyone else reads.
pub struct OutputState {
    /// Frames handed to the device. The authority on playback position.
    pub frames_out: AtomicU64,
    /// Times the callback found the ring empty. Silent underruns are how
    /// players earn a reputation for crackling, so this is always visible.
    pub underruns: AtomicU64,
    pub paused: AtomicBool,
    /// Set once the decode side is done *and* the ring has drained.
    pub finished: AtomicBool,
}

impl OutputState {
    fn new(paused: bool) -> Self {
        Self {
            frames_out: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            paused: AtomicBool::new(paused),
            finished: AtomicBool::new(false),
        }
    }
}

pub struct Output {
    stream: cpal::Stream,
    pub state: Arc<OutputState>,
    pub rate_mode: RateMode,
    /// Rate *and* channels reached the device untouched. Narrower than
    /// `rate_mode.is_bit_perfect()`, which only knows about the rate.
    pub bit_perfect: bool,
    /// The file's channel count, when the device would not take it.
    pub remixed_from: Option<u16>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Output {
    /// Open the default device for a stream of the given shape.
    ///
    /// `source_done` tells the callback that no more audio is coming, so that an
    /// empty ring means end-of-track rather than an underrun.
    ///
    /// `start_paused` decides who is responsible for the ring being non-empty
    /// when the device first pulls, and there is no safe default: the callback
    /// begins running the moment the stream is built, so a caller that has not
    /// already filled the ring must open paused or the first callbacks read an
    /// empty ring. That is a real click at the start of the track and a burst
    /// of counted underruns for a fault nobody committed.
    ///
    /// Two callers, two answers. `staramp play` prefills and opens running;
    /// the player opens paused and resumes once its ring is half full, because
    /// it cannot prefill -- its decoder is on another thread that has not been
    /// asked for anything yet.
    pub fn open(
        plan: Plan,
        mut consumer: Consumer<f32>,
        source_done: Arc<AtomicBool>,
        tap: Arc<Tap>,
        start_paused: bool,
    ) -> Result<Self> {
        let bit_perfect = plan.is_bit_perfect();
        let Plan {
            device,
            device_name,
            sample_rate,
            channels,
            rate_mode,
            remixed_from,
        } = plan;

        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let state = Arc::new(OutputState::new(start_paused));
        let cb_state = Arc::clone(&state);

        let stream = device
            .build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    // Everything here is allocation-free and lock-free. No I/O,
                    // no mutex, no `Vec`. That rule is what lets a blocked disk
                    // read stall the decoder without stalling the audio.
                    if cb_state.paused.load(Ordering::Relaxed) {
                        out.fill(0.0);
                        return;
                    }

                    let chunk = consumer.read_chunk(out.len().min(consumer.slots()));
                    let got = match chunk {
                        Ok(c) => {
                            let (a, b) = c.as_slices();
                            out[..a.len()].copy_from_slice(a);
                            out[a.len()..a.len() + b.len()].copy_from_slice(b);
                            let n = a.len() + b.len();
                            c.commit_all();
                            n
                        }
                        Err(_) => 0,
                    };

                    if got < out.len() {
                        out[got..].fill(0.0);
                        // Running dry after the decoder has finished is the end
                        // of the track, not a fault.
                        if source_done.load(Ordering::Acquire) {
                            cb_state.finished.store(true, Ordering::Release);
                        } else {
                            cb_state.underruns.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    // Tap what is going to the device, not what the decoder
                    // produced: the ring holds ~200ms, and tapping upstream
                    // showed the analyzer reacting that far ahead of the sound.
                    tap.write(&out[..got], channels as usize);

                    cb_state
                        .frames_out
                        .fetch_add((got / channels as usize) as u64, Ordering::Relaxed);
                },
                |err| tracing::error!("output stream error: {err}"),
                None,
            )
            .context("building output stream")?;

        stream.play().context("starting output stream")?;

        Ok(Self {
            stream,
            state,
            rate_mode,
            bit_perfect,
            remixed_from,
            device_name,
            sample_rate,
            channels,
        })
    }

    pub fn pause(&self) {
        self.state.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.state.paused.store(false, Ordering::Relaxed);
    }

    pub fn stop(&self) {
        let _ = self.stream.pause();
    }
}

/// Pick the output format, preferring the file's own rate and channel count.
///
/// Channels first, because they decide which configs the rate can be chosen
/// from. A device that cannot do the file's channel count is not an error: a
/// CoreAudio device advertises only the counts the hardware physically has, so
/// every mono file on a Mac would otherwise refuse to play. Pick the closest
/// count the device does offer and let the caller convert.
fn choose_format(device: &cpal::Device, wanted: u32, wanted_channels: u16) -> Result<(u32, u16)> {
    let all: Vec<_> = device
        .supported_output_configs()
        .context("querying device output configs")?
        .collect();

    if all.is_empty() {
        return Err(anyhow!("device reports no output configuration at all"));
    }

    // The file's own count where it exists; otherwise the fewest that is at
    // least as many, so mono lands on stereo rather than on 7.1.
    let channels = if all.iter().any(|c| c.channels() == wanted_channels) {
        wanted_channels
    } else {
        all.iter()
            .map(|c| c.channels())
            .filter(|&c| c > wanted_channels)
            .min()
            .or_else(|| all.iter().map(|c| c.channels()).max())
            .ok_or_else(|| anyhow!("device offers no usable channel count"))?
    };

    let supported: Vec<_> = all
        .into_iter()
        .filter(|c| c.channels() == channels)
        .collect();

    let accepts = |rate: u32| {
        supported
            .iter()
            .any(|c| c.min_sample_rate().0 <= rate && rate <= c.max_sample_rate().0)
    };

    if accepts(wanted) {
        return Ok((wanted, channels));
    }

    // Stay in the same family: a 44.1 kHz-derived file resampled to 48 kHz is a
    // worse outcome than one resampled to 88.2 kHz.
    let family: &[u32] = if wanted.is_multiple_of(11_025) {
        &[44_100, 88_200, 176_400, 48_000, 96_000, 192_000]
    } else {
        &[48_000, 96_000, 192_000, 44_100, 88_200, 176_400]
    };
    for &candidate in family {
        if accepts(candidate) {
            return Ok((candidate, channels));
        }
    }

    Ok((supported[0].max_sample_rate().0, channels))
}

/// Describe the device's buffer-size range, for diagnostics.
pub fn describe_buffer_size(b: &SupportedBufferSize) -> String {
    match b {
        SupportedBufferSize::Range { min, max } => format!("{min}..{max}"),
        SupportedBufferSize::Unknown => "unknown".into(),
    }
}
