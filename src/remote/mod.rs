//! Playing a library that lives on another machine.
//!
//! The shape of this is deliberately lopsided. The *index* is copied once and
//! queried locally, because it is small -- 31 MB for a 1.1 TB library -- and
//! because every browse, search and smart playlist then runs at the speed of
//! local SQLite instead of the speed of the link. The *audio* is never copied:
//! it is read on demand over SFTP, a window at a time, because a track is
//! thousands of times larger than its index row and waiting for a whole file
//! before playing it is what this design exists to avoid.
//!
//! There is nothing to install on the far machine. star/amp is expected to be
//! there and to have scanned, so that an index exists to fetch, but nothing
//! listens, nothing is a daemon, and no port is opened. `sshd` and its `sftp`
//! subsystem do all of it.

pub mod index;
pub mod sftp;
pub mod ssh;
pub mod stream;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use sftp::session::Session;
use sftp::wire;

/// How much of a track to keep buffered, by default.
///
/// Bytes rather than seconds because the bitrate of what is about to play is
/// not known at the point a file is opened. Sixteen mebibytes is about 45
/// seconds of a 3 Mbps rip and about 20 seconds of DSD64 -- comfortably past
/// any reconnect, and adjustable with `[remote] readahead_mb`.
pub const DEFAULT_WINDOW_MB: u64 = 16;

/// A library on another machine.
pub struct Library {
    master: ssh::Master,
    /// The playing track, and nothing else.
    ///
    /// Two channels rather than one because a session shares a single 2 MiB
    /// SSH window: a four-megabyte cover fetched on the same channel would sit
    /// in front of the audio and starve it. Art is allowed to be slow. Audio
    /// is not.
    audio: Mutex<Arc<Session>>,
    /// The index, artwork, and anything else that is not the playing track.
    bulk: Mutex<Arc<Session>>,
    /// The library root on the far machine, absolute.
    root: String,
    window: u64,
}

impl Library {
    /// Connect to `host` and resolve `root` there.
    ///
    /// `root` may be relative or start with `~`, which is why the first thing
    /// that happens is a `realpath`: SFTP has no shell and so no idea what a
    /// tilde means.
    pub fn connect(host: &str, root: &str, window_mb: Option<u64>) -> Result<Self> {
        let master = ssh::Master::connect(host)?;
        let audio = Session::open(host, master.control_path())?;
        let bulk = Session::open(host, master.control_path())?;

        let root = bulk
            .realpath(root)
            .with_context(|| format!("resolving {root} on {host}"))?;
        // Fail here rather than at the first track: a mistyped root should say
        // so while the user is still looking at the command they typed.
        let attrs = bulk
            .stat(&root)?
            .with_context(|| format!("{host}:{root} does not exist"))?;
        anyhow::ensure!(
            attrs.is_dir() || attrs.permissions.is_none(),
            "{host}:{root} is not a directory"
        );

        Ok(Self {
            master,
            audio: Mutex::new(audio),
            bulk: Mutex::new(bulk),
            root,
            window: window_mb.unwrap_or(DEFAULT_WINDOW_MB) * 1024 * 1024,
        })
    }

    pub fn host(&self) -> &str {
        self.master.host()
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn link(&self) -> &Arc<ssh::LinkState> {
        &self.master.link
    }

    /// A library-relative URI as an absolute path on the far machine.
    ///
    /// Joined as text, and never normalised. The index stored these bytes
    /// exactly as the far machine's filesystem gave them, and handing back
    /// anything else -- a different Unicode normalisation, a different case --
    /// is how a file that exists becomes a file that cannot be opened. It
    /// matters more than it looks: macOS filenames are commonly NFD and Linux
    /// ones are whatever wrote them.
    fn absolute(&self, rel: &str) -> String {
        if rel.starts_with('/') {
            return rel.to_string();
        }
        format!("{}/{}", self.root.trim_end_matches('/'), rel)
    }

    /// A session, reopened if the last one died.
    fn session(&self, bulk: bool) -> Result<Arc<Session>> {
        let slot = if bulk { &self.bulk } else { &self.audio };
        let mut held = slot.lock().unwrap();
        if held.alive() {
            return Ok(Arc::clone(&held));
        }
        // The channel is gone. The master may be too, in which case it has to
        // come back first -- and an SFTP read carries its own offset, so a
        // reopened file resumes exactly where the old one stopped.
        if !self.master.running() {
            self.master.reconnect(0)?;
        }
        let fresh = Session::open(self.master.host(), self.master.control_path())?;
        *held = Arc::clone(&fresh);
        Ok(fresh)
    }

    /// Open a track for playback.
    pub fn media(&self, rel: &str) -> Result<crate::vfs::Media> {
        let session = self.session(false)?;
        let path = self.absolute(rel);
        let file = stream::RemoteFile::open(session, &path, self.window)
            .with_context(|| format!("opening {}:{path}", self.host()))?;
        let len = crate::vfs::RemoteRead::len(&file);
        Ok(crate::vfs::Media::Stream {
            reader: Box::new(file),
            len,
        })
    }

    /// A standalone reader, for a tag library that wants to seek about.
    pub fn reader(&self, rel: &str) -> Result<Box<dyn crate::vfs::RemoteRead>> {
        let session = self.session(true)?;
        let path = self.absolute(rel);
        Ok(Box::new(stream::RemoteFile::open(
            session,
            &path,
            stream::MIN_WINDOW,
        )?))
    }

    /// A whole small file: a cover, a playlist, a cue sheet.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        use std::io::Read;
        let mut out = Vec::new();
        self.reader(rel)?.read_to_end(&mut out)?;
        Ok(out)
    }

    /// The first `n` bytes, and no more.
    ///
    /// What an embedded-artwork probe wants: an ID3 header is ten bytes and
    /// the picture after it is bounded by what that header declares, so the
    /// whole question is answerable without reading the audio.
    pub fn read_head(&self, rel: &str, n: usize) -> Result<Vec<u8>> {
        use std::io::Read;
        let mut out = vec![0u8; 0];
        let mut r = self.reader(rel)?.take(n as u64);
        r.read_to_end(&mut out)?;
        Ok(out)
    }

    pub fn stat(&self, rel: &str) -> Result<Option<wire::Attrs>> {
        self.session(true)?.stat(&self.absolute(rel))
    }

    /// Resolve a path on the far machine, expanding `~`.
    pub fn session_realpath(&self, path: &str) -> Result<String> {
        self.session(true)?.realpath(path)
    }

    /// `stat` a path that is already absolute on the far machine.
    pub fn stat_absolute(&self, path: &str) -> Result<Option<wire::Attrs>> {
        self.session(true)?.stat(path)
    }

    /// Copy a whole file down, streaming it to disk rather than into memory.
    pub fn download(&self, remote: &str, to: &std::path::Path) -> Result<u64> {
        use std::io::{Read, Write};
        let session = self.session(true)?;
        let mut src = stream::RemoteFile::open(session, remote, stream::MIN_WINDOW)
            .with_context(|| format!("opening {}:{remote}", self.host()))?;
        let mut dst = std::io::BufWriter::new(
            std::fs::File::create(to).with_context(|| format!("creating {}", to.display()))?,
        );
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n])?;
            total += n as u64;
        }
        dst.flush()?;
        Ok(total)
    }

    /// Open a track's first bytes and throw them away, so that playing it
    /// costs no round trips.
    ///
    /// Gapless playback across a link needs this. The player opens the next
    /// track at the boundary, with a fifth of a second of audio left in the
    /// ring; three or four round trips do not fit in that, and the gap is
    /// audible at every track change. Called well before the boundary, this
    /// moves that cost somewhere nobody can hear it.
    pub fn warm(&self, rel: &str) {
        let path = self.absolute(rel);
        let Ok(session) = self.session(false) else {
            return;
        };
        let window = self.window;
        // On its own thread: warming is best-effort and must never be
        // something the decode loop waits for.
        std::thread::Builder::new()
            .name("staramp-warm".into())
            .spawn(move || {
                use std::io::Read;
                if let Ok(mut f) = stream::RemoteFile::open(session, &path, window) {
                    let mut sink = [0u8; 64 * 1024];
                    let _ = f.read(&mut sink);
                }
            })
            .ok();
    }
}
