//! The PCM tap the visualizer reads.
//!
//! Written from the **output callback**, not the decode thread. That
//! distinction is the whole point: the decode thread fills a ring roughly
//! 200 ms ahead of the device, so tapping there showed the analyzer reacting a
//! fifth of a second before the sound arrived. Tapping what is being handed to
//! the device leaves only the device's own buffer, around 20 ms, which is below
//! the threshold where a mismatch is noticeable.
//!
//! Lock-free because the callback is real-time: it may not allocate, block, or
//! take a lock. A single writer and a single reader over a fixed circular
//! buffer of atomics is enough, and sample tearing at the read boundary is
//! explicitly acceptable — a visualizer frame that straddles a write is
//! indistinguishable from one that does not.

use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Tap {
    /// `f32` bits. `AtomicU32` rather than a lock, so the callback never waits.
    buf: Vec<std::sync::atomic::AtomicU32>,
    /// Total samples ever written; the write position is this modulo capacity.
    written: AtomicUsize,
}

impl Tap {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two().max(2);
        Self {
            buf: (0..capacity)
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
            written: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Append interleaved audio, downmixed to mono.
    ///
    /// Called from the audio callback: no allocation, no locking, no branching
    /// on anything that could stall.
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
