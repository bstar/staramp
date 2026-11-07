//! Refresh a selected system palette without touching playback or saved settings.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{base16, color::Rgb, system, Theme};
use starkit::theme::{Resolve, ThemeFile};

#[derive(PartialEq, Eq)]
struct Snapshot {
    name: Option<String>,
    colors: BTreeMap<String, Rgb>,
    dark: bool,
}

#[derive(Default)]
pub struct SystemThemeWatch {
    last_check: Option<Instant>,
    snapshot: Option<Snapshot>,
}

impl SystemThemeWatch {
    pub fn poll(&mut self, selected: &str, now: Instant) -> Option<Theme> {
        if !selected.eq_ignore_ascii_case("system") && !selected.eq_ignore_ascii_case("auto") {
            self.last_check = None;
            self.snapshot = None;
            return None;
        }
        if self
            .last_check
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return None;
        }
        self.last_check = Some(now);
        self.update(system::detect().0, system::prefers_dark())
    }

    fn update(&mut self, scheme: Option<base16::Base16Scheme>, dark: bool) -> Option<Theme> {
        // Keep the last usable palette while a source is missing or being replaced.
        let scheme = scheme.filter(base16::Base16Scheme::is_complete)?;
        let snapshot = Snapshot {
            name: scheme.name.clone(),
            colors: scheme.colors.clone(),
            dark,
        };
        if self.snapshot.as_ref() == Some(&snapshot) {
            return None;
        }
        let source = base16::to_theme_toml(&scheme, "system", if dark { "dark" } else { "light" });
        let file = ThemeFile::parse(&source).ok()?;
        let theme = Theme::resolve(&file);
        self.snapshot = Some(snapshot);
        Some(theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(background: &str) -> base16::Base16Scheme {
        let mut source = (0..16)
            .map(|n| format!("base{n:02X}: \"#a9b665\"\n"))
            .collect::<String>();
        source.push_str(&format!("base00: \"{background}\"\nbase05: \"#d4be98\"\n"));
        base16::parse(&source)
    }

    #[test]
    fn a_changed_system_palette_updates_the_player_colors() {
        let mut watch = SystemThemeWatch::default();
        let first = watch.update(Some(palette("#282828")), true).unwrap();
        assert!(watch.update(Some(palette("#282828")), true).is_none());
        let changed = watch.update(Some(palette("#1f1f28")), true).unwrap();
        assert_ne!(first.bg, changed.bg);
        assert_eq!(changed.bg, Rgb::new(31, 31, 40));
        assert!(watch.update(Some(palette("#1f1f28")), true).is_none());
    }

    #[test]
    fn missing_or_partial_sources_keep_the_last_good_palette() {
        let mut watch = SystemThemeWatch::default();
        watch.update(Some(palette("#282828")), true).unwrap();
        assert!(watch.update(None, true).is_none());
        assert!(watch
            .update(Some(base16::parse("base00: '#111111'")), true)
            .is_none());
        assert!(watch.update(Some(palette("#282828")), true).is_none());
        assert!(watch.update(Some(palette("#1f1f28")), true).is_some());
    }

    #[test]
    fn explicit_themes_stop_following_the_system() {
        let mut watch = SystemThemeWatch::default();
        watch.update(Some(palette("#282828")), true).unwrap();
        assert!(watch.poll("nord", Instant::now()).is_none());
        assert!(watch.snapshot.is_none());
        assert!(watch.update(Some(palette("#282828")), true).is_some());
    }

    #[test]
    fn system_polling_is_throttled() {
        let now = Instant::now();
        let mut watch = SystemThemeWatch {
            last_check: Some(now),
            ..Default::default()
        };
        assert!(watch
            .poll("system", now + Duration::from_millis(500))
            .is_none());
        assert_eq!(watch.last_check, Some(now));
    }
}
