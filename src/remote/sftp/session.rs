//! One SFTP conversation, over one multiplexed `ssh` channel.
//!
//! Two threads and a table. The writer serialises requests onto the pipe; the
//! reader takes replies off it and files them by request id; callers wait on a
//! condition variable for their own id. Requests are pipelined -- up to
//! [`MAX_IN_FLIGHT`] of them outstanding -- because one request per round trip
//! over a link with any latency is far below what the audio needs.
//!
//! # Why a reply may arrive for nobody
//!
//! SFTP has no way to cancel a request. A seek makes every read already in
//! flight useless, but their replies are still coming and still have to be
//! taken off the pipe in order, or every packet after them is misframed. So a
//! reply whose slot has been abandoned is read and dropped rather than
//! refused -- see `Reads::forget`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use super::wire::{self, FrameSink};

/// How many requests may be outstanding at once.
///
/// Sixty-four is not a guess. An SSH session channel opens with a 2 MiB window
/// made of 32 KiB packets, so 64 reads of [`READ_LEN`] is exactly what can be
/// in flight before the far end has to wait for a window adjustment. It is
/// also what OpenSSH's own `sftp` uses, for the same reason.
pub const MAX_IN_FLIGHT: usize = 64;

/// How much to ask for in one read.
///
/// Matches the SSH channel's own packet size, so one SFTP reply is one SSH
/// packet with no fragmentation. Bulk-copy tools negotiate far larger reads,
/// which is right for throughput and wrong here: a bigger read coarsens what a
/// seek throws away and delays the first byte.
pub const READ_LEN: u32 = 32 * 1024;

/// How long a caller waits for one reply before calling the link dead.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// A reply, filed by request id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Handle(Vec<u8>),
    Attrs(wire::Attrs),
    Names(Vec<(String, wire::Attrs)>),
    Status(u32),
}

impl Reply {
    /// Turn a status reply into an error, unless it is success.
    fn ok(self, what: &str) -> Result<Self> {
        match self {
            Reply::Status(c) if c != wire::FX_OK => {
                Err(anyhow!("{what}: {}", wire::status_name(c)))
            }
            other => Ok(other),
        }
    }
}

/// What became of one read.
#[derive(Debug)]
enum ReadState {
    Waiting,
    Done(Vec<u8>),
    /// The file ended before this offset.
    Eof,
    Failed(String),
}

#[derive(Default)]
struct Reads {
    slots: HashMap<u32, ReadState>,
    /// Buffers to reuse. Reads are all the same size and there are at most
    /// `MAX_IN_FLIGHT` of them, so this stays small and stops the read path
    /// allocating two megabytes a second.
    spare: Vec<Vec<u8>>,
}

struct Shared {
    next_id: AtomicU32,
    replies: Mutex<HashMap<u32, Option<Reply>>>,
    reply_ready: Condvar,
    reads: Mutex<Reads>,
    read_ready: Condvar,
    alive: AtomicBool,
    /// Why the session ended, if it did.
    fault: Mutex<Option<String>>,
}

impl Shared {
    fn die(&self, why: String) {
        if self.alive.swap(false, Ordering::Release) {
            tracing::debug!("sftp session ended: {why}");
            *self.fault.lock().unwrap() = Some(why);
        }
        // Wake everyone waiting on a reply that is never coming.
        self.reply_ready.notify_all();
        self.read_ready.notify_all();
    }

    fn fault(&self) -> String {
        self.fault
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "the connection ended".into())
    }
}

/// Routes arriving packets into the tables.
struct Router {
    shared: Arc<Shared>,
    /// Where a `DATA` payload is read before being filed. Owned here rather
    /// than reached for under a lock, so nothing is locked across a read.
    scratch: Vec<u8>,
}

impl FrameSink for Router {
    fn data_dest(&mut self, id: u32, len: u32) -> Option<&mut [u8]> {
        // Only check that somebody still wants it. The lock is released
        // before the payload is read.
        let wanted = matches!(
            self.shared.reads.lock().unwrap().slots.get(&id),
            Some(ReadState::Waiting)
        );
        if !wanted {
            return None;
        }
        let n = (len as usize).min(READ_LEN as usize * 2);
        self.scratch.clear();
        self.scratch.resize(n, 0);
        Some(&mut self.scratch)
    }

    fn data_done(&mut self, id: u32, n: usize) {
        let mut reads = self.shared.reads.lock().unwrap();
        let mut buf = reads.spare.pop().unwrap_or_default();
        buf.clear();
        buf.extend_from_slice(&self.scratch[..n]);
        if let Some(slot) = reads.slots.get_mut(&id) {
            *slot = ReadState::Done(buf);
        }
        drop(reads);
        self.shared.read_ready.notify_all();
    }

    fn other(&mut self, kind: u8, body: &[u8]) {
        let mut c = wire::Cursor::new(body);
        let Some(id) = c.u32() else { return };

        let reply = match kind {
            wire::HANDLE => c.bytes().map(|h| Reply::Handle(h.to_vec())),
            wire::ATTRS => c.attrs().map(Reply::Attrs),
            wire::STATUS => c.u32().map(Reply::Status),
            wire::NAME => {
                let mut names = Vec::new();
                let n = c.u32().unwrap_or(0);
                for _ in 0..n {
                    let Some(name) = c.string() else { break };
                    // The "longname" -- an ls -l line, which nothing needs.
                    if c.bytes().is_none() {
                        break;
                    }
                    let Some(a) = c.attrs() else { break };
                    names.push((name, a));
                }
                Some(Reply::Names(names))
            }
            wire::VERSION_REPLY => return,
            _ => None,
        };

        // A status for a read is that read's answer, not a control reply.
        if let Some(Reply::Status(code)) = &reply {
            let mut reads = self.shared.reads.lock().unwrap();
            if let Some(slot) = reads.slots.get_mut(&id) {
                *slot = if *code == wire::FX_EOF {
                    ReadState::Eof
                } else {
                    ReadState::Failed(wire::status_name(*code).to_string())
                };
                drop(reads);
                self.shared.read_ready.notify_all();
                return;
            }
        }

        let mut replies = self.shared.replies.lock().unwrap();
        if let Some(slot) = replies.get_mut(&id) {
            *slot = reply;
        }
        drop(replies);
        self.shared.reply_ready.notify_all();
    }
}

/// A live SFTP conversation.
pub struct Session {
    shared: Arc<Shared>,
    out: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Option<Child>>,
}

impl Session {
    /// Open an SFTP channel over an existing `ssh` master.
    pub fn open(host: &str, ctl: &std::path::Path) -> Result<Arc<Self>> {
        let mut child = Command::new("ssh")
            .args(super::super::ssh::slave_argv(host, ctl))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Never inherited: a diagnostic printed onto the alternate screen
            // corrupts the display, and there is nowhere useful to show it.
            .stderr(Stdio::null())
            .spawn()
            .context("opening an sftp channel")?;

        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let session = Self::over(Box::new(stdin), Box::new(stdout))?;
        *session.child.lock().unwrap() = Some(child);
        Ok(session)
    }

    /// Speak SFTP over an arbitrary pair of streams.
    ///
    /// The transport is a parameter so the protocol can be tested against a
    /// scripted server rather than a real machine: everything below this line
    /// behaves identically whether the far end is `sshd` or a thread.
    pub fn over(out: Box<dyn Write + Send>, mut input: Box<dyn Read + Send>) -> Result<Arc<Self>> {
        let shared = Arc::new(Shared {
            next_id: AtomicU32::new(1),
            replies: Mutex::new(HashMap::new()),
            reply_ready: Condvar::new(),
            reads: Mutex::new(Reads::default()),
            read_ready: Condvar::new(),
            alive: AtomicBool::new(true),
            fault: Mutex::new(None),
        });

        let session = Arc::new(Self {
            shared: Arc::clone(&shared),
            out: Mutex::new(out),
            child: Mutex::new(None),
        });

        {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("staramp-sftp".into())
                .spawn(move || {
                    let mut router = Router {
                        shared: Arc::clone(&shared),
                        scratch: Vec::with_capacity(READ_LEN as usize),
                    };
                    let mut scratch = Vec::new();
                    loop {
                        match wire::read_frame(&mut input, &mut scratch, &mut router) {
                            Ok(()) => {}
                            Err(e) => {
                                shared.die(e.to_string());
                                return;
                            }
                        }
                    }
                })
                .context("starting the sftp reader")?;
        }

        session.send(wire::init())?;
        Ok(session)
    }

    pub fn alive(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }

    fn send(&self, bytes: Vec<u8>) -> Result<()> {
        if !self.alive() {
            anyhow::bail!("{}", self.shared.fault());
        }
        let mut out = self.out.lock().unwrap();
        out.write_all(&bytes)
            .and_then(|()| out.flush())
            .map_err(|e| {
                self.shared.die(e.to_string());
                anyhow!("writing to the sftp channel: {e}")
            })
    }

    fn next_id(&self) -> u32 {
        self.shared.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Send one request and wait for its reply.
    fn call(&self, id: u32, bytes: Vec<u8>, what: &str) -> Result<Reply> {
        self.shared.replies.lock().unwrap().insert(id, None);
        if let Err(e) = self.send(bytes) {
            self.shared.replies.lock().unwrap().remove(&id);
            return Err(e);
        }

        let mut replies = self.shared.replies.lock().unwrap();
        loop {
            match replies.get(&id) {
                Some(Some(_)) => {
                    let r = replies.remove(&id).flatten().expect("just matched");
                    return r.ok(what);
                }
                _ if !self.alive() => {
                    replies.remove(&id);
                    return Err(anyhow!("{what}: {}", self.shared.fault()));
                }
                _ => {}
            }
            let (guard, timeout) = self
                .shared
                .reply_ready
                .wait_timeout(replies, REPLY_TIMEOUT)
                .unwrap();
            replies = guard;
            if timeout.timed_out() && replies.get(&id).is_some_and(Option::is_none) {
                replies.remove(&id);
                return Err(anyhow!("{what}: the server did not answer"));
            }
        }
    }

    /// Resolve a path the way the server would, expanding a relative one
    /// against the login directory.
    ///
    /// SFTP has no idea what `~` means -- it is a shell convention, and there
    /// is no shell here. This is how a configured `~/Music` becomes a path.
    pub fn realpath(&self, path: &str) -> Result<String> {
        let id = self.next_id();
        match self.call(id, wire::realpath(id, path), "resolving a path")? {
            Reply::Names(n) if !n.is_empty() => Ok(n[0].0.clone()),
            _ => Err(anyhow!("{path}: the server resolved it to nothing")),
        }
    }

    /// Size and modification time, or `None` if there is no such file.
    pub fn stat(&self, path: &str) -> Result<Option<wire::Attrs>> {
        let id = self.next_id();
        let reply = self.call(id, wire::stat(id, path), path);
        match reply {
            Ok(Reply::Attrs(a)) => Ok(Some(a)),
            Ok(_) => Ok(None),
            Err(e) if e.to_string().contains("no such file") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Open a file for reading.
    pub fn open_file(&self, path: &str) -> Result<Handle> {
        let id = self.next_id();
        match self.call(id, wire::open_read(id, path), path)? {
            Reply::Handle(h) => Ok(Handle(h)),
            _ => Err(anyhow!("{path}: the server did not return a handle")),
        }
    }

    pub fn fstat(&self, h: &Handle) -> Result<wire::Attrs> {
        let id = self.next_id();
        match self.call(id, wire::fstat(id, &h.0), "measuring an open file")? {
            Reply::Attrs(a) => Ok(a),
            _ => Err(anyhow!("the server did not return attributes")),
        }
    }

    pub fn close(&self, h: &Handle) {
        let id = self.next_id();
        let _ = self.call(id, wire::close(id, &h.0), "closing a file");
    }

    /// One directory's entries, `.` and `..` removed.
    pub fn read_dir(&self, path: &str) -> Result<Vec<(String, wire::Attrs)>> {
        let id = self.next_id();
        let handle = match self.call(id, wire::opendir(id, path), path)? {
            Reply::Handle(h) => Handle(h),
            _ => return Err(anyhow!("{path}: not a directory")),
        };
        let mut out = Vec::new();
        loop {
            let id = self.next_id();
            match self.call(id, wire::readdir(id, &handle.0), path) {
                Ok(Reply::Names(n)) if !n.is_empty() => {
                    out.extend(n.into_iter().filter(|(n, _)| n != "." && n != ".."))
                }
                // `readdir` answers EOF with a status, which `call` turns into
                // an error -- that is the loop's exit, not a failure.
                _ => break,
            }
        }
        self.close(&handle);
        Ok(out)
    }

    /// Issue a read without waiting for it. Returns the request id.
    pub fn read_at(&self, h: &Handle, offset: u64, len: u32) -> Result<u32> {
        let id = self.next_id();
        self.shared
            .reads
            .lock()
            .unwrap()
            .slots
            .insert(id, ReadState::Waiting);
        if let Err(e) = self.send(wire::read(id, &h.0, offset, len)) {
            self.shared.reads.lock().unwrap().slots.remove(&id);
            return Err(e);
        }
        Ok(id)
    }

    /// Wait for an outstanding read. `None` means end of file.
    pub fn collect(&self, id: u32) -> Result<Option<Vec<u8>>> {
        let mut reads = self.shared.reads.lock().unwrap();
        loop {
            match reads.slots.get(&id) {
                Some(ReadState::Done(_)) => {
                    let Some(ReadState::Done(b)) = reads.slots.remove(&id) else {
                        unreachable!("just matched")
                    };
                    return Ok(Some(b));
                }
                Some(ReadState::Eof) => {
                    reads.slots.remove(&id);
                    return Ok(None);
                }
                Some(ReadState::Failed(e)) => {
                    let e = e.clone();
                    reads.slots.remove(&id);
                    return Err(anyhow!("reading: {e}"));
                }
                None => return Err(anyhow!("no such read")),
                Some(ReadState::Waiting) if !self.alive() => {
                    reads.slots.remove(&id);
                    return Err(anyhow!("reading: {}", self.shared.fault()));
                }
                Some(ReadState::Waiting) => {}
            }
            let (guard, timeout) = self
                .shared
                .read_ready
                .wait_timeout(reads, REPLY_TIMEOUT)
                .unwrap();
            reads = guard;
            if timeout.timed_out() && matches!(reads.slots.get(&id), Some(ReadState::Waiting)) {
                reads.slots.remove(&id);
                return Err(anyhow!("reading: the server did not answer"));
            }
        }
    }

    /// Give a buffer back to be reused by a later read.
    pub fn recycle(&self, mut buf: Vec<u8>) {
        let mut reads = self.shared.reads.lock().unwrap();
        if reads.spare.len() < MAX_IN_FLIGHT {
            buf.clear();
            reads.spare.push(buf);
        }
    }

    /// Abandon a read whose answer is no longer wanted.
    ///
    /// The reply is still coming and will still be taken off the pipe -- it
    /// has to be, or the stream desynchronises -- but it will be dropped
    /// rather than stored. This is what a seek does to everything in flight.
    pub fn forget(&self, id: u32) {
        let mut reads = self.shared.reads.lock().unwrap();
        if let Some(ReadState::Done(b)) = reads.slots.remove(&id) {
            if reads.spare.len() < MAX_IN_FLIGHT {
                reads.spare.push(b);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.die("closed".into());
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// An open file on the far end.
///
/// Carries no position: an SFTP read names its own offset, which is why a
/// reconnect can reopen the path and carry straight on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handle(pub Vec<u8>);

/// A scripted SFTP server, for testing the client against something that
/// behaves like `sshd` without needing one.
///
/// It answers out of order on purpose. A server is free to, the client must
/// cope, and that is not a property that can be checked against a real server
/// which mostly happens to answer in order.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct Server {
        pub files: BTreeMap<String, Vec<u8>>,
        /// Answer reads in reverse batches of this size, to prove the client
        /// does not rely on ordering. 1 means answer in order.
        pub batch: usize,
        /// Serve at most this many bytes per read, however much was asked
        /// for. A server is allowed to; it is not EOF.
        pub short_read: Option<usize>,
        /// Every READ this server was asked for. What makes "that seek cost
        /// no network traffic" an assertion rather than a hope.
        pub reads: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[derive(Default)]
    struct Sink(Vec<(u8, Vec<u8>)>);
    impl FrameSink for Sink {
        fn data_dest(&mut self, _id: u32, _len: u32) -> Option<&mut [u8]> {
            None
        }
        fn other(&mut self, kind: u8, body: &[u8]) {
            self.0.push((kind, body.to_vec()));
        }
    }

    impl Server {
        pub fn new(files: &[(&str, Vec<u8>)]) -> Self {
            Self {
                files: files
                    .iter()
                    .map(|(n, b)| (n.to_string(), b.clone()))
                    .collect(),
                batch: 1,
                short_read: None,
                reads: Arc::default(),
            }
        }

        /// Run until the client hangs up.
        pub fn serve(self, mut input: impl Read, mut out: impl Write) {
            let mut handles: BTreeMap<Vec<u8>, String> = BTreeMap::new();
            let mut next_handle = 0u32;
            let mut dirs_done: BTreeMap<Vec<u8>, bool> = BTreeMap::new();
            let mut scratch = Vec::new();
            let mut pending: Vec<Vec<u8>> = Vec::new();

            loop {
                let mut sink = Sink::default();
                if wire::read_frame(&mut input, &mut scratch, &mut sink).is_err() {
                    return;
                }
                for (kind, body) in sink.0 {
                    let mut c = wire::Cursor::new(&body);
                    if kind == wire::INIT {
                        let mut p = wire::Packet::new(wire::VERSION_REPLY);
                        p.u32(wire::VERSION);
                        if out.write_all(&p.finish()).is_err() {
                            return;
                        }
                        let _ = out.flush();
                        continue;
                    }
                    let Some(id) = c.u32() else { continue };
                    let mut replies: Vec<Vec<u8>> = Vec::new();
                    let mut is_read = false;

                    match kind {
                        wire::REALPATH => {
                            let path = c.string().unwrap_or_default();
                            let resolved = path.replace('~', "/home/test");
                            let mut p = wire::Packet::request(wire::NAME, id);
                            p.u32(1);
                            p.str(&resolved);
                            p.str("");
                            p.u32(0);
                            replies.push(p.finish());
                        }
                        wire::STAT => {
                            let path = c.string().unwrap_or_default();
                            replies.push(match self.files.get(&path) {
                                Some(b) => attrs_reply(id, b.len() as u64),
                                None => status(id, wire::FX_NO_SUCH_FILE),
                            });
                        }
                        wire::OPEN | wire::OPENDIR => {
                            let path = c.string().unwrap_or_default();
                            let known = if kind == wire::OPEN {
                                self.files.contains_key(&path)
                            } else {
                                self.files
                                    .keys()
                                    .any(|k| k.starts_with(&format!("{path}/")))
                            };
                            if known {
                                next_handle += 1;
                                let h = next_handle.to_be_bytes().to_vec();
                                handles.insert(h.clone(), path);
                                dirs_done.insert(h.clone(), false);
                                let mut p = wire::Packet::request(wire::HANDLE, id);
                                p.bytes(&h);
                                replies.push(p.finish());
                            } else {
                                replies.push(status(id, wire::FX_NO_SUCH_FILE));
                            }
                        }
                        wire::FSTAT => {
                            let h = c.bytes().unwrap_or_default().to_vec();
                            let n = handles
                                .get(&h)
                                .and_then(|p| self.files.get(p))
                                .map(|b| b.len() as u64);
                            replies.push(match n {
                                Some(n) => attrs_reply(id, n),
                                None => status(id, 4),
                            });
                        }
                        wire::READDIR => {
                            let h = c.bytes().unwrap_or_default().to_vec();
                            let done = dirs_done.get(&h).copied().unwrap_or(true);
                            if done {
                                replies.push(status(id, wire::FX_EOF));
                            } else {
                                dirs_done.insert(h.clone(), true);
                                let base = handles.get(&h).cloned().unwrap_or_default();
                                let kids: Vec<String> = self
                                    .files
                                    .keys()
                                    .filter_map(|k| {
                                        k.strip_prefix(&format!("{base}/")).map(str::to_string)
                                    })
                                    .filter(|k| !k.contains('/'))
                                    .collect();
                                let mut p = wire::Packet::request(wire::NAME, id);
                                p.u32(kids.len() as u32 + 2);
                                for dot in [".", ".."] {
                                    p.str(dot);
                                    p.str("");
                                    p.u32(0);
                                }
                                for k in kids {
                                    p.str(&k);
                                    p.str("");
                                    p.u32(0);
                                }
                                replies.push(p.finish());
                            }
                        }
                        wire::READ => {
                            is_read = true;
                            self.reads
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let h = c.bytes().unwrap_or_default().to_vec();
                            let off = c.u64().unwrap_or(0) as usize;
                            let want = c.u32().unwrap_or(0) as usize;
                            let body = handles.get(&h).and_then(|p| self.files.get(p));
                            replies.push(match body {
                                Some(b) if off < b.len() => {
                                    let mut n = want.min(b.len() - off);
                                    if let Some(cap) = self.short_read {
                                        n = n.min(cap);
                                    }
                                    let mut p = wire::Packet::request(wire::DATA, id);
                                    p.bytes(&b[off..off + n]);
                                    p.finish()
                                }
                                Some(_) => status(id, wire::FX_EOF),
                                None => status(id, 4),
                            });
                        }
                        wire::CLOSE => {
                            let h = c.bytes().unwrap_or_default().to_vec();
                            handles.remove(&h);
                            replies.push(status(id, wire::FX_OK));
                        }
                        _ => replies.push(status(id, 8)),
                    }

                    // Reads may be held back and answered as a reversed batch;
                    // everything else is answered at once.
                    if is_read && self.batch > 1 {
                        pending.extend(replies);
                        if pending.len() >= self.batch {
                            pending.reverse();
                            for r in pending.drain(..) {
                                if out.write_all(&r).is_err() {
                                    return;
                                }
                            }
                            let _ = out.flush();
                        }
                    } else {
                        for r in replies {
                            if out.write_all(&r).is_err() {
                                return;
                            }
                        }
                        let _ = out.flush();
                    }
                }
            }
        }
    }

    fn status(id: u32, code: u32) -> Vec<u8> {
        let mut p = wire::Packet::request(wire::STATUS, id);
        p.u32(code);
        p.str(wire::status_name(code));
        p.str("en");
        p.finish()
    }

    fn attrs_reply(id: u32, size: u64) -> Vec<u8> {
        let mut p = wire::Packet::request(wire::ATTRS, id);
        p.u32(0x1 | 0x8); // size + acmodtime
        p.u64(size);
        p.u32(1_700_000_000);
        p.u32(1_700_000_000);
        p.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client wired to a fake server over real pipes, so the bytes make the
    /// same journey they would over ssh.
    fn connect(server: fake::Server) -> Arc<Session> {
        let (to_server_r, to_server_w) = std::io::pipe().unwrap();
        let (to_client_r, to_client_w) = std::io::pipe().unwrap();
        std::thread::spawn(move || server.serve(to_server_r, to_client_w));
        Session::over(Box::new(to_server_w), Box::new(to_client_r)).unwrap()
    }

    fn tone(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_file_can_be_measured_opened_and_read() {
        let body = tone(5000);
        let s = connect(fake::Server::new(&[("/music/a.flac", body.clone())]));

        let attrs = s.stat("/music/a.flac").unwrap().expect("it is there");
        assert_eq!(attrs.size, Some(5000));

        let h = s.open_file("/music/a.flac").unwrap();
        assert_eq!(s.fstat(&h).unwrap().size, Some(5000));

        let id = s.read_at(&h, 1000, 256).unwrap();
        let got = s.collect(id).unwrap().expect("not eof");
        assert_eq!(got, body[1000..1256]);
        s.close(&h);
    }

    #[test]
    fn a_missing_file_is_an_absence_rather_than_an_error() {
        let s = connect(fake::Server::new(&[]));
        assert_eq!(s.stat("/nope").unwrap(), None);
        assert!(s.open_file("/nope").is_err());
    }

    /// The property that makes pipelining safe: replies are matched by id, so
    /// a server answering in any order at all is fine.
    #[test]
    fn sixty_four_reads_in_flight_all_land_in_the_right_place() {
        let body = tone(64 * 1024);
        let mut server = fake::Server::new(&[("/f", body.clone())]);
        server.batch = 16; // answered in reversed batches of sixteen
        let s = connect(server);

        let h = s.open_file("/f").unwrap();
        let chunk = 1024u32;
        let ids: Vec<(u64, u32)> = (0..64)
            .map(|i| {
                let off = i as u64 * chunk as u64;
                (off, s.read_at(&h, off, chunk).unwrap())
            })
            .collect();

        for (off, id) in ids {
            let got = s.collect(id).unwrap().expect("not eof");
            let at = off as usize;
            assert_eq!(got, body[at..at + chunk as usize], "chunk at {off}");
        }
    }

    /// A short reply is the server's prerogative and means nothing is wrong.
    /// Treating it as the end of the file would truncate every track.
    #[test]
    fn a_short_read_is_not_the_end_of_the_file() {
        let body = tone(4096);
        let mut server = fake::Server::new(&[("/f", body.clone())]);
        server.short_read = Some(100);
        let s = connect(server);

        let h = s.open_file("/f").unwrap();
        let id = s.read_at(&h, 0, 4096).unwrap();
        let got = s.collect(id).unwrap().expect("short, but not eof");
        assert_eq!(got.len(), 100);
        assert_eq!(got, body[..100]);

        // And the rest is still readable.
        let id = s.read_at(&h, 100, 4096).unwrap();
        assert_eq!(s.collect(id).unwrap().unwrap().len(), 100);
    }

    #[test]
    fn reading_past_the_end_reports_the_end() {
        let s = connect(fake::Server::new(&[("/f", tone(10))]));
        let h = s.open_file("/f").unwrap();
        let id = s.read_at(&h, 10, 100).unwrap();
        assert_eq!(s.collect(id).unwrap(), None);
    }

    /// A seek abandons every read in flight. SFTP cannot cancel them, so their
    /// replies still arrive -- and if they were not drained, the very next
    /// packet would be read at the wrong offset and everything after it would
    /// be nonsense.
    #[test]
    fn abandoned_reads_do_not_desynchronise_the_stream() {
        let body = tone(32 * 1024);
        let s = connect(fake::Server::new(&[("/f", body.clone())]));
        let h = s.open_file("/f").unwrap();

        let abandoned: Vec<u32> = (0..8)
            .map(|i| s.read_at(&h, i * 2048, 2048).unwrap())
            .collect();
        // Change of mind, as a seek would.
        for id in &abandoned {
            s.forget(*id);
        }

        // The stream must still be intact.
        let id = s.read_at(&h, 20_000, 512).unwrap();
        let got = s.collect(id).unwrap().expect("not eof");
        assert_eq!(got, body[20_000..20_512]);

        // And the old ids are gone rather than lingering.
        assert!(s.collect(abandoned[0]).is_err());
    }

    #[test]
    fn a_directory_lists_its_children_without_the_dots() {
        let s = connect(fake::Server::new(&[
            ("/m/a.flac", tone(4)),
            ("/m/b.flac", tone(4)),
            ("/m/sub/c.flac", tone(4)),
        ]));
        let mut names: Vec<String> = s
            .read_dir("/m")
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.flac".to_string(), "b.flac".to_string()]);
    }

    /// SFTP has no shell and so no `~`. This is the only thing that expands it.
    #[test]
    fn realpath_is_what_turns_a_tilde_into_a_path() {
        let s = connect(fake::Server::new(&[]));
        assert_eq!(s.realpath("~/Music").unwrap(), "/home/test/Music");
    }

    /// A link that dies must wake everyone waiting on it. Hanging for ever is
    /// how a player stops responding to its own transport controls.
    #[test]
    fn a_dead_link_wakes_its_waiters_rather_than_hanging() {
        let (to_server_r, to_server_w) = std::io::pipe().unwrap();
        let (to_client_r, to_client_w) = std::io::pipe().unwrap();
        // A server that answers the handshake and then goes away.
        std::thread::spawn(move || {
            let _ = to_server_r;
            drop(to_client_w);
        });
        let s = Session::over(Box::new(to_server_w), Box::new(to_client_r)).unwrap();

        let started = std::time::Instant::now();
        let r = s.stat("/anything");
        assert!(r.is_err(), "a dead link cannot answer");
        assert!(
            started.elapsed() < REPLY_TIMEOUT,
            "it must not wait out the timeout"
        );
        assert!(!s.alive());
    }
}

/// The client, driven against OpenSSH's own `sftp-server`.
///
/// The fake server above is written to this client's understanding of the
/// protocol, so it cannot catch a place where that understanding is wrong.
/// This can: it is the same binary `sshd` runs, speaking to us over the same
/// kind of pipe, with `ssh` removed from the middle because `ssh` contributes
/// encryption and authentication rather than anything the protocol depends on.
#[cfg(test)]
mod against_openssh {
    use super::*;

    /// The real `sftp-server`, if this machine has one.
    fn sftp_server() -> Option<std::path::PathBuf> {
        // The usual places, then whatever an OpenSSH in the store provides.
        for p in [
            "/usr/lib/openssh/sftp-server",
            "/usr/libexec/sftp-server",
            "/usr/lib/ssh/sftp-server",
            "/usr/lib/misc/sftp-server",
        ] {
            let p = std::path::Path::new(p);
            if p.is_file() {
                return Some(p.to_path_buf());
            }
        }
        let out = Command::new("sh")
            .arg("-c")
            .arg("ls -d /nix/store/*openssh*/libexec/sftp-server 2>/dev/null | head -1")
            .output()
            .ok()?;
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!path.is_empty()).then(|| std::path::PathBuf::from(path))
    }

    fn connect(server: &std::path::Path) -> Option<Arc<Session>> {
        let mut child = Command::new(server)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        // The child outlives this handle deliberately; the test process ends
        // shortly after and takes it with it.
        std::mem::forget(child);
        Session::over(Box::new(stdin), Box::new(stdout)).ok()
    }

    #[test]
    fn the_real_server_answers_the_way_we_expect() {
        let Some(server) = sftp_server() else {
            eprintln!("no sftp-server on this machine, skipping");
            return;
        };
        let Some(s) = connect(&server) else {
            eprintln!("could not start {}, skipping", server.display());
            return;
        };

        let dir = std::env::temp_dir().join(format!("staramp-sftp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.bin");
        // Larger than one read, so the pipelining is exercised rather than
        // just the handshake.
        let body: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &body).unwrap();
        let name = path.to_string_lossy().to_string();

        // Home directory expansion, which is the whole reason REALPATH is
        // spoken at all.
        let home = s.realpath(".").expect("realpath");
        assert!(home.starts_with('/'), "got {home}");

        let attrs = s.stat(&name).expect("stat").expect("it exists");
        assert_eq!(attrs.size, Some(body.len() as u64));
        assert!(!attrs.is_dir());
        assert!(s
            .stat(&dir.join("nope").to_string_lossy())
            .expect("a missing file is not an error")
            .is_none());

        let h = s.open_file(&name).expect("open");
        assert_eq!(s.fstat(&h).unwrap().size, Some(body.len() as u64));

        // Sixty-four reads outstanding at once, against a server that owes us
        // nothing about the order it answers in.
        let ids: Vec<(usize, u32)> = (0..64)
            .map(|i| {
                let off = i * 4096;
                (off, s.read_at(&h, off as u64, 4096).unwrap())
            })
            .collect();
        for (off, id) in ids {
            let got = s.collect(id).unwrap().expect("not eof");
            assert_eq!(got, body[off..off + 4096], "chunk at {off}");
        }

        // And the end of the file is the end of the file.
        let id = s.read_at(&h, body.len() as u64, 4096).unwrap();
        assert_eq!(s.collect(id).unwrap(), None);
        s.close(&h);

        let names: Vec<String> = s
            .read_dir(&dir.to_string_lossy())
            .expect("readdir")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["tone.bin".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The window, over the real server rather than ours.
    #[test]
    fn a_remote_file_reads_back_whole_from_the_real_server() {
        use crate::remote::stream::{RemoteFile, MIN_WINDOW};
        use std::io::Read;

        let Some(server) = sftp_server() else { return };
        let Some(s) = connect(&server) else { return };

        let dir = std::env::temp_dir().join(format!("staramp-sftp2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        let body: Vec<u8> = (0..1_500_000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &body).unwrap();

        let mut f = RemoteFile::open(s, &path.to_string_lossy(), MIN_WINDOW).unwrap();
        let mut got = Vec::new();
        f.read_to_end(&mut got).unwrap();
        assert_eq!(got.len(), body.len());
        assert_eq!(got, body);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole stack, over the real server: wire, window, custom AVIO, PCM.
    ///
    /// If this passes, a WavPack file on another machine decodes to the same
    /// samples it would decode to on this one -- which is the entire feature,
    /// with only `ssh` and a network taken out of the middle.
    #[test]
    fn a_wavpack_file_decodes_the_same_through_the_real_server() {
        use crate::audio::decode;
        use crate::remote::stream::{RemoteFile, MIN_WINDOW};
        use crate::vfs::{Media, RemoteRead};

        let local = std::path::Path::new("testdata/tone.wv");
        if !local.is_file() {
            eprintln!("testdata/tone.wv missing, skipping");
            return;
        }
        let Some(server) = sftp_server() else { return };
        let Some(s) = connect(&server) else { return };

        let drain = |mut d: Box<dyn decode::Decoder>| {
            let mut all = Vec::new();
            let mut buf = vec![0f32; 4096];
            while let Ok(frames) = d.read(&mut buf) {
                if frames == 0 {
                    break;
                }
                all.extend_from_slice(&buf[..frames * d.spec().samples_per_frame()]);
            }
            all
        };

        let absolute = std::fs::canonicalize(local).unwrap();
        let f = RemoteFile::open(s, &absolute.to_string_lossy(), MIN_WINDOW).unwrap();
        let len = RemoteRead::len(&f);
        let over_sftp = drain(
            decode::open(
                Media::Stream {
                    reader: Box::new(f),
                    len,
                },
                "tone.wv",
            )
            .unwrap(),
        );
        let on_disk = drain(decode::open(Media::Local(absolute), "tone.wv").unwrap());

        assert!(!on_disk.is_empty(), "the local decode produced nothing");
        assert_eq!(over_sftp.len(), on_disk.len(), "same number of samples");
        assert_eq!(over_sftp, on_disk, "sample for sample");
    }
}
