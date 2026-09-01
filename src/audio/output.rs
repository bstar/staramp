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
    fn new() -> Self {
        Self {
            frames_out: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }
}

pub struct Output {
    stream: cpal::Stream,
    pub state: Arc<OutputState>,
    pub rate_mode: RateMode,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Output {
    /// Open the default device for a stream of the given shape.
    ///
    /// `source_done` tells the callback that no more audio is coming, so that an
    /// empty ring means end-of-track rather than an underrun.
    pub fn open(
        sample_rate: u32,
        channels: u16,
        mut consumer: Consumer<f32>,
        source_done: Arc<AtomicBool>,
        tap: Arc<Tap>,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default output device"))?;
        let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());

        let (chosen_rate, rate_mode) = choose_rate(&device, sample_rate, channels)?;

        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(chosen_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let state = Arc::new(OutputState::new());
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
            device_name,
            sample_rate: chosen_rate,
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

/// Pick the output rate, preferring the file's own.
fn choose_rate(device: &cpal::Device, wanted: u32, channels: u16) -> Result<(u32, RateMode)> {
    let supported: Vec<_> = device
        .supported_output_configs()
        .context("querying device output configs")?
        .filter(|c| c.channels() == channels)
        .collect();

    if supported.is_empty() {
        return Err(anyhow!(
            "device supports no {channels}-channel output configuration"
        ));
    }

    let accepts = |rate: u32| {
        supported
            .iter()
            .any(|c| c.min_sample_rate().0 <= rate && rate <= c.max_sample_rate().0)
    };

    if accepts(wanted) {
        return Ok((wanted, RateMode::Native));
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
            return Ok((
                candidate,
                RateMode::Resampled {
                    from: wanted,
                    to: candidate,
                },
            ));
        }
    }

    let fallback = supported[0].max_sample_rate().0;
    Ok((
        fallback,
        RateMode::Resampled {
            from: wanted,
            to: fallback,
        },
    ))
}

/// Describe the device's buffer-size range, for diagnostics.
pub fn describe_buffer_size(b: &SupportedBufferSize) -> String {
    match b {
        SupportedBufferSize::Range { min, max } => format!("{min}..{max}"),
        SupportedBufferSize::Unknown => "unknown".into(),
    }
}
