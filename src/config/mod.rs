//! Configuration.
//!
//! Nothing here has a hardcoded default that points at one machine. The library
//! root in particular is configuration, because on the author's system it lives
//! on a removable disk under `/run/media` and will not always be present.

pub mod edit;

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where the music lives.
    pub library_root: Option<PathBuf>,
    /// Directory of `.m3u` playlists, read and written in place. Pointing this
    /// at MPD's own playlist directory is the supported arrangement.
    pub playlist_dir: Option<PathBuf>,
    pub theme: String,
    pub volume: f32,
    pub output: Output,
    pub art: Art,
    pub cue: Cue,
    pub fx: Fx,
    pub ui: Ui,
    pub vis: Vis,
    pub playlist: Playlist,
    pub eq: Eq,
    #[serde(rename = "replaygain")]
    pub rg: ReplayGainCfg,
    pub session: SessionShare,
    pub remote: Remote,
}

/// A library on another machine, reached over SSH.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Remote {
    /// An ssh destination: a host, a `user@host`, or an alias out of
    /// `~/.ssh/config`. Whatever `ssh` itself would accept, because it is
    /// `ssh` that is handed it.
    pub host: Option<String>,
    /// `library_root` as the *far* machine sees it. `~` is expanded there.
    pub root: Option<String>,
    /// How much of a track to keep buffered ahead of the decoder.
    ///
    /// This is what a dropped link is ridden out on, so it is measured in
    /// seconds of music rather than bytes of file: sixteen mebibytes is about
    /// forty-five seconds of a normal rip.
    pub readahead_mb: Option<u64>,
}

/// How the queue is ordered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Playlist {
    /// `off`, or `album` to order the queue by record, oldest first.
    ///
    /// This reorders playback, not only the list: what follows a record's last
    /// track is the next record's first. A string rather than a switch so the
    /// next grouping -- by artist, by genre -- needs no change to the file.
    pub group_by: String,
    /// Newest records first instead of oldest.
    pub group_desc: bool,
    pub shuffle: bool,
    /// `off`, `all` or `one`.
    pub repeat: String,
}

/// What several windows of the same session share.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionShare {
    /// `view` -- cursor, folds and open panels move together, so two windows
    /// are two views of one screen. `playback` -- only the music is shared and
    /// each window keeps its own place in the list.
    pub share: String,
}

impl Default for SessionShare {
    fn default() -> Self {
        Self {
            share: "view".into(),
        }
    }
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            group_by: "off".into(),
            group_desc: false,
            shuffle: false,
            repeat: "off".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
    /// `native` follows the file's own rate for bit-perfect playback;
    /// `fixed` pins the device and resamples.
    pub mode: String,
    pub fixed_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Vis {
    /// bars · leds · peaks · dots · wave · scope · cava · off
    pub mode: String,
    /// Shifts the analyzer's range, in dB. Raise it if your music sits quiet
    /// and the bars barely move; lower it if they spend their time pinned.
    ///
    /// The `cava` mode ignores this: it scales itself to the material.
    pub gain_db: f32,
    /// How much the `cava` mode smooths, 0 to 1. Higher is more fluid and
    /// slower to react; lower snaps to the music.
    pub smoothing: f64,
    /// Cells per bar, and blank columns between bars.
    ///
    /// The gap has to be a whole column -- no glyph is both part-height and
    /// part-width, so a bar's tip cannot carry a narrow one. Widen the bar to
    /// make the gap a smaller fraction of it, or set `bar_gap = 0` for a solid
    /// spectrum.
    pub bar_width: u16,
    pub bar_gap: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// Blank columns kept either side of the window.
    ///
    /// Defaults to 1: panel borders sitting flush against the terminal edge
    /// read as part of the terminal rather than as the player's own frame.
    /// Vertical padding defaults to 0 because rows are scarcer than columns and
    /// the top and bottom edges do not have the same problem.
    pub padding_x: u16,
    pub padding_y: u16,
    /// Transport button faces: `unicode`, `block`, `nerd`, or `ascii`.
    ///
    /// `nerd` needs a patched font installed *and selected in the terminal* --
    /// a terminal program cannot choose its own typeface, so this only says
    /// which codepoints to emit.
    pub glyphs: String,
    /// How the seek bar is drawn: `ansi`, `bar`, `thin`, or `blocks`.
    ///
    /// `ansi` is plain characters, which every font has. The others draw box
    /// or block glyphs and fill by eighths of a cell, so they creep rather
    /// than stepping -- at the cost of needing a font that carries them.
    pub seek_style: String,
    /// How album covers are drawn: `auto`, `kitty`, `blocks`, or `off`.
    ///
    /// `auto` asks the terminal. Detection is right nearly always and wrong
    /// over ssh and inside multiplexers, which is what the override is for.
    pub graphics: String,
    /// Which panels are open, remembered from the last session.
    ///
    /// Written whenever one is opened or closed, so the window comes back the
    /// shape it was left in rather than the shape it shipped as.
    pub show_album: bool,
    pub show_equalizer: bool,
    pub show_playlist: bool,
}

/// The ten-band equalizer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Eq {
    pub enabled: bool,
    /// Which named preset was last chosen, so the list comes back where it was
    /// left. `gains` is what is actually applied: a curve adjusted by hand is
    /// no longer the preset it started from, and the preset name alone would
    /// throw the adjustment away.
    pub preset: String,
    pub preamp: f32,
    /// Ten band gains in dB, 70 Hz to 16 kHz. Any other length is ignored, so
    /// a hand-edited file cannot half-apply a curve.
    pub gains: Vec<f32>,
}

impl Default for Eq {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: "Flat".into(),
            preamp: 0.0,
            gains: vec![0.0; 10],
        }
    }
}

impl Eq {
    /// The saved curve, or a flat one if the file says something unusable.
    pub fn band_gains(&self) -> [f32; 10] {
        let mut out = [0.0f32; 10];
        if self.gains.len() == out.len() {
            out.copy_from_slice(&self.gains);
        }
        out
    }
}

/// Album art.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Art {
    /// Look covers up on the Cover Art Archive when the files have none.
    ///
    /// Off unless asked for. Finding a cover this way means sending an artist
    /// and an album name to a third party, and that is a decision to be made
    /// rather than a favour to be done.
    pub fetch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Fx {
    pub enabled: bool,
    /// One switch that turns every animation off, for people who want that and
    /// for anyone the motion bothers.
    pub reduced_motion: bool,
    /// Effect for the track-title transition.
    pub track_change: String,
    pub duration_ms: u64,
    /// Let the spectrum drive effect speed.
    pub reactive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Cue {
    /// Which track a pregap belongs to: `previous` keeps the audio attached to
    /// the track before it, which makes the virtual tracks a complete partition
    /// of the file.
    pub pregap: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            library_root: None,
            remote: Remote::default(),
            playlist_dir: None,
            theme: "winamp-classic".into(),
            volume: 1.0,
            art: Art::default(),
            output: Output::default(),
            cue: Cue::default(),
            fx: Fx::default(),
            ui: Ui::default(),
            playlist: Playlist::default(),
            eq: Eq::default(),
            rg: ReplayGainCfg::default(),
            session: SessionShare::default(),
            vis: Vis::default(),
        }
    }
}

impl Default for Vis {
    fn default() -> Self {
        Self {
            mode: "bars".into(),
            smoothing: crate::vis::cava::DEFAULT_SMOOTHING,
            bar_width: 3,
            bar_gap: 1,
            gain_db: 0.0,
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            padding_x: 1,
            padding_y: 0,
            glyphs: "unicode".into(),
            seek_style: "ansi".into(),
            graphics: "auto".into(),
            // What a first run opens with, and what every run after it opens
            // with is whatever the last one was left as.
            show_album: false,
            show_equalizer: false,
            show_playlist: true,
        }
    }
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            enabled: true,
            reduced_motion: false,
            // Short: this fires on every track change.
            track_change: "decrypt".into(),
            duration_ms: 400,
            reactive: true,
        }
    }
}

impl Fx {
    /// Effects are off under NO_COLOR, a dumb terminal, or when not a TTY --
    /// none of those are places an animation belongs.
    pub fn active(&self) -> bool {
        if !self.enabled || self.reduced_motion {
            return false;
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
            return false;
        }
        true
    }
}

impl Output {
    /// The rate to pin the device to, or `None` to follow each file.
    ///
    /// Anything other than `fixed` follows the file, including a typo: a
    /// misspelled mode should not silently cost bit-perfect playback.
    pub fn fixed_rate(&self) -> Option<u32> {
        (self.mode.eq_ignore_ascii_case("fixed") && self.fixed_rate > 0).then_some(self.fixed_rate)
    }
}

impl Default for Output {
    fn default() -> Self {
        Self {
            mode: "native".into(),
            fixed_rate: 48_000,
        }
    }
}

impl Default for Cue {
    fn default() -> Self {
        Self {
            pregap: "previous".into(),
        }
    }
}

/// ReplayGain: how loud a track should be relative to the others.
///
/// Off by default. It changes what you hear, and a player that quietly turns
/// half a library down on first run has made a decision that was not its to
/// make -- especially when only some of the library carries the tags.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ReplayGainCfg {
    /// `off`, `track` -- each track levelled on its own, right for shuffle --
    /// or `album`, which keeps the loudness the artist intended between the
    /// tracks of a record.
    pub mode: String,
    /// Added on top, in dB. ReplayGain targets a conservative level and many
    /// people want some of it back.
    pub preamp: f32,
    /// Pull the gain back when a track's peak says it would clip. On, because
    /// ReplayGain plus a positive preamp clips audibly on loud masters.
    pub prevent_clipping: bool,
}

impl Default for ReplayGainCfg {
    fn default() -> Self {
        Self {
            mode: "off".into(),
            preamp: 0.0,
            prevent_clipping: true,
        }
    }
}

impl ReplayGainCfg {
    pub fn mode(&self) -> crate::audio::dsp::gain::RgMode {
        use crate::audio::dsp::gain::RgMode;
        match self.mode.trim().to_ascii_lowercase().as_str() {
            "track" => RgMode::Track,
            "album" => RgMode::Album,
            _ => RgMode::Off,
        }
    }
}

/// The starting config, with the settings worth knowing about explained.
///
/// Only the ones a listener would reach for. Everything else has a default that
/// is right until it is not, and `staramp` writes those back here itself when
/// they are changed from inside the player.
const TEMPLATE: &str = r#"# staramp configuration.
#
# Everything staramp keeps lives under one directory -- this file, the index,
# playlists, themes and cache. $STARAMP_DIR relocates all of it.

# Where your music lives. This is the one setting with no sensible default.
# Set it, then run `staramp scan` once to build the index.
# library_root = "/path/to/music"

# Directory of .m3u playlists, read and written in place. Defaults to the
# playlists/ folder next to this file. Point it at MPD's own playlist directory
# to share one set: staramp writes the same URI form MPD does, including
# Album/rip.cue/track0007.
# playlist_dir = "/home/you/.config/mpd/playlists"

# "system" follows the desktop (Stylix or COSMIC). Or name one:
# `staramp theme list` shows what is available.
theme = "winamp-classic"

# 0.0 to 1.0.
volume = 1.0

[ui]
# How album covers are drawn: auto, kitty, blocks, or off.
graphics = "auto"
# Transport button faces: unicode, block, nerd, ascii.
# `nerd` needs a patched font selected in your terminal.
glyphs = "unicode"

[art]
# Look covers up on the Cover Art Archive when the disk has none. Off by
# default: a lookup sends an artist and album name to a third party.
fetch = false

[eq]
enabled = false
preset = "Flat"

[replaygain]
# Level tracks to a common loudness using the gain tags already in your files.
# "album" keeps the loudness the artist set between the tracks of a record,
# "track" levels each one on its own, "off" plays them as they are. Off by
# default: it changes what you hear, and only some libraries carry the tags.
mode = "off"
# Added on top, in dB, if ReplayGain leaves things quieter than you like.
preamp = 0.0
prevent_clipping = true

# A library on another machine, played over SSH. No server to install and
# nothing to leave running there: staramp opens one ssh connection and reads
# the files through it, the way it would read a disk.
#
# The far machine needs staramp installed and scanned, so that there is an
# index to copy; `ssh <host>` must already work without a password prompt,
# because there is nowhere to show one.
#
# [remote]
# host = "music-server"
# root = "~/Music"
"#;

impl Config {
    pub fn load() -> Result<Self> {
        let path = crate::paths::config_file()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write a commented starting config, if there is not one already.
    ///
    /// Serialising the defaults would produce a correct file that teaches
    /// nobody anything: the values that matter are the ones with no sensible
    /// default -- `library_root` above all -- and a bare dump does not say
    /// which those are. So this is a template, and it is written once.
    ///
    /// Returns whether it created the file, so a first run can say so.
    pub fn write_template() -> Result<bool> {
        let path = crate::paths::config_file()?;
        if path.exists() {
            return Ok(false);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, TEMPLATE).with_context(|| format!("writing {}", path.display()))?;
        Ok(true)
    }

    pub fn save(&self) -> Result<()> {
        let path = crate::paths::config_file()?;
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Where playlists are read from, defaulting to the one inside staramp's
    /// own directory.
    pub fn resolved_playlist_dir(&self) -> Option<PathBuf> {
        self.playlist_dir
            .clone()
            .or_else(|| crate::paths::playlist_dir().ok().filter(|p| p.is_dir()))
    }

    /// The library root, erroring with something actionable if unset.
    pub fn require_library_root(&self) -> Result<PathBuf> {
        self.library_root.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "no library root configured — run `staramp scan <dir>` first, \
                 or set library_root in {}",
                crate::paths::config_file()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "the config file".into())
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_do_not_point_at_any_particular_machine() {
        let c = Config::default();
        assert!(c.library_root.is_none());
        assert!(c.playlist_dir.is_none());
        assert_eq!(c.output.mode, "native", "bit-perfect by default");
    }

    #[test]
    fn round_trips_through_toml() {
        let c = Config {
            library_root: Some(PathBuf::from("/music")),
            theme: "matte-black".into(),
            ..Config::default()
        };
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.library_root, c.library_root);
        assert_eq!(back.theme, c.theme);
    }

    /// Every setting the player writes back, with a value that is not the
    /// default, in the section and under the name the player uses.
    fn every_written_setting() -> Vec<(&'static str, &'static str, edit::Value)> {
        use edit::Value::*;
        vec![
            (edit::ROOT, "theme", Str("nord".into())),
            (edit::ROOT, "volume", Float(0.35)),
            ("ui", "seek_style", Str("blocks".into())),
            ("ui", "graphics", Str("kitty".into())),
            ("ui", "show_album", Bool(true)),
            ("ui", "show_equalizer", Bool(true)),
            ("ui", "show_playlist", Bool(false)),
            ("vis", "mode", Str("leds".into())),
            ("vis", "bar_width", Int(5)),
            ("vis", "bar_gap", Int(2)),
            ("fx", "enabled", Bool(false)),
            ("art", "fetch", Bool(true)),
            ("playlist", "group_by", Str("album".into())),
            ("playlist", "group_desc", Bool(true)),
            ("playlist", "shuffle", Bool(true)),
            ("playlist", "repeat", Str("all".into())),
            ("eq", "enabled", Bool(true)),
            ("eq", "preset", Str("Rock".into())),
            ("eq", "preamp", Float(-2.5)),
            (
                "eq",
                "gains",
                Floats(vec![0.0, 1.5, -3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 6.0]),
            ),
        ]
    }

    #[test]
    fn everything_the_player_writes_back_reads_back() {
        // The round trip is the whole contract: a setting written into
        // `config.toml` by hand from the running player and then rejected by
        // the loader on the next launch would be worse than not saving it at
        // all, because the file would be broken as well.
        let mut text = String::new();
        for (section, key, value) in every_written_setting() {
            text = edit::apply(&text, section, key, &value);
        }
        let c: Config = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));

        assert_eq!(c.theme, "nord");
        assert!((c.volume - 0.35).abs() < 1e-3, "{}", c.volume);
        assert_eq!(c.ui.seek_style, "blocks");
        assert_eq!(c.ui.graphics, "kitty");
        assert!(c.ui.show_album && c.ui.show_equalizer && !c.ui.show_playlist);
        assert_eq!(c.vis.mode, "leds");
        assert_eq!((c.vis.bar_width, c.vis.bar_gap), (5, 2));
        assert!(!c.fx.enabled);
        assert!(c.art.fetch);
        assert_eq!(c.playlist.group_by, "album");
        assert!(c.playlist.group_desc && c.playlist.shuffle);
        assert_eq!(c.playlist.repeat, "all");
        assert!(c.eq.enabled);
        assert_eq!(c.eq.preset, "Rock");
        assert!((c.eq.preamp + 2.5).abs() < 1e-3);
        assert_eq!(c.eq.band_gains()[9], 6.0);
        assert_eq!(c.eq.band_gains()[2], -3.0);
    }

    #[test]
    fn writing_them_over_a_real_config_disturbs_nothing_else() {
        // The file people hand-edit, with its comments, written through the
        // same path the running player uses.
        let src = "# mine\nlibrary_root = \"/music\"\n\n[ui]\n# faces\nglyphs = \"nerd\"\n";
        let mut text = src.to_string();
        for (section, key, value) in every_written_setting() {
            text = edit::apply(&text, section, key, &value);
        }
        assert!(text.contains("# mine"), "{text}");
        assert!(text.contains("# faces"), "{text}");
        let c: Config = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(c.library_root, Some(PathBuf::from("/music")));
        assert_eq!(c.ui.glyphs, "nerd");
        assert_eq!(c.theme, "nord");
    }

    #[test]
    fn a_curve_of_the_wrong_length_is_ignored_rather_than_half_applied() {
        let c: Config = toml::from_str("[eq]\ngains = [1.0, 2.0]\n").unwrap();
        assert_eq!(c.eq.band_gains(), [0.0; 10]);
    }

    #[test]
    fn a_partial_config_file_fills_in_the_rest() {
        let c: Config = toml::from_str("theme = \"terminal\"\n").unwrap();
        assert_eq!(c.theme, "terminal");
        assert_eq!(c.volume, 1.0);
        assert_eq!(c.output.mode, "native");
    }

    #[test]
    fn the_default_visualizer_mode_is_a_real_one() {
        let c = Config::default();
        assert!(
            crate::vis::mode::VisMode::parse(&c.vis.mode).is_some(),
            "default mode {:?} does not exist",
            c.vis.mode
        );
    }

    #[test]
    fn a_partial_ui_section_keeps_the_other_default() {
        let c: Config = toml::from_str("[ui]\npadding_x = 4\n").unwrap();
        assert_eq!(c.ui.padding_x, 4);
        assert_eq!(c.ui.padding_y, 0);
    }

    #[test]
    fn there_is_horizontal_padding_by_default_but_not_vertical() {
        let c = Config::default();
        assert_eq!(c.ui.padding_x, 1);
        assert_eq!(c.ui.padding_y, 0, "rows are scarcer than columns");
    }

    #[test]
    fn reduced_motion_switches_every_effect_off() {
        let mut c = Config::default();
        assert!(c.fx.active());
        c.fx.reduced_motion = true;
        assert!(!c.fx.active());
    }

    #[test]
    fn no_color_disables_effects() {
        let c = Config::default();
        // Guarded so the assertion is meaningful either way the env is set.
        if std::env::var_os("NO_COLOR").is_some() {
            assert!(!c.fx.active());
        } else {
            assert!(c.fx.active());
        }
    }

    #[test]
    fn an_unset_library_root_gives_an_actionable_error() {
        let e = Config::default().require_library_root().unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("staramp scan"), "unhelpful message: {msg}");
    }
}
