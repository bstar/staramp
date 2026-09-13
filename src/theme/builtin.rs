//! Themes compiled into the binary, so a fresh install has a working look
//! without shipping a data directory alongside it.
//!
//! The themes, and the lookup that finds one, are starkit's; what is here is
//! the player's registry -- the same sixteen built-ins resolved into the
//! player's own [`Theme`] -- behind the free functions the rest of the crate
//! already called.

use std::sync::LazyLock;

use starkit::theme::Registry;

use super::Theme;

/// The player's view of the shared themes.
///
/// One for the process: it holds no mutable state, and the alternative is
/// threading it from `main` through every panel that wants to know what
/// `theme list` would print.
static REGISTRY: LazyLock<Registry<Theme>> = LazyLock::new(|| Registry::new(crate::paths::PATHS));

pub fn load(id: &str) -> Option<Theme> {
    REGISTRY.load(id)
}

pub fn default_theme() -> Theme {
    REGISTRY.default_theme()
}

pub fn ids() -> Vec<&'static str> {
    REGISTRY.ids()
}

/// Resolve a theme by name, in the order a user would expect.
///
/// `"system"` follows the desktop; a user theme overrides a built-in of the
/// same id; and an unknown name falls back rather than refusing to start,
/// because a typo in a config file should not stop the music.
pub fn resolve_named(name: &str) -> (Theme, String) {
    REGISTRY.resolve_named(name)
}

/// Every theme a picker should offer: built-ins, plus `system`.
pub fn selectable() -> Vec<String> {
    REGISTRY.selectable()
}

#[cfg(test)]
mod tests {
    use super::*;
    use starkit::theme::color::Rgb;
    use starkit::theme::{BUILTINS, DEFAULT_ID};

    #[test]
    fn winamp_classic_carries_the_real_viscolor_ramp() {
        let t = load("winamp-classic").unwrap();
        // Sixteen hard steps from VISCOLOR.TXT, not a derived blend.
        assert_eq!(t.vis_ramp[0], Rgb::parse_hex("#218c00").unwrap());
        assert_eq!(t.vis_ramp[15], Rgb::parse_hex("#ef3110").unwrap());
        assert_eq!(t.vis_peak_fg, Rgb::parse_hex("#ffffff").unwrap());
        assert_eq!(t.vis_grid_fg, Rgb::parse_hex("#182129").unwrap());
    }

    #[test]
    fn cosmic_uses_the_system_palette_and_a_readable_dim() {
        let t = load("cosmic").unwrap();
        assert_eq!(t.bg, Rgb::parse_hex("#1b1b1b").unwrap(), "base00");
        assert_eq!(t.accent, Rgb::parse_hex("#49bac8").unwrap(), "base0D cyan");
        // The scheme's own base03 is a border colour, not text: it only reaches
        // 2.5:1 here, so dim is deliberately lifted.
        let dim_contrast = t.bg.contrast(t.dim);
        assert!(dim_contrast >= 4.5, "dim is only {dim_contrast:.2}:1");
        // The ramp is COSMIC's own cyan, not a green-to-red VU: the
        // visualizer should look like the rest of the theme.
        assert_eq!(t.vis_ramp[0], Rgb::parse_hex("#17414a").unwrap());
        assert_eq!(t.vis_ramp[15], Rgb::parse_hex("#dff7fb").unwrap());
        // The signature cyan sits in the middle of it.
        let mid = t.vis_ramp[8];
        assert!(
            mid.b > mid.r && mid.g > mid.r,
            "the middle of the ramp is not cyan: {mid:?}"
        );
    }

    #[test]
    fn an_unknown_theme_falls_back_rather_than_failing() {
        // A typo in a config file should not stop the music.
        let (t, why) = resolve_named("no-such-theme");
        assert_eq!(t.id, DEFAULT_ID);
        assert!(why.contains("no theme"), "{why}");
    }

    #[test]
    fn the_selectable_list_offers_system_first() {
        let v = selectable();
        assert_eq!(v.first().map(|s| s.as_str()), Some("system"));
        assert!(v.iter().any(|s| s == "cosmic"));
        assert!(v.iter().any(|s| s == "winamp-classic"));
    }

    #[test]
    fn the_default_theme_exists() {
        let t = default_theme();
        assert_eq!(t.id, "winamp-classic");
    }

    /// The whole derivation chain, pinned byte for byte.
    ///
    /// `every_builtin_is_legible` -- which now lives in starkit, beside the
    /// roles it checks -- says the result is readable; this one says it is the
    /// *same* result as yesterday, for the player's roles as well as the
    /// shared ones. It is what made moving the resolver into starkit a
    /// refactor rather than a rewrite, because a single changed byte is a
    /// theme that no longer looks the way it did and nothing else in the suite
    /// would notice.
    ///
    /// Regenerate deliberately, after reading the diff:
    /// `STARAMP_UPDATE_GOLDEN=1 cargo test every_builtin_resolves_as_recorded`.
    #[test]
    fn every_builtin_resolves_as_recorded() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/theme-golden");
        let update = std::env::var_os("STARAMP_UPDATE_GOLDEN").is_some_and(|v| !v.is_empty());
        if update {
            std::fs::create_dir_all(&dir).expect("the golden directory has to be writable");
        }

        for b in BUILTINS {
            let got = load(b.id).expect("a built-in that no longer loads").dump();
            let path = dir.join(format!("{}.txt", b.id));

            if update {
                std::fs::write(&path, &got).expect("writing a golden file");
                continue;
            }

            let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e} -- regenerate with STARAMP_UPDATE_GOLDEN=1",
                    path.display()
                )
            });
            if got == want {
                continue;
            }
            // The dumps are eighty lines long, so say which role moved rather
            // than printing both of them.
            match got.lines().zip(want.lines()).find(|(g, w)| g != w) {
                Some((g, w)) => panic!("{}: resolves to `{g}`, recorded as `{w}`", b.id),
                None => panic!(
                    "{}: {} roles resolved, {} recorded",
                    b.id,
                    got.lines().count(),
                    want.lines().count()
                ),
            }
        }
    }
}
