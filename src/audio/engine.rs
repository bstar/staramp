//! Wiring the decoder to the output.
//!
//! Three pieces, deliberately separated: a decode thread that may block on I/O,
//! a lock-free ring, and an output callback that may not block on anything. The
//! decode thread can stall on a cold USB disk for 100 ms and the audio keeps
//! playing out of the ring; that is the whole point of the arrangement.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;

use super::decode::StreamSpec;
use super::output::{Output, RateMode};
use super::ring;
use super::source;
use crate::playlist::uri::TrackUri;
use crate::vfs::Vfs;

/// How long the decode thread sleeps when the ring is full. Short enough to
/// refill promptly, long enough not to spin a core.
const BACKOFF: Duration = Duration::from_millis(2);

/// Fraction of the ring to fill before letting the device start.
const PREFILL: f32 = 0.5;

/// Give up waiting for the prefill after this long. A very short file, or a slow
/// first read off a cold disk, must not stall startup indefinitely.
const PREFILL_TIMEOUT: Duration = Duration::from_millis(500);

/// Block until the ring holds enough audio to cover the first few callbacks.
fn prefill(
    consumer: &rtrb::Consumer<f32>,
    sample_rate: u32,
    channels: u16,
    source_done: &AtomicBool,
) {
    let target = (ring::capacity_samples(sample_rate, channels) as f32 * PREFILL) as usize;
    let deadline = std::time::Instant::now() + PREFILL_TIMEOUT;
    while consumer.slots() < target {
        // A file shorter than the prefill target would otherwise spin here.
        if source_done.load(Ordering::Acquire) || std::time::Instant::now() > deadline {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

pub struct Playback {
    output: Output,
    source_done: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    decoded_frames: Arc<AtomicU64>,
    decoder: JoinHandle<Result<()>>,
    pub spec: StreamSpec,
    pub total_frames: Option<u64>,
    /// Where playback began, so reported positions are absolute in the file.
    start_frame: u64,
    /// Track title, for cue virtual tracks.
    pub description: Option<String>,
    /// The audio file actually being read.
    pub backing_path: std::path::PathBuf,
}

impl Playback {
    /// Open a file, start the output, and begin decoding into it.
    pub fn start(path: &Path) -> Result<Self> {
        Self::start_at(path, 0.0)
    }

    /// As `start`, beginning `start_secs` into the file.
    ///
    /// The seek happens before the decode thread takes ownership, which keeps
    /// mid-playback seeking (a command-bus concern) out of this phase entirely.
    pub fn start_at(path: &Path, start_secs: f64) -> Result<Self> {
        // A path that ends in `<something>.cue/trackNNNN` is a virtual track,
        // not a file. Parsing it here means the CLI and the playlist layer
        // address tracks the same way.
        let uri = TrackUri::parse(&path.to_string_lossy());
        // The CLI names files directly, so the URIs are already absolute and
        // there is no root to resolve them against.
        let vfs = Vfs::local("");
        let opened = source::open(&vfs, None, &uri)?;
        let mut dec = opened.decoder;
        let spec = dec.spec();
        let total_frames = dec.total_frames();

        let mut start_frame = 0u64;
        if start_secs > 0.0 {
            let target = (start_secs * spec.sample_rate as f64) as u64;
            start_frame = dec.seek(target)?;
        }

        let (mut producer, consumer) = ring::create(spec.sample_rate, spec.channels);
        let source_done = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let decoded_frames = Arc::new(AtomicU64::new(0));

        let ch = spec.channels as usize;
        let thread_done = Arc::clone(&source_done);
        let thread_stop = Arc::clone(&stop);
        let thread_frames = Arc::clone(&decoded_frames);

        let decoder =
            thread::Builder::new()
                .name("staramp-decode".into())
                .spawn(move || -> Result<()> {
                    // Sized to a comfortable multiple of a typical device quantum,
                    // and unrelated to the ring: this is just working space.
                    let mut buf = vec![0f32; 4096 * ch];
                    loop {
                        if thread_stop.load(Ordering::Relaxed) {
                            break;
                        }

                        let free = producer.slots();
                        if free < ch {
                            thread::sleep(BACKOFF);
                            continue;
                        }

                        let want_frames = (free / ch).min(buf.len() / ch);
                        let frames = dec.read(&mut buf[..want_frames * ch])?;
                        if frames == 0 {
                            break;
                        }

                        for &s in &buf[..frames * ch] {
                            // Space was checked above, so this cannot fail; if it
                            // somehow does, dropping a sample beats panicking on the
                            // decode thread.
                            let _ = producer.push(s);
                        }
                        thread_frames.fetch_add(frames as u64, Ordering::Relaxed);
                    }

                    // Ordering matters: the callback treats an empty ring as the end
                    // of the track only once this is set.
                    thread_done.store(true, Ordering::Release);
                    Ok(())
                })?;

        // Prime the ring before the device starts pulling. Without this the very
        // first callback fires against an empty ring and records an underrun --
        // an audible click at the start of every track.
        prefill(&consumer, spec.sample_rate, spec.channels, &source_done);

        // `staramp play` has no visualizer, so its tap is a stub.
        let output = Output::open(
            spec.sample_rate,
            spec.channels,
            consumer,
            Arc::clone(&source_done),
            Arc::new(crate::audio::tap::Tap::new(2)),
            // Already prefilled above, so it can start pulling immediately.
            false,
        )?;

        Ok(Self {
            output,
            source_done,
            stop,
            decoded_frames,
            decoder,
            spec,
            total_frames,
            start_frame,
            description: opened.virtual_track.as_ref().and_then(|t| t.title.clone()),
            backing_path: opened.backing_path,
        })
    }

    /// True once every decoded frame has been handed to the device.
    pub fn finished(&self) -> bool {
        self.output.state.finished.load(Ordering::Acquire)
    }

    /// Frames handed to the device — the authority on playback position.
    pub fn position_frames(&self) -> u64 {
        self.start_frame + self.output.state.frames_out.load(Ordering::Relaxed)
    }

    pub fn position_secs(&self) -> f64 {
        self.position_frames() as f64 / self.spec.sample_rate as f64
    }

    pub fn underruns(&self) -> u64 {
        self.output.state.underruns.load(Ordering::Relaxed)
    }

    pub fn decoded_frames(&self) -> u64 {
        self.decoded_frames.load(Ordering::Relaxed)
    }

    pub fn rate_mode(&self) -> RateMode {
        self.output.rate_mode
    }

    pub fn device_name(&self) -> &str {
        &self.output.device_name
    }

    pub fn output_rate(&self) -> u32 {
        self.output.sample_rate
    }

    pub fn pause(&self) {
        self.output.pause();
    }

    pub fn resume(&self) {
        self.output.resume();
    }

    /// Stop the decode thread and the device, and wait for the thread to exit.
    pub fn stop(self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.output.stop();
        match self.decoder.join() {
            Ok(r) => r,
            Err(_) => anyhow::bail!("decode thread panicked"),
        }
    }
}
