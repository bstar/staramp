//! Playing one virtual track out of a larger file.
//!
//! A cue virtual track is a `[start, end)` window over a backing file. Wrapping
//! the backing decoder rather than teaching every layer about cue sheets means
//! seek, progress, MPRIS `SetPosition` and the progress bar all work in
//! track-relative frames and need no cue awareness at all.

use anyhow::Result;

use super::{Decoder, StreamSpec};

pub struct SliceDecoder {
    inner: Box<dyn Decoder>,
    start: u64,
    /// Exclusive. `None` means "to end of file", which is the last track of
    /// every `FILE` stanza.
    end: Option<u64>,
    /// True once the decoder has been positioned at `start`.
    positioned: bool,
}

impl SliceDecoder {
    pub fn new(inner: Box<dyn Decoder>, start: u64, end: Option<u64>) -> Self {
        Self {
            inner,
            start,
            end,
            positioned: false,
        }
    }

    /// Retarget the window without rebuilding anything.
    ///
    /// This is what makes a cue album gapless *and* cheap: advancing from one
    /// virtual track to the next in the same backing file opens no file, seeks
    /// nowhere, and re-reads no seek table. The album becomes a single linear
    /// read, which is both perfectly gapless and the fastest possible access
    /// pattern — and for the libav-backed cue albums it means one decoder for the
    /// whole disc instead of one per track.
    pub fn retarget(&mut self, start: u64, end: Option<u64>) {
        self.start = start;
        self.end = end;
        // Already at the right place if the previous track ended exactly here.
        self.positioned = self.inner.position() == start;
    }

    fn ensure_positioned(&mut self) -> Result<()> {
        if !self.positioned {
            self.inner.seek(self.start)?;
            self.positioned = true;
        }
        Ok(())
    }

    /// Frames remaining in this track, if it has an end.
    fn remaining(&self) -> Option<u64> {
        self.end.map(|e| e.saturating_sub(self.inner.position()))
    }
}

impl Decoder for SliceDecoder {
    fn retarget_slice(&mut self, start: u64, end: Option<u64>) -> bool {
        self.retarget(start, end);
        true
    }

    fn spec(&self) -> StreamSpec {
        self.inner.spec()
    }

    fn codec(&self) -> &str {
        self.inner.codec()
    }

    /// The backing file's bitrate. A cue track is a window onto that file, so
    /// its own byte count is not separable -- and for a constant-quality
    /// encode the file average is the right answer anyway.
    fn bitrate_kbps(&self) -> Option<u32> {
        self.inner.bitrate_kbps()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        self.ensure_positioned()?;

        let ch = self.spec().samples_per_frame();
        if ch == 0 {
            return Ok(0);
        }

        // Never read past the track boundary, or the next track's audio bleeds
        // into this one.
        let mut want = out.len() / ch;
        if let Some(left) = self.remaining() {
            if left == 0 {
                return Ok(0);
            }
            want = want.min(left as usize);
        }
        if want == 0 {
            return Ok(0);
        }

        self.inner.read(&mut out[..want * ch])
    }

    /// Seek within the track. `frame` is relative to the track's start.
    fn seek(&mut self, frame: u64) -> Result<u64> {
        let target = self.start.saturating_add(frame);
        let clamped = match self.end {
            Some(e) => target.min(e),
            None => target,
        };
        let landed = self.inner.seek(clamped)?;
        self.positioned = true;
        Ok(landed.saturating_sub(self.start))
    }

    fn position(&self) -> u64 {
        self.inner.position().saturating_sub(self.start)
    }

    fn total_frames(&self) -> Option<u64> {
        match self.end {
            Some(e) => Some(e.saturating_sub(self.start)),
            // Unbounded track: whatever is left of the backing file.
            None => self
                .inner
                .total_frames()
                .map(|t| t.saturating_sub(self.start)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A decoder over a synthetic ramp, so every frame is identifiable.
    struct Ramp {
        pos: u64,
        len: u64,
    }

    impl Decoder for Ramp {
        fn spec(&self) -> StreamSpec {
            StreamSpec {
                sample_rate: 44_100,
                channels: 1,
                bit_depth: Some(16),
            }
        }
        fn codec(&self) -> &str {
            "ramp"
        }
        fn bitrate_kbps(&self) -> Option<u32> {
            None
        }
        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let n = out.len().min((self.len - self.pos) as usize);
            for (i, slot) in out[..n].iter_mut().enumerate() {
                *slot = (self.pos + i as u64) as f32;
            }
            self.pos += n as u64;
            Ok(n)
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

    fn ramp(len: u64) -> Box<dyn Decoder> {
        Box::new(Ramp { pos: 0, len })
    }

    #[test]
    fn reads_only_within_the_window() {
        let mut s = SliceDecoder::new(ramp(1000), 100, Some(150));
        let mut buf = [0f32; 200];
        let n = s.read(&mut buf).unwrap();
        assert_eq!(n, 50, "must stop at the track boundary");
        assert_eq!(buf[0], 100.0, "must start at the window start");
        assert_eq!(buf[49], 149.0);
        assert_eq!(s.read(&mut buf).unwrap(), 0, "then report end of track");
    }

    #[test]
    fn an_unbounded_track_runs_to_end_of_file() {
        let mut s = SliceDecoder::new(ramp(120), 100, None);
        let mut buf = [0f32; 200];
        assert_eq!(s.read(&mut buf).unwrap(), 20);
        assert_eq!(buf[0], 100.0);
    }

    #[test]
    fn positions_and_seeks_are_track_relative() {
        let mut s = SliceDecoder::new(ramp(1000), 300, Some(400));
        assert_eq!(s.position(), 0, "track starts at zero, not at 300");
        assert_eq!(s.total_frames(), Some(100));

        assert_eq!(s.seek(25).unwrap(), 25);
        assert_eq!(s.position(), 25);

        let mut buf = [0f32; 10];
        s.read(&mut buf).unwrap();
        assert_eq!(buf[0], 325.0, "but the underlying read is absolute");
    }

    #[test]
    fn seeking_past_the_end_clamps_into_the_track() {
        let mut s = SliceDecoder::new(ramp(1000), 300, Some(400));
        assert_eq!(s.seek(9999).unwrap(), 100);
        let mut buf = [0f32; 10];
        assert_eq!(s.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn retarget_to_an_adjacent_track_does_not_seek() {
        // The gapless fast path: consecutive cue tracks in one file.
        let mut s = SliceDecoder::new(ramp(1000), 0, Some(50));
        let mut buf = [0f32; 50];
        assert_eq!(s.read(&mut buf).unwrap(), 50);

        s.retarget(50, Some(100));
        assert!(s.positioned, "already at frame 50, so no seek is needed");
        let n = s.read(&mut buf).unwrap();
        assert_eq!(n, 50);
        assert_eq!(buf[0], 50.0, "continues straight on with no gap");
    }

    #[test]
    fn retarget_to_a_track_elsewhere_in_the_file_seeks_to_it() {
        // The same fast path with shuffle on, or with repeat-one: the next
        // track is in this file but not the next one along, so the window has
        // to be moved *and* the decoder repositioned. Still no file opened.
        let mut s = SliceDecoder::new(ramp(1000), 0, Some(50));
        let mut buf = [0f32; 50];
        assert_eq!(s.read(&mut buf).unwrap(), 50);

        s.retarget(700, Some(750));
        assert!(!s.positioned, "frame 700 is not where we left off");
        assert_eq!(s.read(&mut buf).unwrap(), 50);
        assert_eq!(buf[0], 700.0, "landed on the requested track");
        assert_eq!(s.position(), 50, "and reports its own position");
        assert_eq!(s.total_frames(), Some(50));
    }

    #[test]
    fn retarget_back_to_the_same_window_replays_it() {
        // Repeat-one on a cue track: `next` hands back the track just played.
        let mut s = SliceDecoder::new(ramp(1000), 100, Some(150));
        let mut buf = [0f32; 50];
        assert_eq!(s.read(&mut buf).unwrap(), 50);
        assert_eq!(s.read(&mut buf).unwrap(), 0, "the track is done");

        s.retarget(100, Some(150));
        assert_eq!(s.read(&mut buf).unwrap(), 50, "it plays again");
        assert_eq!(buf[0], 100.0);
    }
}
