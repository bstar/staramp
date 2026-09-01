//! Fetching the far machine's library index.
//!
//! The whole index, copied once and then queried locally. That sounds
//! extravagant until it is measured: the reference library is 1.1 TB of audio
//! and 31 MB of index, so the copy costs seconds and every browse, search and
//! smart playlist afterwards costs nothing. Querying it across the link
//! instead would mean a round trip per keystroke in the search box.
//!
//! It also means `library::browse`, `library::db`, the FTS index and the
//! smart-playlist compiler are used exactly as they are, with no idea that
//! anything is remote. Every URI in the index is already relative to the
//! library root -- no host, no scheme -- so the file is portable as it stands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::Library;

/// Where the server's index lives, relative to its home directory.
///
/// The same layout `paths::index_file` produces, because the far machine is
/// running this program.
const REMOTE_INDEX: &str = "~/.local/staramp/index.sqlite";

/// Where a host's copy is kept locally.
pub fn local_copy(host: &str) -> Result<PathBuf> {
    let dir = crate::paths::cache_dir()?.join("remote");
    std::fs::create_dir_all(&dir)?;
    let h = blake3::hash(host.as_bytes());
    Ok(dir.join(format!("{}.sqlite", &h.to_hex()[..16])))
}

/// Fetch the index if the copy in hand is not already the current one.
///
/// Returns the local path either way.
pub fn sync(lib: &Library) -> Result<PathBuf> {
    let local = local_copy(lib.host())?;
    let remote = lib
        .session_realpath(REMOTE_INDEX)
        .unwrap_or_else(|_| REMOTE_INDEX.to_string());

    let attrs = lib.stat_absolute(&remote)?.with_context(|| {
        format!(
            "{}:{remote} does not exist — run `staramp scan` there first",
            lib.host()
        )
    })?;

    // Size and mtime together are what the scanner itself uses to decide a
    // file is unchanged, and they are free: the stat has already happened.
    let stamp = stamp_path(&local);
    let want = format!(
        "{} {}",
        attrs.size.unwrap_or(0),
        attrs.mtime.unwrap_or_default()
    );
    if local.is_file() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        tracing::debug!("index for {} is current", lib.host());
        return Ok(local);
    }

    tracing::info!(
        "fetching the index for {} ({} bytes)",
        lib.host(),
        attrs.size.unwrap_or(0)
    );

    // Downloaded beside the target and renamed into place, so an interrupted
    // fetch never leaves a half-written database that looks complete.
    let part = local.with_extension("part");
    lib.download(&remote, &part)?;

    // ⚠ The index is in WAL mode (`schema::PRAGMAS`). If a scan is running on
    // the far machine right now, the committed data is split between the main
    // file and its `-wal` sidecar, and taking only the main file yields a
    // stale or torn snapshot. SQLite removes the sidecar on a clean close, so
    // most of the time there is nothing here to do -- but "most of the time"
    // is how a library ends up corrupt on the one day it matters.
    let wal_part = with_suffix(&part, "-wal");
    let _ = std::fs::remove_file(&wal_part);
    if lib.stat_absolute(&format!("{remote}-wal"))?.is_some() {
        tracing::debug!("the far index has a -wal sidecar; taking it too");
        if let Err(e) = lib.download(&format!("{remote}-wal"), &wal_part) {
            tracing::debug!("could not fetch the -wal sidecar: {e}");
        }
    }

    check(&part).with_context(|| {
        format!(
            "the index fetched from {} did not survive the trip",
            lib.host()
        )
    })?;

    // Fold the sidecar in and drop it, so what is kept is one self-contained
    // file rather than a pair that must stay together.
    let _ = std::fs::remove_file(with_suffix(&local, "-wal"));
    let _ = std::fs::remove_file(with_suffix(&local, "-shm"));
    std::fs::rename(&part, &local)
        .with_context(|| format!("moving the index into {}", local.display()))?;
    if wal_part.exists() {
        let _ = std::fs::rename(&wal_part, with_suffix(&local, "-wal"));
    }
    let _ = std::fs::write(&stamp, &want);
    Ok(local)
}

/// Open the copy and make sure it is a database rather than a lump of bytes.
///
/// `quick_check` rather than `integrity_check`: it catches the damage a torn
/// copy actually produces and takes a fraction of the time on a 31 MB file.
fn check(path: &Path) -> Result<()> {
    let db = crate::library::db::Db::open_readonly(path)?;
    let verdict: String = db
        .conn
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .context("running quick_check")?;
    anyhow::ensure!(verdict == "ok", "quick_check said: {verdict}");
    // An index with no tracks is not corrupt, but it is not usable either,
    // and saying so here beats an empty browser with no explanation.
    let n = db.track_count().unwrap_or(0);
    anyhow::ensure!(n > 0, "the index holds no tracks");
    Ok(())
}

fn stamp_path(local: &Path) -> PathBuf {
    with_suffix(local, ".stamp")
}

/// Append to a path's file name rather than replacing its extension.
///
/// `Path::with_extension` would turn `index.sqlite` into `index-wal`, which is
/// a different file entirely and not the one SQLite is looking for.
fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `with_extension` is the obvious call here and it is wrong: SQLite
    /// wants `index.sqlite-wal`, not `index-wal`.
    #[test]
    fn a_sidecar_keeps_the_whole_file_name() {
        let p = Path::new("/cache/remote/abc.sqlite");
        assert_eq!(
            with_suffix(p, "-wal"),
            Path::new("/cache/remote/abc.sqlite-wal")
        );
        assert_eq!(stamp_path(p), Path::new("/cache/remote/abc.sqlite.stamp"));
    }

    #[test]
    fn every_host_gets_its_own_copy() {
        let a = local_copy("music").unwrap();
        let b = local_copy("laptop").unwrap();
        assert_ne!(a, b);
        assert_eq!(a, local_copy("music").unwrap(), "and a stable one");
    }
}
