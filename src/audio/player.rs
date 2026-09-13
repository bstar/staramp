//! The player: a queue, a decode thread, and an output that survives track
//! changes.
//!
//! The output stream is only ever torn down for a sample-rate change or a stop.
//! Track boundaries do not touch it, which is what makes gapless possible at
//! all: the decode thread simply starts filling the same ring from the next
//! decoder, mid-buffer, with no discontinuity for the callback to notice.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};

use super::decode::Decoder;
use super::dsp::eq::{EqHandle, EqState};
use super::dsp::gain::{soft_clip, ReplayGain, RgMode};
use super::output::Output;
use super::ring;
use super::source;
use super::tap::Tap;
use crate::playlist::queue::{Queue, QueueItem, RepeatMode};
use crate::playlist::uri::TrackUri;
use crate::vfs::Vfs;

const BACKOFF: Duration = Duration::from_millis(2);

/// Does the signal need holding back from full scale before the ring?
///
/// Only where something could have pushed it past: an equalizer doing
/// anything at all, or a gain above unity. A transparent chain at or below
/// unity is passed through untouched, because that is the bit-perfect path --
/// the header says so, and a limiter quietly sitting in it would make that a
/// lie for any track that legitimately peaks near full scale.
fn needs_limiting(settings: &super::dsp::eq::EqSettings, scale: f32) -> bool {
    !settings.is_transparent() || scale > 1.0
}

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
    /// The library this player reads from. Held so the UI can ask what kind
    /// of library it is without a second copy of the answer.
    vfs: Arc<Vfs>,
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
    crossfade: Arc<AtomicBool>,
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
            // Nothing here reads a file, but the UI still asks what library
            // it is looking at. An empty local root is the honest answer.
            vfs: Arc::new(Vfs::local_files()),
            cmds: tx,
            state: Arc::new(PlayerState::new()),
            eq: Arc::new(EqHandle::new(44_100)),
            queue: Arc::new(Mutex::new(Queue::new())),
            tap: Arc::new(Tap::new(16384)),
            vis_bands: Arc::new(Mutex::new(Vec::new())),
            volume: Arc::new(Mutex::new(1.0)),
            crossfade: Arc::new(AtomicBool::new(false)),
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

    /// The library this player reads from.
    pub fn vfs(&self) -> &Arc<Vfs> {
        &self.vfs
    }

    pub fn new(vfs: Arc<Vfs>, fixed_rate: Option<u32>, warm_budget: Option<u64>) -> Result<Self> {
        let for_handle = Arc::clone(&vfs);
        let (tx, rx) = bounded(64);
        let state = Arc::new(PlayerState::new());
        let eq = Arc::new(EqHandle::new(44_100));
        let queue = Arc::new(Mutex::new(Queue::new()));
        let volume = Arc::new(Mutex::new(1.0f32));
        let crossfade = Arc::new(AtomicBool::new(false));
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
            let crossfade = Arc::clone(&crossfade);
            thread::Builder::new()
                .name("staramp-player".into())
                .spawn(move || {
                    run(
                        vfs,
                        rx,
                        state,
                        eq,
                        queue,
                        volume,
                        tap,
                        rg,
                        crossfade,
                        fixed_rate,
                        warm_budget,
                    );
                })?
        };

        Ok(Self {
            vfs: for_handle,
            cmds: tx,
            state,
            eq,
            queue,
            tap,
            vis_bands: Arc::new(Mutex::new(Vec::new())),
            volume,
            crossfade,
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

    pub fn set_crossfade(&self, enabled: bool) {
        self.crossfade.store(enabled, Ordering::Relaxed);
    }

    pub fn crossfade(&self) -> bool {
        self.crossfade.load(Ordering::Relaxed)
    }

    /// Publish analyzer output for mirroring instances.
    pub fn publish_bands(&self, bands: &[f32]) {
        if let Ok(mut b) = self.vis_bands.try_lock() {
            b.clear();
            b.extend_from_slice(bands);
        }
    }

    pub fn set_eq_profile(
        &self,
        enabled: bool,
        profile: crate::audio::dsp::apo::Profile,
        sample_rate: u32,
    ) {
        self.eq.store_profile(enabled, profile, sample_rate);
    }

    pub fn current_item(&self) -> Option<QueueItem> {
        self.queue.lock().unwrap().current().cloned()
    }

    pub fn set_queue_tracks(&self, items: Vec<QueueItem>) {
        self.queue.lock().unwrap().set_tracks(items);
    }

    pub fn refresh_queue_tracks(&self, items: Vec<QueueItem>) {
        self.queue.lock().unwrap().refresh_tracks(items);
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

/// A decoder opened and advanced through its first block away from the
/// playback thread. At the boundary the player only has to swap this in; the
/// ordinary producer loop then feeds its already-decoded PCM into the ring.
struct PreparedTrack {
    decoder: Box<dyn Decoder>,
    src_spec: super::decode::StreamSpec,
    album: Option<source::CueAlbum>,
    first_pcm: Vec<f32>,
    first_frames: usize,
}

/// The successor already playing under the tail of the current track. Its
/// decoder is positioned wherever the mix has consumed it to, so handing it
/// over at the boundary is a swap of decoders and nothing else.
struct CrossfadeTrack {
    /// What it was prepared for, checked at the handover: the queue can be
    /// edited during the seconds the fade takes.
    uri: TrackUri,
    decoder: Box<dyn Decoder>,
    src_spec: super::decode::StreamSpec,
    album: Option<source::CueAlbum>,
    gain: f32,
}

/// Gives the normal decode loop the block read during preparation before it
/// asks the underlying decoder for more. Keeping the prepared PCM here rather
/// than pushing it wholesale at the boundary matters: the ring may have room
/// for only one device quantum, and dropping the rest would make the cure less
/// gapless than the original path.
struct PrefixedDecoder {
    inner: Box<dyn Decoder>,
    prefix: Vec<f32>,
    cursor: usize,
    channels: usize,
    base_position: u64,
    emitted_frames: u64,
}

impl PrefixedDecoder {
    fn new(inner: Box<dyn Decoder>, prefix: Vec<f32>, prefix_frames: usize) -> Self {
        let channels = inner.spec().channels as usize;
        let base_position = inner.position().saturating_sub(prefix_frames as u64);
        Self {
            inner,
            prefix,
            cursor: 0,
            channels,
            base_position,
            emitted_frames: 0,
        }
    }
}

impl Decoder for PrefixedDecoder {
    fn spec(&self) -> super::decode::StreamSpec {
        self.inner.spec()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let capacity = out.len() / self.channels * self.channels;
        let from_prefix = (self.prefix.len() - self.cursor).min(capacity);
        out[..from_prefix].copy_from_slice(&self.prefix[self.cursor..self.cursor + from_prefix]);
        self.cursor += from_prefix;

        let prefix_frames = from_prefix / self.channels;
        let frames = if prefix_frames > 0 {
            // Do not ask the inner decoder for more in the same call. If that
            // read failed, the caller would discard the successfully copied
            // prefix along with the error even though our cursor had moved.
            prefix_frames
        } else {
            self.inner.read(&mut out[..capacity])?
        };
        self.emitted_frames += frames as u64;
        Ok(frames)
    }

    fn seek(&mut self, frame: u64) -> Result<u64> {
        self.cursor = self.prefix.len();
        let landed = self.inner.seek(frame)?;
        self.base_position = landed;
        self.emitted_frames = 0;
        Ok(landed)
    }

    fn position(&self) -> u64 {
        self.base_position + self.emitted_frames
    }

    fn total_frames(&self) -> Option<u64> {
        self.inner.total_frames()
    }

    fn retarget_slice(&mut self, start: u64, end: Option<u64>) -> bool {
        self.cursor = self.prefix.len();
        if self.inner.retarget_slice(start, end) {
            self.base_position = 0;
            self.emitted_frames = 0;
            true
        } else {
            false
        }
    }

    fn codec(&self) -> &str {
        self.inner.codec()
    }

    fn bitrate_kbps(&self) -> Option<u32> {
        self.inner.bitrate_kbps()
    }
}

enum AlbumPreload {
    Opening {
        from: TrackUri,
        to: TrackUri,
        result: Receiver<Result<Option<PreparedTrack>>>,
    },
    Ready {
        from: TrackUri,
        to: TrackUri,
        track: PreparedTrack,
    },
}

/// Where the current track stands on the way to its crossfade.
///
/// One value rather than a handful of flags, because the failure this
/// replaced was a flag combination nobody meant: the successor was taken into
/// the mix on one pass and, with nothing left to take, overwritten with `None`
/// on the next, after which a re-spawned preload clamped every read to a
/// boundary already behind the decoder and the ring drained to silence.
enum Crossfade {
    /// Too early, or crossfade is off.
    Idle,
    /// The successor is being opened on a worker thread.
    Opening {
        from: TrackUri,
        to: TrackUri,
        gain: f32,
        result: Receiver<Result<Option<PreparedTrack>>>,
    },
    /// The successor is open and waiting for the fade point. Reads of the
    /// current track stop exactly there so the mix starts on it.
    Ready {
        from: TrackUri,
        to: TrackUri,
        gain: f32,
        track: PreparedTrack,
    },
    /// The successor is playing under the current track. `done` counts the
    /// mixed frames, which is what sets the gain.
    Fading { next: CrossfadeTrack, done: u64 },
    /// The fade point passed with nothing to mix, or the successor would not
    /// open. The track plays out and hands over the ordinary way; nothing is
    /// retried until the track changes, so a failing successor cannot spawn
    /// an opener on every pass.
    Missed,
}

/// How long a stream may stay silent waiting to fill.
///
/// A backstop, not the mechanism: the decode loop fills half a ring in a couple
/// of iterations, so this only fires when something has gone wrong -- a file
/// that will not read, a decoder that errors on its first call. Starting a few
/// milliseconds late is a click; never starting is silence, and silence is the
/// worse failure by a distance.
const PRIME_TIMEOUT: Duration = Duration::from_millis(150);

/// The overlap length when crossfade is enabled.
const CROSSFADE_SECS: u64 = 5;

#[allow(clippy::too_many_arguments)]
fn run(
    vfs: Arc<Vfs>,
    rx: Receiver<Command>,
    state: Arc<PlayerState>,
    eq: Arc<EqHandle>,
    queue: Arc<Mutex<Queue>>,
    volume: Arc<Mutex<f32>>,
    tap: Arc<Tap>,
    rg: Arc<arc_swap::ArcSwap<(RgMode, f32, bool)>>,
    crossfade: Arc<AtomicBool>,
    fixed_rate: Option<u32>,
    // How many bytes of a paused track may be pulled into the page cache,
    // or `None` to leave the disk alone. See [`super::warm`].
    warm_budget: Option<u64>,
) {
    // The decode thread's own read-only handle, as the art worker has its own
    // and for the same reason: a handle already open turns a vanished index
    // into an error rather than a hang, and nothing here can wait on the UI.
    // Best-effort -- without it, cue tracks are opened by reading their sheet.
    //
    // Through the vfs, not `paths::index_file`: for a remote library the local
    // index describes a different library entirely, and cue track boundaries
    // read out of it would be wrong where they were not simply missing.
    let index = vfs
        .index_path()
        .ok()
        .and_then(|p| crate::library::db::Db::open_readonly(&p).ok());

    // The URI whose successor has already been warmed, so it happens once per
    // track rather than on every pass through the loop.
    let mut warmed: Option<String> = None;
    let mut album_preload: Option<AlbumPreload> = None;
    let mut fade = Crossfade::Idle;

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
    // The file the open decoder is reading, which for a cue virtual track is
    // the album image rather than the sheet. Kept so that pausing can pull it
    // into the page cache before the drive it lives on spins down.
    let mut backing: Option<std::path::PathBuf> = None;
    let mut eq_state = EqState::new(2);
    let mut scratch: Vec<f32> = Vec::new();
    let mut crossfade_scratch: Vec<f32> = Vec::new();

    loop {
        // Drain commands first: responsiveness beats a full ring.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Quit => return,
                Command::Stop => {
                    album_preload = None;
                    fade = Crossfade::Idle;
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
                    let _ = warm_paused(&backing, warm_budget);
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
                    if now {
                        let _ = warm_paused(&backing, warm_budget);
                    }
                }
                Command::PlayIndex(i) => {
                    album_preload = None;
                    fade = Crossfade::Idle;
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.jump_to(i)
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    if let Some((uri, gain)) = track {
                        rg_scalar = level(&gain);
                        open_track(
                            &vfs,
                            index.as_ref(),
                            &uri,
                            &mut decoder,
                            &mut stream,
                            &state,
                            &eq,
                            &mut eq_state,
                            &tap,
                            &mut cue,
                            &mut backing,
                            fixed_rate,
                        );
                    }
                }
                Command::Next => {
                    album_preload = None;
                    fade = Crossfade::Idle;
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.next()
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    match track {
                        Some((uri, gain)) => {
                            rg_scalar = level(&gain);
                            open_track(
                                &vfs,
                                index.as_ref(),
                                &uri,
                                &mut decoder,
                                &mut stream,
                                &state,
                                &eq,
                                &mut eq_state,
                                &tap,
                                &mut cue,
                                &mut backing,
                                fixed_rate,
                            )
                        }
                        None => {
                            decoder = None;
                            state.playing.store(false, Ordering::Relaxed);
                        }
                    }
                }
                Command::Prev => {
                    album_preload = None;
                    fade = Crossfade::Idle;
                    let track = {
                        let mut q = queue.lock().unwrap();
                        q.prev()
                            .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                    };
                    if let Some((uri, gain)) = track {
                        rg_scalar = level(&gain);
                        open_track(
                            &vfs,
                            index.as_ref(),
                            &uri,
                            &mut decoder,
                            &mut stream,
                            &state,
                            &eq,
                            &mut eq_state,
                            &tap,
                            &mut cue,
                            &mut backing,
                            fixed_rate,
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
                            // A successor already mixed in has been consumed
                            // past its start; it is reopened if the fade point
                            // comes round again.
                            fade = Crossfade::Idle;
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
                            fade = Crossfade::Idle;
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

        // Move the crossfade along. Only `Idle` and the two preparing states
        // have anything to do here: a fade in progress is driven by the reads
        // below and ends at the outgoing decoder's EOF, and a missed one waits
        // for the track to change.
        if let (Some(s), Some(d)) = (stream.as_ref(), decoder.as_ref()) {
            let fade_frames = CROSSFADE_SECS.saturating_mul(s.sample_rate as u64);
            match (crossfade.load(Ordering::Relaxed), d.total_frames(), &fade) {
                (false, _, Crossfade::Idle | Crossfade::Missed) | (_, None, _) => {}
                // Switched off part-way through the preparation: forget the
                // successor. A fade already audible is left to finish.
                (false, _, Crossfade::Opening { .. } | Crossfade::Ready { .. }) => {
                    fade = Crossfade::Idle;
                }
                (false, ..) => {}
                (true, Some(total), Crossfade::Idle) => {
                    let lead =
                        (CROSSFADE_SECS + WARM_LEAD_SECS).saturating_mul(s.sample_rate as u64);
                    if fade_frames > 0 && d.position().saturating_add(lead) >= total {
                        start_crossfade(&vfs, &queue, s, &level, fixed_rate, &mut fade);
                    }
                }
                (true, Some(total), Crossfade::Opening { .. } | Crossfade::Ready { .. }) => {
                    poll_crossfade(&mut fade);
                    let fade_start = total.saturating_sub(fade_frames);
                    if d.position() >= fade_start {
                        // The reads stop on the boundary while the successor
                        // is ready, so this is normally exact. A late start is
                        // still taken: a fade cut short by a chunk beats none.
                        let edge = {
                            let q = queue.lock().unwrap();
                            let from = q.current().map(|t| t.uri.clone());
                            let to = q
                                .peek_next()
                                .and_then(|i| q.tracks().get(i))
                                .map(|t| t.uri.clone());
                            from.zip(to)
                        };
                        match edge.and_then(|(from, to)| take_crossfade(&mut fade, &from, &to)) {
                            Some(next) => fade = Crossfade::Fading { next, done: 0 },
                            // Not ready, or prepared for an edge the queue no
                            // longer has. Reads carry on unclamped from here,
                            // so the decision is final for this track.
                            None if d.position() > fade_start => fade = Crossfade::Missed,
                            None => {}
                        }
                    }
                }
                (true, Some(_), Crossfade::Fading { .. } | Crossfade::Missed) => {}
            }
        }

        // Then push audio.
        let mut did_work = false;
        if let (Some(s), Some(d)) = (stream.as_mut(), decoder.as_mut()) {
            let ch = s.channels as usize;
            let free = s.producer.slots();
            if free >= ch * 64 {
                let want = (free / ch).min(4096);
                let read_want = match &fade {
                    // Stop exactly on the fade point so the successor's first
                    // sample lands there. Zero is not a stall: the pass above
                    // takes the successor next time round, and if it cannot,
                    // the track is marked missed and the clamp goes away.
                    Crossfade::Ready { .. } => {
                        let total = d.total_frames().unwrap_or(0);
                        let fade_start = total
                            .saturating_sub(CROSSFADE_SECS.saturating_mul(s.sample_rate as u64));
                        if d.position() >= fade_start {
                            want
                        } else {
                            want.min(fade_start.saturating_sub(d.position()) as usize)
                        }
                    }
                    // Everything else reads to the end and lets EOF decide the
                    // handover. The total is only an estimate -- `Adapting`
                    // rounds a resampled length up, and the resampler then
                    // finishes a frame or two short of it -- so a clamp
                    // against it would either stall here or hand over before
                    // the decoder has actually finished.
                    _ => want,
                };
                if read_want == 0 {
                    // On the fade point with the successor ready; taken above
                    // on the next pass.
                } else {
                    if scratch.len() < read_want * ch {
                        scratch.resize(read_want * ch, 0.0);
                    }
                    match d.read(&mut scratch[..read_want * ch]) {
                        Ok(0) if matches!(fade, Crossfade::Fading { .. }) => {
                            // The outgoing track is spent and its successor has
                            // been playing under it for the length of the fade.
                            // The queue advances as at any EOF; the output is
                            // not touched, because the successor's opening is
                            // already in the ring.
                            let Crossfade::Fading { next, .. } =
                                std::mem::replace(&mut fade, Crossfade::Idle)
                            else {
                                unreachable!("guarded by the match arm");
                            };
                            album_preload = None;
                            let advanced = {
                                let mut q = queue.lock().unwrap();
                                q.next()
                                    .and_then(|_| q.current().map(|t| (t.uri.clone(), t.rg)))
                            };
                            match advanced {
                                Some((uri, gain)) if uri == next.uri => {
                                    rg_scalar = level(&gain);
                                    install_crossfaded(next, &mut decoder, s, &state, &mut cue);
                                }
                                Some((uri, gain)) => {
                                    // The queue changed under the fade. What was
                                    // mixed in is not what plays next, so open
                                    // the real successor the ordinary way.
                                    rg_scalar = level(&gain);
                                    open_track(
                                        &vfs,
                                        index.as_ref(),
                                        &uri,
                                        &mut decoder,
                                        &mut stream,
                                        &state,
                                        &eq,
                                        &mut eq_state,
                                        &tap,
                                        &mut cue,
                                        &mut backing,
                                        fixed_rate,
                                    );
                                }
                                None => {
                                    s.source_done.store(true, Ordering::Release);
                                    decoder = None;
                                    state.playing.store(false, Ordering::Relaxed);
                                }
                            }
                            did_work = true;
                        }
                        Ok(0) => {
                            // Track finished. Advance without touching the output.
                            fade = Crossfade::Idle;
                            poll_album_preload(&mut album_preload);
                            let album_edge = {
                                let q = queue.lock().unwrap();
                                q.album_gapless_successor().map(|(from, to)| {
                                    (q.tracks()[from].uri.clone(), q.tracks()[to].uri.clone())
                                })
                            };
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
                                    let window =
                                        cue.as_ref().and_then(|a| a.window_onto(&uri)).cloned();
                                    let moved = match (&window, decoder.as_mut()) {
                                        (Some(t), Some(d)) => {
                                            d.retarget_slice(t.start_frame, t.end_frame)
                                        }
                                        _ => false,
                                    };
                                    if moved {
                                        let t = window.expect("moved implies a window");
                                        state.duration_frames.store(
                                            t.duration_frames().unwrap_or(0),
                                            Ordering::Relaxed,
                                        );
                                        state.position_frames.store(0, Ordering::Relaxed);
                                        state.track_revision.fetch_add(1, Ordering::Relaxed);
                                    } else if let Some(prepared) = album_edge
                                        .as_ref()
                                        .filter(|(_, to)| *to == uri)
                                        .and_then(|(from, to)| {
                                            take_album_preload(&mut album_preload, from, to)
                                        })
                                    {
                                        install_prepared(
                                            prepared,
                                            &mut decoder,
                                            s,
                                            &state,
                                            &mut cue,
                                        );
                                    } else {
                                        album_preload = None;
                                        open_track(
                                            &vfs,
                                            index.as_ref(),
                                            &uri,
                                            &mut decoder,
                                            &mut stream,
                                            &state,
                                            &eq,
                                            &mut eq_state,
                                            &tap,
                                            &mut cue,
                                            &mut backing,
                                            fixed_rate,
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
                            let fading = matches!(fade, Crossfade::Fading { .. });
                            if let Crossfade::Fading { next, done } = &mut fade {
                                if crossfade_scratch.len() < frames * ch {
                                    crossfade_scratch.resize(frames * ch, 0.0);
                                }
                                let next_frames = next
                                    .decoder
                                    .read(&mut crossfade_scratch[..frames * ch])
                                    .unwrap_or(0);
                                let fade_frames = (CROSSFADE_SECS * s.sample_rate as u64).max(1);
                                for frame in 0..frames {
                                    let progress = ((*done + frame as u64 + 1) as f32
                                        / fade_frames as f32)
                                        .min(1.0);
                                    for channel in 0..ch {
                                        let i = frame * ch + channel;
                                        let incoming = if frame < next_frames {
                                            crossfade_scratch[i] * next.gain
                                        } else {
                                            0.0
                                        };
                                        scratch[i] = scratch[i] * rg_scalar * (1.0 - progress)
                                            + incoming * progress;
                                    }
                                }
                                *done += frames as u64;
                            }
                            let settings = eq.load();
                            let vol = *volume.lock().unwrap();
                            let buf = &mut scratch[..frames * ch];
                            eq_state.process(&settings, buf, ch);
                            // One multiply for both: ReplayGain levels the track,
                            // volume is where the listener put the slider.
                            // While fading, the mix above has already applied
                            // each track's own gain.
                            let scale = vol * if fading { 1.0 } else { rg_scalar };
                            if scale != 1.0 {
                                for x in buf.iter_mut() {
                                    *x *= scale;
                                }
                            }
                            // Only where something could have pushed the signal
                            // past full scale: an equalizer that is doing
                            // anything, or a gain above unity. A transparent
                            // chain at or below unity is passed through
                            // untouched, because that is the bit-perfect path
                            // and a limiter in it would be a lie.
                            let limiting = needs_limiting(&settings, scale);
                            for &x in buf.iter() {
                                let _ = s.producer.push(if limiting { soft_clip(x) } else { x });
                            }
                            state.position_frames.store(d.position(), Ordering::Relaxed);
                            state.underruns.store(
                                s.output.state.underruns.load(Ordering::Relaxed),
                                Ordering::Relaxed,
                            );
                            warm_next(&vfs, &queue, d.as_ref(), s.sample_rate, &mut warmed);
                            // The album preload is the gapless path's; a
                            // successor already open for the fade makes it a
                            // second copy of the same file.
                            if !fading {
                                maybe_prepare_album_successor(
                                    &vfs,
                                    &queue,
                                    d.as_ref(),
                                    s,
                                    cue.as_ref(),
                                    fixed_rate,
                                    &mut album_preload,
                                );
                            }
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
        }

        if !did_work {
            thread::sleep(BACKOFF);
        }
    }
}

/// Begin preparing an intentional album successor before the current decoder
/// reaches EOF. This never runs for ordinary playlist order, shuffle, a
/// play-next override, or tracks whose tags do not prove adjacency.
#[allow(clippy::too_many_arguments)]
fn maybe_prepare_album_successor(
    vfs: &Arc<Vfs>,
    queue: &Arc<Mutex<Queue>>,
    decoder: &dyn Decoder,
    stream: &Stream,
    cue: Option<&source::CueAlbum>,
    fixed_rate: Option<u32>,
    preload: &mut Option<AlbumPreload>,
) {
    poll_album_preload(preload);

    let Some(total) = decoder.total_frames() else {
        return;
    };
    let lead = WARM_LEAD_SECS * stream.sample_rate as u64;
    if decoder.position().saturating_add(lead) < total {
        return;
    }

    let edge = {
        let q = queue.lock().unwrap();
        q.album_gapless_successor()
            .map(|(from, to)| (q.tracks()[from].uri.clone(), q.tracks()[to].uri.clone()))
    };
    let Some((from, to)) = edge else {
        *preload = None;
        return;
    };

    // A consecutive CUE window is already sitting at precisely the right PCM
    // frame in the live decoder. Opening a second copy would be slower and
    // less exact than the existing retarget path.
    if cue.and_then(|album| album.window_onto(&to)).is_some() {
        *preload = None;
        return;
    }

    let already_matches = match preload.as_ref() {
        Some(AlbumPreload::Opening {
            from: have_from,
            to: have_to,
            ..
        })
        | Some(AlbumPreload::Ready {
            from: have_from,
            to: have_to,
            ..
        }) => have_from == &from && have_to == &to,
        None => false,
    };
    if already_matches {
        return;
    }

    // Replacing the receiver cancels interest in stale work. The old opener
    // may finish in the background, but cannot perturb playback or be used for
    // a queue edge that no longer exists.
    *preload = None;
    let (tx, result) = bounded(1);
    let worker_vfs = Arc::clone(vfs);
    let worker_to = to.clone();
    let target = super::decode::StreamSpec {
        sample_rate: stream.sample_rate,
        channels: stream.channels,
        bit_depth: None,
    };
    let spawned = thread::Builder::new()
        .name("staramp-album-preload".into())
        .spawn(move || {
            let prepared = prepare_album_track(&worker_vfs, &worker_to, target, fixed_rate);
            let _ = tx.send(prepared);
        });
    match spawned {
        Ok(_) => {
            *preload = Some(AlbumPreload::Opening { from, to, result });
        }
        Err(e) => tracing::warn!("cannot start album preload: {e}"),
    }
}

fn prepare_album_track(
    vfs: &Vfs,
    uri: &TrackUri,
    target: super::decode::StreamSpec,
    _fixed_rate: Option<u32>,
) -> Result<Option<PreparedTrack>> {
    let index = vfs
        .index_path()
        .ok()
        .and_then(|path| crate::library::db::Db::open_readonly(&path).ok());
    let opened = source::open(vfs, index.as_ref(), uri)?;
    let source::OpenedTrack {
        decoder,
        album,
        virtual_track: _,
        backing_path: _,
    } = opened;
    let src_spec = decoder.spec();

    // Crossfade requires both tracks to produce the current device shape. The
    // normal player may reopen the device for a native rate/channel change,
    // but doing that during an overlap would cut off the outgoing track.
    let mut decoder =
        if src_spec.sample_rate == target.sample_rate && src_spec.channels == target.channels {
            decoder
        } else {
            Box::new(super::decode::adapt::Adapting::new(decoder, target)?) as Box<dyn Decoder>
        };

    let channels = decoder.spec().channels as usize;
    let mut first_pcm = vec![0.0; 4096 * channels];
    let first_frames = decoder.read(&mut first_pcm)?;
    first_pcm.truncate(first_frames * channels);
    Ok(Some(PreparedTrack {
        decoder,
        src_spec,
        album,
        first_pcm,
        first_frames,
    }))
}

fn poll_album_preload(preload: &mut Option<AlbumPreload>) {
    let Some(job) = preload.take() else { return };
    match job {
        AlbumPreload::Opening { from, to, result } => match result.try_recv() {
            Ok(Ok(Some(track))) => {
                *preload = Some(AlbumPreload::Ready { from, to, track });
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::warn!("cannot prepare next album track: {e}"),
            Err(TryRecvError::Empty) => {
                *preload = Some(AlbumPreload::Opening { from, to, result });
            }
            Err(TryRecvError::Disconnected) => {}
        },
        ready @ AlbumPreload::Ready { .. } => *preload = Some(ready),
    }
}

fn take_album_preload(
    preload: &mut Option<AlbumPreload>,
    from: &TrackUri,
    to: &TrackUri,
) -> Option<PreparedTrack> {
    poll_album_preload(preload);
    match preload.take() {
        Some(AlbumPreload::Ready {
            from: have_from,
            to: have_to,
            track,
        }) if &have_from == from && &have_to == to => Some(track),
        _ => None,
    }
}

/// Collect the worker's result. A successor that will not open turns the
/// track's crossfade into `Missed`, which is what stops it being retried.
fn poll_crossfade(fade: &mut Crossfade) {
    let Crossfade::Opening { result, .. } = &*fade else {
        return;
    };
    match result.try_recv() {
        Err(TryRecvError::Empty) => {}
        Ok(Ok(Some(track))) => {
            let Crossfade::Opening { from, to, gain, .. } =
                std::mem::replace(fade, Crossfade::Missed)
            else {
                unreachable!("checked above");
            };
            *fade = Crossfade::Ready {
                from,
                to,
                gain,
                track,
            };
        }
        Ok(Ok(None)) | Err(TryRecvError::Disconnected) => *fade = Crossfade::Missed,
        Ok(Err(e)) => {
            tracing::warn!("cannot prepare crossfade track: {e}");
            *fade = Crossfade::Missed;
        }
    }
}

/// Open the queue's successor on a worker thread, converted to the device's
/// shape so it can be mixed straight into the current stream. Called from
/// `Idle` only; `fade` is left `Idle` when there is nothing to prepare.
fn start_crossfade(
    vfs: &Arc<Vfs>,
    queue: &Arc<Mutex<Queue>>,
    stream: &Stream,
    level: &dyn Fn(&ReplayGain) -> f32,
    fixed_rate: Option<u32>,
    fade: &mut Crossfade,
) {
    let (from, to, gain) = {
        let q = queue.lock().unwrap();
        let from = q.current().map(|t| t.uri.clone());
        let to = q.peek_next().and_then(|i| q.tracks().get(i));
        match (from, to) {
            (Some(from), Some(to)) => (from, to.uri.clone(), level(&to.rg)),
            _ => return,
        }
    };
    // Repeat-one: a track cannot fade into itself.
    if to == from {
        return;
    }
    let target = super::decode::StreamSpec {
        sample_rate: stream.sample_rate,
        channels: stream.channels,
        bit_depth: None,
    };
    let worker_vfs = Arc::clone(vfs);
    let worker_to = to.clone();
    let (tx, result) = bounded(1);
    let spawned = thread::Builder::new()
        .name("staramp-crossfade".into())
        .spawn(move || {
            let prepared = prepare_album_track(&worker_vfs, &worker_to, target, fixed_rate);
            let _ = tx.send(prepared);
        });
    match spawned {
        Ok(_) => {
            *fade = Crossfade::Opening {
                from,
                to,
                gain,
                result,
            };
        }
        Err(e) => {
            tracing::warn!("cannot start crossfade preparation: {e}");
            *fade = Crossfade::Missed;
        }
    }
}

/// The ready successor for exactly this edge, or `None`. A successor prepared
/// for an edge the queue no longer has is dropped and the state returned to
/// `Idle`, so the caller decides whether there is time to prepare another.
fn take_crossfade(fade: &mut Crossfade, from: &TrackUri, to: &TrackUri) -> Option<CrossfadeTrack> {
    if !matches!(fade, Crossfade::Ready { .. }) {
        return None;
    }
    let Crossfade::Ready {
        from: have_from,
        to: have_to,
        gain,
        track,
    } = std::mem::replace(fade, Crossfade::Idle)
    else {
        unreachable!("checked above");
    };
    if &have_from != from || &have_to != to {
        return None;
    }
    let PreparedTrack {
        decoder,
        src_spec,
        album,
        first_pcm,
        first_frames,
    } = track;
    Some(CrossfadeTrack {
        uri: have_to,
        decoder: Box::new(PrefixedDecoder::new(decoder, first_pcm, first_frames)),
        src_spec,
        album,
        gain,
    })
}

/// Make the crossfaded successor the current track. Unlike `install_prepared`
/// there is no prefix to replay: the decoder has been read by the mix for the
/// whole fade and simply carries on from where that left it.
fn install_crossfaded(
    next: CrossfadeTrack,
    decoder: &mut Option<Box<dyn Decoder>>,
    stream: &mut Stream,
    state: &Arc<PlayerState>,
    cue: &mut Option<source::CueAlbum>,
) {
    let CrossfadeTrack {
        uri: _,
        decoder: next,
        src_spec,
        album,
        gain: _,
    } = next;
    *cue = album;

    state
        .sample_rate
        .store(src_spec.sample_rate as u64, Ordering::Relaxed);
    state
        .channels
        .store(src_spec.channels as u64, Ordering::Relaxed);
    state
        .bit_depth
        .store(src_spec.bit_depth.unwrap_or(0) as u64, Ordering::Relaxed);
    state
        .bitrate_kbps
        .store(next.bitrate_kbps().unwrap_or(0) as u64, Ordering::Relaxed);
    state.codec.store(Arc::new(next.codec().to_string()));
    state
        .duration_frames
        .store(next.total_frames().unwrap_or(0), Ordering::Relaxed);
    state
        .position_frames
        .store(next.position(), Ordering::Relaxed);
    state.playing.store(true, Ordering::Relaxed);
    state.bit_perfect.store(
        stream.output.bit_perfect
            && src_spec.sample_rate == stream.sample_rate
            && src_spec.channels == stream.channels,
        Ordering::Relaxed,
    );
    state.track_revision.fetch_add(1, Ordering::Relaxed);
    stream.source_done.store(false, Ordering::Release);
    *decoder = Some(next);
}

#[allow(clippy::too_many_arguments)]
fn install_prepared(
    prepared: PreparedTrack,
    decoder: &mut Option<Box<dyn Decoder>>,
    stream: &mut Stream,
    state: &Arc<PlayerState>,
    cue: &mut Option<source::CueAlbum>,
) {
    let PreparedTrack {
        decoder: next,
        src_spec,
        album,
        first_pcm,
        first_frames,
    } = prepared;
    *cue = album;

    let next: Box<dyn Decoder> = Box::new(PrefixedDecoder::new(next, first_pcm, first_frames));

    state
        .sample_rate
        .store(src_spec.sample_rate as u64, Ordering::Relaxed);
    state
        .channels
        .store(src_spec.channels as u64, Ordering::Relaxed);
    state
        .bit_depth
        .store(src_spec.bit_depth.unwrap_or(0) as u64, Ordering::Relaxed);
    state
        .bitrate_kbps
        .store(next.bitrate_kbps().unwrap_or(0) as u64, Ordering::Relaxed);
    state.codec.store(Arc::new(next.codec().to_string()));
    state
        .duration_frames
        .store(next.total_frames().unwrap_or(0), Ordering::Relaxed);
    state
        .position_frames
        .store(next.position(), Ordering::Relaxed);
    state.playing.store(true, Ordering::Relaxed);
    state.bit_perfect.store(
        stream.output.bit_perfect
            && src_spec.sample_rate == stream.sample_rate
            && src_spec.channels == stream.channels,
        Ordering::Relaxed,
    );
    state.track_revision.fetch_add(1, Ordering::Relaxed);
    stream.source_done.store(false, Ordering::Release);
    *decoder = Some(next);
}

/// How far before the end of a track its successor is made ready.
///
/// Only matters for a library that is not on this machine. Locally, opening
/// the next track is a `File::open`; across a link it is several round trips,
/// and the player opens the next track *at* the boundary with a fifth of a
/// second of audio left in the ring. Ten seconds is enough lead for any link
/// worth playing music over, and it costs nothing on a track that is skipped
/// before it gets there.
const WARM_LEAD_SECS: u64 = 10;

/// Ask the library to make the next track ready, once, near the boundary.
/// Pull the paused track's file into the page cache.
///
/// Called on the way into a pause, while the drive is still awake from having
/// been read seconds ago. What resuming would otherwise wait for is a disk
/// that parked itself in the meantime -- the output ring holds well under a
/// second, so the decode thread reaches the platter almost immediately.
///
/// Local libraries only. A remote one is already read behind a window of its
/// own, and `backing` is left empty for it.
/// Returns whether it dispatched, so the gating can be tested without
/// reaching into the page cache to ask what the kernel did with it.
fn warm_paused(backing: &Option<std::path::PathBuf>, budget: Option<u64>) -> bool {
    let (Some(path), Some(max)) = (backing, budget) else {
        return false;
    };
    super::warm::ahead(path.clone(), max);
    true
}

fn warm_next(
    vfs: &Vfs,
    queue: &Arc<Mutex<Queue>>,
    d: &dyn Decoder,
    sample_rate: u32,
    warmed: &mut Option<String>,
) {
    // Nothing to gain locally, and the check is cheaper than the lock.
    if !vfs.is_remote() || sample_rate == 0 {
        return;
    }
    let Some(total) = d.total_frames() else {
        return;
    };
    let lead = WARM_LEAD_SECS * sample_rate as u64;
    if d.position() + lead < total {
        return;
    }

    let next = {
        let q = queue.lock().unwrap();
        q.peek_next()
            .and_then(|i| q.tracks().get(i))
            .map(|t| t.uri.clone())
    };
    let Some(next) = next else { return };
    let key = next.to_string();
    if warmed.as_deref() == Some(key.as_str()) {
        return;
    }
    *warmed = Some(key);
    // `backing_path` rather than the URI: a cue virtual track's bytes are in
    // the audio file the sheet points at, and warming `album.cue/track0007`
    // would ask the far machine for a file that is not the one about to be
    // read. When the next track is another window onto the file already open
    // this warms it a second time, harmlessly -- the read is 64 KiB and the
    // handle closes straight after.
    vfs.warm(next.backing_path());
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
    vfs: &Vfs,
    index: Option<&crate::library::db::Db>,
    uri: &TrackUri,
    decoder: &mut Option<Box<dyn Decoder>>,
    stream: &mut Option<Stream>,
    state: &Arc<PlayerState>,
    eq: &EqHandle,
    eq_state: &mut EqState,
    tap: &Arc<Tap>,
    cue: &mut Option<source::CueAlbum>,
    backing: &mut Option<std::path::PathBuf>,
    fixed_rate: Option<u32>,
) {
    let opened = match source::open(vfs, index, uri) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("cannot open {uri}: {e}");
            *decoder = None;
            *cue = None;
            *backing = None;
            return;
        }
    };
    // Kept so the next track of the same album needs no disk.
    *cue = opened.album.clone();
    // The audio file itself, not the sheet: a cue virtual track is a window
    // onto an album image, and the image is what a spun-down drive makes
    // expensive to reach.
    *backing = (!vfs.is_remote()).then(|| opened.backing_path.clone());
    // What the file holds, which is what the UI reports about the track.
    let src_spec = opened.decoder.spec();

    // What the device will take. Asked before the ring is built, because the
    // ring is sized in the device's frames and a stream the device would not
    // accept has to be converted on the way in.
    let plan = match super::output::plan(&src_spec, fixed_rate) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("cannot choose an output format: {e}");
            *decoder = None;
            return;
        }
    };
    let spec = plan.out_spec(&src_spec);
    eq.rebuild(spec.sample_rate);

    let mut dec = opened.decoder;
    if plan.needs_adapting(&src_spec) {
        tracing::info!(
            "adapting {} Hz {}ch -> {} Hz {}ch for {}",
            src_spec.sample_rate,
            src_spec.channels,
            spec.sample_rate,
            spec.channels,
            plan.device_name
        );
        match super::decode::adapt::Adapting::new(dec, spec) {
            Ok(a) => dec = Box::new(a),
            Err(e) => {
                tracing::error!("cannot adapt the stream to the device: {e}");
                *decoder = None;
                return;
            }
        }
    }

    // Rebuild the output only when the shape of the audio actually changed.
    // Within an album that is essentially never, so track changes stay gapless.
    // Compared against the *device's* shape rather than the file's, so two
    // tracks that differ only in a rate the device was going to convert away
    // now share one stream instead of tearing it down between them.
    let needs_new_stream = stream
        .as_ref()
        .map(|s| needs_rebuild(s.sample_rate, s.channels, &spec))
        .unwrap_or(true);

    if needs_new_stream {
        *stream = None; // release the device before claiming it again
        let (producer, consumer) = ring::create(spec.sample_rate, spec.channels);
        let source_done = Arc::new(AtomicBool::new(false));
        match Output::open(
            plan,
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
                    .store(output.bit_perfect, Ordering::Relaxed);
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

    // The file's own shape, not the device's: this describes the track, and a
    // 48 kHz file is still a 48 kHz file on a device that would only take 44.1.
    // Whether it reached the device untouched is `state.bit_perfect`.
    state
        .sample_rate
        .store(src_spec.sample_rate as u64, Ordering::Relaxed);
    state
        .channels
        .store(src_spec.channels as u64, Ordering::Relaxed);
    state
        .bit_depth
        .store(src_spec.bit_depth.unwrap_or(0) as u64, Ordering::Relaxed);
    state
        .bitrate_kbps
        .store(dec.bitrate_kbps().unwrap_or(0) as u64, Ordering::Relaxed);
    state.codec.store(Arc::new(dec.codec().to_string()));
    // From the adapted decoder, so it is counted in the same frames as the
    // position -- which the callback reports, and the callback only ever sees
    // the device's.
    state
        .duration_frames
        .store(dec.total_frames().unwrap_or(0), Ordering::Relaxed);
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

    *decoder = Some(dec);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decode::StreamSpec;

    struct NumberedDecoder {
        samples: Vec<f32>,
        cursor: usize,
    }

    impl Decoder for NumberedDecoder {
        fn spec(&self) -> StreamSpec {
            spec(44_100, 2)
        }

        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let count = out.len().min(self.samples.len() - self.cursor);
            out[..count].copy_from_slice(&self.samples[self.cursor..self.cursor + count]);
            self.cursor += count;
            Ok(count / 2)
        }

        fn seek(&mut self, frame: u64) -> Result<u64> {
            self.cursor = frame as usize * 2;
            Ok(frame)
        }

        fn position(&self) -> u64 {
            (self.cursor / 2) as u64
        }

        fn total_frames(&self) -> Option<u64> {
            Some((self.samples.len() / 2) as u64)
        }

        fn codec(&self) -> &str {
            "test"
        }

        fn bitrate_kbps(&self) -> Option<u32> {
            None
        }
    }

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
    fn prepared_pcm_is_emitted_once_even_when_the_ring_takes_small_chunks() {
        let samples: Vec<f32> = (0..20).map(|n| n as f32).collect();
        let inner = NumberedDecoder {
            samples: samples.clone(),
            // Preparation already decoded the first four stereo frames.
            cursor: 8,
        };
        let mut decoder = PrefixedDecoder::new(Box::new(inner), samples[..8].to_vec(), 4);
        let mut heard = Vec::new();
        for capacity in [2, 4, 6, 8, 8] {
            let mut out = vec![-1.0; capacity];
            let frames = decoder.read(&mut out).unwrap();
            heard.extend_from_slice(&out[..frames * 2]);
            if frames == 0 {
                break;
            }
        }
        assert_eq!(heard, samples, "the prepared boundary lost or repeated PCM");
        assert_eq!(decoder.position(), 10);
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

    /// The bit-perfect claim in one test: with nothing turned on, nothing
    /// touches the samples on their way to the device.
    #[test]
    fn a_transparent_chain_at_unity_is_never_limited() {
        use crate::audio::dsp::eq::EqSettings;
        let flat = EqSettings::flat(44_100);
        assert!(flat.is_transparent());
        assert!(
            !needs_limiting(&flat, 1.0),
            "the bit-perfect path was limited"
        );
        assert!(!needs_limiting(&flat, 0.5), "attenuation cannot clip");

        // And where something could push past full scale, it is.
        assert!(
            needs_limiting(&flat, 1.5),
            "a gain above unity was not limited"
        );
        let boosted = EqSettings::build(true, 6.0, &[6.0; 10], 44_100);
        assert!(!boosted.is_transparent());
        assert!(
            needs_limiting(&boosted, 1.0),
            "a boosting equalizer was not limited"
        );
    }

    /// What limiting does when it happens: hold the signal inside full scale
    /// without touching anything that was already comfortably inside it.
    #[test]
    fn limiting_holds_full_scale_without_touching_ordinary_levels() {
        use crate::audio::dsp::gain::soft_clip;
        for x in [0.0f32, 0.1, -0.5, 0.89, -0.89] {
            assert_eq!(soft_clip(x), x, "{x} was altered below the knee");
        }
        for x in [1.0f32, 4.0, 1e6, -1e6, -3.0] {
            let y = soft_clip(x);
            assert!(y.abs() <= 1.0, "{x} left the output at {y}");
            assert_eq!(y.signum(), x.signum(), "{x} changed sign");
        }
    }

    /// Pausing is the moment the drive is still awake, so it is the moment to
    /// read. Both halves have to be present: a remote library leaves no
    /// backing path, and the setting can take the budget away.
    #[test]
    fn warming_on_pause_needs_a_local_file_and_a_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.flac");
        std::fs::write(&path, [0u8; 1024]).unwrap();
        let some = Some(path);

        assert!(
            warm_paused(&some, Some(crate::audio::warm::DEFAULT_MAX_BYTES)),
            "a local track with a budget should be warmed"
        );
        assert!(
            !warm_paused(&some, None),
            "warming turned off in the config must reach the disk not at all"
        );
        assert!(
            !warm_paused(&None, Some(crate::audio::warm::DEFAULT_MAX_BYTES)),
            "a remote library has no local file to warm"
        );
        assert!(!warm_paused(&None, None));
    }
}
