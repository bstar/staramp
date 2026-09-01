//! SFTP version 3, on the wire.
//!
//! Version 3 and read-only, which together make this small. The protocol has
//! been frozen since 2001, we never write, and nine request types cover
//! everything a player needs: open a file, read a range of it, ask its size,
//! and occasionally list a directory.
//!
//! Nothing here knows about SSH. OpenSSH's own `ssh` binary carries the
//! transport, the authentication, the host keys and the encryption; what
//! arrives on the pipe is already plaintext SFTP.
//!
//! The one performance-critical shape is [`FrameSink::data_dest`]. A `DATA`
//! reply is nine bytes of header and then up to 32 KiB of audio, and the
//! header alone identifies which slice of the read-ahead window those bytes
//! belong in -- so they are read off the pipe straight into their final
//! resting place. Everything else in this module may allocate freely, because
//! everything else happens once per file rather than once per 32 KiB.

use std::io::{self, Read};

/// The version we speak. Higher ones exist and buy us nothing: they add
/// writes, ACLs and richer attributes, none of which a reader wants.
pub const VERSION: u32 = 3;

// Requests.
pub const INIT: u8 = 1;
pub const VERSION_REPLY: u8 = 2;
pub const OPEN: u8 = 3;
pub const CLOSE: u8 = 4;
pub const READ: u8 = 5;
pub const FSTAT: u8 = 8;
pub const OPENDIR: u8 = 11;
pub const READDIR: u8 = 12;
pub const REALPATH: u8 = 16;
pub const STAT: u8 = 17;

// Replies.
pub const STATUS: u8 = 101;
pub const HANDLE: u8 = 102;
pub const DATA: u8 = 103;
pub const NAME: u8 = 104;
pub const ATTRS: u8 = 105;

/// The only open flag we ever set.
pub const FXF_READ: u32 = 0x0000_0001;

// Status codes.
pub const FX_OK: u32 = 0;
pub const FX_EOF: u32 = 1;
pub const FX_NO_SUCH_FILE: u32 = 2;
pub const FX_PERMISSION_DENIED: u32 = 3;

/// Attribute-present bits.
const ATTR_SIZE: u32 = 0x0000_0001;
const ATTR_UIDGID: u32 = 0x0000_0002;
const ATTR_PERMISSIONS: u32 = 0x0000_0004;
const ATTR_ACMODTIME: u32 = 0x0000_0008;
const ATTR_EXTENDED: u32 = 0x8000_0000;

/// The largest packet we will read.
///
/// A desynchronised stream turns whatever four bytes happen to be next into a
/// length, and a player that allocates 3 GB because a byte shifted is a worse
/// failure than one that stops. Real replies are a 32 KiB read plus change;
/// the ceiling is generous enough that only nonsense reaches it.
pub const MAX_PACKET: usize = 4 * 1024 * 1024;

/// A packet being built.
///
/// The four length bytes are written last, over a placeholder, so a caller
/// never has to know the size in advance.
pub struct Packet {
    buf: Vec<u8>,
}

impl Packet {
    pub fn new(kind: u8) -> Self {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&[0, 0, 0, 0]);
        buf.push(kind);
        Self { buf }
    }

    /// A request packet: the type, then its id.
    pub fn request(kind: u8, id: u32) -> Self {
        let mut p = Self::new(kind);
        p.u32(id);
        p
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// An SFTP string: a length and then bytes, with no terminator and no
    /// character set. Paths are handed over exactly as they came out of the
    /// index -- see the byte-for-byte guarantee on `TrackUri`.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.buf.extend_from_slice(b);
        self
    }

    pub fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }

    pub fn finish(mut self) -> Vec<u8> {
        let n = (self.buf.len() - 4) as u32;
        self.buf[..4].copy_from_slice(&n.to_be_bytes());
        self.buf
    }
}

/// Reading fields out of a packet body.
///
/// Every accessor is fallible and none can panic on a short or hostile body,
/// which matters because the far end is a program we did not write.
pub struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }

    pub fn u32(&mut self) -> Option<u32> {
        let end = self.at.checked_add(4)?;
        let v = u32::from_be_bytes(self.b.get(self.at..end)?.try_into().ok()?);
        self.at = end;
        Some(v)
    }

    pub fn u64(&mut self) -> Option<u64> {
        let end = self.at.checked_add(8)?;
        let v = u64::from_be_bytes(self.b.get(self.at..end)?.try_into().ok()?);
        self.at = end;
        Some(v)
    }

    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        let end = self.at.checked_add(n)?;
        let v = self.b.get(self.at..end)?;
        self.at = end;
        Some(v)
    }

    /// A string, lossily. SFTP v3 does not say what character set a filename
    /// is in, and a name we cannot render is still a name we must not drop.
    pub fn string(&mut self) -> Option<String> {
        Some(String::from_utf8_lossy(self.bytes()?).into_owned())
    }

    /// Skip an ATTRS structure without interpreting it.
    pub fn skip_attrs(&mut self) -> Option<()> {
        self.attrs().map(|_| ())
    }

    /// An ATTRS structure. Only `size` and `mtime` are ever load-bearing.
    pub fn attrs(&mut self) -> Option<Attrs> {
        let flags = self.u32()?;
        let mut a = Attrs::default();
        if flags & ATTR_SIZE != 0 {
            a.size = Some(self.u64()?);
        }
        if flags & ATTR_UIDGID != 0 {
            self.u32()?;
            self.u32()?;
        }
        if flags & ATTR_PERMISSIONS != 0 {
            a.permissions = Some(self.u32()?);
        }
        if flags & ATTR_ACMODTIME != 0 {
            self.u32()?;
            a.mtime = Some(self.u32()?);
        }
        if flags & ATTR_EXTENDED != 0 {
            let n = self.u32()?;
            // Bounded by what is left, so a huge count cannot spin.
            for _ in 0..n.min(self.b.len() as u32) {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Some(a)
    }
}

/// What a `STAT` or `FSTAT` told us.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Attrs {
    pub size: Option<u64>,
    pub mtime: Option<u32>,
    pub permissions: Option<u32>,
}

impl Attrs {
    /// True for a directory, by the POSIX mode bits.
    pub fn is_dir(&self) -> bool {
        self.permissions.is_some_and(|p| p & 0o170000 == 0o040000)
    }
}

/// Where an arriving packet goes.
///
/// Split in two so the hot path can avoid a copy: `data_dest` is asked, before
/// the payload is read, for the buffer those bytes belong in.
pub trait FrameSink {
    /// The destination for `len` bytes of `DATA` for request `id`.
    ///
    /// `None` discards the payload, which is how a read issued before a seek
    /// is dropped -- SFTP has no way to cancel a request in flight, so the
    /// reply must still be taken off the pipe and thrown away.
    ///
    /// A slice shorter than `len` is filled and the rest discarded, so a
    /// server answering with more than was asked for cannot overrun anything.
    fn data_dest(&mut self, id: u32, len: u32) -> Option<&mut [u8]>;

    /// The payload named by the last `data_dest` has been read into it.
    ///
    /// Separate from `data_dest` so that the buffer handed out can be the
    /// sink's own scratch and the copy into its final home can happen here,
    /// with no lock held across the read from the pipe. A lock taken around a
    /// blocking read is a lock held for however long the network feels like.
    fn data_done(&mut self, _id: u32, _n: usize) {}

    /// Every other reply, body already in hand.
    fn other(&mut self, kind: u8, body: &[u8]);
}

/// Read exactly one packet, routing it into `sink`.
///
/// `scratch` is reused across calls for non-`DATA` bodies.
pub fn read_frame<R: Read>(
    r: &mut R,
    scratch: &mut Vec<u8>,
    sink: &mut dyn FrameSink,
) -> io::Result<()> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
    let kind = head[4];

    if len == 0 || len > MAX_PACKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("sftp packet of {len} bytes"),
        ));
    }
    // `len` counts the type byte we already have.
    let rest = len - 1;

    if kind != DATA {
        scratch.clear();
        scratch.resize(rest, 0);
        r.read_exact(scratch)?;
        sink.other(kind, scratch);
        return Ok(());
    }

    // DATA: id, then a length-prefixed payload, straight into place.
    if rest < 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated sftp DATA header",
        ));
    }
    let mut dh = [0u8; 8];
    r.read_exact(&mut dh)?;
    let id = u32::from_be_bytes(dh[..4].try_into().unwrap());
    let n = u32::from_be_bytes(dh[4..].try_into().unwrap()) as usize;
    if n != rest - 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("sftp DATA says {n} bytes inside a {rest}-byte packet"),
        ));
    }

    match sink.data_dest(id, n as u32) {
        Some(dest) => {
            let take = dest.len().min(n);
            r.read_exact(&mut dest[..take])?;
            discard(r, n - take)?;
            sink.data_done(id, take);
        }
        None => discard(r, n)?,
    }
    Ok(())
}

/// Read and throw away `n` bytes, in bounded chunks.
fn discard<R: Read>(r: &mut R, n: usize) -> io::Result<()> {
    let mut left = n;
    let mut sink = [0u8; 8192];
    while left > 0 {
        let take = left.min(sink.len());
        r.read_exact(&mut sink[..take])?;
        left -= take;
    }
    Ok(())
}

/// A human-readable name for a status code, for error messages.
pub fn status_name(code: u32) -> &'static str {
    match code {
        FX_OK => "ok",
        FX_EOF => "end of file",
        FX_NO_SUCH_FILE => "no such file",
        FX_PERMISSION_DENIED => "permission denied",
        4 => "failure",
        5 => "bad message",
        6 => "no connection",
        7 => "connection lost",
        8 => "operation unsupported",
        _ => "unknown error",
    }
}

// ---------------------------------------------------------------------------
// Request builders
// ---------------------------------------------------------------------------

pub fn init() -> Vec<u8> {
    let mut p = Packet::new(INIT);
    p.u32(VERSION);
    p.finish()
}

pub fn open_read(id: u32, path: &str) -> Vec<u8> {
    let mut p = Packet::request(OPEN, id);
    p.str(path);
    p.u32(FXF_READ);
    // No attributes: we are not creating anything.
    p.u32(0);
    p.finish()
}

pub fn close(id: u32, handle: &[u8]) -> Vec<u8> {
    let mut p = Packet::request(CLOSE, id);
    p.bytes(handle);
    p.finish()
}

pub fn read(id: u32, handle: &[u8], offset: u64, len: u32) -> Vec<u8> {
    let mut p = Packet::request(READ, id);
    p.bytes(handle);
    p.u64(offset);
    p.u32(len);
    p.finish()
}

pub fn fstat(id: u32, handle: &[u8]) -> Vec<u8> {
    let mut p = Packet::request(FSTAT, id);
    p.bytes(handle);
    p.finish()
}

pub fn stat(id: u32, path: &str) -> Vec<u8> {
    let mut p = Packet::request(STAT, id);
    p.str(path);
    p.finish()
}

pub fn opendir(id: u32, path: &str) -> Vec<u8> {
    let mut p = Packet::request(OPENDIR, id);
    p.str(path);
    p.finish()
}

pub fn readdir(id: u32, handle: &[u8]) -> Vec<u8> {
    let mut p = Packet::request(READDIR, id);
    p.bytes(handle);
    p.finish()
}

pub fn realpath(id: u32, path: &str) -> Vec<u8> {
    let mut p = Packet::request(REALPATH, id);
    p.str(path);
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects whatever arrives, and can be told where DATA should land.
    #[derive(Default)]
    struct Collect {
        dest: Vec<(u32, Vec<u8>)>,
        others: Vec<(u8, Vec<u8>)>,
        /// Ids to refuse, standing in for reads dropped after a seek.
        refuse: Vec<u32>,
    }

    impl FrameSink for Collect {
        fn data_dest(&mut self, id: u32, len: u32) -> Option<&mut [u8]> {
            if self.refuse.contains(&id) {
                return None;
            }
            self.dest.push((id, vec![0u8; len as usize]));
            self.dest.last_mut().map(|(_, b)| b.as_mut_slice())
        }
        fn other(&mut self, kind: u8, body: &[u8]) {
            self.others.push((kind, body.to_vec()));
        }
    }

    fn data_packet(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut p = Packet::request(DATA, id);
        p.bytes(payload);
        p.finish()
    }

    #[test]
    fn a_packet_carries_its_own_length() {
        let bytes = init();
        // 4 length + 1 type + 4 version
        assert_eq!(bytes.len(), 9);
        assert_eq!(u32::from_be_bytes(bytes[..4].try_into().unwrap()), 5);
        assert_eq!(bytes[4], INIT);
        assert_eq!(u32::from_be_bytes(bytes[5..9].try_into().unwrap()), 3);
    }

    #[test]
    fn a_read_request_says_where_and_how_much() {
        let bytes = read(7, b"h", 1 << 40, 32768);
        let mut c = Cursor::new(&bytes[5..]);
        assert_eq!(c.u32(), Some(7), "request id");
        assert_eq!(c.bytes(), Some(&b"h"[..]), "handle");
        assert_eq!(c.u64(), Some(1 << 40), "offset past 4 GB");
        assert_eq!(c.u32(), Some(32768), "length");
    }

    /// The whole reason `FrameSink` is shaped the way it is.
    #[test]
    fn data_lands_in_the_buffer_the_sink_names() {
        let mut sink = Collect::default();
        let packet = data_packet(3, b"audio bytes");
        read_frame(&mut &packet[..], &mut Vec::new(), &mut sink).unwrap();
        assert_eq!(sink.dest, vec![(3, b"audio bytes".to_vec())]);
        assert!(sink.others.is_empty(), "DATA must not go the slow way");
    }

    /// A read issued before a seek: the reply still arrives and must be
    /// consumed off the pipe, or every packet after it is misframed.
    #[test]
    fn a_refused_data_reply_is_drained_not_left_on_the_pipe() {
        let mut sink = Collect::default();
        sink.refuse.push(1);

        let mut stream = data_packet(1, &vec![0xAAu8; 4096]);
        stream.extend_from_slice(&data_packet(2, b"wanted"));

        let mut r = &stream[..];
        let mut scratch = Vec::new();
        read_frame(&mut r, &mut scratch, &mut sink).unwrap();
        read_frame(&mut r, &mut scratch, &mut sink).unwrap();

        assert_eq!(sink.dest, vec![(2, b"wanted".to_vec())]);
        assert!(r.is_empty(), "both packets were consumed");
    }

    /// A server may answer with less than was asked for, and that is not EOF.
    /// It may also, in principle, answer with more; neither may overrun.
    #[test]
    fn a_destination_shorter_than_the_payload_is_filled_and_the_rest_dropped() {
        struct Small(Vec<u8>);
        impl FrameSink for Small {
            fn data_dest(&mut self, _id: u32, _len: u32) -> Option<&mut [u8]> {
                Some(&mut self.0)
            }
            fn other(&mut self, _kind: u8, _body: &[u8]) {}
        }
        let mut sink = Small(vec![0u8; 4]);
        let mut stream = data_packet(1, b"12345678");
        stream.extend_from_slice(&data_packet(2, b"abcd"));

        let mut r = &stream[..];
        let mut scratch = Vec::new();
        read_frame(&mut r, &mut scratch, &mut sink).unwrap();
        assert_eq!(&sink.0, b"1234");
        // The four dropped bytes were consumed, so the next packet still frames.
        read_frame(&mut r, &mut scratch, &mut sink).unwrap();
        assert_eq!(&sink.0, b"abcd");
    }

    #[test]
    fn a_status_reply_comes_back_whole() {
        let mut p = Packet::request(STATUS, 9);
        p.u32(FX_EOF);
        p.str("end of file");
        p.str("en");
        let packet = p.finish();

        let mut sink = Collect::default();
        read_frame(&mut &packet[..], &mut Vec::new(), &mut sink).unwrap();
        let (kind, body) = &sink.others[0];
        assert_eq!(*kind, STATUS);
        let mut c = Cursor::new(body);
        assert_eq!(c.u32(), Some(9));
        assert_eq!(c.u32(), Some(FX_EOF));
    }

    #[test]
    fn attributes_read_only_the_fields_that_are_present() {
        // size and mtime set, nothing else.
        let mut p = Packet::new(ATTRS);
        p.u32(ATTR_SIZE | ATTR_ACMODTIME);
        p.u64(123_456_789);
        p.u32(111); // atime
        p.u32(222); // mtime
        let bytes = p.finish();

        let mut c = Cursor::new(&bytes[5..]);
        let a = c.attrs().unwrap();
        assert_eq!(a.size, Some(123_456_789));
        assert_eq!(a.mtime, Some(222));
        assert_eq!(a.permissions, None);
    }

    #[test]
    fn a_directory_is_recognised_by_its_mode_bits() {
        let dir = Attrs {
            permissions: Some(0o040755),
            ..Default::default()
        };
        let file = Attrs {
            permissions: Some(0o100644),
            ..Default::default()
        };
        assert!(dir.is_dir());
        assert!(!file.is_dir());
        assert!(!Attrs::default().is_dir(), "unknown is not a directory");
    }

    // -- hostile and malformed input --------------------------------------

    #[test]
    fn an_absurd_length_is_refused_rather_than_allocated() {
        let mut packet = (MAX_PACKET as u32 + 1).to_be_bytes().to_vec();
        packet.push(STATUS);
        let err = read_frame(&mut &packet[..], &mut Vec::new(), &mut Collect::default())
            .expect_err("must refuse");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_zero_length_packet_is_refused() {
        let packet = [0u8, 0, 0, 0, STATUS];
        assert!(read_frame(&mut &packet[..], &mut Vec::new(), &mut Collect::default()).is_err());
    }

    #[test]
    fn a_data_packet_that_lies_about_its_payload_is_refused() {
        // Header says 99 bytes; the packet holds 4.
        let mut body = 1u32.to_be_bytes().to_vec(); // id
        body.extend_from_slice(&99u32.to_be_bytes()); // claimed length
        body.extend_from_slice(b"abcd");
        let mut packet = ((body.len() + 1) as u32).to_be_bytes().to_vec();
        packet.push(DATA);
        packet.extend_from_slice(&body);

        assert!(read_frame(&mut &packet[..], &mut Vec::new(), &mut Collect::default()).is_err());
    }

    #[test]
    fn a_truncated_body_is_an_error_not_a_panic() {
        let mut packet = 64u32.to_be_bytes().to_vec();
        packet.push(STATUS);
        packet.extend_from_slice(b"only a few bytes");
        assert!(read_frame(&mut &packet[..], &mut Vec::new(), &mut Collect::default()).is_err());
    }

    /// Every accessor must fail rather than panic on a short body, because the
    /// far end is a program we did not write.
    #[test]
    fn reading_past_the_end_returns_none() {
        let mut c = Cursor::new(&[0, 0, 0]);
        assert_eq!(c.u32(), None);
        let mut c = Cursor::new(&[0, 0, 0, 8, 1]);
        assert_eq!(c.bytes(), None, "a length longer than the body");
        let mut c = Cursor::new(&[]);
        assert_eq!(c.attrs(), None);
    }

    /// A random byte stream must never panic the parser, whatever it says.
    #[test]
    fn arbitrary_bytes_never_panic_the_parser() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..2000 {
            let n = (rand() % 64) as usize;
            let body: Vec<u8> = (0..n).map(|_| rand() as u8).collect();
            let mut c = Cursor::new(&body);
            let _ = c.u32();
            let _ = c.bytes();
            let _ = c.attrs();
            let _ = c.string();

            let mut framed = ((body.len() + 1) as u32).to_be_bytes().to_vec();
            framed.push((rand() % 256) as u8);
            framed.extend_from_slice(&body);
            let _ = read_frame(&mut &framed[..], &mut Vec::new(), &mut Collect::default());
        }
    }
}
