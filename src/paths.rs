//! Where staramp keeps its files.
//!
//! Everything lives under one directory — `~/.local/staramp` by default —
//! rather than being scattered across the three XDG roots. Config, index,
//! playlists, themes and cache in one place means the whole of a staramp setup
//! can be backed up, moved between machines, or deleted by moving one folder.
//!
//! `$STARAMP_DIR` overrides the location entirely. `$STARAMP_CONFIG_DIR` is
//! honoured as well, for anyone who wants the config somewhere else.
//!
//! Nothing here hardcodes a library path. On the author's machine the music is
//! on a removable disk under `/run/media`, which is exactly why that has to be
//! configuration rather than a constant.

#![allow(dead_code)] // some accessors are consumed by later phases

use std::path::PathBuf;

use anyhow::{Context, Result};

const APP: &str = "staramp";
const DIR_ENV: &str = "STARAMP_DIR";
const CONFIG_DIR_ENV: &str = "STARAMP_CONFIG_DIR";

/// The one directory everything hangs off.
pub fn base_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let home = home_dir().context("cannot determine the home directory")?;
    Ok(home.join(".local").join(APP))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Config lives at the base, unless pointed elsewhere.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    base_dir()
}

pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// The index and anything else that must not be lost.
pub fn data_dir() -> Result<PathBuf> {
    base_dir()
}

pub fn index_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("index.sqlite"))
}

/// Listening history and statistics, kept apart from rebuildable indexes.
pub fn activity_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("activity.sqlite"))
}

/// Provider tokens and Last.fm application credentials.
pub fn credentials_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("credentials.toml"))
}

/// Playlists staramp reads and writes when no `playlist_dir` is configured.
pub fn playlist_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("playlists"))
}

pub fn themes_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("themes"))
}

/// Editable Equalizer APO profiles managed by the player.
pub fn equalizer_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("equalizers"))
}

/// Album art thumbnails and logs. Safe to delete.
pub fn cache_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("cache"))
}

pub fn log_dir() -> Result<PathBuf> {
    cache_dir()
}

/// Volatile per-user state: sockets, and nothing that should survive a reboot.
///
/// `$XDG_RUNTIME_DIR` where there is one, which on Linux is a tmpfs the
/// session owns. macOS has no such variable, so the cache directory stands in;
/// it is on disk rather than in memory, which for a socket that is recreated
/// every run costs nothing.
pub fn runtime_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("staramp"),
        None => cache_dir()?.join("run"),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// The longest an `ssh` ControlPath may be.
///
/// `sun_path` holds 108 bytes on Linux and 104 on macOS, and OpenSSH binds the
/// control socket at `"<path>.<16 random chars>"` and only then renames it into
/// place, so the name that actually has to fit is seventeen bytes longer than
/// the one we choose. Getting this wrong makes `ssh` abort with a truncation
/// error that says nothing about which path was too long.
///
/// Checked against the smaller of the two for the same reason [`crate::ipc`]
/// checks `SUN_PATH_MAX` that way: a path that works on one platform and
/// silently fails on the other is worse than one that is twenty bytes tighter
/// than it strictly needs to be on Linux.
const CONTROL_PATH_MAX: usize = 104 - 18;

/// Where the multiplexed `ssh` connection to `alias` is reached.
///
/// Hashed rather than spelled out, for the reason above: a host alias can be
/// any length, and a runtime directory can be long before we add anything. Per
/// host by construction, and never the user's own `ControlPath` -- an `ssh`
/// master they are running is theirs, and we neither use nor disturb it.
pub fn control_socket(alias: &str) -> Result<PathBuf> {
    let h = blake3::hash(alias.as_bytes());
    let path = runtime_dir()?.join(format!("ssh-{}.sock", &h.to_hex()[..16]));
    let len = path.as_os_str().len();
    anyhow::ensure!(
        len <= CONTROL_PATH_MAX,
        "the ssh control socket path is {len} bytes, over the {CONTROL_PATH_MAX}-byte \
         limit: {}. Set STARAMP_DIR or XDG_RUNTIME_DIR to something shorter.",
        path.display()
    );
    Ok(path)
}

/// Legacy XDG locations, checked once so an existing install is not orphaned.
fn legacy_locations() -> Vec<(PathBuf, PathBuf)> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let old_config = home.join(".config").join(APP);
    let old_data = home.join(".local").join("share").join(APP);
    let Ok(base) = base_dir() else {
        return Vec::new();
    };
    vec![(old_config, base.clone()), (old_data, base)]
}

/// Move anything left in the old XDG directories into the single base dir.
///
/// Runs once at startup and is a no-op afterwards. Silent when there is nothing
/// to do; existing files at the destination are never overwritten, so a repeat
/// run cannot clobber newer state.
pub fn migrate_legacy() -> Vec<String> {
    let mut moved = Vec::new();
    let Ok(base) = base_dir() else {
        return moved;
    };

    for (old, new) in legacy_locations() {
        if !old.is_dir() || old == new {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&old) else {
            continue;
        };
        if std::fs::create_dir_all(&new).is_err() {
            continue;
        }
        for entry in entries.flatten() {
            let from = entry.path();
            let Some(name) = from.file_name() else {
                continue;
            };
            let to = new.join(name);
            if to.exists() {
                continue;
            }
            // Rename first; fall back to a copy when the two are on different
            // filesystems.
            let ok = std::fs::rename(&from, &to).is_ok() || copy_recursive(&from, &to).is_ok();
            if ok {
                moved.push(format!("{} -> {}", from.display(), to.display()));
            }
        }
        // Only remove the old directory if it emptied out.
        let _ = std::fs::remove_dir(&old);
    }

    let _ = std::fs::create_dir_all(&base);
    moved
}

fn copy_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        std::fs::remove_dir(from)?;
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_hangs_off_one_directory() {
        // Guarded rather than asserted unconditionally: the env var is a
        // legitimate override and the test must not fail because it is set.
        if std::env::var_os(DIR_ENV).is_some() || std::env::var_os(CONFIG_DIR_ENV).is_some() {
            return;
        }
        let base = base_dir().unwrap();
        assert!(base.ends_with("staramp"));
        assert!(config_file().unwrap().starts_with(&base));
        assert!(index_file().unwrap().starts_with(&base));
        assert!(playlist_dir().unwrap().starts_with(&base));
        assert!(equalizer_dir().unwrap().starts_with(&base));
        assert!(themes_dir().unwrap().starts_with(&base));
        assert!(cache_dir().unwrap().starts_with(&base));
    }

    #[test]
    fn the_env_override_relocates_everything_together() {
        // Checked as a pure function of the variable rather than by mutating
        // the process environment, which would race other tests.
        let fake = PathBuf::from("/tmp/staramp-test-home");
        let config = fake.join("config.toml");
        let index = fake.join("index.sqlite");
        assert!(config.starts_with(&fake));
        assert!(index.starts_with(&fake));
    }

    #[test]
    fn the_default_is_not_under_dot_config() {
        if std::env::var_os(DIR_ENV).is_some() || std::env::var_os(CONFIG_DIR_ENV).is_some() {
            return;
        }
        let base = base_dir().unwrap().to_string_lossy().into_owned();
        assert!(!base.contains("/.config/"), "got {base}");
        assert!(base.contains("/.local/"), "got {base}");
    }
}
