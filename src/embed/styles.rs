//! STAR/FOLD-specific presentation preferences, separate from the standalone UI.

use std::fs;
use std::io;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::paths;
use crate::ui::panels::player::SeekStyle;
use crate::vis::mode::VisMode;

const MAX_PROFILE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: u32,
    visualizer: String,
    seek_style: String,
}

pub struct Styles {
    pub vis: VisMode,
    pub seek: SeekStyle,
    binding: Option<Option<String>>,
    path: Option<PathBuf>,
    writable: bool,
}

impl Styles {
    pub fn new(cfg: &Config) -> Self {
        Self {
            vis: VisMode::parse(&cfg.vis.mode).unwrap_or_default(),
            seek: SeekStyle::parse(&cfg.ui.seek_style).unwrap_or_default(),
            binding: None,
            path: None,
            writable: true,
        }
    }

    /// The first Configure fixes the profile for this process. Reconfigure
    /// never resets an already changed style.
    pub fn configure(&mut self, profile: Option<String>) -> Result<Option<String>> {
        if let Some(ref id) = profile {
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                bail!("invalid embed profile identifier");
            }
        }
        if let Some(bound) = &self.binding {
            if bound != &profile {
                bail!("embed profile cannot change during a session");
            }
            return Ok(None);
        }
        self.binding = Some(profile.clone());
        let Some(id) = profile else { return Ok(None) };
        let path = match paths::config_dir() {
            Ok(dir) => dir.join("embed").join(format!("{id}.toml")),
            Err(e) => {
                self.writable = false;
                return Ok(Some(format!(
                    "embed profile path unavailable: {e}; changes will not be saved"
                )));
            }
        };
        self.path = Some(path.clone());
        match load(&path) {
            Ok(Some((vis, seek))) => {
                self.vis = vis;
                self.seek = seek;
                Ok(None)
            }
            Ok(None) => Ok(None),
            Err(e) => {
                self.writable = false;
                Ok(Some(format!(
                    "embed profile {}: {e}; existing file will not be overwritten",
                    path.display()
                )))
            }
        }
    }

    pub fn next_vis(&mut self) -> Option<String> {
        self.vis = self.vis.next();
        self.save_notice()
    }

    pub fn prev_vis(&mut self) -> Option<String> {
        self.vis = self.vis.prev();
        self.save_notice()
    }

    pub fn next_seek(&mut self) -> Option<String> {
        self.seek = self.seek.next();
        self.save_notice()
    }

    fn save_notice(&self) -> Option<String> {
        if !self.writable {
            return Some("embed profile is invalid or unavailable; style changed for this session but was not saved".into());
        }
        let path = self.path.as_ref()?;
        save(path, self.vis, self.seek)
            .err()
            .map(|e| format!("could not save embed profile {}: {e}", path.display()))
    }
}

fn load(path: &PathBuf) -> Result<Option<(VisMode, SeekStyle)>> {
    if let Some(dir) = path.parent() {
        match fs::symlink_metadata(dir) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                bail!("profile directory is not a regular directory");
            }
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let preflight = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !preflight.file_type().is_file() || preflight.len() > MAX_PROFILE_BYTES {
        bail!("profile is not a regular file within the 16 KiB limit");
    }
    #[cfg(unix)]
    let open = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
    };
    #[cfg(not(unix))]
    let open = fs::File::open(path);
    let file = match open {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_PROFILE_BYTES {
        bail!("profile is not a regular file within the 16 KiB limit");
    }
    use std::io::Read;
    let mut text = String::new();
    file.take(MAX_PROFILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_PROFILE_BYTES {
        bail!("profile exceeds 16 KiB");
    }
    let data: ProfileFile = toml::from_str(&text).context("invalid profile TOML")?;
    if data.version != 1 {
        bail!("unsupported profile version");
    }
    let vis = VisMode::parse(&data.visualizer).context("invalid visualizer")?;
    let seek = SeekStyle::parse(&data.seek_style).context("invalid seek style")?;
    Ok(Some((vis, seek)))
}

fn save(path: &PathBuf, vis: VisMode, seek: SeekStyle) -> Result<()> {
    let dir = path.parent().context("profile has no parent")?;
    // Recheck on each save: a profile changed externally to malformed data
    // after Configure must not be replaced by an older in-memory style.
    let _ = load(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)?;
    if !fs::symlink_metadata(dir)?.file_type().is_dir() {
        bail!("profile directory is not a regular directory");
    }
    let data = ProfileFile {
        version: 1,
        visualizer: vis.name().into(),
        seek_style: seek.name().into(),
    };
    starkit::fs::write_private(path, toml::to_string_pretty(&data)?.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trip_and_bad_file_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starfold.toml");
        save(&path, VisMode::Peaks, SeekStyle::BAR).unwrap();
        assert_eq!(load(&path).unwrap(), Some((VisMode::Peaks, SeekStyle::BAR)));
        fs::write(&path, "garbage = [").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn style_cycles_wrap_native_order() {
        let mut s = Styles::new(&Config::default());
        for _ in 0..VisMode::all().len() {
            s.next_vis();
        }
        assert_eq!(s.vis, VisMode::parse(&Config::default().vis.mode).unwrap());
        let start = s.seek;
        for _ in 0..SeekStyle::ALL.len() {
            s.next_seek();
        }
        assert_eq!(s.seek, start);
    }

    #[test]
    fn malformed_existing_profile_is_not_replaced_by_style_controls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starfold.toml");
        let malformed = b"version = 1\nvisualizer = 'unknown'\nseek_style = 'bar'\n";
        fs::write(&path, malformed).unwrap();
        assert!(load(&path).is_err());

        // A failed load leaves the session usable, but explicitly disables
        // saves so a later key cannot destroy the original file.
        let mut styles = Styles::new(&Config::default());
        styles.binding = Some(Some("starfold".into()));
        styles.path = Some(path.clone());
        styles.writable = false;
        assert!(styles.next_vis().unwrap().contains("not saved"));
        assert!(styles.prev_vis().unwrap().contains("not saved"));
        assert!(styles.next_seek().unwrap().contains("not saved"));
        assert_eq!(fs::read(path).unwrap(), malformed);
    }

    #[test]
    fn profile_identifier_must_be_a_bounded_plain_name() {
        for invalid in ["", ".", "../starfold", "two/parts", "with space", "café"] {
            let mut styles = Styles::new(&Config::default());
            assert!(styles.configure(Some(invalid.into())).is_err(), "{invalid}");
            assert!(styles.binding.is_none(), "invalid name must not bind");
        }
        let mut styles = Styles::new(&Config::default());
        assert!(styles.configure(Some("a".repeat(65))).is_err());
        assert!(styles.binding.is_none());
    }

    #[test]
    fn reconfigure_with_same_profile_preserves_changed_styles() {
        let mut styles = Styles::new(&Config::default());
        styles.binding = Some(Some("starfold".into()));
        styles.vis = VisMode::Scope;
        styles.seek = SeekStyle::BLOCKS;
        assert_eq!(styles.configure(Some("starfold".into())).unwrap(), None);
        assert_eq!(styles.vis, VisMode::Scope);
        assert_eq!(styles.seek, SeekStyle::BLOCKS);
        assert!(styles.configure(Some("another".into())).is_err());
        assert_eq!(styles.vis, VisMode::Scope);
        assert_eq!(styles.seek, SeekStyle::BLOCKS);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_profile_is_rejected_without_touching_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let link = dir.path().join("starfold.toml");
        fs::write(&target, b"sentinel").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(load(&link).is_err());
        assert_eq!(fs::read(target).unwrap(), b"sentinel");
    }

    #[cfg(unix)]
    #[test]
    fn fifo_profile_is_rejected_without_blocking() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starfold.toml");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(load(&path).is_err());
    }
}
