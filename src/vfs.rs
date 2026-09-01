//! Where a track's bytes come from.
//!
//! Every URI in the index is stored relative to the library root — no host, no
//! scheme, no absolute path — so the same index describes the same music
//! whether the files are under the local root or on the far end of a link. This
//! module is the one place that turns such a URI into something a decoder can
//! read, and it is deliberately the *only* place: below it, nothing knows or
//! asks where the bytes came from.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Where a URI's bytes are, as a decoder input.
///
/// The local arm carries a bare `PathBuf` and nothing else, on purpose. It lets
/// symphonia keep `MediaSource for File` — whose `is_seekable` and `byte_len`
/// are an fstat rather than a virtual call — and lets libav keep
/// `avformat_open_input(path)`, with libavformat's own `file:` protocol and its
/// own read-ahead. Wrapping a local path in a `Box<dyn Read + Seek>` would
/// silently swap both of those for a generic buffered path, which is a real
/// regression on a spinning disk and buys nothing.
pub enum Media {
    Local(PathBuf),
    Stream {
        reader: Box<dyn RemoteRead>,
        len: u64,
    },
}

/// An open file on a transport that is not this machine's filesystem.
///
/// The bounds are not a choice. They are exactly the intersection of what
/// symphonia's `MediaSource` and ffmpeg-next's `StreamIo::from_read_seek`
/// each require, so anything satisfying both decoders satisfies this.
pub trait RemoteRead: Read + Seek + Send + Sync + 'static {
    /// Total bytes. Known from the stat that opened the handle, so free.
    fn len(&self) -> u64;
}

/// A library root, and how to read files under it.
///
/// One variant today. The point of the type is that `absolutise` is no longer
/// something any caller can do for itself: a path leaves the index through
/// [`Vfs::media`] or not at all.
pub enum Vfs {
    Local {
        root: PathBuf,
    },
    /// A library reached over SSH. The index is a local copy; the audio is
    /// read on demand.
    Remote(std::sync::Arc<crate::remote::Library>),
}

impl Vfs {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        Vfs::Local { root: root.into() }
    }

    /// A root-relative URI as a local filesystem path, when there is one.
    ///
    /// An absolute path is used as-is, which is what the CLI passes: `staramp
    /// ui /path/to/album` builds URIs that are already absolute and a root of
    /// `""` that must not be joined onto them.
    ///
    /// `None` for a remote library, and every caller has to mean it: this is
    /// the boundary where "read the file yourself" stops being possible.
    pub fn local_path(&self, rel: &str) -> Option<PathBuf> {
        match self {
            Vfs::Local { root } => Some(absolutise(root, rel)),
            Vfs::Remote(_) => None,
        }
    }

    /// The index that describes *this* library.
    ///
    /// For a remote one that is the copy fetched from the far machine, not
    /// the local index -- which describes a different library entirely, and
    /// whose URIs would resolve to the wrong files or to none.
    pub fn index_path(&self) -> Result<PathBuf> {
        match self {
            Vfs::Local { .. } => crate::paths::index_file(),
            Vfs::Remote(l) => crate::remote::index::local_copy(l.host()),
        }
    }

    /// True when the bytes are not on this machine.
    pub fn is_remote(&self) -> bool {
        matches!(self, Vfs::Remote(_))
    }

    /// A name for this URI to show and to put in error messages.
    pub fn label(&self, rel: &str) -> String {
        match self {
            Vfs::Local { .. } => self
                .local_path(rel)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| rel.to_string()),
            Vfs::Remote(l) => format!("{}:{rel}", l.host()),
        }
    }

    /// A root-relative URI as a decoder input.
    pub fn media(&self, rel: &str) -> Result<Media> {
        match self {
            Vfs::Local { root } => Ok(Media::Local(absolutise(root, rel))),
            Vfs::Remote(l) => l.media(rel),
        }
    }

    /// A whole small file: a cue sheet, a playlist, a cover.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        match self {
            Vfs::Local { root } => Ok(std::fs::read(absolutise(root, rel))?),
            Vfs::Remote(l) => l.read(rel),
        }
    }

    /// The first `n` bytes and no more.
    pub fn read_head(&self, rel: &str, n: usize) -> Result<Vec<u8>> {
        match self {
            Vfs::Local { root } => {
                use std::io::Read;
                let mut out = Vec::new();
                std::fs::File::open(absolutise(root, rel))?
                    .take(n as u64)
                    .read_to_end(&mut out)?;
                Ok(out)
            }
            Vfs::Remote(l) => l.read_head(rel, n),
        }
    }

    /// A seekable reader, for a tag library that wants to wander.
    pub fn reader(&self, rel: &str) -> Result<Box<dyn RemoteRead>> {
        match self {
            Vfs::Local { root } => Ok(Box::new(LocalFile {
                len: std::fs::metadata(absolutise(root, rel))
                    .map(|m| m.len())
                    .unwrap_or(0),
                file: std::fs::File::open(absolutise(root, rel))?,
            })),
            Vfs::Remote(l) => l.reader(rel),
        }
    }

    /// Ask for a track to be made ready before it is needed.
    ///
    /// Nothing at all for a local library, where opening a file is free. Over
    /// a link it is what keeps a track change gapless -- see
    /// [`crate::remote::Library::warm`].
    pub fn warm(&self, rel: &str) {
        if let Vfs::Remote(l) = self {
            l.warm(rel);
        }
    }
}

/// A local file, as a [`RemoteRead`]. Not a contradiction: the trait is about
/// what a caller may do with a handle, not about where the bytes live.
struct LocalFile {
    file: std::fs::File,
    len: u64,
}

impl Read for LocalFile {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(b)
    }
}
impl Seek for LocalFile {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(p)
    }
}
impl RemoteRead for LocalFile {
    fn len(&self) -> u64 {
        self.len
    }
}

fn absolutise(root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// A [`RemoteRead`] as something symphonia will accept.
///
/// Two methods, and one of them is load-bearing in a way that is easy to miss:
/// `is_seekable` returning false makes symphonia refuse `FormatReader::seek`
/// and fall back to reading forward from wherever it is, which would break
/// every cue slice's seek to its start frame -- and cue virtual tracks are
/// 27% of the reference library's playlists.
pub struct RemoteSource {
    pub inner: Box<dyn RemoteRead>,
    pub len: u64,
}

impl Read for RemoteSource {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(b)
    }
}

impl Seek for RemoteSource {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(p)
    }
}

impl symphonia::core::io::MediaSource for RemoteSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_uri_is_joined_onto_the_root() {
        let v = Vfs::local("/music");
        assert_eq!(
            v.local_path("Artist/Album/01.flac").unwrap(),
            PathBuf::from("/music/Artist/Album/01.flac")
        );
    }

    /// The CLI passes absolute URIs with an empty root, and joining those onto
    /// anything would break `staramp ui /path/to/album`.
    #[test]
    fn an_absolute_uri_ignores_the_root() {
        let v = Vfs::local("");
        assert_eq!(
            v.local_path("/elsewhere/a.flac").unwrap(),
            PathBuf::from("/elsewhere/a.flac")
        );
        let v = Vfs::local("/music");
        assert_eq!(
            v.local_path("/elsewhere/a.flac").unwrap(),
            PathBuf::from("/elsewhere/a.flac")
        );
    }
}
