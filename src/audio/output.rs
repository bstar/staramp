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
use cpal::traits::{DeviceTrait, StreamTrait};
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
    device: Option<starkit::pcm_output::Plan>,
    pub source: starkit::pcm::Spec,
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
            sample_format: src.sample_format,
            channel_mask: if self.device.is_none() && self.channels == src.channels {
                src.channel_mask
            } else {
                ffmpeg_next::ChannelLayout::default(i32::from(self.channels)).bits()
            },
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
        self.rate_mode.is_bit_perfect()
            && self.remixed_from.is_none()
            && self.device.as_ref().is_none_or(|d| !d.converted)
    }

    /// Whether a decoder producing `src` needs converting first.
    pub fn needs_adapting(&self, src: &crate::audio::decode::StreamSpec) -> bool {
        src.sample_rate != self.sample_rate
            || src.channels != self.channels
            || (src.channel_mask != 0 && src.channel_mask != self.out_spec(src).channel_mask)
    }
}

/// Choose the device and the format it will run at, for a stream of `spec`.
///
/// `fixed_rate` is `[output] mode = "fixed"`: pin the device to one rate and
/// convert everything to it, instead of following each file. It costs
/// bit-perfect playback and buys never rebuilding the stream at a track
/// boundary -- worth having where the device only offers one rate anyway.
pub fn plan(spec: &crate::audio::decode::StreamSpec, fixed_rate: Option<u32>) -> Result<Plan> {
    if super::relay::enabled() {
        return Ok(Plan {
            source: spec.pcm(),
            device: None,
            device_name: "SSH client".into(),
            sample_rate: if super::relay::precise() {
                spec.sample_rate
            } else {
                48000
            },
            channels: if super::relay::precise() {
                spec.channels
            } else {
                2
            },
            rate_mode: if super::relay::precise() || spec.sample_rate == 48000 {
                RateMode::Native
            } else {
                RateMode::Resampled {
                    from: spec.sample_rate,
                    to: 48000,
                }
            },
            remixed_from: (!super::relay::precise() && spec.channels != 2).then_some(spec.channels),
        });
    }
    let device = starkit::pcm_output::Plan::select(
        starkit::pcm::Spec {
            sample_rate: fixed_rate.unwrap_or(spec.sample_rate),
            ..spec.pcm()
        },
        true,
    )?;
    let device_name = device.device.name().unwrap_or_else(|_| "<unknown>".into());
    let sample_rate = device.config.sample_rate.0;
    let channels = device.config.channels;

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
        source: spec.pcm(),
        device: Some(device),
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
    /// Current starvation, published independently of a blocked decoder.
    pub buffering: AtomicBool,
    /// Set once the decode side is done *and* the ring has drained.
    pub finished: AtomicBool,
}

impl OutputState {
    pub(super) fn new(paused: bool) -> Self {
        Self {
            frames_out: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            paused: AtomicBool::new(paused),
            buffering: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    fn read_result(&self, got: usize, wanted: usize, done: bool) {
        self.buffering
            .store(got < wanted && !done, Ordering::Relaxed);
        if got < wanted {
            if done {
                self.finished.store(true, Ordering::Release);
            } else {
                self.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub struct Output {
    device_pause: Option<Arc<AtomicBool>>,
    errors: Option<crossbeam_channel::Receiver<String>>,
    stream: Option<cpal::Stream>,
    relay_stop: Arc<AtomicBool>,
    relay_worker: Option<std::thread::JoinHandle<()>>,
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
        consumer: Consumer<starkit::pcm::Sample>,
        source_done: Arc<AtomicBool>,
        tap: Arc<Tap>,
        start_paused: bool,
    ) -> Result<Self> {
        if plan.device.is_none() {
            return Self::relay(plan, consumer, source_done, tap, start_paused);
        }
        let bit_perfect = plan.is_bit_perfect();
        let Plan {
            device,
            device_name,
            source,
            sample_rate,
            channels,
            rate_mode,
            remixed_from,
        } = plan;
        let state = Arc::new(OutputState::new(start_paused));
        let cb_state = state.clone();
        let pause = Arc::new(AtomicBool::new(start_paused));
        let (error_tx, error_rx) = crossbeam_channel::bounded(1);
        let stream = device.unwrap().open(
            consumer,
            source,
            pause.clone(),
            error_tx,
            move |values, wanted| {
                cb_state.read_result(values.len(), wanted, source_done.load(Ordering::Acquire));
                tap.write_f64(values, channels as usize);
                cb_state
                    .frames_out
                    .fetch_add((values.len() / channels as usize) as u64, Ordering::Relaxed);
            },
        )?;
        stream.play().context("starting output stream")?;

        Ok(Self {
            device_pause: Some(pause),
            errors: Some(error_rx),
            stream: Some(stream),
            relay_stop: Arc::new(AtomicBool::new(false)),
            relay_worker: None,
            state,
            rate_mode,
            bit_perfect,
            remixed_from,
            device_name,
            sample_rate,
            channels,
        })
    }

    fn relay(
        plan: Plan,
        mut consumer: Consumer<starkit::pcm::Sample>,
        done: Arc<AtomicBool>,
        tap: Arc<Tap>,
        paused: bool,
    ) -> Result<Self> {
        let state = Arc::new(OutputState::new(paused));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let cb = state.clone();
        let epoch = super::relay::next_epoch();
        let cached = super::relay::cache_mode();
        if cached {
            tap.begin_cache(epoch);
        }
        let worker = std::thread::Builder::new()
            .name("staramp-relay-output".into())
            .spawn(move || {
                let mut next = std::time::Instant::now();
                while !worker_stop.load(Ordering::Relaxed) {
                    if cb.paused.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        next = std::time::Instant::now();
                        continue;
                    }
                    let precise = super::relay::precise();
                    let channels = if precise { plan.channels as usize } else { 2 };
                    let capacity = if precise {
                        plan.source
                            .block_frames()
                            .min(starkit::pcm::MAX_BLOCK_BYTES / (channels * 8))
                            * channels
                    } else {
                        super::relay::FRAMES * 2
                    };
                    let mut count = consumer.slots().min(capacity) / channels * channels;
                    let mut processed = false;
                    if count > 0 {
                        // Never combine bypass samples and processed samples in
                        // a packet; both metadata and integer conversion depend on it.
                        let chunk = consumer.read_chunk(count).unwrap();
                        let (a, b) = chunk.as_slices();
                        processed = a.first().or_else(|| b.first()).unwrap().processed;
                        count = a
                            .iter()
                            .chain(b.iter())
                            .take_while(|s| s.processed == processed)
                            .count()
                            / channels
                            * channels;
                        // Read cursor is unchanged until values below are consumed.
                    }
                    cb.read_result(count, capacity, done.load(Ordering::Acquire));
                    if count == 0 {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        next = std::time::Instant::now();
                        continue;
                    }
                    let values: Vec<f64> =
                        (0..count).map(|_| consumer.pop().unwrap().value).collect();
                    let pcm = if precise {
                        let spec = if processed {
                            starkit::pcm::Spec {
                                format: starkit::pcm::SampleFormat::F64,
                                valid_bits: Some(match plan.source.format {
                                    starkit::pcm::SampleFormat::F32 => 24,
                                    _ => plan.source.valid_bits.unwrap_or(64),
                                }),
                                ..plan.source
                            }
                        } else {
                            plan.source
                        };
                        match spec.encode(&values) {
                            Ok(data) => Some((plan.source, spec.format, processed, data)),
                            Err(_) => break,
                        }
                    } else {
                        None
                    };
                    let samples = if precise {
                        Vec::new()
                    } else {
                        values
                            .iter()
                            .map(|v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                            .collect()
                    };
                    if !super::relay::send(
                        super::relay::Block {
                            epoch,
                            samples,
                            pcm,
                        },
                        &worker_stop,
                    ) {
                        break;
                    }
                    if cached {
                        tap.write_cached(epoch, &values, channels);
                    } else {
                        tap.write_f64(&values, channels);
                    }
                    cb.frames_out
                        .fetch_add((count / channels) as u64, Ordering::Relaxed);
                    // Precise SSH playback is clocked by the local device and
                    // bounded by its credits plus the relay queue. A second
                    // wall clock here loses scheduler time every packet and
                    // prevents the frontend from refilling after an underrun.
                    if precise {
                        continue;
                    }
                    next += std::time::Duration::from_secs_f64(
                        count as f64 / (plan.sample_rate as f64 * channels as f64),
                    );
                    while std::time::Instant::now() < next && !worker_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    // Backpressure changes the clock; never burst to catch up.
                    if next < std::time::Instant::now() {
                        next = std::time::Instant::now();
                    }
                }
            })?;
        Ok(Self {
            device_pause: None,
            errors: None,
            stream: None,
            relay_stop: stop,
            relay_worker: Some(worker),
            state,
            rate_mode: plan.rate_mode,
            bit_perfect: false,
            remixed_from: plan.remixed_from,
            device_name: plan.device_name,
            sample_rate: plan.sample_rate,
            channels: plan.channels,
        })
    }

    pub fn pause(&self) {
        if let Some(pause) = &self.device_pause {
            pause.store(true, Ordering::Relaxed);
        }
        self.state.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        if let Some(pause) = &self.device_pause {
            pause.store(false, Ordering::Relaxed);
        }
        self.state.buffering.store(true, Ordering::Relaxed);
        self.state.paused.store(false, Ordering::Relaxed);
    }

    pub fn error(&self) -> Option<String> {
        self.errors.as_ref().and_then(|rx| rx.try_recv().ok())
    }
    pub fn stop(&self) {
        self.relay_stop.store(true, Ordering::Relaxed);
        if let Some(stream) = &self.stream {
            let _ = stream.pause();
        }
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

impl Drop for Output {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.relay_worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod relay_tests {
    use super::*;
    #[test]
    fn relay_channel_conversion_replaces_the_source_speaker_mask() {
        let source = crate::audio::decode::StreamSpec {
            sample_rate: 44100,
            channels: 1,
            channel_mask: ffmpeg_next::ChannelLayout::MONO.bits(),
            sample_format: starkit::pcm::SampleFormat::I32,
            bit_depth: Some(16),
        };
        let mut plan = Plan {
            device: None,
            source: source.pcm(),
            device_name: "SSH client".into(),
            sample_rate: 48000,
            channels: 2,
            rate_mode: RateMode::Resampled {
                from: 44100,
                to: 48000,
            },
            remixed_from: Some(1),
        };
        let stereo = plan.out_spec(&source);
        stereo.pcm().validate().unwrap();
        assert_eq!(
            stereo.channel_mask,
            ffmpeg_next::ChannelLayout::STEREO.bits()
        );
        plan.channels = 1;
        plan.sample_rate = 44100;
        plan.remixed_from = None;
        plan.rate_mode = RateMode::Native;
        assert_eq!(plan.out_spec(&source).channel_mask, source.channel_mask);
    }

    #[test]
    fn callback_reports_storage_starvation_recovery_and_eof_independently_of_decoder() {
        let state = OutputState::new(false);
        state.read_result(1920, 1920, false);
        assert!(!state.buffering.load(Ordering::Relaxed));
        state.read_result(0, 1920, false);
        assert!(state.buffering.load(Ordering::Relaxed));
        assert!(!state.finished.load(Ordering::Acquire));
        state.read_result(1920, 1920, false);
        assert!(!state.buffering.load(Ordering::Relaxed));
        state.read_result(0, 1920, true);
        assert!(!state.buffering.load(Ordering::Relaxed));
        assert!(state.finished.load(Ordering::Acquire));
        assert_eq!(state.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pcm_output_is_bounded_pauses_and_joins_without_a_device() {
        let receiver = super::super::relay::receiver();
        for _ in receiver.try_iter() {}
        let (mut producer, consumer) = crate::audio::ring::create(48000, 2);
        for _ in 0..1920 {
            producer.push(0.5.into()).unwrap();
        }
        let output = Output::relay(
            Plan {
                source: starkit::pcm::Spec {
                    sample_rate: 48000,
                    channels: 2,
                    channel_mask: 3,
                    format: starkit::pcm::SampleFormat::I32,
                    valid_bits: Some(16),
                },
                device: None,
                device_name: "test".into(),
                sample_rate: 48000,
                channels: 2,
                rate_mode: RateMode::Native,
                remixed_from: None,
            },
            consumer,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Tap::new(64)),
            true,
        )
        .unwrap();
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        output.resume();
        let block = receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(block.samples.len(), 1920);
        assert!(block.samples.iter().all(|sample| *sample == 16384));
        assert!(!output.bit_perfect);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !output.state.buffering.load(Ordering::Relaxed)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            output.state.buffering.load(Ordering::Relaxed),
            "empty output must report a stall without any decoder notification"
        );
        output.pause();
        output.resume();
        assert!(output.state.buffering.load(Ordering::Relaxed));
        drop(output);
    }
}
