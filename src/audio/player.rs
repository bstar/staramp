//! The player: a queue, a decode thread, and an output that survives track
//! changes.
//!
//! The output stream is only ever torn down for a sample-rate change or a stop.
//! Track boundaries do not touch it, which is what makes gapless possible at
//! all: the decode thread simply starts filling the same ring from the next
//! decoder, mid-buffer, with no discontinuity for the callback to notice.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};

use super::decode::Decoder;
use super::dsp::eq::{EqHandle, EqSettings, EqState};
use super::dsp::gain::{ReplayGain, RgMode};
use super::output::{Output, RateMode};
use super::ring;
use super::source;
use super::tap::Tap;
use crate::playlist::queue::{Queue, QueueItem, RepeatMode};
use crate::playlist::uri::TrackUri;

const BACKOFF: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug)]
pub enum Command {
    PlayIndex(usize),
    Pause,
    Resume,
    TogglePause,
    Stop,
    Next,
    Prev,
    SeekTo(f64),
    SeekBy(f64),
    Quit,
}

/// What the UI reads. All atomics, so rendering never blocks the audio path.
pub struct PlayerState {
    pub position_frames: AtomicU64,
    pub duration_frames: AtomicU64,
    pub sample_rate: AtomicU64,
    pub channels: AtomicU64,
    pub bit_depth: AtomicU64,
    /// Average bitrate of the current track, kbps. `0` when unknown.
    pub bitrate_kbps: AtomicU64,
    /// Codec short name of the current track, e.g. `flac`, `mp3`, `alac`.
    ///
    /// A string, so not an atomic -- but it changes only at a track boundary
    /// and is read once a frame by the UI, never by the audio callback.
    pub codec: ArcSwap<String>,
    pub underruns: AtomicU64,
    pub playing: AtomicBool,
    pub paused: AtomicBool,
    pub bit_perfect: AtomicBool,
    /// Bumped whenever the current track changes, so the UI can refresh lazily.
    pub track_revision: AtomicU64,
}

impl PlayerState {
    fn new() -> Self {
        Self {
            position_frames: AtomicU64::new(0),
            duration_frames: AtomicU64::new(0),
            sample_rate: AtomicU64::new(0),
            channels: AtomicU64::new(2),
            bit_depth: AtomicU64::new(0),
            bitrate_kbps: AtomicU64::new(0),
            codec: ArcSwap::from(Arc::new(String::new())),
            underruns: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            bit_perfect: AtomicBool::new(false),
            track_revision: AtomicU64::new(0),
        }
    }

    pub fn state(&self) -> PlayState {
        if !self.playing.load(Ordering::Relaxed) {
            PlayState::Stopped
        } else if self.paused.load(Ordering::Relaxed) {
            PlayState::Paused
        } else {
            PlayState::Playing
        }
    }

    pub fn position_secs(&self) -> f64 {
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1);
        self.position_frames.load(Ordering::Relaxed) as f64 / rate as f64
    }

    pub fn duration_secs(&self) -> f64 {
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1);
        self.duration_frames.load(Ordering::Relaxed) as f64 / rate as f64
    }
}

pub struct Player {
    cmds: Sender<Command>,
    pub state: Arc<PlayerState>,
    pub eq: Arc<EqHandle>,
    /// The queue, shared so the UI can render it. Locked only by non-audio code.
    pub queue: Arc<Mutex<Queue>>,
    pub tap: Arc<Tap>,
    /// Latest analyzer output, published so a mirroring instance can draw the
    /// same visualizer without having the audio.
    pub vis_bands: Arc<Mutex<Vec<f32>>>,
    volume: Arc<Mutex<f32>>,
    /// ReplayGain settings: mode, preamp in dB, and whether to pull back from
    /// clipping. Swapped rather than locked, and read at a track boundary.
    rg: Arc<arc_swap::ArcSwap<(RgMode, f32, bool)>>,
    worker: Option<JoinHandle<()>>,
}

impl Player {
    /// A player that owns no audio device.
    ///
    /// Used by an instance mirroring another: it keeps the same state and queue
    /// so every piece of UI code works unchanged, but nothing here opens a
    /// device or decodes anything. State arrives over IPC instead.
    pub fn detached() -> Self {
        let (tx, _rx) = bounded(64);
        Self {
            cmds: tx,
            state: Arc::new(PlayerState::new()),
            eq: Arc::new(EqHandle::new(44_100)),
            queue: Arc::new(Mutex::new(Queue::new())),
            tap: Arc::new(Tap::new(16384)),
            vis_bands: Arc::new(Mutex::new(Vec::new())),
            volume: Arc::new(Mutex::new(1.0)),
            rg: Arc::new(arc_swap::ArcSwap::from_pointee((RgMode::Off, 0.0, true))),
            worker: None,
        }
    }

    /// How loud a track should be relative to the others.
    ///
    /// Published rather than passed in, so it can change while something is
    /// playing. It takes effect at the next track: ReplayGain is constant for
    /// a track by definition, and changing it mid-song would be a level jump.
    pub fn set_replaygain(&self, mode: RgMode, preamp_db: f32, prevent_clipping: bool) {
        self.rg.store(Arc::new((mode, preamp_db, prevent_clipping)));
    }

    pub fn new(library_root: PathBuf) -> Result<Self> {
        let (tx, rx) = bounded(64);
        let state = Arc::new(PlayerState::new());
        let eq = Arc::new(EqHandle::new(44_100));
        let queue = Arc::new(Mutex::new(Queue::new()));
        let volume = Arc::new(Mutex::new(1.0f32));
        // Comfortably more than the largest FFT the visualizer asks for.
        let tap = Arc::new(Tap::new(16384));
        let rg = Arc::new(arc_swap::ArcSwap::from_pointee((RgMode::Off, 0.0f32, true)));

        let worker = {
            let state = Arc::clone(&state);
            let eq = Arc::clone(&eq);
            let queue = Arc::clone(&queue);
            let volume = Arc::clone(&volume);
            let tap = Arc::clone(&tap);
            let rg = Arc::clone(&rg);
            thread::Builder::new()
                .name("staramp-player".into())
                .spawn(move || {
                    run(library_root, rx, state, eq, queue, volume, tap, rg);
                })?
        };

        Ok(Self {
            cmds: tx,
            state,
            eq,
            queue,
            tap,
            vis_bands: Arc::new(Mutex::new(Vec::new())),
            volume,
            rg,
            worker: Some(worker),
        })
    }

    pub fn send(&self, cmd: Command) {
        // A full command queue means the worker is wedged; dropping a keypress
        // beats blocking the UI thread on it.
        let _ = self.cmds.try_send(cmd);
    }

    pub fn set_volume(&self, v: f32) {
        *self.volume.lock().unwrap() = v.clamp(0.0, 1.0);
    }

    pub fn volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    /// Publish analyzer output for mirroring instances.
    pub fn publish_bands(&self, bands: &[f32]) {
        if let Ok(mut b) = self.vis_bands.try_lock() {
            b.clear();
            b.extend_from_slice(bands);
        }
    }

    pub fn set_eq(&self, settings: EqSettings) {
        self.eq.store(settings);
    }

    pub fn current_item(&self) -> Option<QueueItem> {
        self.queue.lock().unwrap().current().cloned()
    }

    pub fn set_queue_tracks(&self, items: Vec<QueueItem>) {
        self.queue.lock().unwrap().set_tracks(items);
    }

    pub fn toggle_shuffle(&self) -> bool {
        // Only protect the current track if there actually is one. Pinning with
        // nothing playing makes every shuffle start on the same track.
        let pin = self.state.state() != PlayState::Stopped;
        self.queue.lock().unwrap().toggle_shuffle_pinning(pin)
    }

    pub fn cycle_repeat(&self) -> RepeatMode {
        self.queue.lock().unwrap().cycle_repeat()
    }

    /// Reshuffle and start playing somewhere new.
    pub fn shuffle_now(&self) -> Option<usize> {
        let landed = self.queue.lock().unwrap().shuffle_now();
        if let Some(i) = landed {
            self.send(Command::PlayIndex(i));
        }
        landed
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.cmds.send(Command::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Everything the worker needs for the currently open output.
struct Stream {
    output: Output,
    producer: rtrb::Producer<f32>,
    source_done: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
    /// False until the ring holds enough to cover the first few callbacks.
    ///
    /// A freshly opened device starts pulling at once, so without this the
    /// first callback of every new stream finds an empty ring and records an
    /// underrun -- an audible click at the start of a track. `engine.rs` blocks
    /// on a prefill for exactly this reason; here the loop does it without
    /// blocking, because this thread is also the one serving commands.
    primed: bool,
    /// When the device was opened, so priming cannot wait forever.
    opened_at: std::time::Instant,
}

/// How long a stream may stay silent waiting to fill.
///
/// A backstop, not the mechanism: the decode loop fills half a ring in a couple
/// of iterations, so this only fires when something has gone wrong -- a file
/// that will not read, a decoder that errors on its first call. Starting a few
/// milliseconds late is a click; never starting is silence, and silence is the
/// worse failure by a distance.
const PRIME_TIMEOUT: Duration = Duration::from_millis(150);

#[allow(clippy::too_many_arguments)]
fn run(
    root: PathBuf,
    rx: Receiver<Command>,
    state: Arc<PlayerState>,
    eq: Arc<EqHandle>,
    queue: Arc<Mutex<Queue>>,
    volume: Arc<Mutex<f32>>,
    tap: Arc<Tap>,
    rg: Arc<arc_swap::ArcSwap<(RgMode, f32, bool)>>,
) {
    let mut stream: Option<Stream> = None;
    // Recomputed when a track opens. Constant for a track by definition, so
    // there is nothing to do between boundaries.
    let mut rg_scalar = 1.0f32;
    // A track's tags resolved against the current settings. Read at the
    // boundary rather than per buffer: the settings can change mid-track, and
    // taking them here is what makes the change land on a track edge instead of
    // stepping the level under the listener.
    let level = |gain: &ReplayGain| {
        let s = rg.load();
        let scalar = gain.scalar(s.0, s.1, s.2);
        // Logged because "is ReplayGain doing anything?" is otherwise
        // unanswerable from outside: a track with no tags and a track the
        // setting is ignoring both sound exactly like no gain at all.
        tracing::debug!("replaygain {:?}: x{scalar:.3}", s.0);
        scalar
    };
    let mut decoder: Option<Box<dyn Decoder>> = None;
    // The album the open decoder is a window onto, when it came from a cue
    // sheet. What makes advancing inside one free.
    let mut cue: Option<source::CueAlbum> = None;
    let mut eq_state = EqState::new(2);
    let mut scratch: Vec<f32> = Vec::new();

    loop {
        // Drain commands first: responsiveness beats a full ring.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Quit => return,
                Command::Stop => {
                    decoder = None;
                    stream = None;
                    state.playing.store(false, Ordering::Relaxed);
                    state.paused.store(false, Ordering::Relaxed);
                    state.position_frames.store(0, Ordering::Relaxed);
                }
                Command::Pause => {
                    if let Some(s) = &stream {
                        s.output.pause();
                    }
                    state.paused.store(true, Ordering::Relaxed);
                }
                Command::Resume => {
                    if let Some(s) = &stream {
                        s.output.resume();
                    }
                    state.paused.store(false, Ordering::Relaxed);
                }
                Command::TogglePause => {
                    let now = !state.paused.load(Ordering::Relaxed);
                    if let Some(s) = &stream {
                        if now {
                            s.output.pause()
                        } else {
                            s.output.resume()
                        }
                    }
                    state.paused.store(now, Ordering::Relaxed);
                }
                Command::PlayIndex(i) => {
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.jump_to(i)
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    if let Some((uri, gain)) = track {
                        rg_scalar = level(&gain);
                        open_track(
                            &root,
                            &uri,
                            &mut decoder,
                            &mut stream,
                            &state,
                            &mut eq_state,
                            &tap,
                            &mut cue,
                        );
                    }
                }
                Command::Next => {
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.next()
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    match track {
                        Some((uri, gain)) => {
                            rg_scalar = level(&gain);
                            open_track(
                                &root,
                                &uri,
                                &mut decoder,
                                &mut stream,
                                &state,
                                &mut eq_state,
                                &tap,
                                &mut cue,
                            )
                        }
                        None => {
                            decoder = None;
                            state.playing.store(false, Ordering::Relaxed);
                        }
                    }
                }
                Command::Prev => {
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.prev()
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    if let Some((uri, gain)) = track {
                        rg_scalar = level(&gain);
                        open_track(
                            &root,
                            &uri,
                            &mut decoder,
                            &mut stream,
                            &state,
                            &mut eq_state,
                            &tap,
                            &mut cue,
                        );
                    }
                }
                Command::SeekTo(secs) => {
                    if let Some(d) = decoder.as_mut() {
                        let rate = d.spec().sample_rate as f64;
                        let frame = (secs.max(0.0) * rate) as u64;
                        if let Ok(landed) = d.seek(frame) {
                            state.position_frames.store(landed, Ordering::Relaxed);
                            // The ring still holds up to `RING_MS` of pre-seek
                            // audio, which is heard before the jump lands.
                            // `drain` does not yet remove it -- see its note.
                            if let Some(s) = &mut stream {
                                drain(&mut s.producer);
                            }
                            eq_state.reset();
                        }
                    }
                }
                Command::SeekBy(delta) => {
                    if let Some(d) = decoder.as_mut() {
                        let rate = d.spec().sample_rate as f64;
                        let cur = d.position() as f64 / rate;
                        let frame = ((cur + delta).max(0.0) * rate) as u64;
                        if let Ok(landed) = d.seek(frame) {
                            state.position_frames.store(landed, Ordering::Relaxed);
                            if let Some(s) = &mut stream {
                                drain(&mut s.producer);
                            }
                            eq_state.reset();
                        }
                    }
                }
            }
        }

        // Start a freshly opened device as soon as it has something to play,
        // or after the deadline whatever happens. Checked here rather than
        // after a successful read so that a decoder erroring on its first call
        // cannot leave the stream paused for ever.
        if let Some(s) = stream.as_mut() {
            if !s.primed {
                let cap = crate::audio::ring::capacity_samples(s.sample_rate, s.channels);
                if s.producer.slots() <= cap / 2 || s.opened_at.elapsed() >= PRIME_TIMEOUT {
                    s.primed = true;
                    // Priming decides *when* it may start, not *whether* it
                    // should. Pausing during a track change and having the
                    // next track start itself anyway is a bug worth naming.
                    if !state.paused.load(Ordering::Relaxed) {
                        s.output.resume();
                    }
                }
            }
        }

        // Then push audio.
        let mut did_work = false;
        if let (Some(s), Some(d)) = (stream.as_mut(), decoder.as_mut()) {
            let ch = s.channels as usize;
            let free = s.producer.slots();
            if free >= ch * 64 {
                let want = (free / ch).min(4096);
                if scratch.len() < want * ch {
                    scratch.resize(want * ch, 0.0);
                }
                match d.read(&mut scratch[..want * ch]) {
                    Ok(0) => {
                        // Track finished. Advance without touching the output.
                        let next = {
                            let mut q = queue.lock().unwrap();
                            q.next()
                                .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                        };
                        match next {
                            Some((uri, gain)) => {
                                // Even the free path through a cue album needs
                                // this: the window moves without an open, and
                                // the level would otherwise stay on the track
                                // before it.
                                rg_scalar = level(&gain);
                                // Advancing inside a cue album is a move of the
                                // window, not an open. The decoder is already
                                // sitting on the first sample of the next
                                // track, and there is at most a ring's worth of
                                // audio left to cover any work done here --
                                // opening properly meant re-reading the sheet
                                // and reopening the file, which was measured at
                                // up to 276 ms against a 200 ms ring.
                                let window = cue
                                    .as_ref()
                                    .and_then(|a| a.window_onto(&root, &uri))
                                    .cloned();
                                let moved = match (&window, decoder.as_mut()) {
                                    (Some(t), Some(d)) => {
                                        d.retarget_slice(t.start_frame, t.end_frame)
                                    }
                                    _ => false,
                                };
                                if moved {
                                    let t = window.expect("moved implies a window");
                                    state
                                        .duration_frames
                                        .store(t.duration_frames().unwrap_or(0), Ordering::Relaxed);
                                    state.position_frames.store(0, Ordering::Relaxed);
                                    state.track_revision.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    open_track(
                                        &root,
                                        &uri,
                                        &mut decoder,
                                        &mut stream,
                                        &state,
                                        &mut eq_state,
                                        &tap,
                                        &mut cue,
                                    );
                                }
                            }
                            None => {
                                s.source_done.store(true, Ordering::Release);
                                decoder = None;
                                state.playing.store(false, Ordering::Relaxed);
                            }
                        }
                        did_work = true;
                    }
                    Ok(frames) => {
                        let settings = eq.load();
                        let vol = *volume.lock().unwrap();
                        let buf = &mut scratch[..frames * ch];
                        eq_state.process(&settings, buf, ch);
                        // One multiply for both: ReplayGain levels the track,
                        // volume is where the listener put the slider.
                        let scale = vol * rg_scalar;
                        if scale != 1.0 {
                            for x in buf.iter_mut() {
                                *x *= scale;
                            }
                        }
                        for &x in buf.iter() {
                            let _ = s.producer.push(x);
                        }
                        state.position_frames.store(d.position(), Ordering::Relaxed);
                        state.underruns.store(
                            s.output.state.underruns.load(Ordering::Relaxed),
                            Ordering::Relaxed,
                        );
                        did_work = true;
                    }
                    Err(e) => {
                        tracing::error!("decode error: {e}");
                        decoder = None;
                        state.playing.store(false, Ordering::Relaxed);
                    }
                }
            }
        }

        if !did_work {
            thread::sleep(BACKOFF);
        }
    }
}

/// Where a flush belongs, and does not yet happen.
///
/// The read cursor belongs to the consumer, which lives in the output callback,
/// so the producer cannot clear the ring on its own. Until the callback learns
/// to drop what it holds, a seek is heard up to `ring::RING_MS` after it is
/// asked for, and the audio from before the jump plays out first.
///
/// Left as a named call rather than deleted so the two seek paths keep pointing
/// at the one place a fix belongs.
fn drain(p: &mut rtrb::Producer<f32>) {
    let _ = p;
}

#[allow(clippy::too_many_arguments)]
fn open_track(
    root: &Path,
    uri: &TrackUri,
    decoder: &mut Option<Box<dyn Decoder>>,
    stream: &mut Option<Stream>,
    state: &Arc<PlayerState>,
    eq_state: &mut EqState,
    tap: &Arc<Tap>,
    cue: &mut Option<source::CueAlbum>,
) {
    let opened = match source::open(root, uri) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("cannot open {uri}: {e}");
            *decoder = None;
            *cue = None;
            return;
        }
    };
    // Kept so the next track of the same album needs no disk.
    *cue = opened.album.clone();
    let spec = opened.decoder.spec();

    // Rebuild the output only when the shape of the audio actually changed.
    // Within an album that is essentially never, so track changes stay gapless.
    let needs_new_stream = stream
        .as_ref()
        .map(|s| needs_rebuild(s.sample_rate, s.channels, &spec))
        .unwrap_or(true);

    if needs_new_stream {
        *stream = None; // release the device before claiming it again
        let (producer, consumer) = ring::create(spec.sample_rate, spec.channels);
        let source_done = Arc::new(AtomicBool::new(false));
        match Output::open(
            spec.sample_rate,
            spec.channels,
            consumer,
            Arc::clone(&source_done),
            Arc::clone(tap),
            // Held silent until the ring has something in it. Asked for at
            // open rather than by pausing straight afterwards: the callback
            // starts running inside `Output::open`, so a pause on the line
            // after it is a race the first callbacks can win, against an empty
            // ring.
            true,
        ) {
            Ok(output) => {
                state
                    .bit_perfect
                    .store(output.rate_mode.is_bit_perfect(), Ordering::Relaxed);
                *stream = Some(Stream {
                    output,
                    producer,
                    source_done,
                    sample_rate: spec.sample_rate,
                    channels: spec.channels,
                    primed: false,
                    opened_at: std::time::Instant::now(),
                });
            }
            Err(e) => {
                tracing::error!("cannot open output: {e}");
                *decoder = None;
                return;
            }
        }
        *eq_state = EqState::new(spec.channels as usize);
    } else {
        eq_state.reset();
    }

    state
        .sample_rate
        .store(spec.sample_rate as u64, Ordering::Relaxed);
    state
        .channels
        .store(spec.channels as u64, Ordering::Relaxed);
    state
        .bit_depth
        .store(spec.bit_depth.unwrap_or(0) as u64, Ordering::Relaxed);
    state.bitrate_kbps.store(
        opened.decoder.bitrate_kbps().unwrap_or(0) as u64,
        Ordering::Relaxed,
    );
    state
        .codec
        .store(Arc::new(opened.decoder.codec().to_string()));
    state.duration_frames.store(
        opened.decoder.total_frames().unwrap_or(0),
        Ordering::Relaxed,
    );
    state.position_frames.store(0, Ordering::Relaxed);
    state.playing.store(true, Ordering::Relaxed);
    state.paused.store(false, Ordering::Relaxed);
    state.track_revision.fetch_add(1, Ordering::Relaxed);

    if let Some(s) = stream.as_ref() {
        s.source_done.store(false, Ordering::Release);
        // A stream still filling stays paused; the push loop starts it once the
        // ring can cover the first callbacks.
        if s.primed {
            s.output.resume();
        }
    }

    *decoder = Some(opened.decoder);
}

/// Does moving to a track with this spec require tearing down the output?
///
/// This is the whole gapless property in one function. The cpal stream is only
/// ever rebuilt for a change in the *shape* of the audio; a track boundary on
/// its own never touches it, so the decode thread just starts filling the same
/// ring from the next decoder and the callback sees an unbroken stream.
///
/// Albums are rate-homogeneous in practice, so within an album this is always
/// false and playback is continuous. Across a boundary where the rate really
/// does change, a rebuild is unavoidable — that is the cost of bit-perfect
/// output, and it is why the plan chose follow-the-file over a fixed rate.
pub fn needs_rebuild(
    current_rate: u32,
    current_channels: u16,
    next: &super::decode::StreamSpec,
) -> bool {
    current_rate != next.sample_rate || current_channels != next.channels
}

/// Convenience for the RateMode the UI displays.
pub fn rate_mode_label(m: RateMode) -> &'static str {
    match m {
        RateMode::Native => "bit-perfect",
        RateMode::Resampled { .. } => "resampled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::StreamSpec;

    fn spec(rate: u32, ch: u16) -> StreamSpec {
        StreamSpec {
            sample_rate: rate,
            channels: ch,
            bit_depth: Some(16),
        }
    }

    #[test]
    fn a_track_boundary_within_an_album_does_not_rebuild_the_output() {
        // The gapless property: same shape, same stream, no interruption.
        assert!(!needs_rebuild(44_100, 2, &spec(44_100, 2)));
        assert!(!needs_rebuild(96_000, 2, &spec(96_000, 2)));
        assert!(!needs_rebuild(192_000, 2, &spec(192_000, 2)));
    }

    #[test]
    fn a_rate_change_does_rebuild() {
        // Unavoidable, and the price of bit-perfect output rather than
        // resampling everything to one fixed rate.
        assert!(needs_rebuild(44_100, 2, &spec(96_000, 2)));
        assert!(needs_rebuild(96_000, 2, &spec(44_100, 2)));
    }

    #[test]
    fn a_channel_count_change_rebuilds() {
        assert!(needs_rebuild(44_100, 2, &spec(44_100, 1)));
    }

    #[test]
    fn bit_depth_alone_does_not_rebuild() {
        // A 16-bit track followed by a 24-bit one at the same rate is still one
        // continuous stream; everything is f32 internally by then.
        let mut s = spec(44_100, 2);
        s.bit_depth = Some(24);
        assert!(!needs_rebuild(44_100, 2, &s));
    }

    #[test]
    fn player_state_reports_the_three_transport_states() {
        let s = PlayerState::new();
        assert_eq!(s.state(), PlayState::Stopped);
        s.playing.store(true, Ordering::Relaxed);
        assert_eq!(s.state(), PlayState::Playing);
        s.paused.store(true, Ordering::Relaxed);
        assert_eq!(s.state(), PlayState::Paused);
    }

    #[test]
    fn position_and_duration_convert_using_the_stream_rate() {
        let s = PlayerState::new();
        s.sample_rate.store(44_100, Ordering::Relaxed);
        s.position_frames.store(44_100 * 30, Ordering::Relaxed);
        s.duration_frames.store(44_100 * 210, Ordering::Relaxed);
        assert!((s.position_secs() - 30.0).abs() < 1e-6);
        assert!((s.duration_secs() - 210.0).abs() < 1e-6);
    }

    #[test]
    fn an_unset_rate_does_not_divide_by_zero() {
        let s = PlayerState::new();
        s.position_frames.store(1000, Ordering::Relaxed);
        assert!(s.position_secs().is_finite());
    }
}
