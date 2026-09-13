//! Keeping a paused track's bytes off a sleeping disk.
//!
//! Pausing stops the output device; it does not stop the drive underneath from
//! spinning down. An external disk parks itself after a few idle minutes, and
//! the first read after that costs several seconds while it spins back up --
//! which is exactly where the decode thread lands the moment playback resumes,
//! because the output ring holds well under a second of audio.
//!
//! So while the drive is still awake -- at the pause, seconds after it was
//! last read -- the file is read straight through and thrown away. Nothing
//! here keeps the bytes. The kernel's page cache does, and reads after that
//! are served from memory without the platter being touched at all.
//!
//! Letting the kernel hold them rather than holding them ourselves is the
//! whole point. Under memory pressure it can drop the pages and the worst
//! outcome is the delay that happens today; a buffer of our own would simply
//! be another thing competing for the same memory, and could not be dropped.

use std::io;
use std::path::{Path, PathBuf};

/// The most one file may be pulled in.
///
/// A 24-bit/192 kHz album image or a DSD rip runs to hundreds of megabytes,
/// and reading one is a long burst of I/O for a track somebody may never
/// resume. Below the default a whole cue image still fits, which is the case
/// this feature exists for.
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Pull `path` into the page cache, on a thread of its own.
///
/// Returns immediately. Nothing waits on this and nothing reports it: it is a
/// hint to the operating system that happens to be spelled as a read, and a
/// failure means the next play costs what it costs today.
pub fn ahead(path: PathBuf, max_bytes: u64) {
    let _ = std::thread::Builder::new()
        .name("staramp-warm".into())
        .spawn(move || match read_through(&path, max_bytes) {
            Ok(Some(n)) => tracing::debug!("warmed {n} bytes of {}", path.display()),
            Ok(None) => {}
            Err(e) => tracing::debug!("could not warm {}: {e}", path.display()),
        });
}

/// Read a file to nowhere, so the pages stay resident. `None` when it was
/// skipped for being too large.
fn read_through(path: &Path, max_bytes: u64) -> io::Result<Option<u64>> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > max_bytes {
        tracing::debug!(
            "not warming {}: {len} bytes is over the {max_bytes}-byte limit",
            path.display()
        );
        return Ok(None);
    }

    // `posix_fadvise(WILLNEED)` is the call that looks right here and is not:
    // it is advisory, asynchronous, and the kernel caps how far ahead it will
    // read from one hint, so a large file comes back partly resident. A plain
    // sequential read is dull and certain, and the cost is a copy into a
    // buffer that is thrown away -- against a disk that has to spin up, that
    // is not a cost worth optimising.
    let mut sink = io::sink();
    let mut file = file;
    io::copy(&mut file, &mut sink).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_file_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.flac");
        let bytes = vec![7u8; 64 * 1024];
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();

        assert_eq!(
            read_through(&path, DEFAULT_MAX_BYTES).unwrap(),
            Some(bytes.len() as u64)
        );
    }

    /// A track somebody may never resume is not worth a long burst of I/O.
    #[test]
    fn a_file_over_the_limit_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.dsf");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[0u8; 4096])
            .unwrap();

        assert_eq!(read_through(&path, 1024).unwrap(), None);
        // And the limit is inclusive of a file that exactly reaches it.
        assert_eq!(read_through(&path, 4096).unwrap(), Some(4096));
    }

    /// Warming is best-effort. A path that is not there is not an incident.
    #[test]
    fn a_missing_file_is_an_error_rather_than_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_through(&dir.path().join("gone.flac"), DEFAULT_MAX_BYTES).is_err());
        // And the public entry point swallows it.
        ahead(dir.path().join("gone.flac"), DEFAULT_MAX_BYTES);
    }

    /// The setting exists because the cure is worse than the disease on an
    /// SSD: a burst of I/O for a spin-up that never happens.
    #[test]
    fn the_config_turns_it_into_a_budget_or_nothing() {
        use crate::config::Disk;

        let on = Disk::default();
        assert!(on.warm_on_pause, "warming is on unless turned off");
        assert_eq!(on.warm_max_bytes(), DEFAULT_MAX_BYTES);

        let off = Disk {
            warm_on_pause: false,
            ..Disk::default()
        };
        assert!(
            off.warm_on_pause.then(|| off.warm_max_bytes()).is_none(),
            "turning it off must leave the player with no budget at all"
        );

        // A silly figure cannot overflow into a small one.
        let huge = Disk {
            warm_on_pause: true,
            warm_max_mb: u64::MAX,
        };
        assert_eq!(huge.warm_max_bytes(), u64::MAX);
    }
}
