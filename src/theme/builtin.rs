//! Themes compiled into the binary, so a fresh install has a working look
//! without shipping a data directory alongside it.

use super::resolve::Theme;
use super::schema::ThemeFile;

pub struct Builtin {
    pub id: &'static str,
    pub toml: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "winamp-classic",
        toml: include_str!("../../themes/winamp-classic.toml"),
    },
    Builtin {
        id: "cosmic",
        toml: include_str!("../../themes/cosmic.toml"),
    },
    Builtin {
        id: "catppuccin-mocha",
        toml: include_str!("../../themes/catppuccin-mocha.toml"),
    },
    Builtin {
        id: "catppuccin-latte",
        toml: include_str!("../../themes/catppuccin-latte.toml"),
    },
    Builtin {
        id: "gruvbox-dark",
        toml: include_str!("../../themes/gruvbox-dark.toml"),
    },
    Builtin {
        id: "nord",
        toml: include_str!("../../themes/nord.toml"),
    },
    Builtin {
        id: "tokyo-night",
        toml: include_str!("../../themes/tokyo-night.toml"),
    },
    Builtin {
        id: "dracula",
        toml: include_str!("../../themes/dracula.toml"),
    },
    Builtin {
        id: "rose-pine",
        toml: include_str!("../../themes/rose-pine.toml"),
    },
    Builtin {
        id: "everforest",
        toml: include_str!("../../themes/everforest.toml"),
    },
    Builtin {
        id: "solarized-dark",
        toml: include_str!("../../themes/solarized-dark.toml"),
    },
    Builtin {
        id: "one-dark",
        toml: include_str!("../../themes/one-dark.toml"),
    },
    Builtin {
        id: "kanagawa",
        toml: include_str!("../../themes/kanagawa.toml"),
    },
    Builtin {
        id: "ayu-dark",
        toml: include_str!("../../themes/ayu-dark.toml"),
    },
    Builtin {
        id: "matte-black",
        toml: include_str!("../../themes/matte-black.toml"),
    },
    Builtin {
        id: "terminal",
        toml: include_str!("../../themes/terminal.toml"),
    },
];

pub const DEFAULT_ID: &str = "winamp-classic";

pub fn load(id: &str) -> Option<Theme> {
    let b = BUILTINS.iter().find(|b| b.id == id)?;
    ThemeFile::parse(b.toml).ok().map(|f| Theme::resolve(&f))
}

pub fn default_theme() -> Theme {
    load(DEFAULT_ID).expect("the default theme must always parse")
}

pub fn ids() -> Vec<&'static str> {
    BUILTINS.iter().map(|b| b.id).collect()
}

/// Resolve a theme by name, in the order a user would expect.
///
/// `"system"` follows the desktop; a user theme overrides a built-in of the
/// same id; and an unknown name falls back rather than refusing to start,
/// because a typo in a config file should not stop the music.
pub fn resolve_named(name: &str) -> (Theme, String) {
    if name.eq_ignore_ascii_case("system") || name.eq_ignore_ascii_case("auto") {
        if let Some((t, source)) = super::system::theme() {
            return (t, format!("system theme via {source}"));
        }
        return (default_theme(), "system theme not detected".into());
    }

    if let Ok(dir) = crate::paths::themes_dir() {
        let path = dir.join(format!("{name}.toml"));
        if path.is_file() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match ThemeFile::parse(&text) {
                    Ok(f) => return (Theme::resolve(&f), format!("user theme {name}")),
                    Err(e) => {
                        return (default_theme(), format!("{name}: {e}"));
                    }
                }
            }
        }
    }

    match load(name) {
        Some(t) => (t, format!("built-in {name}")),
        None => (default_theme(), format!("no theme `{name}`")),
    }
}

/// Every theme a picker should offer: built-ins, plus `system`.
pub fn selectable() -> Vec<String> {
    let mut v = vec!["system".to_string()];
    v.extend(ids().iter().map(|s| s.to_string()));
    if let Ok(dir) = crate::paths::themes_dir() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("toml") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        if !v.iter().any(|x| x == stem) {
                            v.push(stem.to_string());
                        }
                    }
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_resolves() {
        for b in BUILTINS {
            let f = ThemeFile::parse(b.toml)
                .unwrap_or_else(|e| panic!("{} failed to parse: {e}", b.id));
            let t = Theme::resolve(&f);
            assert_eq!(t.id, b.id, "id mismatch for {}", b.id);
        }
    }

    #[test]
    fn system_resolves_to_something_usable_even_with_no_desktop() {
        let (t, why) = resolve_named("system");
        assert_eq!(t.vis_ramp.len(), 16);
        assert!(!why.is_empty());
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

    #[test]
    fn winamp_classic_carries_the_real_viscolor_ramp() {
        let t = load("winamp-classic").unwrap();
        use super::super::color::Rgb;
        // Sixteen hard steps from VISCOLOR.TXT, not a derived blend.
        assert_eq!(t.vis_ramp[0], Rgb::parse_hex("#218c00").unwrap());
        assert_eq!(t.vis_ramp[15], Rgb::parse_hex("#ef3110").unwrap());
        assert_eq!(t.vis_peak_fg, Rgb::parse_hex("#ffffff").unwrap());
        assert_eq!(t.vis_grid_fg, Rgb::parse_hex("#182129").unwrap());
    }

    #[test]
    fn cosmic_uses_the_system_palette_and_a_readable_dim() {
        use super::super::color::Rgb;
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
    fn every_builtin_is_legible() {
        // Body text against its own background, WCAG AA for normal text.
        for b in BUILTINS {
            let t = load(b.id).unwrap();
            let c = t.bg.contrast(t.fg);
            assert!(c >= 4.5, "{}: body text contrast is only {c:.2}:1", b.id);

            let sel = t.row_selected_bg.contrast(t.row_selected_fg);
            assert!(sel >= 4.5, "{}: selected row contrast {sel:.2}:1", b.id);

            let play = t.bg.contrast(t.row_playing_fg);
            assert!(play >= 3.0, "{}: playing row contrast {play:.2}:1", b.id);

            // dim carries hints, track numbers and durations, so it is text.
            let dim = t.bg.contrast(t.dim);
            assert!(dim >= 4.5, "{}: dim text contrast is only {dim:.2}:1", b.id);
        }
    }
}
