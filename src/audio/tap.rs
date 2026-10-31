//! The PCM tap the visualizer reads.
//!
//! Written from the **output callback**, not the decode thread. That
//! distinction is the whole point: the decode thread fills a ring roughly
//! 200 ms ahead of the device, so tapping there showed the analyzer reacting a
//! fifth of a second before the sound arrived. Tapping what is being handed to
//! the device leaves only the device's own buffer, around 20 ms, which is below
//! the threshold where a mismatch is noticeable.
//!
//! The local callback path is lock-free because it is real-time: it may not allocate, block, or
//! take a lock. A single writer and a single reader over a fixed circular
//! buffer of atomics is enough, and sample tearing at the read boundary is
//! explicitly acceptable — a visualizer frame that straddles a write is
//! indistinguishable from one that does not.
//!
//! SSH whole-song caching uses a separate, temporary mono history written by
//! the relay worker. Its reader follows frontend played-frame acknowledgements,
//! so it keeps working after decoding and downloading reach EOF.

use std::io::Write;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct CachedTap {
    epoch: u64,
    file: std::fs::File,
    frames: u64,
}

/// Short feedback gaps are normal over SSH; a stationary device clock for
/// 350 ms means there are no fresh audible samples to visualize.
#[derive(Default)]
pub struct PlaybackClock {
    position: Option<(u64, u64)>,
    advanced: Option<std::time::Instant>,
}
impl PlaybackClock {
    pub fn active(
        &mut self,
        epoch: u64,
        frame: u64,
        playing: bool,
        loading: bool,
        now: std::time::Instant,
    ) -> bool {
        if !playing || loading || frame == 0 {
            self.position = None;
            self.advanced = None;
            return false;
        }
        let position = (epoch, frame);
        if self.position != Some(position) {
            self.position = Some(position);
            self.advanced = Some(now);
        }
        self.advanced
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(350))
    }
}

pub struct Tap {
    cached: Mutex<Option<CachedTap>>,
    /// `f32` bits. `AtomicU32` rather than a lock, so the callback never waits.
    buf: Vec<std::sync::atomic::AtomicU32>,
    /// Total samples ever written; the write position is this modulo capacity.
    written: AtomicUsize,
}

impl Tap {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two().max(2);
        Self {
            cached: Mutex::new(None),
            buf: (0..capacity)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
            written: AtomicUsize::new(0),
        }
    }

    /// Relay-only history. Keep mono visualization samples on disk so greedy
    /// download cannot move the visualizer ahead of the audible device clock.
    /// Neither this lock nor file I/O runs in a hardware audio callback.
    pub fn begin_cache(&self, epoch: u64) {
        *self.cached.lock().unwrap() = tempfile::tempfile().ok().map(|file| CachedTap {
            epoch,
            file,
            frames: 0,
        });
    }

    pub fn write_cached(&self, epoch: u64, interleaved: &[f64], channels: usize) {
        if channels == 0 {
            return;
        }
        let mut cached = self.cached.lock().unwrap();
        let Some(cache) = cached.as_mut().filter(|c| c.epoch == epoch) else {
            return;
        };
        let bytes: Vec<u8> = interleaved
            .chunks_exact(channels)
            .flat_map(|frame| ((frame.iter().sum::<f64>() / channels as f64) as f32).to_le_bytes())
            .collect();
        if cache.file.write_all(&bytes).is_ok() {
            cache.frames += (bytes.len() / 4) as u64;
        }
    }

    pub fn read_cached(&self, epoch: u64, played: u64, out: &mut [f32]) -> bool {
        out.fill(0.0);
        let cached = self.cached.lock().unwrap();
        let Some(cache) = cached.as_ref().filter(|c| c.epoch == epoch) else {
            return false;
        };
        let end = played.min(cache.frames);
        let count = (end as usize).min(out.len());
        if count == 0 {
            return false;
        }
        let mut bytes = vec![0u8; count * 4];
        if cache
            .file
            .read_exact_at(&mut bytes, (end - count as u64) * 4)
            .is_err()
        {
            return false;
        }
        let start = out.len() - count;
        for (slot, bytes) in out[start..].iter_mut().zip(bytes.as_chunks::<4>().0) {
            *slot = f32::from_le_bytes(*bytes);
        }
        true
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Append interleaved audio, downmixed to mono.
    ///
    /// Called from the audio callback: no allocation, no locking, no branching
    /// on anything that could stall.
    /// Downconvert only the visualizer copy, never the playback ring.
    pub fn write_f64(&self, interleaved: &[f64], channels: usize) {
        let mut scratch = [0.0f32; 256];
        for frames in interleaved.chunks(channels * scratch.len()) {
            let count = frames.len() / channels;
            for (out, frame) in scratch.iter_mut().zip(frames.chunks_exact(channels)) {
                *out = (frame.iter().sum::<f64>() / channels as f64) as f32;
            }
            self.write(&scratch[..count], 1);
        }
    }
    pub fn write(&self, interleaved: &[f32], channels: usize) {
        if channels == 0 || self.buf.is_empty() {
            return;
        }
        let mask = self.buf.len() - 1;
        let mut at = self.written.load(Ordering::Relaxed);

        for frame in interleaved.chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.buf[at & mask].store(mono.to_bits(), Ordering::Relaxed);
            at = at.wrapping_add(1);
        }
        // Publish the new position last, so a reader never sees a cursor that
        // points past samples not yet stored.
        self.written.store(at, Ordering::Release);
    }

    /// Copy the most recent `out.len()` samples, oldest first.
    ///
    /// Always succeeds. The previous mutex version could fail to acquire and
    /// leave the visualizer showing a stale frame.
    pub fn read(&self, out: &mut [f32]) -> bool {
        if self.buf.is_empty() || out.is_empty() {
            return false;
        }
        let end = self.written.load(Ordering::Acquire);
        // Nothing has ever been written. A window following another session
        // holds a detached player whose tap no callback feeds, and reporting
        // its bufferful of zeros as a successful read made the analyzer chew
        // on silence -- so the spectrum sat flat while the leader's real bands
        // arrived over the socket and were never used.
        if end == 0 {
            return false;
        }
        let mask = self.buf.len() - 1;
        let n = out.len().min(self.buf.len());

        for (i, slot) in out[..n].iter_mut().enumerate() {
            // Walk back n samples from the write cursor.
            let idx = end.wrapping_sub(n - i) & mask;
            *slot = f32::from_bits(self.buf[idx].load(Ordering::Relaxed));
        }
        for slot in out[n..].iter_mut() {
            *slot = 0.0;
        }
        true
    }

    /// Samples written so far, for tests.
    pub fn written(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_cached_song_visualizes_audible_position_instead_of_download_tail() {
        let tap = Tap::new(8192);
        tap.begin_cache(7);
        let rate = 44100.0;
        let segment = 8192;
        let audio: Vec<f64> = [220.0, 4000.0, 0.0]
            .into_iter()
            .flat_map(|hz| {
                (0..segment).map(move |n| {
                    if hz == 0.0 {
                        0.0
                    } else {
                        0.2 * (std::f64::consts::TAU * hz * n as f64 / rate).sin()
                    }
                })
            })
            .collect();
        // Complete the entire download, including the silent outro, before playback.
        tap.write_cached(7, &audio, 1);
        let mut samples = vec![0.0; 2048];
        let mut peaks = Vec::new();
        for played in [segment, segment * 2] {
            assert!(tap.read_cached(7, played as u64, &mut samples));
            assert!(samples.iter().any(|v| v.abs() > 0.1));
            let mut analyzer = crate::vis::spectrum::Spectrum::new(2048, 32, rate as f32);
            analyzer.analyze(&samples, 0.1);
            let (peak, level) = analyzer
                .bands()
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            assert!(
                *level > 0.1,
                "cached audible tone produced a flat visualizer"
            );
            peaks.push(peak);
        }
        assert!(
            peaks[1] > peaks[0],
            "visualizer did not follow audible frequency changes"
        );
        assert!(tap.read_cached(7, audio.len() as u64, &mut samples));
        assert!(samples.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn downloading_before_playback_does_not_supply_visualizer_samples() {
        let tap = Tap::new(64);
        tap.begin_cache(1);
        tap.write_cached(1, &[0.5; 4096], 2);
        let mut samples = [1.0; 64];
        assert!(!tap.read_cached(1, 0, &mut samples));
        assert_eq!(samples, [0.0; 64]);
        assert!(tap.read_cached(1, 16, &mut samples));
        assert_eq!(&samples[..48], &[0.0; 48]);
        assert_eq!(&samples[48..], &[0.5; 16]);
    }

    #[test]
    fn seeking_or_changing_tracks_cannot_reuse_old_cached_samples() {
        let tap = Tap::new(64);
        tap.begin_cache(1);
        tap.write_cached(1, &[0.5; 128], 1);
        tap.begin_cache(2);
        tap.write_cached(1, &[0.9; 128], 1); // late packet from the old track
        let mut samples = [1.0; 64];
        assert!(!tap.read_cached(1, 64, &mut samples));
        assert!(!tap.read_cached(2, 64, &mut samples));
        tap.write_cached(2, &[-0.25; 128], 1);
        assert!(tap.read_cached(2, 64, &mut samples));
        assert_eq!(samples, [-0.25; 64]);
    }

    #[test]
    fn buffering_paused_and_stopped_playback_do_not_animate() {
        let now = std::time::Instant::now();
        let mut clock = PlaybackClock::default();
        for (frame, playing, loading) in [
            (0, true, true),
            (0, true, false),
            (100, true, true),
            (100, false, false),
        ] {
            assert!(!clock.active(1, frame, playing, loading, now));
        }
        assert!(clock.active(1, 100, true, false, now));
        assert!(!clock.active(1, 100, false, false, now)); // pause
        assert!(clock.active(1, 101, true, false, now)); // resume
    }

    #[test]
    fn stalled_device_clock_stops_animation_but_normal_ssh_feedback_gaps_do_not() {
        use std::time::Duration;
        let now = std::time::Instant::now();
        let mut clock = PlaybackClock::default();
        assert!(clock.active(1, 100, true, false, now));
        assert!(clock.active(1, 100, true, false, now + Duration::from_millis(200)));
        assert!(!clock.active(1, 100, true, false, now + Duration::from_millis(400)));
        assert!(clock.active(1, 200, true, false, now + Duration::from_millis(450)));
    }

    #[test]
    fn reads_back_the_most_recent_samples_in_order() {
        let tap = Tap::new(64);
        let input: Vec<f32> = (0..16).map(|i| i as f32).collect();
        tap.write(&input, 1);

        let mut out = [0.0f32; 16];
        assert!(tap.read(&mut out));
        assert_eq!(out.to_vec(), input, "oldest first, newest last");
    }

    #[test]
    fn downmixes_to_mono() {
        let tap = Tap::new(16);
        // Left 1.0, right -1.0 cancels; left 1.0, right 1.0 stays.
        tap.write(&[1.0, -1.0, 1.0, 1.0], 2);
        let mut out = [0.0f32; 2];
        tap.read(&mut out);
        assert_eq!(out, [0.0, 1.0]);
    }

    #[test]
    fn wraps_around_and_keeps_the_newest() {
        let tap = Tap::new(8);
        let input: Vec<f32> = (0..20).map(|i| i as f32).collect();
        tap.write(&input, 1);

        let mut out = [0.0f32; 8];
        tap.read(&mut out);
        // The last eight written: 12..19.
        assert_eq!(out.to_vec(), (12..20).map(|i| i as f32).collect::<Vec<_>>());
    }

    #[test]
    fn a_read_larger_than_the_buffer_is_zero_padded_not_out_of_bounds() {
        let tap = Tap::new(8);
        tap.write(&[1.0; 8], 1);
        let mut out = [9.0f32; 32];
        assert!(tap.read(&mut out));
        assert!(out[8..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn reading_never_fails_once_there_is_audio_to_read() {
        // The point of this one is that a read cannot fail *spuriously* -- the
        // mutex version could lose the race for the lock and leave a stale
        // frame on screen. There is no lock now, so it cannot.
        //
        // It used to assert that a read before any write succeeds too, which
        // is a different claim and a wrong one: it made a tap nobody writes to
        // indistinguishable from silence.
        let tap = Tap::new(64);
        let mut out = [0.0f32; 16];
        tap.write(&[0.25; 32], 1);
        for _ in 0..100 {
            assert!(tap.read(&mut out), "a read failed with audio available");
        }
    }

    #[test]
    fn capacity_is_rounded_up_to_a_power_of_two() {
        assert_eq!(Tap::new(100).capacity(), 128);
        assert_eq!(Tap::new(1).capacity(), 2);
    }

    #[test]
    fn a_partial_trailing_frame_is_ignored_rather_than_mismixed() {
        let tap = Tap::new(16);
        // Three samples for a stereo stream: the odd one has no pair.
        tap.write(&[1.0, 1.0, 1.0], 2);
        assert_eq!(tap.written(), 1);
    }

    #[test]
    fn zero_channels_does_not_divide_by_zero() {
        let tap = Tap::new(16);
        tap.write(&[1.0, 2.0], 0);
        assert_eq!(tap.written(), 0);
    }

    #[test]
    fn a_tap_nothing_has_written_to_reports_no_audio() {
        // The detached player a following window holds. Answering `true` here
        // hands the analyzer a bufferful of zeros, which looks exactly like
        // silence and leaves the visualizer flat.
        let tap = Tap::new(1024);
        let mut out = [1.0f32; 64];
        assert!(
            !tap.read(&mut out),
            "an unwritten tap claimed to have audio"
        );

        tap.write(&[0.5; 128], 2);
        assert!(tap.read(&mut out), "a written tap should read");
    }
}
