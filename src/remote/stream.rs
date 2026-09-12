//! A remote file, as something a decoder can read and seek.
//!
//! Between the network and the decoder sits a window: a few seconds to a
//! minute of the *compressed* file, held so that a link that hiccups does not
//! become a gap in the music. It is not the same thing as `audio::ring`, and
//! neither can do the other's job -- they are in series:
//!
//! ```text
//! sftp -> window (tens of seconds of file bytes) -> decoder -> ring (200ms of PCM) -> cpal
//!         hides the network                                    hides decode jitter
//! ```
//!
//! # Three regions, not one
//!
//! A forward-only read-ahead is the obvious design and it is wrong for two of
//! the formats this has to play. `avformat_open_input` reads the front of a
//! file, and for an MP4 or M4A with its `moov` atom at the end it then seeks
//! to the very end and back again. With one sliding window that is a full
//! flush and refill on every open. So the first and last 128 KiB are pinned
//! separately and permanently, and the sliding part never notices those seeks
//! happen.
//!
//! A quarter of the window is also kept *behind* the read cursor. FLAC frame
//! resynchronisation and libav's index walking both step backwards by small
//! amounts, and paying a round trip for that would be absurd.
//!
//! # Reads are issued, not awaited
//!
//! `Session::read_at` only writes a request to the pipe; the reply is
//! collected later. So the window issues as many reads as the pipeline allows
//! and then collects only what the caller actually needs, which is what turns
//! one-request-per-round-trip into a stream that keeps up.

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use anyhow::Result;

use super::sftp::session::{Handle, Session, MAX_IN_FLIGHT, READ_LEN};

/// How much of each end of the file is pinned.
pub const PINNED: u64 = 128 * 1024;

/// The smallest and largest sliding window.
///
/// The ceiling is where a time-based target and a byte-based one meet: 45
/// seconds is 1.7 MB of a 300 kbps MP3, 5.6 MB of a 1000 kbps FLAC and 31.5 MB
/// of DSD64, so one flat 32 MiB cap delivers at least that across the whole
/// range a real library holds.
pub const MIN_WINDOW: u64 = 4 * 1024 * 1024;
pub const MAX_WINDOW: u64 = 32 * 1024 * 1024;

/// How much of the window is kept behind the cursor.
const HISTORY_NUMERATOR: u64 = 1;
const HISTORY_DENOMINATOR: u64 = 4;

/// The window size for a track of a given bitrate, if it is known.
pub fn window_for(bitrate_kbps: Option<u32>, secs: u64) -> u64 {
    match bitrate_kbps {
        Some(k) if k > 0 => {
            let want = (k as u64) * 1000 / 8 * secs;
            want.clamp(MIN_WINDOW, MAX_WINDOW)
        }
        _ => MIN_WINDOW,
    }
}

/// One read issued and not yet collected.
struct InFlight {
    id: u32,
    at: u64,
}

/// A file on the far end, buffered.
pub struct RemoteFile {
    session: Arc<Session>,
    handle: Handle,
    path: String,
    len: u64,
    /// Where the next `read` starts.
    pos: u64,
    capacity: u64,

    /// A contiguous run of the file, `[base, base + data.len())`.
    base: u64,
    data: Vec<u8>,
    /// Reads issued but not collected, contiguous and ascending from
    /// `base + data.len()`.
    inflight: VecDeque<InFlight>,
    issued_to: u64,

    /// The pinned ends. Empty until something asks for them.
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl RemoteFile {
    pub fn open(session: Arc<Session>, path: &str, capacity: u64) -> Result<Self> {
        let handle = session.open_file(path)?;
        let len = session.fstat(&handle)?.size.unwrap_or(0);
        Ok(Self {
            session,
            handle,
            path: path.to_string(),
            len,
            pos: 0,
            capacity: capacity.clamp(MIN_WINDOW, MAX_WINDOW),
            base: 0,
            data: Vec::new(),
            inflight: VecDeque::new(),
            issued_to: 0,
            head: Vec::new(),
            tail: Vec::new(),
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Bytes ready to be read without touching the network.
    ///
    /// What the player's decode loop gates on: below a floor it declines to
    /// call the decoder at all, so a stalled link cannot freeze the transport
    /// controls along with the audio.
    pub fn ready(&self) -> u64 {
        let end = self.base + self.data.len() as u64;
        end.saturating_sub(self.pos)
    }

    fn history(&self) -> u64 {
        self.capacity * HISTORY_NUMERATOR / HISTORY_DENOMINATOR
    }

    /// Which pinned region, if either, wholly contains `[at, at + n)`.
    fn pinned(&mut self, at: u64, n: usize) -> Option<&[u8]> {
        let end = at.saturating_add(n as u64);
        if end <= PINNED.min(self.len) {
            if self.head.is_empty() {
                self.head = self.fetch(0, PINNED.min(self.len)).ok()?;
            }
            let from = at as usize;
            return self.head.get(from..(from + n).min(self.head.len()));
        }
        let tail_at = self.len.saturating_sub(PINNED);
        if at >= tail_at && end <= self.len && self.len > 0 {
            if self.tail.is_empty() {
                self.tail = self.fetch(tail_at, self.len - tail_at).ok()?;
            }
            let from = (at - tail_at) as usize;
            return self.tail.get(from..(from + n).min(self.tail.len()));
        }
        None
    }

    /// Read a range, start to finish, waiting for all of it.
    ///
    /// Only used for the pinned ends, which are fetched once per file.
    fn fetch(&self, at: u64, n: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(n as usize);
        let mut off = at;
        while out.len() < n as usize {
            let want = ((n - out.len() as u64).min(READ_LEN as u64)) as u32;
            let id = self.session.read_at(&self.handle, off, want)?;
            match self.session.collect(id)? {
                Some(b) if !b.is_empty() => {
                    off += b.len() as u64;
                    out.extend_from_slice(&b);
                }
                // End of file, or a server that returned nothing: either way
                // there is no more to be had.
                _ => break,
            }
        }
        Ok(out)
    }

    /// Throw away everything buffered and start again at `at`.
    ///
    /// The reads already in flight cannot be cancelled -- SFTP has no such
    /// message -- so they are disowned instead. Their replies still arrive and
    /// are still taken off the pipe, and then dropped.
    fn restart(&mut self, at: u64) {
        for f in self.inflight.drain(..) {
            self.session.forget(f.id);
        }
        self.data.clear();
        self.base = at;
        self.issued_to = at;
    }

    /// Issue reads until the pipeline is as full as it should be.
    ///
    /// Bounded by two things: how many requests may be outstanding, and how
    /// far ahead of the cursor the window reaches. A full window quiesces to
    /// issuing exactly as fast as the decoder consumes.
    fn top_up(&mut self) {
        let want_to = (self.pos + self.capacity).min(self.len);
        while self.inflight.len() < MAX_IN_FLIGHT && self.issued_to < want_to {
            let n = (want_to - self.issued_to).min(READ_LEN as u64) as u32;
            match self.session.read_at(&self.handle, self.issued_to, n) {
                Ok(id) => {
                    self.inflight.push_back(InFlight {
                        id,
                        at: self.issued_to,
                    });
                    self.issued_to += n as u64;
                }
                // The link is gone. `collect` reports it properly; there is
                // nothing useful to do here.
                Err(_) => break,
            }
        }
    }

    /// Collect one outstanding read into the buffer. `false` at end of file.
    fn absorb(&mut self) -> io::Result<bool> {
        let Some(f) = self.inflight.pop_front() else {
            return Ok(false);
        };
        // Replies are collected in the order they were issued, so the bytes
        // always append contiguously.
        debug_assert_eq!(f.at, self.base + self.data.len() as u64);
        match self.session.collect(f.id) {
            Ok(Some(b)) => {
                if b.is_empty() {
                    return Ok(false);
                }
                // A short reply is not the end of the file; the rest of that
                // range simply has not been asked for yet, so re-aim.
                let got = b.len() as u64;
                self.data.extend_from_slice(&b);
                self.session.recycle(b);
                let expected_end = f.at + READ_LEN as u64;
                if f.at + got < expected_end.min(self.len) {
                    // Everything issued after this one starts at the wrong
                    // offset now, so drop it and re-issue from here.
                    self.restart_after_short_read(f.at + got);
                }
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(io::Error::other(e.to_string())),
        }
    }

    /// After a short reply, disown what was issued past it and resume there.
    fn restart_after_short_read(&mut self, resume: u64) {
        for f in self.inflight.drain(..) {
            self.session.forget(f.id);
        }
        self.issued_to = resume;
    }

    /// Drop history beyond what is worth keeping.
    fn trim(&mut self) {
        let keep_from = self.pos.saturating_sub(self.history());
        if keep_from > self.base {
            let drop = (keep_from - self.base) as usize;
            if drop >= self.data.len() {
                self.data.clear();
                self.base = self.pos;
            } else {
                self.data.drain(..drop);
                self.base = keep_from;
            }
        }
    }
}

impl Read for RemoteFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let want = out.len().min((self.len - self.pos) as usize);

        // A pinned end serves the whole read or none of it, and never
        // disturbs the sliding window -- which is the point of pinning them.
        if let Some(b) = self.pinned(self.pos, want) {
            let n = b.len().min(want);
            if n > 0 {
                out[..n].copy_from_slice(&b[..n]);
                self.pos += n as u64;
                return Ok(n);
            }
        }

        // Outside what is buffered, and outside what is on its way: nothing
        // in hand or in flight will ever cover this position, so start again
        // from here and disown what was asked for.
        let end = self.base + self.data.len() as u64;
        let unreachable = self.pos < self.base
            || self.pos > self.issued_to
            || (self.pos > end && self.inflight.is_empty());
        if unreachable {
            self.restart(self.pos);
        }

        self.top_up();

        // Wait only for as much as is actually being asked for.
        while self.base + self.data.len() as u64 <= self.pos {
            if !self.absorb()? {
                if self.inflight.is_empty() {
                    self.top_up();
                    if self.inflight.is_empty() {
                        return Ok(0);
                    }
                    continue;
                }
                return Ok(0);
            }
        }

        let from = (self.pos - self.base) as usize;
        let have = self.data.len() - from;
        let n = have.min(want);
        out[..n].copy_from_slice(&self.data[from..from + n]);
        self.pos += n as u64;
        self.trim();
        self.top_up();
        Ok(n)
    }
}

impl Seek for RemoteFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = match to {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => self.len as i128 + n as i128,
            SeekFrom::Current(n) => self.pos as i128 + n as i128,
        };
        if at < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the file",
            ));
        }
        // Seeking past the end is legal and reads there return nothing.
        self.pos = at as u64;
        Ok(self.pos)
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        for f in self.inflight.drain(..) {
            self.session.forget(f.id);
        }
        self.session.close(&self.handle);
    }
}

impl crate::vfs::RemoteRead for RemoteFile {
    fn len(&self) -> u64 {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::sftp::session::fake;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A file of predictable bytes, so a wrong offset is obvious rather than
    /// plausible.
    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    struct Rig {
        file: RemoteFile,
        oracle: std::io::Cursor<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }

    fn rig(bytes: Vec<u8>, capacity: u64, short: Option<usize>) -> Rig {
        let mut server = fake::Server::new(&[("/f", bytes.clone())]);
        server.short_read = short;
        let reads = Arc::clone(&server.reads);
        let (sr, sw) = std::io::pipe().unwrap();
        let (cr, cw) = std::io::pipe().unwrap();
        std::thread::spawn(move || server.serve(sr, cw));
        let session = Session::over(Box::new(sw), Box::new(cr)).unwrap();
        Rig {
            file: RemoteFile::open(session, "/f", capacity).unwrap(),
            oracle: std::io::Cursor::new(bytes),
            reads,
        }
    }

    #[test]
    fn a_whole_file_reads_back_byte_for_byte() {
        let bytes = body(300_000);
        let mut r = rig(bytes.clone(), MIN_WINDOW, None);
        let mut got = Vec::new();
        r.file.read_to_end(&mut got).unwrap();
        assert_eq!(got, bytes);
    }

    #[test]
    fn the_length_is_what_the_server_says() {
        let r = rig(body(1234), MIN_WINDOW, None);
        assert_eq!(r.file.len(), 1234);
    }

    /// The one that matters. Random reads and seeks, compared against a plain
    /// in-memory cursor doing the same thing -- every offset, every length,
    /// every result. Circular buffers with pinned regions and abandoned reads
    /// in flight are not code anyone should trust by reading it.
    #[test]
    fn random_reads_and_seeks_match_a_plain_cursor() {
        let bytes = body(400_000);
        // A window far smaller than the file, so it is forced to slide,
        // restart and discard constantly rather than just holding everything.
        let mut r = rig(bytes.clone(), MIN_WINDOW, None);

        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for step in 0..3000 {
            // Whatever the operation, the two must end up in the same place.
            assert_eq!(
                r.file.stream_position().unwrap(),
                r.oracle.position(),
                "step {step}: cursors drifted apart"
            );
            match rand() % 10 {
                // Mostly read, as a decoder does.
                0..=6 => {
                    let n = (rand() % 9000 + 1) as usize;
                    let mut a = vec![0u8; n];
                    let at = r.oracle.position();
                    let ra = r.file.read(&mut a).unwrap();

                    // `Read` is explicitly allowed to come back with less
                    // than was asked for, and this one does whenever the
                    // window holds less. What must match is the bytes and
                    // where the cursor ends up -- not the count.
                    let mut b = vec![0u8; ra];
                    r.oracle.read_exact(&mut b).unwrap();
                    assert_eq!(a[..ra], b[..], "step {step}: contents at {at}");

                    // Nothing read means the end of the file and nothing
                    // else. Anything looser would truncate a track silently
                    // whenever the link stuttered.
                    if ra == 0 {
                        assert!(at >= r.file.len(), "step {step}: short of the end");
                    }
                }
                // Jump anywhere, including past the end.
                7 => {
                    let to = rand() % 450_000;
                    assert_eq!(
                        r.file.seek(SeekFrom::Start(to)).unwrap(),
                        r.oracle.seek(SeekFrom::Start(to)).unwrap(),
                        "step {step}"
                    );
                }
                // Backwards, the case the history window exists for.
                8 => {
                    let back = -((rand() % 40_000) as i64);
                    let a = r.file.seek(SeekFrom::Current(back));
                    let b = r.oracle.seek(SeekFrom::Current(back));
                    assert_eq!(a.is_ok(), b.is_ok(), "step {step}");
                    if let (Ok(a), Ok(b)) = (a, b) {
                        assert_eq!(a, b, "step {step}");
                    }
                }
                // From the end, the case the pinned tail exists for.
                _ => {
                    let back = -((rand() % 60_000) as i64);
                    assert_eq!(
                        r.file.seek(SeekFrom::End(back)).unwrap(),
                        r.oracle.seek(SeekFrom::End(back)).unwrap(),
                        "step {step}"
                    );
                }
            }
        }
    }

    /// The same, against a server that never gives back as much as it is
    /// asked for. A short reply re-aims everything issued after it, which is
    /// the fiddliest path in the file.
    #[test]
    fn a_stingy_server_still_yields_the_right_bytes() {
        let bytes = body(200_000);
        let mut r = rig(bytes.clone(), MIN_WINDOW, Some(777));

        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for step in 0..800 {
            if rand() % 4 == 0 {
                let to = rand() % 220_000;
                r.file.seek(SeekFrom::Start(to)).unwrap();
                r.oracle.seek(SeekFrom::Start(to)).unwrap();
            }
            let n = (rand() % 5000 + 1) as usize;
            let mut a = vec![0u8; n];
            let at = r.oracle.position();
            let ra = r.file.read(&mut a).unwrap();
            let mut b = vec![0u8; ra];
            r.oracle.read_exact(&mut b).unwrap();
            assert_eq!(a[..ra], b[..], "step {step} at {at}");
            if ra == 0 {
                assert!(at >= r.file.len(), "step {step}: short of the end");
            }
        }
    }

    /// The MP4 pattern: read the front, jump to the very end for the `moov`
    /// atom, then come back. With one sliding window that is a full flush and
    /// refill every time a track opens.
    #[test]
    fn the_pinned_ends_make_an_mp4_style_open_cost_nothing_twice() {
        let bytes = body(2_000_000);
        let mut r = rig(bytes.clone(), MIN_WINDOW, None);

        let mut buf = vec![0u8; 4096];
        r.file.read_exact(&mut buf).unwrap();
        assert_eq!(buf, bytes[..4096]);

        r.file.seek(SeekFrom::End(-4096)).unwrap();
        let mut tail = vec![0u8; 4096];
        r.file.read_exact(&mut tail).unwrap();
        assert_eq!(tail, bytes[bytes.len() - 4096..]);

        // Both ends are now pinned. Doing it again must touch the network
        // not at all.
        let before = r.reads.load(Ordering::Relaxed);
        for _ in 0..20 {
            r.file.seek(SeekFrom::Start(0)).unwrap();
            r.file.read_exact(&mut buf).unwrap();
            r.file.seek(SeekFrom::End(-4096)).unwrap();
            r.file.read_exact(&mut tail).unwrap();
        }
        assert_eq!(
            r.reads.load(Ordering::Relaxed),
            before,
            "re-reading the pinned ends must not go to the network"
        );
        assert_eq!(buf, bytes[..4096]);
        assert_eq!(tail, bytes[bytes.len() - 4096..]);
    }

    /// Scrubbing inside what is already buffered is the common case, and it
    /// should cost nothing at all.
    #[test]
    fn seeking_inside_the_window_costs_no_network_traffic() {
        let bytes = body(3_000_000);
        let mut r = rig(bytes.clone(), MIN_WINDOW, None);

        // Read well past the pinned head so the sliding window is in use.
        let mut buf = vec![0u8; 600_000];
        r.file.read_exact(&mut buf).unwrap();

        let before = r.reads.load(Ordering::Relaxed);
        let anchor = r.file.stream_position().unwrap();
        for back in [1000u64, 5000, 20_000, 50_000] {
            r.file.seek(SeekFrom::Start(anchor - back)).unwrap();
            let mut b = vec![0u8; 256];
            r.file.read_exact(&mut b).unwrap();
            let at = (anchor - back) as usize;
            assert_eq!(b, bytes[at..at + 256], "reading {back} bytes back");
        }
        assert_eq!(
            r.reads.load(Ordering::Relaxed),
            before,
            "history is kept behind the cursor precisely so this is free"
        );
    }

    #[test]
    fn reading_at_or_past_the_end_yields_nothing() {
        let mut r = rig(body(1000), MIN_WINDOW, None);
        r.file.seek(SeekFrom::Start(1000)).unwrap();
        assert_eq!(r.file.read(&mut [0u8; 64]).unwrap(), 0);
        r.file.seek(SeekFrom::Start(50_000)).unwrap();
        assert_eq!(r.file.read(&mut [0u8; 64]).unwrap(), 0);
    }

    #[test]
    fn seeking_before_the_start_is_refused_rather_than_wrapping() {
        let mut r = rig(body(1000), MIN_WINDOW, None);
        assert!(r.file.seek(SeekFrom::Current(-1)).is_err());
        assert!(r.file.seek(SeekFrom::End(-5000)).is_err());
    }

    #[test]
    fn a_file_smaller_than_the_pinned_regions_still_reads() {
        let bytes = body(64);
        let mut r = rig(bytes.clone(), MIN_WINDOW, None);
        let mut got = Vec::new();
        r.file.read_to_end(&mut got).unwrap();
        assert_eq!(got, bytes);
    }

    #[test]
    fn an_empty_file_is_not_an_error() {
        let mut r = rig(Vec::new(), MIN_WINDOW, None);
        assert_eq!(r.file.len(), 0);
        assert_eq!(r.file.read(&mut [0u8; 16]).unwrap(), 0);
    }

    // -- window sizing ------------------------------------------------------

    #[test]
    fn the_window_is_sized_by_bitrate_within_bounds() {
        // 45s of a 1000 kbps FLAC is about 5.6 MB.
        let flac = window_for(Some(1000), 45);
        assert!((5_000_000..7_000_000).contains(&flac), "{flac}");

        // An MP3 does not need 32 MB, and DSD is capped rather than unbounded.
        assert_eq!(window_for(Some(96), 45), MIN_WINDOW, "floored");
        assert_eq!(window_for(Some(50_000), 45), MAX_WINDOW, "capped");
        assert_eq!(window_for(None, 45), MIN_WINDOW, "unknown bitrate");
        assert_eq!(window_for(Some(0), 45), MIN_WINDOW, "nonsense bitrate");
    }

    /// 45 seconds of DSD64 is about 31.5 MB, which is the whole reason the
    /// ceiling is where it is: one flat cap covers the entire range a real
    /// library holds.
    #[test]
    fn the_ceiling_still_holds_forty_five_seconds_of_dsd() {
        assert!(window_for(Some(5600), 45) <= MAX_WINDOW);
        assert!(window_for(Some(5600), 45) >= 30_000_000);
    }
}
