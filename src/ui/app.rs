//! The application: state, event loop, and layout.
//!
//! Layout is Winamp's docked windows — main player, equalizer, playlist —
//! stacked, each independently show/hideable, each with its own border. That
//! structure is the point: it is what makes the thing read as Winamp rather
//! than as one more bordered box full of sections.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};

use crate::audio::dsp::eq::{self, EqSettings};
use crate::audio::player::{Command, PlayState, Player};
use crate::config::edit::Value;
use crate::fx::reactive::{reactive_dt, OnsetDetector};
use crate::fx::{EffectKind, TextEffect};
use crate::mirror::Mirror;
use crate::playlist::queue::QueueItem;
use crate::session::{self, Session};
use crate::theme::builtin;
use crate::theme::resolve::Theme;
use crate::ui::keymap::{self, Action};
use crate::ui::panels::album;
use crate::ui::panels::chooser::{self, ChooserView};
use crate::ui::panels::picker::{self, PickerView, PlaylistEntry};
use crate::ui::panels::playlist::{self, PlaylistView};
use crate::ui::panels::resume::ResumeView;
use crate::ui::panels::settings::{self, SettingsView};
use crate::ui::panels::{equalizer, equalizer::EqView, header, player, player::PlayerView};
use crate::ui::term;
use crate::vis::meter::Meters;
use crate::vis::mode::VisMode;
use crate::vis::spectrum::{Motion, Spectrum};

/// Animation cadence. The reference's separation of animation from analysis is
/// worth keeping: the FFT does not need to run at frame rate.
const FRAME: Duration = Duration::from_millis(33);

/// Below this the docked layout has nowhere to go.
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 8;

/// Padding per side, never so much that the layout is squeezed out.
fn clamp_padding(requested: u16, available: u16, minimum: u16) -> u16 {
    requested.min(available.saturating_sub(minimum) / 2)
}
/// Seconds for the seek bar's highlight to cross it once.
const SEEK_SHEEN_PERIOD: f32 = 3.5;

const MARQUEE_STEP: Duration = Duration::from_millis(200);

/// Where the docked windows sit.
///
/// Computed once and used by both the renderer and the mouse handler, so a
/// click and the thing it appears to be over are always the same rect.
struct Regions {
    /// The padded area everything is drawn inside.
    area: Rect,
    player: Rect,
    album: Option<Rect>,
    equalizer: Option<Rect>,
    playlist: Option<Rect>,
    status: Rect,
}

/// The cover chooser's state while it is open.
///
/// The rows are built once when it opens rather than every frame: they come
/// from the worker's published album, which can change underneath, and a list
/// that reorders itself while somebody is arrowing down it is a trap.
struct Chooser {
    /// The track it was opened for. Closing on a track change would be rude;
    /// choosing for the wrong album would be worse.
    uri: String,
    rows: Vec<chooser::Row>,
    /// How many leading rows are images already on disk. The rest are
    /// releases, indexed from this.
    local: usize,
    cursor: usize,
    scroll: usize,
}

/// What a settings row does when it is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Graphics,
    FetchArt,
    ChooseCover,
    RetryCover,
    Shuffle,
    Repeat,
    LoadPlaylist,
    EqEnabled,
    EqPreset,
    EqReset,
    GroupOrder,
    GroupDirection,
    ClearAlbumOrder,
    JoinSession,
    ReplaceQueue,
    AddAppend,
    AddReplace,
    AddCancel,
    SavePlaylist,
    SaveOverwrite,
    SaveAsNew,
    SaveCancel,
}

/// Which list the one overlay is showing.
///
/// A panel can offer more than one, so this rather than `panel` alone decides
/// whether a click on `settings` switches lists or closes the box, and which
/// rows a refresh rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Settings,
    Filter,
    /// A window opened on a playlist while a session was already running,
    /// asking what to do about it.
    Joining,
    /// What `space` in the browser should do with what it has picked.
    Adding,
    /// Where an edited playlist should be written.
    Saving,
}

impl Overlay {
    fn heading(self) -> &'static str {
        match self {
            Overlay::Settings => "SETTINGS",
            Overlay::Filter => "FILTER",
            Overlay::Joining => "ALREADY PLAYING",
            Overlay::Adding => "ADD",
            Overlay::Saving => "SAVE",
        }
    }
}

/// A panel's settings while they are open.
///
/// The rows are rebuilt after every change rather than patched, so what is on
/// screen is always what the setting actually is -- including when changing one
/// thing changes another's wording.
struct SettingsState {
    panel: Focus,
    kind: Overlay,
    title: String,
    rows: Vec<settings::Row>,
    items: Vec<Setting>,
    cursor: usize,
    scroll: usize,
}

/// Every setting that outlives the session, as `config.toml` last had it.
///
/// One place that knows what persists, diffed on the save timer rather than a
/// `remember` call beside every mutation. The panels alone are opened and
/// closed from four places each -- a key, the header's `close`, a settings
/// row, and the album panel opening itself to show a cover -- and a rule that
/// has to be remembered at each of them is a rule that will be forgotten at
/// one of them.
///
/// Seeded from the file at startup, so a setting that was never touched is
/// never written and a hand-edited config keeps its shape.
#[derive(Debug, Clone, PartialEq)]
struct Remembered {
    theme: String,
    volume: f32,
    seek_style: String,
    graphics: String,
    buttons: String,
    show_album: bool,
    show_equalizer: bool,
    show_playlist: bool,
    vis_mode: String,
    bar_width: u16,
    bar_gap: u16,
    animations: bool,
    fetch_art: bool,
    group_by: String,
    group_desc: bool,
    shuffle: bool,
    repeat: String,
    eq_enabled: bool,
    eq_preset: String,
    eq_preamp: f32,
    eq_gains: [f32; 10],
}

impl Remembered {
    fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            theme: cfg.theme.clone(),
            volume: cfg.volume,
            seek_style: cfg.ui.seek_style.clone(),
            graphics: cfg.ui.graphics.clone(),
            buttons: cfg.ui.buttons.clone(),
            show_album: cfg.ui.show_album,
            show_equalizer: cfg.ui.show_equalizer,
            show_playlist: cfg.ui.show_playlist,
            vis_mode: cfg.vis.mode.clone(),
            bar_width: cfg.vis.bar_width,
            bar_gap: cfg.vis.bar_gap,
            animations: cfg.fx.active(),
            fetch_art: cfg.art.fetch,
            group_by: cfg.playlist.group_by.clone(),
            group_desc: cfg.playlist.group_desc,
            shuffle: cfg.playlist.shuffle,
            repeat: cfg.playlist.repeat.clone(),
            eq_enabled: cfg.eq.enabled,
            eq_preset: cfg.eq.preset.clone(),
            eq_preamp: cfg.eq.preamp,
            eq_gains: cfg.eq.band_gains(),
        }
    }
}

/// What a held left button is adjusting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Seek,
    Volume,
    /// Sliding one EQ band up and down.
    EqBand(usize),
}

/// Where along a bar a click at `x` falls, 0 to 1.
///
/// The left edge is the start of the track, so this reads the cell's leading
/// edge -- seeking to a click should land where the pointer is, not half a cell
/// past it.
fn bar_fraction(bar: Rect, x: u16) -> f64 {
    if bar.width == 0 {
        return 0.0;
    }
    (x.saturating_sub(bar.x) as f64 / bar.width as f64).clamp(0.0, 1.0)
}

/// The level a click at `x` sets on a slider.
///
/// Unlike a seek bar this reads the cell's *trailing* edge, because a slider
/// cell is either lit or not: clicking the last cell has to mean full, and
/// clicking the first has to mean one step rather than zero.
/// What the mouse moves the volume by.
///
/// One cell of the ten-cell slider. The pointer and the bar step together, so
/// every position the mouse can ask for is one the bar can draw and every
/// drag position draws something new.
const VOLUME_STEP: f32 = 0.10;

/// What the keyboard moves it by.
///
/// A tenth of the mouse's step, for the levels between the ones the bar can
/// draw. Those move the readout and not the rectangles, which is the right
/// way round: the number is exact and the bar is a gauge.
const KEY_VOLUME_STEP: f32 = 0.01;

/// Round `v` to the nearest multiple of `step`.
///
/// For the pointer, which names an absolute position: the nearest level it
/// could have meant.
fn snap(v: f32, step: f32) -> f32 {
    if step <= 0.0 {
        return v.clamp(0.0, 1.0);
    }
    ((v / step).round() * step).clamp(0.0, 1.0)
}

/// Move `v` to the next multiple of `delta` in that direction.
///
/// For a key or a wheel, which name a direction rather than a place. Not
/// `snap(v + delta)`: from 37% that would round 32% down to 30%, a seven
/// point drop for one press. Stepping to the next position on the grid gives
/// 35%, and from a value already on the grid it moves a whole step rather
/// than standing still.
fn step_toward(v: f32, delta: f32) -> f32 {
    let step = delta.abs();
    if step <= 0.0 {
        return v.clamp(0.0, 1.0);
    }
    // The nudge keeps a value already on the grid from rounding to itself.
    let n = v / step;
    let next = if delta > 0.0 {
        (n + 1e-4).floor() + 1.0
    } else {
        (n - 1e-4).ceil() - 1.0
    };
    (next * step).clamp(0.0, 1.0)
}

fn slider_fraction(v: Rect, x: u16) -> f32 {
    if v.width == 0 {
        return 0.0;
    }
    ((x.saturating_sub(v.x) + 1) as f32 / v.width as f32).clamp(0.0, 1.0)
}

/// Is this cell inside the rect?
/// How a switch reads in a settings list.
fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Two clicks closer together than this, on the same cell, are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(450);

/// What the queue is called before any playlist is loaded into it.
const DEFAULT_QUEUE_NAME: &str = "queue";

/// Which playlist file a session should be resumed from.
///
/// The stored path wins. Falling back to the name matters for sessions written
/// before the picker recorded a path, and for a playlist directory that has
/// moved: without it, resuming drops you into whatever the queue happened to
/// be built from, which is the entire library.
fn resume_playlist_path(session: &Session, known: &[PlaylistEntry]) -> Option<PathBuf> {
    if let Some(p) = session.playlist.as_ref().filter(|p| p.is_file()) {
        return Some(p.clone());
    }
    if session.playlist_name.is_empty() {
        return None;
    }
    known
        .iter()
        .find(|e| e.name == session.playlist_name)
        .map(|e| e.path.clone())
}

/// What separates the status bar's state indicators.
const SEPARATOR: &str = "  \u{b7}  ";

/// The always-on state indicators, in the order they are drawn.
///
/// Each carries the key that changes it, so the bar doubles as a reminder, and
/// a flag saying whether it is lit -- which is per-segment because "shuffle is
/// on" and "repeat is on" are different facts.
fn status_indicators(
    vis: VisMode,
    shuffled: bool,
    repeat: crate::playlist::queue::RepeatMode,
) -> Vec<(String, bool)> {
    use crate::playlist::queue::RepeatMode;
    vec![
        (
            format!("w {}", vis.name().to_uppercase()),
            vis != VisMode::Off,
        ),
        (
            format!("s SHUF{}", if shuffled { " ON" } else { "" }),
            shuffled,
        ),
        (
            format!(
                "r {}",
                match repeat {
                    RepeatMode::Off => "REP",
                    RepeatMode::All => "REP ALL",
                    RepeatMode::One => "REP ONE",
                }
            ),
            repeat != RepeatMode::Off,
        ),
    ]
}

/// Total width of the indicators, separators included.
fn indicator_width(segments: &[(String, bool)]) -> u16 {
    let labels: usize = segments.iter().map(|(l, _)| l.chars().count()).sum();
    let gaps = segments.len().saturating_sub(1) * SEPARATOR.chars().count();
    (labels + gaps) as u16
}

/// The marquee line: `Artist — Title (Album)`.
///
/// `None` when there is no title to build around, so the caller can fall back
/// to the URI. The album is parenthesised and dropped entirely when unknown,
/// rather than leaving empty brackets on files with no album tag.
fn track_line(artist: Option<&str>, title: Option<&str>, album: Option<&str>) -> Option<String> {
    let title = title.map(str::trim).filter(|t| !t.is_empty())?;
    let mut line = match artist.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{a} — {title}"),
        None => title.to_string(),
    };
    if let Some(al) = album.map(str::trim).filter(|a| !a.is_empty()) {
        line.push_str(&format!(" ({al})"));
    }
    Some(line)
}

/// A codec name fit to show a person.
///
/// The decoders report the registry's own short name -- symphonia's
/// `pcm_s16le`, libav's `wmav2` and `dsd_lsbf_planar` -- which is precise but
/// not what anyone calls the format. Anything unrecognised is passed through
/// uppercased rather than hidden, so a format we have not listed still shows.
fn codec_label(codec: &str) -> String {
    let c = codec.to_ascii_lowercase();
    let named = match c.as_str() {
        "mp3" | "mp2" | "mp1" => "MP3",
        "flac" => "FLAC",
        "vorbis" => "Vorbis",
        "opus" => "Opus",
        "aac" | "aac_latm" => "AAC",
        "alac" => "ALAC",
        "ape" | "monkeys_audio" => "APE",
        "wavpack" => "WavPack",
        "musepack" | "musepack7" | "musepack8" | "mpc7" | "mpc8" => "Musepack",
        "tta" => "TTA",
        "tak" => "TAK",
        "shorten" => "Shorten",
        "ac3" | "eac3" => "AC-3",
        "dts" => "DTS",
        _ => {
            if c.starts_with("pcm_") {
                "PCM"
            } else if c.starts_with("dsd") {
                "DSD"
            } else if c.starts_with("wmav") || c == "wmalossless" || c == "wmapro" {
                "WMA"
            } else {
                return codec.to_ascii_uppercase();
            }
        }
    };
    named.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        bar_fraction, clamp_padding, codec_label, hit, indicator_width, slider_fraction, snap,
        status_indicators, step_toward, track_line, KEY_VOLUME_STEP, MIN_HEIGHT, MIN_WIDTH,
        VOLUME_STEP,
    };
    use super::{resume_playlist_path, PlaylistEntry};
    use crate::playlist::queue::RepeatMode;
    use crate::vis::mode::VisMode;
    use ratatui::layout::Rect;
    use std::path::PathBuf;

    fn a_session(playlist: Option<&str>, name: &str) -> crate::session::Session {
        crate::session::Session {
            playlist: playlist.map(PathBuf::from),
            playlist_name: name.into(),
            index: 0,
            total: 1,
            position: 10.0,
            duration: 100.0,
            artist: String::new(),
            title: String::new(),
            uri: String::new(),
            shuffle: false,
            repeat: "Off".into(),
            volume: 1.0,
            cursor: 0,
            folded: Vec::new(),
            album_order: Vec::new(),
            saved_at: 0,
        }
    }

    fn an_entry(name: &str, path: &str) -> PlaylistEntry {
        PlaylistEntry {
            name: name.into(),
            path: PathBuf::from(path),
            tracks: 1,
            missing: 0,
        }
    }

    #[test]
    fn a_session_without_a_path_is_recovered_by_name() {
        // What the picker used to write: a name and no path. Resuming that
        // dropped the user into the whole library instead of their playlist.
        let known = vec![
            an_entry("Angra", "/pl/angra.m3u"),
            an_entry("At Vance", "/pl/at-vance.m3u"),
        ];
        assert_eq!(
            resume_playlist_path(&a_session(None, "At Vance"), &known),
            Some(PathBuf::from("/pl/at-vance.m3u"))
        );
    }

    #[test]
    fn an_unknown_playlist_resolves_to_nothing_rather_than_a_guess() {
        let known = vec![an_entry("Angra", "/pl/angra.m3u")];
        assert_eq!(resume_playlist_path(&a_session(None, "Gone"), &known), None);
        assert_eq!(resume_playlist_path(&a_session(None, ""), &known), None);
        assert_eq!(resume_playlist_path(&a_session(None, "Angra"), &[]), None);
    }

    #[test]
    fn a_missing_stored_path_still_falls_back_to_the_name() {
        // The playlist directory moved: the recorded path no longer exists,
        // but the name still identifies it.
        let known = vec![an_entry("At Vance", "/pl/at-vance.m3u")];
        let s = a_session(Some("/old/place/at-vance.m3u"), "At Vance");
        assert_eq!(
            resume_playlist_path(&s, &known),
            Some(PathBuf::from("/pl/at-vance.m3u"))
        );
    }

    #[test]
    fn the_status_bar_always_names_the_visualizer() {
        // The reason this exists: the mode used to be visible only in a note
        // that expired after three seconds.
        for m in VisMode::all() {
            let segs = status_indicators(*m, false, RepeatMode::Off);
            let vis = &segs[0].0;
            assert!(
                vis.to_lowercase().contains(m.name()),
                "{} is not named in {vis:?}",
                m.name()
            );
            assert!(vis.starts_with("w "), "the key that cycles it is missing");
        }
    }

    #[test]
    fn an_indicator_is_lit_by_its_own_state_only() {
        // Shuffle used to light up whenever repeat was on, which said
        // something untrue about shuffle.
        let segs = status_indicators(VisMode::Leds, false, RepeatMode::All);
        assert!(!segs[1].1, "shuffle is lit while off");
        assert!(segs[2].1, "repeat is not lit while on");

        let segs = status_indicators(VisMode::Off, true, RepeatMode::Off);
        assert!(!segs[0].1, "the visualizer is lit while off");
        assert!(segs[1].1, "shuffle is not lit while on");
        assert!(!segs[2].1, "repeat is lit while off");
    }

    #[test]
    fn the_reported_width_matches_what_is_drawn() {
        // The left-hand text is truncated to whatever this leaves, so an
        // undercount would let the two overlap.
        let segs = status_indicators(VisMode::Dots, true, RepeatMode::One);
        let drawn: usize = segs.iter().map(|(l, _)| l.chars().count()).sum::<usize>()
            + (segs.len() - 1) * super::SEPARATOR.chars().count();
        assert_eq!(indicator_width(&segs) as usize, drawn);
    }

    #[test]
    fn hit_testing_excludes_the_far_edges() {
        let r = Rect::new(10, 5, 4, 2);
        assert!(hit(r, 10, 5));
        assert!(hit(r, 13, 6));
        assert!(!hit(r, 14, 5), "one past the right edge is outside");
        assert!(!hit(r, 10, 7), "one past the bottom is outside");
        assert!(!hit(r, 9, 5));
        assert!(!hit(r, 10, 4));
        assert!(
            !hit(Rect::new(10, 5, 0, 1), 10, 5),
            "an empty rect hits nothing"
        );
    }

    #[test]
    fn a_click_on_the_seek_bar_lands_where_the_pointer_is() {
        let bar = Rect::new(8, 4, 20, 1);
        assert_eq!(bar_fraction(bar, 8), 0.0, "the left edge is the start");
        assert_eq!(bar_fraction(bar, 18), 0.5);
        assert!(bar_fraction(bar, 27) < 1.0, "the last cell is not the end");
        // Out of range in either direction is clamped rather than wrapping.
        assert_eq!(bar_fraction(bar, 0), 0.0);
        assert_eq!(bar_fraction(bar, 500), 1.0);
    }

    #[test]
    fn the_mouse_moves_the_volume_in_tenths() {
        // One cell a step, so a drag across the ten-cell slider offers
        // exactly the eleven levels it can draw and no others.
        for step in 0..=10 {
            let want = step as f32 * VOLUME_STEP;
            assert!(
                (snap(want, VOLUME_STEP) - want).abs() < 1e-6,
                "{want} is not on the grid it came from"
            );
        }
        // Anything between two steps goes to the nearer one.
        assert!((snap(0.34, VOLUME_STEP) - 0.30).abs() < 1e-6);
        assert!((snap(0.36, VOLUME_STEP) - 0.40).abs() < 1e-6);
    }

    /// The pointer and the bar step together, by construction.
    #[test]
    fn a_mouse_step_is_exactly_one_cell() {
        use crate::ui::panels::player::VOLUME_SLIDER;
        assert!(
            (VOLUME_STEP * VOLUME_SLIDER as f32 - 1.0).abs() < 1e-6,
            "the pointer and the slider disagree about how many steps there are"
        );
    }

    #[test]
    fn the_keyboard_is_ten_times_finer_than_the_mouse() {
        // The point of having two: a key reaches the levels between the ones
        // the bar can draw, and the readout shows them.
        const { assert!(KEY_VOLUME_STEP < VOLUME_STEP) };
        assert!((VOLUME_STEP / KEY_VOLUME_STEP - 10.0).abs() < 1e-6);
        assert!((snap(0.37, KEY_VOLUME_STEP) - 0.37).abs() < 1e-6);
    }

    #[test]
    fn a_coarse_step_lands_on_its_own_grid() {
        // A volume left at 37% by the keyboard and then nudged by the wheel
        // goes to the next multiple of ten, either way. `snap(v + delta)`
        // would send it up to 50% instead, skipping a whole cell, because
        // 47% rounds up.
        assert!((step_toward(0.37, VOLUME_STEP) - 0.40).abs() < 1e-6);
        assert!((step_toward(0.37, -VOLUME_STEP) - 0.30).abs() < 1e-6);
    }

    #[test]
    fn a_step_from_the_grid_moves_a_whole_step() {
        // The failure mode of rounding toward the nearest: at 40% exactly, a
        // press would round back to 40% and the control would appear stuck.
        assert!((step_toward(0.40, VOLUME_STEP) - 0.50).abs() < 1e-6);
        assert!((step_toward(0.40, -VOLUME_STEP) - 0.30).abs() < 1e-6);
        assert!((step_toward(0.37, KEY_VOLUME_STEP) - 0.38).abs() < 1e-6);
    }

    #[test]
    fn stepping_stops_at_the_ends() {
        assert_eq!(step_toward(1.0, VOLUME_STEP), 1.0);
        assert_eq!(step_toward(0.0, -VOLUME_STEP), 0.0);
        assert_eq!(step_toward(0.42, 0.0), 0.42);
    }

    #[test]
    fn snapping_never_leaves_the_range() {
        assert_eq!(snap(1.04, VOLUME_STEP), 1.0);
        assert_eq!(snap(-0.2, VOLUME_STEP), 0.0);
        // A zero step is a caller's mistake, not a divide by zero.
        assert_eq!(snap(0.42, 0.0), 0.42);
    }

    #[test]
    fn a_click_on_the_last_volume_cell_means_full() {
        let v = Rect::new(60, 7, 8, 1);
        assert_eq!(slider_fraction(v, 67), 1.0);
        assert_eq!(slider_fraction(v, 63), 0.5);
        assert!(
            slider_fraction(v, 60) > 0.0,
            "the first cell is one step, not silence"
        );
        assert_eq!(slider_fraction(v, 200), 1.0);
    }

    #[test]
    fn the_track_line_puts_the_album_in_parentheses() {
        assert_eq!(
            track_line(Some("Angra"), Some("Nova Era"), Some("Rebirth")),
            Some("Angra — Nova Era (Rebirth)".into())
        );
    }

    #[test]
    fn a_missing_album_leaves_no_empty_brackets() {
        assert_eq!(
            track_line(Some("Angra"), Some("Nova Era"), None),
            Some("Angra — Nova Era".into())
        );
        assert_eq!(
            track_line(Some("Angra"), Some("Nova Era"), Some("   ")),
            Some("Angra — Nova Era".into())
        );
    }

    #[test]
    fn an_untagged_track_falls_back_rather_than_showing_a_stub() {
        assert_eq!(track_line(None, None, Some("Rebirth")), None);
        assert_eq!(track_line(Some("Angra"), None, None), None);
        assert_eq!(
            track_line(None, Some("Nova Era"), Some("Rebirth")),
            Some("Nova Era (Rebirth)".into())
        );
    }

    #[test]
    fn codec_names_are_shown_the_way_people_write_them() {
        assert_eq!(codec_label("flac"), "FLAC");
        assert_eq!(codec_label("mp3"), "MP3");
        assert_eq!(codec_label("vorbis"), "Vorbis");
        assert_eq!(codec_label("alac"), "ALAC");
        assert_eq!(codec_label("monkeys_audio"), "APE");
        assert_eq!(codec_label("wavpack"), "WavPack");
        assert_eq!(codec_label("musepack8"), "Musepack");
    }

    #[test]
    fn registry_names_collapse_to_the_family() {
        // The exact PCM layout and DSD packing are not what a listener wants
        // on the status line.
        assert_eq!(codec_label("pcm_s16le"), "PCM");
        assert_eq!(codec_label("pcm_f32be"), "PCM");
        assert_eq!(codec_label("dsd_lsbf_planar"), "DSD");
        assert_eq!(codec_label("wmav2"), "WMA");
    }

    #[test]
    fn an_unlisted_codec_is_shown_rather_than_dropped() {
        assert_eq!(codec_label("someformat"), "SOMEFORMAT");
        assert_eq!(codec_label(""), "");
    }

    #[test]
    fn padding_is_applied_when_there_is_room() {
        assert_eq!(clamp_padding(1, 100, MIN_WIDTH), 1);
        assert_eq!(clamp_padding(4, 100, MIN_WIDTH), 4);
        assert_eq!(clamp_padding(0, 100, MIN_WIDTH), 0);
    }

    #[test]
    fn padding_gives_way_rather_than_squeezing_the_layout_out() {
        // At exactly the minimum width there is no room to spare.
        assert_eq!(clamp_padding(4, MIN_WIDTH, MIN_WIDTH), 0);
        // Four spare columns is two per side.
        assert_eq!(clamp_padding(4, MIN_WIDTH + 4, MIN_WIDTH), 2);
    }

    #[test]
    fn padding_never_underflows_on_a_tiny_terminal() {
        assert_eq!(clamp_padding(8, 10, MIN_WIDTH), 0);
        assert_eq!(clamp_padding(8, 0, MIN_HEIGHT), 0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Player,
    Album,
    Equalizer,
    Playlist,
}

/// The equalizer's own state.
///
/// Its own struct because it is self-contained: five values that only the
/// equalizer panel and the gain publisher ever read, and nothing else in the
/// app needs to know they exist.
struct EqState {
    gains: [f32; 10],
    preamp: f32,
    enabled: bool,
    /// Index into `eq::PRESETS`, not a name -- the panel cycles through them
    /// by position and the name is only wanted when the setting is written.
    preset: usize,
    /// Which band the keyboard is on.
    band: usize,
}

impl EqState {
    fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            gains: cfg.eq.band_gains(),
            preamp: cfg.eq.preamp,
            enabled: cfg.eq.enabled,
            preset: eq::PRESETS
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(&cfg.eq.preset))
                .unwrap_or(0),
            band: 0,
        }
    }
}

/// The title animation, and what governs it.
///
/// Grouped because the whole feature is optional: with `active` false none of
/// the rest is read, and keeping them together says so.
struct Effects {
    /// The animation in flight, if there is one.
    running: Option<TextEffect>,
    /// The animating title, computed in the loop so drawing stays `&self`.
    title: String,
    kind: EffectKind,
    duration: f32,
    active: bool,
    reactive: bool,
}

impl Effects {
    fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            running: None,
            title: String::new(),
            kind: EffectKind::parse(&cfg.fx.track_change).unwrap_or(EffectKind::Decrypt),
            duration: cfg.fx.duration_ms as f32 / 1000.0,
            active: cfg.fx.active(),
            reactive: cfg.fx.reactive,
        }
    }
}

/// How the player is drawn: the palette, the characters, and the two small
/// animations that belong to the chrome rather than to the music.
struct Look {
    theme: Theme,
    /// Every theme that can be cycled to, and where in that list we are.
    ids: Vec<String>,
    index: usize,
    /// Which transport button faces to draw.
    glyphs: crate::ui::panels::player::Glyphs,
    /// Which characters the seek bar is drawn from.
    seek_style: crate::ui::panels::player::SeekStyle,
    /// How far the seek bar's highlight has travelled, 0 to 1.
    ///
    /// Held at zero unless something is actually playing: a highlight
    /// sweeping a paused bar says the track is moving when it is not.
    seek_phase: f32,
    /// How far a title too long for its line has scrolled.
    marquee: usize,
    last_marquee: Instant,
}

/// Which panels are open, where the keyboard is, and the geometry the last
/// frame was drawn into.
struct Panels {
    eq: bool,
    album: bool,
    playlist: bool,
    help: bool,
    picker: bool,
    focus: Focus,
    /// How far down the help has been scrolled.
    ///
    /// It needs to scroll at all because the key list is 58 lines against an
    /// inner height of at most 36: everything past `windows` had never been on
    /// screen, which is a poor way to document a keyboard.
    help_scroll: u16,
    /// The area the last frame was drawn into, so a page key can move by a
    /// screenful rather than by a hardcoded ten.
    last_area: Rect,
    padding_x: u16,
    padding_y: u16,
}

/// Everything the visualizer needs: the two analysers, the audio they read,
/// and how the result is drawn.
struct Visuals {
    analyzer: Spectrum,
    /// Newest audio, read from the player's tap once a frame.
    tap_buf: Vec<f32>,
    /// The same analysis with slower time constants, for the fluid mode.
    /// Kept alongside the shared one rather than replacing it: only one of
    /// them runs per frame, and each keeps its own envelope state.
    fluid: Spectrum,
    /// Bars the fluid mode should produce, set from the panel width as it is
    /// drawn -- it puts one bar in every column.
    fluid_bars: usize,
    /// Newest samples, for the trace modes that draw the waveform itself.
    wave: Vec<f32>,
    /// How wide the visualizer's bars are, and the gap between them.
    bars: crate::ui::panels::visualizer::BarLayout,
    mode: VisMode,
    meters: Meters,
    onset: OnsetDetector,
}

impl Visuals {
    fn from_config(cfg: &crate::config::Config) -> Self {
        Self {
            analyzer: {
                let mut a = Spectrum::new(2048, 20, 44_100.0);
                a.set_gain_db(cfg.vis.gain_db);
                a
            },
            tap_buf: vec![0.0; 4096],
            fluid: {
                let mut a = Spectrum::with_motion(2048, 64, 44_100.0, Motion::Fluid);
                a.set_gain_db(cfg.vis.gain_db);
                a.set_smoothing(cfg.vis.smoothing as f32);
                a
            },
            fluid_bars: 64,
            wave: vec![0.0; 1024],
            bars: crate::ui::panels::visualizer::BarLayout::sanitised(
                cfg.vis.bar_width,
                cfg.vis.bar_gap,
            ),
            mode: VisMode::parse(&cfg.vis.mode).unwrap_or_default(),
            meters: Meters::new(),
            onset: OnsetDetector::new(),
        }
    }
}

/// What has changed since `config.toml` was last written, and when that was.
///
/// Settings are diffed and written on a timer rather than on every keystroke,
/// so all three of these are read together or not at all.
struct Saving {
    /// Settings changed but not yet written. Keyed on section and name, so a
    /// slider dragged across the panel writes one line, once.
    pending: std::collections::BTreeMap<(String, String), Value>,
    /// The settings as the file last had them, to diff against.
    saved: Remembered,
    last_written: Instant,
}

/// What the user is part-way through typing, holding or dragging.
///
/// All of it is transient: every field is empty when nothing is being edited,
/// which is why the whole struct has a `Default` and none of it comes from
/// the config.
#[derive(Default)]
struct Editing {
    /// What `space` does, once it has been asked.
    ///
    /// Remembered per loaded playlist for as long as the browser stays open, so
    /// filling one up is `space` `space` `space` rather than a modal each time.
    /// Cleared on the way back to the player, and cleared if the playlist
    /// underneath changes -- an answer about `kaledon` says nothing about the
    /// next thing loaded.
    add_mode: Option<(String, Setting)>,
    /// What `space` is holding while the modal asks about it.
    adding: Option<(String, Vec<QueueItem>)>,
    /// A new playlist's name, while it is being typed.
    naming: Option<String>,
    /// The words the playlist is narrowed to. Empty is everything.
    words: String,
    /// The filter box, while it is open: what has been typed so far, seeded
    /// with the filter in force so `/` again shows what is being matched.
    typing: Option<String>,
    /// Bumped when the filter changes, so the rows rebuild.
    gen: u64,
    /// What a held left button is currently adjusting.
    drag: Option<Drag>,
    /// Where and when the last left click landed, for double-click detection.
    last_click: Option<(u16, u16, Instant)>,
}

/// One session spread across several windows, and this window's place in it.
///
/// Two halves of the same subject. `link`, and the `uri`/`revision`/`group`/
/// `bands` that come with it, are what a *follower* receives from whichever
/// instance owns the audio. `shared`, `seen` and `last` are the view every
/// window reconciles against, whether it leads or follows.
struct Sharing {
    /// Set when this instance is mirroring another rather than owning audio.
    link: Option<Mirror>,
    /// The leader's current track URI. A mirror's own queue items carry no
    /// path -- the leader sends tags, not URIs -- so this is the only way the
    /// album panel can look anything up while mirroring.
    uri: String,
    revision: u64,
    /// Whether the leader has its queue in album order. A mirror draws the
    /// dividers from this rather than working them out: its items arrive with
    /// no years, so ordering them itself would scramble the leader's order.
    group: bool,
    /// Analyzer output received from the instance being mirrored.
    bands: Vec<f32>,
    /// The owner's view revision, from its status line, so a follower knows
    /// when it is worth asking for the view itself.
    their_view: u64,
    /// The view every window of this session agrees about.
    ///
    /// Authoritative, and owned by whichever instance holds the socket. A
    /// window reconciles against it every frame: publishing what it changed,
    /// adopting what another window changed.
    shared: crate::view::Shared,
    /// How much of the view is shared, from `[session] share`.
    share: crate::view::Share,
    /// The revision this window has already taken account of.
    seen: u64,
    /// The view as of the last reconciliation, to notice our own changes
    /// against. Comparing with the shared copy instead would make another
    /// window's change look like ours and start an argument.
    last: crate::view::View,
    /// A playlist named on the command line while a session was already
    /// running, until the question about it is answered.
    joining: Option<std::path::PathBuf>,
}

/// The playlist as this window is looking at it.
///
/// Distinct from the queue the *player* owns: that is the order tracks play
/// in, and this is a view over it -- what is folded, what is selected, where
/// the cursor is, and the rows last drawn. A follower has all of this without
/// owning any audio.
struct QueueView {
    /// The queue in play order, cloned when it changes rather than every frame.
    items: Vec<QueueItem>,
    /// The lines the playlist draws, built during the frame and kept because a
    /// click arrives between frames and has to map to the same rows the last
    /// frame drew.
    rows: playlist::Rows,
    /// The queue revision, grouping and fold state `rows` was built from.
    rows_from: (u64, bool, u64, Option<usize>, u64),
    /// Records folded shut, by album title. Kept by album rather than by
    /// position so a fold survives the order being reversed, the queue being
    /// reloaded, or the session being resumed tomorrow.
    folded: std::collections::HashSet<String>,
    /// Bumped whenever `folded` changes, so the rows are rebuilt for it.
    fold_gen: u64,
    /// A position in the queue's view -- a track, not a row. Dividers are not
    /// something you can select, so the cursor never lands on one.
    cursor: usize,
    /// A *row*, which is not the same thing once dividers take space.
    scroll: usize,
    /// Which way round album order runs. Held here as well as in the queue so
    /// it survives the mode being switched off and on again.
    group_desc: bool,
    /// Rows marked for a bulk operation, by index into the queue's `tracks`.
    ///
    /// Track indices rather than URIs because a playlist may hold the same URI
    /// more than once -- the reference library's own playlists have 793
    /// repeated lines -- so a URI names no particular row. `tracks` only ever
    /// moves under the three editing operations, which remap this in the same
    /// breath, so an index survives shuffle, grouping and folds.
    ///
    /// Per window: `view.rs` shares "what is being looked at", and a half-made
    /// selection is intent about what you are *about to do*, like focus.
    tagged: std::collections::HashSet<usize>,
    /// Copied rows, held as items so they survive loading another playlist.
    clipboard: Vec<QueueItem>,
    /// The queue has been changed and not written anywhere.
    ///
    /// Adding never touches a file. The playlist on disk changes only when it
    /// is saved, and that asks first.
    dirty: bool,
    /// Which playlist is loaded, for the playlist pane's title.
    name: String,
    /// Where the current queue came from, so the session can name it.
    source: Option<PathBuf>,
}

/// What is open on top of the player.
///
/// At most one of these is showing at a time and every one of them starts
/// closed, which is why the whole struct derives `Default`. `browse` is the
/// exception that earns its place here: it is not itself an overlay but the
/// model the library browser is built from, kept so reopening is instant.
#[derive(Default)]
struct Overlays {
    /// Playlists from the configured directory, and the picker over them.
    playlists: Vec<PlaylistEntry>,
    picker_cursor: usize,
    picker_scroll: usize,
    /// The cover chooser, open over the current album.
    chooser: Option<Chooser>,
    /// A panel's settings, open over everything.
    settings: Option<SettingsState>,
    /// A previous session offered but not yet accepted or declined.
    resume: Option<Session>,
    /// The library browser, when it is open. Per window, not shared.
    library: Option<crate::ui::panels::library::Library>,
    /// Held across opens: building it costs ~290 ms and nothing invalidates it
    /// until the index is rescanned.
    browse: Option<Arc<crate::library::browse::Model>>,
}

pub struct App {
    player: Arc<Player>,
    mpris: Option<crate::mpris::MprisHandle>,
    ipc_stop: Arc<std::sync::atomic::AtomicBool>,
    ipc_path: Option<PathBuf>,
    last_track_revision: u64,
    last_state: PlayState,
    look: Look,
    panels: Panels,

    /// Dropouts counted the last time one happened, and when that was.
    ///
    /// The device's counter only ever climbs, so showing it directly meant one
    /// stutter during a cold read left `xrun 1` on screen for the rest of the
    /// session -- a warning about something that had stopped happening. What
    /// is worth seeing is that it is happening *now*.
    dropouts_at: Option<(u64, std::time::Instant)>,
    /// The count at the start of the quiet stretch, so a burst is counted from
    /// where it began rather than from the beginning of the session.
    dropout_base: u64,

    /// Album details and cover art, resolved on their own thread.
    ///
    /// `None` when there is no index yet, which is simply a panel that says so.
    art: Option<crate::library::art::Watcher>,
    /// The URI the worker was last asked about, so it is asked once per track.
    art_uri: Option<String>,
    /// Set while a lookup asked for by hand is in flight, holding the worker's
    /// publication count from the moment it was asked. When that count moves,
    /// the answer -- found or not -- has arrived.
    retrying: Option<u64>,
    /// How far the highlight has travelled across the retry word.
    retry_phase: f32,
    /// Whether covers are drawn as real pixels, and the encoded one if so.
    graphics: crate::ui::graphics::Graphics,
    /// Whether the art worker may ask the archive. Shared with the worker so
    /// the setting can be changed while the player is running.
    art_fetch: Arc<std::sync::atomic::AtomicBool>,

    eq: EqState,
    vis: Visuals,
    saving: Saving,
    edit: Editing,
    session: Sharing,
    queue: QueueView,
    over: Overlays,

    fx: Effects,

    last_frame: Instant,
    quit: bool,
    status: Option<(String, Instant)>,
}

impl App {
    /// An instance that mirrors another rather than owning audio.
    ///
    /// Built on a detached player, so every panel reads the same structures and
    /// none of the rendering path needs to know which mode it is in.
    /// A window joining a session another instance already owns.
    ///
    /// It gets a real player even though it will not be playing anything: the
    /// worker thread costs nothing while idle and opens no audio device until
    /// it is asked to, and having one means this window can pick the session up
    /// if the instance that owns it goes away. A detached player cannot -- its
    /// command receiver is dropped, so it could never be handed anything.
    ///
    /// Nothing sends it a transport command in the meantime, and that is not a
    /// convention to remember: every mutation goes through `owns`, which sends
    /// rather than applies while this window is following.
    pub fn mirroring(library_root: PathBuf, cfg: &crate::config::Config) -> Result<Self> {
        Self::mirroring_on(Arc::new(crate::vfs::Vfs::local(library_root)), cfg)
    }

    /// As `mirroring`, over a library that may not be on this machine.
    pub fn mirroring_on(vfs: Arc<crate::vfs::Vfs>, cfg: &crate::config::Config) -> Result<Self> {
        let player = Arc::new(Player::new(Arc::clone(&vfs), cfg.output.fixed_rate())?);
        let mut app = Self::with_player(player, Vec::new(), cfg)?;
        app.queue.source = None;
        app.spawn_art(vfs);
        Ok(app)
    }

    pub fn new(
        library_root: PathBuf,
        items: Vec<QueueItem>,
        cfg: &crate::config::Config,
    ) -> Result<Self> {
        Self::on(Arc::new(crate::vfs::Vfs::local(library_root)), items, cfg)
    }

    /// As `new`, over a library that may not be on this machine.
    pub fn on(
        vfs: Arc<crate::vfs::Vfs>,
        items: Vec<QueueItem>,
        cfg: &crate::config::Config,
    ) -> Result<Self> {
        let player = Arc::new(Player::new(Arc::clone(&vfs), cfg.output.fixed_rate())?);
        let mut app = Self::with_player(player, items, cfg)?;
        app.spawn_art(vfs);
        Ok(app)
    }

    /// Open the browser, building the model if this is the first time.
    ///
    /// Built on demand rather than at startup: it costs ~290 ms against a
    /// 32,000-track index and most sessions never open it. Held afterwards, so
    /// the second `l` is instant.
    fn open_library(&mut self) {
        if self.over.library.is_some() {
            self.over.library = None;
            return;
        }
        if let Some(model) = self.over.browse.clone() {
            self.over.library = Some(crate::ui::panels::library::Library::new(model));
            return;
        }
        let Ok(index) = self.player.vfs().index_path() else {
            return self.note("no index to browse -- run `staramp scan`".into());
        };
        let built = crate::library::db::Db::open_readonly(&index)
            .and_then(|db| crate::library::browse::Model::load(&db));
        match built {
            Ok(m) => {
                let m = Arc::new(m);
                self.over.browse = Some(Arc::clone(&m));
                self.over.library = Some(crate::ui::panels::library::Library::new(m));
            }
            Err(e) => self.note(format!("cannot read the index: {e}")),
        }
    }

    /// Keys typed into the new-playlist name box.
    ///
    /// Raw keys, like the search line and the resume prompt, because while a
    /// name is being typed every letter is a letter.
    fn name_type(&mut self, k: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(name) = &mut self.edit.naming else {
            return false;
        };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => name.clear(),
            // Anything that cannot be a file name is simply not accepted --
            // better than writing a path separator into what was meant to be a
            // name, or silently rewriting what somebody typed.
            KeyCode::Char(c) if !ctrl && c != '/' && c != '\\' => name.push(c),
            KeyCode::Backspace => {
                name.pop();
            }
            KeyCode::Esc => self.edit.naming = None,
            KeyCode::Enter => {
                let name = name.trim().to_string();
                self.edit.naming = None;
                if name.is_empty() {
                    return true;
                }
                let dir = self
                    .queue
                    .source
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .or_else(|| crate::paths::playlist_dir().ok());
                let Some(dir) = dir else {
                    self.note("nowhere to save playlists".into());
                    return true;
                };
                let path = dir.join(format!("{name}.m3u"));
                if path.exists() {
                    // Saving as new must never quietly become an overwrite.
                    self.note(format!("{name} already exists -- pick another name"));
                    self.edit.naming = Some(name);
                    return true;
                }
                self.write_playlist(path);
            }
            _ => return false,
        }
        true
    }

    /// Keys typed into the filter box.
    ///
    /// Enter takes what was typed as the filter -- nothing at all clears it
    /// -- and Esc leaves the filter as it was. Raw keys, like the name box:
    /// while words are being typed every letter is a letter.
    fn filter_type(&mut self, k: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(text) = &mut self.edit.typing else {
            return false;
        };
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => text.clear(),
            KeyCode::Char(c) if !ctrl => text.push(c),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Esc => self.edit.typing = None,
            KeyCode::Enter => {
                let text = text.trim().to_string();
                self.edit.typing = None;
                if text != self.edit.words {
                    self.edit.words = text;
                    self.edit.gen += 1;
                }
                if self.edit.words.is_empty() {
                    self.note("filter cleared".into());
                } else {
                    self.note(format!("filter: {}", self.edit.words));
                }
            }
            _ => return false,
        }
        true
    }

    /// Open the filter box over the playlist, from wherever the focus is.
    ///
    /// Seeded with the filter in force, so pressing `/` again shows what the
    /// list is being narrowed by and lets it be edited rather than retyped.
    fn open_filter_box(&mut self) {
        self.panels.playlist = true;
        self.panels.focus = Focus::Playlist;
        self.over.settings = None;
        self.edit.typing = Some(self.edit.words.clone());
    }

    /// Keys typed into the search line, before any table sees them.
    ///
    /// The same shape as the resume prompt's raw-key answer, and for the same
    /// reason: while a box is being typed into, `j` has to be a letter rather
    /// than a cursor move, and that cannot be expressed in a table keyed only
    /// on the key event.
    fn library_type(&mut self, k: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(lib) = &mut self.over.library else {
            return false;
        };
        if !lib.typing {
            return false;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => lib.search.clear(),
            KeyCode::Char('w') if ctrl => {
                while lib.search.pop().is_some_and(|c| !c.is_whitespace()) {}
            }
            KeyCode::Char(c) if !ctrl => lib.search.push(c),
            KeyCode::Backspace => {
                lib.search.pop();
            }
            // Committed: the filter stays, the keys go back to the columns.
            KeyCode::Enter => {
                lib.typing = false;
                return true;
            }
            // Cancelled: the text and the filter both go.
            KeyCode::Esc => {
                lib.typing = false;
                lib.search.clear();
                lib.refilter();
                return true;
            }
            // Arrows still move the list, so you can filter, step to the one
            // you meant and carry on typing.
            _ => return false,
        }
        lib.refilter();
        true
    }

    /// Dropouts worth mentioning: the ones in the burst still going on.
    ///
    /// Clears itself once the audio has been clean for a while, so the warning
    /// is about the present rather than about something that happened once
    /// during a cold read at startup.
    fn dropouts_now(&mut self) -> u64 {
        // Long enough to cover a stutter that comes in bursts, short enough
        // that a settled player stops apologising for itself.
        const FORGET: Duration = Duration::from_secs(20);

        let total = self
            .player
            .state
            .underruns
            .load(std::sync::atomic::Ordering::Relaxed);
        match self.dropouts_at {
            Some((last, at)) => {
                if total > last {
                    self.dropouts_at = Some((total, Instant::now()));
                } else if at.elapsed() >= FORGET {
                    // Gone quiet. The next burst counts from here.
                    self.dropouts_at = None;
                    self.dropout_base = total;
                    return 0;
                }
                total - self.dropout_base
            }
            None if total > self.dropout_base => {
                self.dropouts_at = Some((total, Instant::now()));
                total - self.dropout_base
            }
            None => 0,
        }
    }

    /// What the playlist's header row says right now.
    fn playlist_header(&self) -> Vec<crate::ui::panels::header::Item> {
        crate::ui::panels::header::playlist_words(
            self.queue.tagged.len(),
            self.queue.clipboard.len(),
        )
    }

    /// The rows a bulk action applies to, by index into `tracks`.
    ///
    /// The tags when there are any, otherwise the row under the cursor -- so
    /// `del` and `y` are useful without tagging anything first.
    fn acting_on(&self) -> Vec<usize> {
        if !self.queue.tagged.is_empty() {
            let mut v: Vec<usize> = self.queue.tagged.iter().copied().collect();
            v.sort_unstable();
            return v;
        }
        self.cursor_track().into_iter().collect()
    }

    /// Mark the row under the cursor, and step down.
    ///
    /// Stepping means tagging a run is `t t t` rather than `t j t j t`, which
    /// is the whole point of tagging rather than acting a row at a time.
    fn tag_row(&mut self) {
        let Some(track) = self.cursor_track() else {
            return;
        };
        if !self.queue.tagged.remove(&track) {
            self.queue.tagged.insert(track);
        }
        self.move_cursor(1);
    }

    fn clear_tags(&mut self) {
        let n = self.queue.tagged.len();
        self.queue.tagged.clear();
        if n > 0 {
            self.note(format!("{n} untagged"));
        }
    }

    fn copy_tagged(&mut self) {
        let want: std::collections::HashSet<usize> = self.acting_on().into_iter().collect();
        let q = self.player.queue.lock().unwrap();
        // In the order they were shown in, not storage order: copying a
        // scattered handful out of a grouped list and pasting it back in a
        // different order is not what anybody meant.
        self.queue.clipboard = q
            .view()
            .iter()
            .filter(|i| want.contains(i))
            .filter_map(|&i| q.tracks().get(i).cloned())
            .collect();
        drop(q);
        let n = self.queue.clipboard.len();
        self.note(match n {
            0 => "nothing to copy".into(),
            1 => "copied 1 track".into(),
            n => format!("copied {n} tracks"),
        });
    }

    fn paste_tagged(&mut self) {
        if self.queue.clipboard.is_empty() {
            return self.note("nothing copied".into());
        }
        let items = self.queue.clipboard.clone();
        let at = self.queue.cursor + 1;
        let uris: Vec<String> = items.iter().map(|t| t.uri.to_string()).collect();
        let done = match self.ask_session("paste-at", serde_json::json!({"at": at, "uris": uris})) {
            Some(Ok(n)) => n,
            Some(Err(e)) => return self.note(format!("the session would not take them: {e}")),
            None => self.player.queue.lock().unwrap().insert_at(at, items),
        };
        self.after_edit();
        self.note(format!("put {done} here"));
    }

    fn move_tagged(&mut self) {
        // Not `acting_on`: with nothing tagged that moves the cursor row to
        // where it already is -- a no-op that would still dirty the playlist
        // and put a `*` in the title nobody asked for.
        if self.queue.tagged.is_empty() {
            return self.note("nothing tagged \u{2014} t marks a row".into());
        }
        let want = self.acting_on();
        let at = self.queue.cursor;
        let rows: Vec<usize> = {
            let q = self.player.queue.lock().unwrap();
            want.iter().filter_map(|&t| q.view_position(t)).collect()
        };
        let n = match self.ask_session("move-to", serde_json::json!({"at": at, "rows": rows})) {
            Some(Ok(n)) => n,
            Some(Err(e)) => return self.note(format!("the session would not move them: {e}")),
            None => self.player.queue.lock().unwrap().move_to(&rows, at),
        };
        self.queue.tagged.clear();
        self.after_edit();
        self.note(format!("moved {n}"));
    }

    fn remove_tagged(&mut self) {
        let want = self.acting_on();
        if want.is_empty() {
            return;
        }
        // Protected only while something is actually playing -- the same test,
        // and the same reason, as the pin in `toggle_shuffle`. With no decoder
        // open there is no track to contradict, and refusing to delete row one
        // of a stopped queue is a rule nobody can see.
        let protect = self.player.state.state() != PlayState::Stopped;
        let (rows, asked, kept) = {
            let q = self.player.queue.lock().unwrap();
            let rows: Vec<usize> = want.iter().filter_map(|&t| q.view_position(t)).collect();
            let kept = protect && rows.contains(&q.view_cursor());
            (rows.clone(), rows.len(), kept)
        };
        let n = match self.ask_session("remove-at", serde_json::json!({"rows": rows})) {
            Some(Ok(n)) => n,
            Some(Err(e)) => return self.note(format!("the session would not remove them: {e}")),
            None => self.player.queue.lock().unwrap().remove(&rows, protect),
        };
        self.queue.tagged.clear();
        self.after_edit();
        self.note(match (n, kept) {
            (_, true) if n + 1 == asked && n == 0 => {
                "only the playing track was tagged, and it stays".into()
            }
            (n, true) => format!("removed {n} \u{b7} the playing track stays"),
            (n, _) => format!("removed {n}"),
        });
    }

    /// After anything that moved rows about.
    ///
    /// The tags go, rather than being filtered for range. Every edit rebuilds
    /// `tracks`, so a surviving index is a name for a different song -- keeping
    /// the in-range ones looks tidy and points them at the wrong rows.
    fn after_edit(&mut self) {
        self.queue.dirty = true;
        let len = self.player.queue.lock().unwrap().len();
        self.queue.cursor = self.queue.cursor.min(len.saturating_sub(1));
        self.queue.tagged.clear();
    }

    /// Ask the session to edit its rows, and say what it actually did.
    ///
    /// `None` when this window owns the queue and should edit it directly.
    /// Otherwise the answer comes from the session rather than from hope --
    /// the lesson of `enqueue`, where a window counted the tracks it had sent
    /// and reported them added while an older leader was refusing the verb.
    ///
    /// The revision goes with the request because a position is only
    /// meaningful at one, and a delete aimed at a list that has moved cannot
    /// be taken back.
    fn ask_session(&self, verb: &str, body: serde_json::Value) -> Option<Result<usize, String>> {
        let m = self.session.link.as_ref()?;
        let mut body = body;
        let revision = self.player.queue.lock().unwrap().revision();
        body["revision"] = revision.into();
        let reply = m.ask(&format!("{verb} {body}"));
        Some(match reply.as_deref().map(str::trim) {
            Some(r) => match r.parse::<usize>() {
                Ok(n) => Ok(n),
                Err(_) => Err(r.trim_start_matches("error: ").to_string()),
            },
            None => Err("the session did not answer".into()),
        })
    }

    /// Click to select, double click to play, wheel to scroll.
    ///
    /// No click-outside dismissal, unlike every overlay next door: the browser
    /// is a view rather than a modal, and there is no outside to click. `esc`
    /// closes it.
    fn library_mouse(&mut self, m: MouseEvent, area: Rect) {
        use crate::ui::panels::library::{hit, layout, Hit};
        let Some(lib) = &mut self.over.library else {
            return;
        };
        let l = layout(area, lib.focus);
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if matches!(m.kind, MouseEventKind::ScrollUp) {
                    -3
                } else {
                    3
                };
                if let Hit::Row { column, .. } = hit(&l, x, y) {
                    lib.focus = column;
                }
                lib.step(delta);
            }
            MouseEventKind::Down(MouseButton::Left) => match hit(&l, x, y) {
                Hit::Row { column, row } => {
                    let row = lib.scroll(column) + row as usize;
                    let again = self.register_click(x, y);
                    if let Some(lib) = &mut self.over.library {
                        lib.select(column, row);
                    }
                    if again {
                        self.library_play();
                    }
                }
                Hit::Head(column) => lib.focus = column,
                Hit::Search => lib.typing = true,
                _ => {}
            },
            _ => {}
        }
    }

    /// Play what is selected. Filled in with the rest of stage 4.
    fn library_play(&mut self) {}

    /// Start the album worker, if there is an index for it to read.
    ///
    /// Best-effort throughout: without an index the panel simply says the
    /// track is not in the library, which is the truth.
    fn spawn_art(&mut self, vfs: Arc<crate::vfs::Vfs>) {
        // A remote library is browsed through a copy of its index, so that is
        // the one the art worker must read too -- the local one describes a
        // different library entirely.
        let index = match vfs.as_ref() {
            crate::vfs::Vfs::Remote(l) => crate::remote::index::local_copy(l.host()),
            crate::vfs::Vfs::Local { .. } => crate::paths::index_file(),
        };
        let Ok(index) = index else {
            return;
        };
        self.art = crate::library::art::Watcher::spawn(index, vfs, Arc::clone(&self.art_fetch));
    }

    fn with_player(
        player: Arc<Player>,
        items: Vec<QueueItem>,
        cfg: &crate::config::Config,
    ) -> Result<Self> {
        player.set_queue_tracks(items);
        // Before anything asks the queue where it is. A resume looks its track
        // up by URI and jumps to it, and it has to jump into the order the
        // listener will actually see.
        {
            let mut q = player.queue.lock().unwrap();
            if cfg.playlist.group_by.eq_ignore_ascii_case("album") {
                q.set_grouping(Some(cfg.playlist.group_desc));
            }
            // Shuffle after the grouping: it overrides one rather than
            // replacing it, so the albums are there to come back to.
            q.set_shuffle(cfg.playlist.shuffle);
            q.set_repeat(match cfg.playlist.repeat.to_ascii_lowercase().as_str() {
                "all" => crate::playlist::queue::RepeatMode::All,
                "one" => crate::playlist::queue::RepeatMode::One,
                _ => crate::playlist::queue::RepeatMode::Off,
            });
        }
        player.set_volume(cfg.volume);
        // Handed over once: the decode thread reads it at each track boundary,
        // which is the only moment it can change without a step in the level.
        player.set_replaygain(cfg.rg.mode(), cfg.rg.preamp, cfg.rg.prevent_clipping);

        // Desktop integration is best-effort: no session bus means no MPRIS,
        // not a player that refuses to start.
        let mpris = if crate::mpris::session_bus_available() {
            crate::mpris::spawn(Arc::clone(&player))
        } else {
            None
        };

        let theme_ids = builtin::selectable();
        let theme_index = theme_ids
            .iter()
            .position(|id| id.eq_ignore_ascii_case(&cfg.theme))
            .unwrap_or(0);
        let (theme, _) = builtin::resolve_named(&theme_ids[theme_index]);

        // Remote control is best-effort too.
        let ipc_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let view = crate::view::shared();
        let ipc_path = crate::ipc::spawn(
            Arc::clone(&player),
            Arc::clone(&view),
            Arc::clone(&ipc_stop),
        );
        if let Some(p) = &ipc_path {
            tracing::info!("ipc listening on {}", p.display());
        }

        let app = Self {
            player,
            mpris,
            ipc_stop,
            ipc_path,
            last_track_revision: 0,
            last_state: PlayState::Stopped,
            panels: Panels {
                eq: cfg.ui.show_equalizer,
                album: cfg.ui.show_album,
                playlist: cfg.ui.show_playlist,
                help: false,
                picker: false,
                focus: if cfg.ui.show_playlist {
                    Focus::Playlist
                } else {
                    Focus::Player
                },
                help_scroll: 0,
                last_area: Rect::default(),
                padding_x: cfg.ui.padding_x,
                padding_y: cfg.ui.padding_y,
            },
            look: Look {
                theme,
                ids: theme_ids,
                index: theme_index,
                glyphs: crate::ui::panels::player::Glyphs::default(),
                seek_style: crate::ui::panels::player::SeekStyle::parse(&cfg.ui.seek_style)
                    .unwrap_or_default(),
                seek_phase: 0.0,
                marquee: 0,
                last_marquee: Instant::now(),
            },
            art: None,
            art_uri: None,
            retrying: None,
            retry_phase: 0.0,
            graphics: crate::ui::graphics::Graphics::disabled(),
            art_fetch: Arc::new(std::sync::atomic::AtomicBool::new(cfg.art.fetch)),
            // Whichever panel is open to be focused. A cursor in a panel that
            // is not on screen has nothing to move.
            dropouts_at: None,
            dropout_base: 0,
            // Built on the first frame: the queue's revision has already moved
            // past the zero this starts from.
            eq: EqState::from_config(cfg),
            vis: Visuals::from_config(cfg),
            edit: Editing::default(),
            over: Overlays::default(),
            queue: QueueView {
                // Built on the first frame: the queue's revision has already
                // moved past the zero this starts from.
                items: Vec::new(),
                rows: playlist::Rows::default(),
                rows_from: (0, false, 0, None, 0),
                folded: std::collections::HashSet::new(),
                fold_gen: 0,
                cursor: 0,
                scroll: 0,
                group_desc: cfg.playlist.group_desc,
                tagged: Default::default(),
                clipboard: Vec::new(),
                dirty: false,
                name: DEFAULT_QUEUE_NAME.into(),
                source: None,
            },
            session: Sharing {
                link: None,
                uri: String::new(),
                revision: u64::MAX,
                group: false,
                bands: Vec::new(),
                their_view: 0,
                shared: view,
                share: crate::view::Share::parse(&cfg.session.share),
                seen: 0,
                last: crate::view::View::default(),
                joining: None,
            },
            saving: Saving {
                pending: std::collections::BTreeMap::new(),
                saved: Remembered::from_config(cfg),
                last_written: Instant::now(),
            },
            fx: Effects::from_config(cfg),

            last_frame: Instant::now(),
            quit: false,
            status: None,
        };
        // The curve is only a setting until it reaches the audio thread.
        app.apply_eq();
        Ok(app)
    }

    /// Supply the playlists the picker offers. Opening straight onto the picker
    /// when nothing else was asked for is the point of the whole feature.
    /// Follow another running instance instead of owning the audio device.
    pub fn set_mirror(&mut self, m: Mirror) {
        self.session.link = Some(m);
        // A mirror has nothing of its own to resume or choose.
        self.over.resume = None;
        self.panels.picker = false;
    }

    pub fn is_mirror(&self) -> bool {
        self.session.link.is_some()
    }

    /// Route a command to whichever side actually owns playback.
    fn command(&self, c: &Command, remote: &str) {
        match &self.session.link {
            Some(m) => m.send(remote),
            None => self.player.send(match c {
                Command::PlayIndex(i) => Command::PlayIndex(*i),
                Command::Pause => Command::Pause,
                Command::Resume => Command::Resume,
                Command::TogglePause => Command::TogglePause,
                Command::Stop => Command::Stop,
                Command::Next => Command::Next,
                Command::Prev => Command::Prev,
                Command::SeekTo(v) => Command::SeekTo(*v),
                Command::SeekBy(v) => Command::SeekBy(*v),
                Command::Quit => Command::Quit,
            }),
        }
    }

    /// The instance that owned the session has gone. Try to become it.
    ///
    /// Every surviving window notices at the same moment and races to bind the
    /// socket. Exactly one can, so that is the whole election -- no protocol,
    /// no negotiation. The losers find the winner and follow it instead.
    ///
    /// The winner already holds a full copy of the queue and the view, so
    /// there is nothing to reload: it seeds itself from what it was already
    /// showing, which is milliseconds old rather than the five seconds the
    /// session file would be.
    fn take_over(&mut self) {
        let path = crate::ipc::spawn(
            Arc::clone(&self.player),
            Arc::clone(&self.session.shared),
            Arc::clone(&self.ipc_stop),
        );
        let Some(path) = path else {
            // Somebody else got there first. Follow them.
            self.session.link = crate::mirror::Mirror::connect();
            if self.session.link.is_none() {
                self.note("the session has gone".into());
            }
            return;
        };

        // Where the music had got to, as of the last frame.
        let uri = self.session.uri.clone();
        let at = self.player.state.position_secs();
        let was_playing = self.player.state.state() == PlayState::Playing;

        self.ipc_path = Some(path);
        self.session.link = None;
        // The view was the other instance's copy; from here it is ours.
        self.session.seen = 0;

        let track = {
            let q = self.player.queue.lock().unwrap();
            let wanted = crate::playlist::uri::TrackUri::parse(&uri);
            q.tracks().iter().position(|t| t.uri == wanted)
        };
        match track {
            Some(i) if !uri.is_empty() => {
                self.player.send(Command::PlayIndex(i));
                self.player
                    .send(Command::SeekTo(crate::session::resume_position(at)));
                if !was_playing {
                    self.player.send(Command::Pause);
                }
                self.note("picked up the session where it was left".into());
            }
            _ => self.note("picked up the session".into()),
        }
    }

    /// Put a playlist into the session, whoever owns it.
    fn load_playlist_into_session(&mut self, path: &std::path::Path) {
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "playlist".into());
        match crate::load_playlist(path) {
            Ok(items) if !items.is_empty() => {
                let n = items.len();
                if self.owns(&format!("load-playlist {}", path.display())) {
                    self.player.set_queue_tracks(items);
                }
                self.queue.name = name.clone();
                self.queue.source = Some(path.to_path_buf());
                self.panels.playlist = true;
                self.queue.cursor = 0;
                self.queue.scroll = 0;
                self.note(format!("{name} \u{2014} {n} tracks"));
            }
            Ok(_) => self.note(format!("{name} is empty")),
            Err(e) => self.note(format!("{name}: {e}")),
        }
    }

    /// A playlist named on the command line, waiting on the question above.
    pub fn ask_about(&mut self, path: std::path::PathBuf) {
        self.session.joining = Some(path);
        self.open_overlay(Focus::Playlist, Overlay::Joining);
    }

    /// The view as this window currently has it.
    fn view_now(&self) -> crate::view::View {
        let mut folded: Vec<String> = self.queue.folded.iter().cloned().collect();
        folded.sort();
        crate::view::View {
            playlist_name: self.queue.name.clone(),
            playlist_path: self
                .queue
                .source
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            // By URI: two windows agree on the track, not on where it sits.
            cursor: self
                .cursor_track()
                .and_then(|t| {
                    self.player
                        .queue
                        .lock()
                        .unwrap()
                        .tracks()
                        .get(t)
                        .map(|i| i.uri.to_string())
                })
                .unwrap_or_default(),
            folded,
            show_album: self.panels.album,
            show_equalizer: self.panels.eq,
            show_playlist: self.panels.playlist,
            revision: 0,
        }
    }

    /// Take on a view another window published.
    fn adopt_view(&mut self, v: &crate::view::View) {
        if !v.playlist_name.is_empty() {
            self.queue.name = v.playlist_name.clone();
        }
        self.queue.source =
            (!v.playlist_path.is_empty()).then(|| std::path::PathBuf::from(&v.playlist_path));
        self.queue.folded = v.folded.iter().cloned().collect();
        self.queue.fold_gen = self.queue.fold_gen.wrapping_add(1);
        // Panel *intent*. Whether a panel actually appears is still decided by
        // this terminal's own size in `regions`, so a window too small for the
        // playlist does not close it everywhere.
        self.panels.album = v.show_album;
        self.panels.eq = v.show_equalizer;
        self.panels.playlist = v.show_playlist;
        if !v.cursor.is_empty() {
            let wanted = crate::playlist::uri::TrackUri::parse(&v.cursor);
            let q = self.player.queue.lock().unwrap();
            let at = q
                .tracks()
                .iter()
                .position(|t| t.uri == wanted)
                .and_then(|t| q.view_position(t));
            drop(q);
            if let Some(at) = at {
                self.queue.cursor = at;
            }
        }
    }

    /// Keep this window and the others showing the same thing.
    ///
    /// Two questions in order. Has *this* window changed anything since the
    /// last time we looked? Then it is the authority and publishes. Otherwise,
    /// has anyone else? Then it follows.
    ///
    /// Asking in that order is what stops two windows fighting: a window only
    /// ever publishes a change it actually made, so a stale copy cannot argue
    /// its way back over a newer one.
    fn sync_view(&mut self) {
        if self.session.share == crate::view::Share::Playback {
            return;
        }
        let mine = self.view_now();

        // A window that has just opened has nothing to say yet. It arrives
        // with its cursor at the top and every panel as its config left them,
        // and without this it would publish all of that over whatever the
        // other windows were looking at -- opening a second window would yank
        // the first one back to the first track.
        if self.session.seen == 0 {
            let theirs = match &self.session.link {
                None => Some(self.session.shared.lock().unwrap().clone()),
                Some(m) => m.view(),
            };
            match theirs {
                Some(theirs) if theirs.revision > 0 => {
                    self.session.seen = theirs.revision;
                    self.adopt_view(&theirs);
                    self.session.last = self.view_now();
                    return;
                }
                // Nothing to join: this window is the first, and what it has
                // is the session's view from now on.
                _ => {}
            }
        }

        // Something changed here.
        if mine.differs(&self.session.last) {
            self.session.seen = match &self.session.link {
                None => crate::view::publish(&self.session.shared, &mine),
                Some(m) => {
                    if let Ok(body) = serde_json::to_string(&mine) {
                        m.send(&format!("set-view {body}"));
                    }
                    // The owner decides the revision; this is what we expect it
                    // to become, and a mismatch simply means another window got
                    // in first and we adopt on the next frame.
                    self.session.seen + 1
                }
            };
            self.session.last = mine;
            return;
        }

        // Nothing changed here. Has it elsewhere?
        let elsewhere = match &self.session.link {
            None => {
                // One guard, taken once. Locking again inside the `then` is a
                // deadlock: the guard from the condition is a temporary that
                // lives to the end of the statement, and this mutex is not
                // reentrant. It wedged the whole instance -- the socket thread
                // blocked on the same lock, so it stopped answering too.
                let held = self.session.shared.lock().unwrap();
                (held.revision != self.session.seen).then(|| held.clone())
            }
            Some(m) => (self.session.their_view != self.session.seen)
                .then(|| m.view())
                .flatten(),
        };
        if let Some(theirs) = elsewhere {
            self.session.seen = theirs.revision;
            self.adopt_view(&theirs);
            self.session.last = self.view_now();
        }
    }

    /// Is the session's list actually in album order?
    ///
    /// Asked of whichever instance owns it. A window that is following another
    /// renders the order it was handed rather than sorting again -- it does not
    /// know about a hand-made arrangement, so its own sort could disagree with
    /// the list it is showing.
    fn session_grouped(&self) -> bool {
        match &self.session.link {
            Some(_) => self.session.group,
            None => self.player.queue.lock().unwrap().grouped_now(),
        }
    }

    /// Does this instance own the session, and if not, ask the one that does.
    ///
    /// Returns true when the change should be made here. The single place that
    /// decides, so a mutation cannot be written to happen locally *and* be
    /// sent -- which is the shape the old `command` had, and how a volume
    /// change came to resume a paused player.
    #[must_use]
    fn owns(&self, remote: &str) -> bool {
        match &self.session.link {
            Some(m) => {
                m.send(remote);
                false
            }
            None => true,
        }
    }

    /// Tell the instance being mirrored, and nobody else.
    ///
    /// For the settings that never went through the command channel in the
    /// first place -- volume is a mutex the decode thread reads per buffer --
    /// so there is no local command to pair with the remote one.
    fn remote(&self, req: &str) {
        if let Some(m) = &self.session.link {
            m.send(req);
        }
    }

    /// Pull the leader's state into the local structures the UI already reads.
    ///
    /// Writing into the same PlayerState and Queue means every panel works
    /// unchanged; nothing in the rendering path knows whether it is looking at
    /// local playback or somebody else's.
    fn poll_mirror(&mut self) {
        let Some(m) = &self.session.link else { return };
        let Some(st) = m.poll() else {
            if !m.alive() {
                self.take_over();
            }
            return;
        };

        use std::sync::atomic::Ordering::Relaxed;
        let s = &self.player.state;
        s.playing.store(st.state != "stopped", Relaxed);
        s.paused.store(st.state == "paused", Relaxed);
        // Store the real rate, zero included: the technical line hides itself
        // when nothing is playing, and clamping to 1 made it read "0.0 kHz".
        s.sample_rate.store(st.rate, Relaxed);
        s.bit_depth.store(st.depth, Relaxed);
        s.channels.store(st.channels.max(1), Relaxed);
        s.bitrate_kbps.store(st.bitrate_kbps, Relaxed);
        s.codec.store(std::sync::Arc::new(st.codec.clone()));
        s.bit_perfect.store(st.bit_perfect, Relaxed);
        // Positions are held in frames, so a rate is still needed for the
        // conversion even before the leader reports one.
        let rate = if st.rate > 0 { st.rate } else { 44_100 };
        s.position_frames
            .store((st.position * rate as f64) as u64, Relaxed);
        s.duration_frames
            .store((st.duration * rate as f64) as u64, Relaxed);
        self.player.set_volume(st.volume);

        self.session.uri = st.uri.clone();
        self.session.group = st.group;
        self.session.their_view = st.view_revision;

        if !st.bands.is_empty() {
            self.session.bands = st.bands.clone();
        }

        // Refetch the queue only when the leader says it changed.
        if st.revision != self.session.revision {
            // Whole tracks, URIs and years included, so this window can name a
            // row to play it and can order the list by record itself.
            if let Some(items) = m.queue() {
                self.player.set_queue_tracks(items);
            }
            self.session.revision = st.revision;
        }

        {
            let mut q = self.player.queue.lock().unwrap();
            if q.shuffled() != st.shuffle {
                q.set_shuffle(st.shuffle);
            }
            if st.index >= 0 {
                // A position in the order, which is what the leader sends --
                // and following it is not a jump anybody asked for, so it must
                // not move the revision every frame.
                q.set_view_cursor(st.index as usize);
            }
        }
    }

    pub fn set_playlists(&mut self, playlists: Vec<PlaylistEntry>) {
        self.panels.picker = !playlists.is_empty();
        self.over.playlists = playlists;
    }

    /// Offer a previous session. Takes precedence over the playlist picker,
    /// since resuming answers the same question the picker asks.
    pub fn offer_resume(&mut self, s: Session) {
        if !s.worth_resuming() {
            return;
        }
        self.panels.picker = false;
        self.over.resume = Some(s);
    }

    /// Remember which file the queue came from, and what it is called.
    ///
    /// Both, because a playlist named on the command line used to leave the
    /// panel titled `queue` -- the name is only set on the paths that go
    /// through the picker or a resume. Anything keyed on the name, saving
    /// included, would then be talking about the wrong thing.
    pub fn set_source_playlist(&mut self, p: Option<PathBuf>) {
        if let Some(name) = p.as_ref().and_then(|p| p.file_stem()) {
            self.queue.name = name.to_string_lossy().into_owned();
        }
        self.queue.source = p;
    }

    /// Handle a key while the resume prompt is up.
    ///
    /// Returns true when the key was consumed. Anything unrecognised declines
    /// and falls through, so a press of `space` starts playing rather than
    /// being swallowed by a prompt the user has already answered by acting.
    fn answer_resume(&mut self, k: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        match k.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.accept_resume();
                true
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.decline_resume();
                true
            }
            KeyCode::Char('q') => {
                self.quit = true;
                true
            }
            _ => {
                self.decline_resume();
                false
            }
        }
    }

    /// Load the offered session and pick up where it stopped.
    fn accept_resume(&mut self) {
        let Some(s) = self.over.resume.take() else {
            return;
        };

        // Reload the playlist it came from, if it is still there.
        let mut restored = None;
        if let Some(path) = resume_playlist_path(&s, &self.over.playlists) {
            match crate::load_playlist(&path) {
                Ok(items) if !items.is_empty() => {
                    self.player.set_queue_tracks(items);
                    self.queue.name = s.playlist_name.clone();
                    self.queue.source = Some(path);
                    restored = Some(true);
                }
                _ => restored = Some(false),
            }
        } else if !s.playlist_name.is_empty() && s.playlist_name != DEFAULT_QUEUE_NAME {
            restored = Some(false);
        }

        {
            let mut q = self.player.queue.lock().unwrap();
            // Match on the URI rather than the stored index: the playlist may
            // have been edited since, and resuming into the wrong track is
            // worse than resuming to the top.
            let found = q
                .tracks()
                .iter()
                .position(|t| t.uri.to_string() == s.uri)
                .or(Some(s.index).filter(|i| *i < q.len()));
            if let Some(i) = found {
                q.jump_to(i);
            }
            if q.shuffled() != s.shuffle {
                q.set_shuffle(s.shuffle);
            }
        }

        // The view comes back with the music. Folding is remembered by album
        // title, so it lands on the same records even if the playlist has been
        // edited since -- which is the whole reason it is not stored by row.
        self.queue.folded = s.folded.iter().map(|t| t.to_lowercase()).collect();
        self.queue.fold_gen = self.queue.fold_gen.wrapping_add(1);
        if !s.album_order.is_empty() {
            let order: Vec<String> = s.album_order.iter().map(|t| t.to_lowercase()).collect();
            self.player.queue.lock().unwrap().set_manual_order(order);
        }

        self.player.set_volume(s.volume);
        if let Some(i) = self.player.queue.lock().unwrap().current_index() {
            self.player.send(Command::PlayIndex(i));
        }
        // Seek once the track is open. The command bus is ordered, so this
        // lands after the track change rather than racing it.
        self.player
            .send(Command::SeekTo(session::resume_position(s.position)));
        self.player.send(Command::Pause);

        // Where the list was left, if that position is still in it -- the
        // playlist may have been edited since. Otherwise the playing track,
        // which is where the cursor would have been put anyway.
        let playing = self.player.queue.lock().unwrap().view_cursor();
        let len = self.player.queue.lock().unwrap().len();
        self.queue.cursor = if s.cursor < len { s.cursor } else { playing };
        self.queue.scroll = 0;
        // Say when the playlist could not be brought back. Resuming the track
        // into whatever queue happened to be built is defensible, but doing it
        // silently leaves the user looking at the whole library wondering what
        // happened to their playlist.
        self.note(match restored {
            Some(false) => format!(
                "resumed the track, but \"{}\" is gone — showing the library",
                s.playlist_name
            ),
            _ => format!("resumed — {}", s.describe()),
        });
    }

    fn decline_resume(&mut self) {
        self.over.resume = None;
        session::Session::clear();
        // Fall through to the ordinary starting point.
        self.panels.picker = !self.over.playlists.is_empty();
    }

    /// Capture the current state, if there is anything worth capturing.
    fn snapshot(&self) -> Option<Session> {
        // Nothing playing means nothing to come back to. Saving anyway leaves a
        // placeholder that `worth_resuming` rejects but that still looks like a
        // live session on disk.
        if self.player.state.state() == PlayState::Stopped {
            return None;
        }
        let q = self.player.queue.lock().unwrap();
        let item = q.current()?.clone();
        let index = q.current_index()?;
        let (shuffle, repeat, total) = (q.shuffled(), q.repeat(), q.len());
        let q_manual = q.manual_order().to_vec();
        drop(q);

        Some(Session {
            playlist: self.queue.source.clone(),
            playlist_name: self.queue.name.clone(),
            index,
            total,
            position: self.player.state.position_secs(),
            duration: self.player.state.duration_secs(),
            artist: item.artist.clone().unwrap_or_default(),
            title: item.title.clone().unwrap_or_default(),
            uri: item.uri.to_string(),
            shuffle,
            repeat: repeat.to_string(),
            volume: self.player.volume(),
            cursor: self.queue.cursor,
            album_order: q_manual,
            folded: {
                // Sorted, so a session file does not churn between saves for
                // no reason anyone can see.
                let mut v: Vec<String> = self.queue.folded.iter().cloned().collect();
                v.sort();
                v
            },
            saved_at: crate::library::db::now_secs(),
        })
    }

    /// Save periodically, so a crash or a closed terminal loses little.
    fn save_session(&mut self, force: bool) {
        if !force && self.saving.last_written.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.saving.last_written = Instant::now();
        self.flush_settings();
        if let Some(s) = self.snapshot() {
            let _ = s.save();
        }
    }

    fn load_selected_playlist(&mut self) {
        let Some(entry) = self.over.playlists.get(self.over.picker_cursor) else {
            return;
        };
        let (path, name) = (entry.path.clone(), entry.name.clone());
        match crate::load_playlist(&path) {
            Ok(items) if !items.is_empty() => {
                let n = items.len();
                // The queue belongs to whichever instance owns the session; the
                // name and the path travel with the view either way.
                if self.owns(&format!("load-playlist {}", path.display())) {
                    self.player.set_queue_tracks(items);
                }
                self.queue.name = name.clone();
                // Loading a playlist you cannot see is not loading it.
                self.panels.playlist = true;
                // Remember where it came from, or the session saved from here
                // has no playlist to resume into and falls back to the whole
                // library. The picker is how a playlist is normally chosen, so
                // this was every session started the ordinary way.
                self.queue.source = Some(path);
                self.queue.cursor = 0;
                self.queue.scroll = 0;
                self.panels.picker = false;
                self.note(format!("{name} — {n} tracks"));
            }
            Ok(_) => self.note(format!("{name} is empty")),
            Err(e) => self.note(format!("{name}: {e}")),
        }
    }

    fn move_picker(&mut self, delta: i32) {
        if self.over.playlists.is_empty() {
            return;
        }
        let n = self.over.playlists.len() as i64;
        self.over.picker_cursor =
            ((self.over.picker_cursor as i64 + delta as i64).clamp(0, n - 1)) as usize;
    }

    pub fn run(&mut self) -> Result<()> {
        let mut term = term::init()?;
        let result = self.event_loop(&mut term);
        term::restore()?;
        self.shutdown();
        result
    }

    /// Release the socket here rather than leaving it to the IPC thread, which
    /// otherwise races process exit and loses. A leftover socket is recovered
    /// from on the next start, but leaving one behind is still untidy.
    fn shutdown(&mut self) {
        if !self.is_mirror() {
            self.save_session(true);
        }
        // Even a mirror has settings of its own -- its theme, its panels --
        // and they are not the leader's to hold.
        self.flush_settings();
        self.ipc_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(p) = self.ipc_path.take() {
            let _ = std::fs::remove_file(p);
        }
    }

    fn event_loop(&mut self, term: &mut term::Tui) -> Result<()> {
        while !self.quit {
            let now = Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.25);
            self.last_frame = now;

            if self.vis.mode != VisMode::Off {
                if self.player.tap.read(&mut self.vis.tap_buf) {
                    if self.vis.mode.uses_fluid() {
                        self.feed_fluid(dt);
                        self.vis.meters.update(self.vis.fluid.bands(), dt);
                        self.player.publish_bands(self.vis.fluid.bands());
                    } else {
                        self.vis.analyzer.analyze(&self.vis.tap_buf, dt);
                        self.vis.meters.update(self.vis.analyzer.bands(), dt);
                        // Publish for any instance mirroring this one.
                        self.player.publish_bands(self.vis.analyzer.bands());
                    }
                    if self.vis.mode.needs_waveform() {
                        // The trace modes want the newest samples, not a
                        // spectrum.
                        let n = self.vis.wave.len().min(self.vis.tap_buf.len());
                        self.vis.wave[..n]
                            .copy_from_slice(&self.vis.tap_buf[self.vis.tap_buf.len() - n..]);
                    }
                } else if !self.session.bands.is_empty() {
                    // A window following another has no audio of its own --
                    // its player is detached and its tap never fills -- so the
                    // spectrum arrives over the socket instead. The leader has
                    // been sending it all along; this is where it is used.
                    //
                    // The count is the leader's, taken from the leader's width.
                    // `Meters::update` resizes when it changes and the panel
                    // resamples to its own bar count, so two windows of
                    // different widths both draw a full spectrum.
                    self.vis.meters.update(&self.session.bands.clone(), dt);
                }
            }
            self.advance_seek_phase(dt);
            self.advance_retry_phase(dt);
            self.vis.onset.feed(self.vis.analyzer.bands(), dt);
            self.advance_effects(dt);

            if now.duration_since(self.look.last_marquee) >= MARQUEE_STEP {
                self.look.marquee = self.look.marquee.wrapping_add(1);
                self.look.last_marquee = now;
            }

            self.poll_mirror();
            self.sync_view();
            self.check_track_change();
            if self.over.resume.is_none() && !self.is_mirror() {
                self.save_session(false);
            }
            self.publish_mpris();
            term.draw(|f| self.draw(f.area(), f.buffer_mut()))?;

            if event::poll(FRAME)? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        if let Some(e) = self.fx.running.as_mut() {
                            e.finish();
                        }
                        // The resume prompt answers raw keys rather than
                        // actions: `n` should decline without becoming a
                        // global binding that shadows something else.
                        if self.over.resume.is_some() && self.answer_resume(k) {
                            continue;
                        }
                        // Naming a playlist eats keys ahead of everything, so
                        // `l` in a playlist name does not open the browser.
                        if self.edit.naming.is_some() && self.name_type(k) {
                            continue;
                        }
                        // So does the filter box, for the same reason.
                        if self.edit.typing.is_some() && self.filter_type(k) {
                            continue;
                        }
                        // The search line is the only other thing that eats raw
                        // keys, and for the same reason as the resume prompt.
                        if self.library_type(k) {
                            continue;
                        }
                        let action = if self.over.library.is_some() {
                            keymap::library(k).or_else(|| keymap::resolve(k))
                        } else {
                            keymap::resolve(k)
                        };
                        if let Some(action) = action {
                            self.handle(action);
                        }
                    }
                    Event::Mouse(m) => {
                        if let Some(e) = self.fx.running.as_mut() {
                            e.finish();
                        }
                        let size = term.size()?;
                        self.handle_mouse(m, Rect::new(0, 0, size.width, size.height));
                    }
                    // A font zoom arrives as a resize, and it changes the
                    // cell size the button images were built for.
                    Event::Resize(..) => self.graphics.remeasure(),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Write a setting back to `config.toml`, so a change made from the UI is
    /// still there next time.
    ///
    /// Best effort by design: a read-only config, or one somewhere staramp
    /// cannot write, should not stop the setting taking effect for this
    /// session. The failure is reported rather than swallowed, because a
    /// change that silently will not persist is worse than one that says so.
    /// Hold a setting to be written to `config.toml`.
    ///
    /// Held rather than written: dragging the volume slider changes a setting
    /// on every mouse event, and `edit::set` reads the whole file, rewrites it
    /// and renames it over the original each time. Held changes are flushed on
    /// the session's own timer and again on the way out, so the file is
    /// written at most once every few seconds however hard a slider is pulled.
    fn remember(&mut self, section: &str, key: &str, value: Value) {
        self.saving
            .pending
            .insert((section.to_string(), key.to_string()), value);
    }

    /// Every persistent setting as it stands now.
    fn settings_now(&self) -> Remembered {
        let q = self.player.queue.lock().unwrap();
        let (shuffle, repeat) = (q.shuffled(), q.repeat().to_string().to_lowercase());
        drop(q);
        Remembered {
            theme: self.look.ids[self.look.index].clone(),
            volume: self.player.volume(),
            seek_style: self.look.seek_style.name().to_string(),
            graphics: self.graphics.mode().name().to_string(),
            buttons: self.graphics.buttons_mode().name().to_string(),
            show_album: self.panels.album,
            show_equalizer: self.panels.eq,
            show_playlist: self.panels.playlist,
            vis_mode: self.vis.mode.name().to_string(),
            bar_width: self.vis.bars.width,
            bar_gap: self.vis.bars.gap,
            animations: self.fx.active,
            fetch_art: self.art_fetch(),
            group_by: self.group_by_name(),
            group_desc: self.queue.group_desc,
            shuffle,
            repeat,
            eq_enabled: self.eq.enabled,
            eq_preset: eq::PRESETS[self.eq.preset].name.to_string(),
            eq_preamp: self.eq.preamp,
            eq_gains: self.eq.gains,
        }
    }

    fn group_by_name(&self) -> String {
        match self.player.queue.lock().unwrap().grouping() {
            Some(_) => "album".into(),
            None => "off".into(),
        }
    }

    /// Queue up whatever has changed since the file was last written.
    ///
    /// A diff rather than a write per keystroke, so an untouched setting never
    /// appears in the file and a hand-written one keeps its comment.
    /// Queue up whatever *this window* has changed since it last wrote.
    ///
    /// Deliberately diffed against what this window last saved rather than
    /// against the file. Re-reading the file first looks like the fix for two
    /// windows disagreeing and is the opposite: a setting another window
    /// changed would then read as a difference here, and this one would write
    /// its own older value straight back over it. Diffing against what we
    /// wrote means a window only ever writes what it actually changed, and
    /// leaves every other key alone.
    fn collect_settings(&mut self) {
        let now = self.settings_now();
        let was = self.saving.saved.clone();
        if now == was {
            return;
        }
        let mut set = |section: &str, key: &str, v: Value| {
            self.saving
                .pending
                .insert((section.to_string(), key.to_string()), v);
        };
        use crate::config::edit::ROOT;
        if now.theme != was.theme {
            set(ROOT, "theme", Value::Str(now.theme.clone()));
        }
        if now.volume != was.volume {
            set(ROOT, "volume", Value::Float(now.volume as f64));
        }
        if now.seek_style != was.seek_style {
            set("ui", "seek_style", Value::Str(now.seek_style.clone()));
        }
        if now.graphics != was.graphics {
            set("ui", "graphics", Value::Str(now.graphics.clone()));
        }
        if now.buttons != was.buttons {
            set("ui", "buttons", Value::Str(now.buttons.clone()));
        }
        if now.show_album != was.show_album {
            set("ui", "show_album", Value::Bool(now.show_album));
        }
        if now.show_equalizer != was.show_equalizer {
            set("ui", "show_equalizer", Value::Bool(now.show_equalizer));
        }
        if now.show_playlist != was.show_playlist {
            set("ui", "show_playlist", Value::Bool(now.show_playlist));
        }
        if now.vis_mode != was.vis_mode {
            set("vis", "mode", Value::Str(now.vis_mode.clone()));
        }
        if now.bar_width != was.bar_width {
            set("vis", "bar_width", Value::Int(now.bar_width as i64));
        }
        if now.bar_gap != was.bar_gap {
            set("vis", "bar_gap", Value::Int(now.bar_gap as i64));
        }
        if now.animations != was.animations {
            set("fx", "enabled", Value::Bool(now.animations));
        }
        if now.fetch_art != was.fetch_art {
            set("art", "fetch", Value::Bool(now.fetch_art));
        }
        if now.group_by != was.group_by {
            set("playlist", "group_by", Value::Str(now.group_by.clone()));
        }
        if now.group_desc != was.group_desc {
            set("playlist", "group_desc", Value::Bool(now.group_desc));
        }
        if now.shuffle != was.shuffle {
            set("playlist", "shuffle", Value::Bool(now.shuffle));
        }
        if now.repeat != was.repeat {
            set("playlist", "repeat", Value::Str(now.repeat.clone()));
        }
        if now.eq_enabled != was.eq_enabled {
            set("eq", "enabled", Value::Bool(now.eq_enabled));
        }
        if now.eq_preset != was.eq_preset {
            set("eq", "preset", Value::Str(now.eq_preset.clone()));
        }
        if now.eq_preamp != was.eq_preamp {
            set("eq", "preamp", Value::Float(now.eq_preamp as f64));
        }
        if now.eq_gains != was.eq_gains {
            let g = now.eq_gains.iter().map(|v| *v as f64).collect();
            set("eq", "gains", Value::Floats(g));
        }
        self.saving.saved = now;
    }

    /// Write everything held, oldest key first.
    ///
    /// Best effort: a config that cannot be written is a setting that lasts
    /// the session, not a player that stops.
    fn flush_settings(&mut self) {
        self.collect_settings();
        if self.saving.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.saving.pending);
        let path = match crate::paths::config_file() {
            Ok(p) => p,
            Err(e) => return self.note(format!("cannot find config.toml: {e}")),
        };
        for ((section, key), value) in pending {
            if let Err(e) = crate::config::edit::set(&path, &section, &key, &value) {
                self.note(format!("saved for this session only: {e}"));
                return;
            }
        }
    }

    /// Move the seek bar's highlight along.
    ///
    /// Only while playing, and never when the user has asked for no motion.
    /// The music drives its pace when the reactive setting is on, the same as
    /// the title transition, so the bar breathes with the track rather than
    /// sweeping to its own clock.
    fn advance_seek_phase(&mut self, dt: f32) {
        if !self.fx.active || self.player.state.state() != PlayState::Playing {
            self.look.seek_phase = 0.0;
            return;
        }
        let step = if self.fx.reactive {
            reactive_dt(dt, self.vis.onset.energy())
        } else {
            dt
        };
        self.look.seek_phase = (self.look.seek_phase + step / SEEK_SHEEN_PERIOD).fract();
    }

    /// Step the retry highlight, and notice when the lookup has come back.
    fn advance_retry_phase(&mut self, dt: f32) {
        // The worker republishes the same URI, so the only way to know the
        // answer is a new one is that it has published again.
        if let (Some(at), Some(w)) = (self.retrying, self.art.as_ref()) {
            if w.serial() != at {
                self.retrying = None;
            }
        }
        if self.retrying.is_none() {
            self.retry_phase = 0.0;
            return;
        }
        if !self.fx.active {
            return;
        }
        self.retry_phase = (self.retry_phase + dt / album::SWEEP_PERIOD).fract();
    }

    /// How the retry word should be drawn this frame.
    fn retry_look(&self) -> album::Retry {
        match (self.retrying.is_some(), self.fx.active) {
            (false, _) => album::Retry::Idle,
            (true, false) => album::Retry::Waiting,
            (true, true) => album::Retry::Working(self.retry_phase),
        }
    }

    /// Advance the fluid analysis by one frame.
    ///
    /// One bar per column, so the band count follows the panel width and is
    /// re-planned whenever it changes. The whole tap window is transformed
    /// every frame: the smoothing lives in the envelope followers, which are
    /// defined against elapsed time, so overlapping windows cost nothing and
    /// a short frame simply moves the bars less far.
    fn feed_fluid(&mut self, dt: f32) {
        use std::sync::atomic::Ordering::Relaxed;
        let rate = self.player.state.sample_rate.load(Relaxed).max(8_000) as f32;
        self.vis.fluid.set_bands(self.vis.fluid_bars.max(1), rate);
        self.vis.fluid.set_rate(rate);
        self.vis.fluid.analyze(&self.vis.tap_buf, dt);
    }

    /// Notice a track change and kick off the title transition.
    fn check_track_change(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        let rev = self.player.state.track_revision.load(Relaxed);
        if rev == self.last_track_revision {
            return;
        }
        self.last_track_revision = rev;
        let title = self
            .player
            .current_item()
            .map(|i| match (&i.artist, &i.title) {
                (Some(a), Some(t)) => format!("{a} — {t}"),
                (None, Some(t)) => t.clone(),
                _ => i.uri.to_string(),
            })
            .unwrap_or_default();
        self.begin_title_effect(&title);
        self.pump_album();
        if let Some(m) = &self.mpris {
            m.notify(crate::mpris::MprisEvent::TrackChanged);
        }
    }

    /// Step the title transition, letting the music set its pace.
    fn advance_effects(&mut self, dt: f32) {
        let Some(e) = self.fx.running.as_mut() else {
            return;
        };
        let step = if self.fx.reactive {
            reactive_dt(dt, self.vis.onset.energy())
        } else {
            dt
        };
        e.advance(step);
        self.fx.title = e.render();
        if e.finished() {
            self.fx.running = None;
        }
    }

    /// Start a title transition for a newly playing track.
    fn begin_title_effect(&mut self, title: &str) {
        if !self.fx.active || self.fx.kind == EffectKind::None {
            self.fx.running = None;
            return;
        }
        let mut e = TextEffect::new(
            self.fx.kind,
            title,
            self.fx.duration,
            self.last_track_revision.wrapping_mul(2654435761),
        );
        self.fx.title = e.render();
        self.fx.running = Some(e);
    }

    /// Tell the desktop about anything that changed since the last frame.
    fn publish_mpris(&mut self) {
        let Some(m) = &self.mpris else { return };

        let state = self.player.state.state();
        if state != self.last_state {
            self.last_state = state;
            m.notify(crate::mpris::MprisEvent::StateChanged);
        }
    }

    fn handle(&mut self, action: Action) {
        use Action::*;

        // The picker is modal: it owns navigation while open, so `enter` loads
        // a playlist rather than playing whatever the queue cursor was on.
        // Settings are modal while open: the arrows move the list rather than
        // the playlist behind it.
        if let Some(st) = &self.over.settings {
            let last = st.rows.len().saturating_sub(1) as i32;
            match action {
                CursorUp => return self.move_settings(-1),
                CursorDown => return self.move_settings(1),
                PageUp | Home => return self.move_settings(-last - 1),
                PageDown | End => return self.move_settings(last + 1),
                Activate => return self.take_setting(),
                CloseOverlay => {
                    self.over.settings = None;
                    return;
                }
                // Asking for the list that is already up closes it, the way
                // every other overlay here behaves. Asking for the other one
                // switches, which `open_filter` sorts out.
                OpenFilter => return self.open_filter(),
                FilterQueue => {
                    self.over.library = None;
                    return self.open_filter_box();
                }
                Quit => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }
        if let Some(c) = &self.over.chooser {
            let last = c.rows.len().saturating_sub(1);
            match action {
                CursorUp => return self.move_chooser(-1),
                CursorDown => return self.move_chooser(1),
                PageUp => return self.move_chooser(-10),
                PageDown => return self.move_chooser(10),
                Home => return self.move_chooser(-(last as i32) - 1),
                End => return self.move_chooser(last as i32 + 1),
                Activate => return self.take_chosen_cover(),
                CloseOverlay | ChooseCover | ToggleAlbumPanel => {
                    self.over.chooser = None;
                    return;
                }
                Quit => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }
        if self.panels.picker {
            match action {
                CursorUp => return self.move_picker(-1),
                CursorDown => return self.move_picker(1),
                PageUp => return self.move_picker(-10),
                PageDown => return self.move_picker(10),
                Home => {
                    self.over.picker_cursor = 0;
                    return;
                }
                End => {
                    self.over.picker_cursor = self.over.playlists.len().saturating_sub(1);
                    return;
                }
                Activate => return self.load_selected_playlist(),
                CloseOverlay | TogglePlaylistPanel | OpenPlaylistPicker => {
                    // Refusing to close an empty picker would trap the user with
                    // nothing to pick and no way out.
                    self.panels.picker = false;
                    return;
                }
                Quit => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }

        // The help is modal to the keyboard, which it never was: every key
        // used to act on the player behind it, and now one of them removes
        // rows you cannot see while you are reading about them.
        if self.panels.help {
            match action {
                Help | CloseOverlay => {
                    self.panels.help = false;
                    self.panels.help_scroll = 0;
                    return;
                }
                CursorUp => {
                    self.panels.help_scroll = self.panels.help_scroll.saturating_sub(1);
                    return;
                }
                CursorDown => {
                    self.panels.help_scroll += 1;
                    return;
                }
                PageUp | Home => {
                    self.panels.help_scroll = 0;
                    return;
                }
                PageDown | End => {
                    self.panels.help_scroll += 10;
                    return;
                }
                Quit => {
                    self.quit = true;
                    return;
                }
                _ => return,
            }
        }

        // After the overlays, because the target picker and the confirmations
        // open on top of the browser and must keep the keys while they are up.
        // Before the main match, or the playlist behind it would take the
        // cursor keys.
        if let Some(lib) = &mut self.over.library {
            let page = self.panels.last_area.height.saturating_sub(6).max(1) as i32;
            match action {
                CursorUp => return lib.step(-1),
                CursorDown => return lib.step(1),
                PageUp => return lib.step(-page),
                PageDown => return lib.step(page),
                Home => return lib.jump(false),
                End => return lib.jump(true),
                LibraryLeft => return lib.shift(-1),
                LibraryRight | FocusNext => return lib.shift(1),
                LibrarySearch => {
                    lib.typing = true;
                    return;
                }
                LibraryAdd => return self.library_add(false),
                SavePlaylist => return self.save_playlist(),
                LibraryAddAlbum => return self.library_add(true),
                OpenLibrary | CloseOverlay => {
                    self.over.library = None;
                    // Leaving for the player forgets what `space` meant, so
                    // coming back asks again rather than acting on an answer
                    // given about a different sitting.
                    self.edit.add_mode = None;
                    self.edit.adding = None;
                    return;
                }
                Quit => {
                    self.quit = true;
                    return;
                }
                // The rows these act on are behind the browser. Falling
                // through would tag, move and -- worst -- delete a list that is
                // not on screen, unlike `z` and `b` below, which act on music
                // you can still hear.
                TagRow | ClearTags | CopyTagged | PasteTagged | MoveTagged | RemoveTagged => {
                    return self.note("no rows to tag here \u{2014} esc goes back".into())
                }
                // Everything else falls through on purpose: the music is still
                // playing behind this, and skipping a track should not mean
                // leaving the browser.
                _ => {}
            }
        }

        match action {
            Quit => self.quit = true,
            OpenLibrary => self.open_library(),
            LibraryLeft | LibraryRight | LibrarySearch | LibraryAdd | LibraryAddAlbum => {}
            TagRow => self.tag_row(),
            ClearTags => self.clear_tags(),
            CopyTagged => self.copy_tagged(),
            PasteTagged => self.paste_tagged(),
            MoveTagged => self.move_tagged(),
            RemoveTagged => self.remove_tagged(),
            SavePlaylist => self.save_playlist(),

            CloseOverlay => self.panels.help = false,
            Help => self.panels.help = !self.panels.help,
            PlayPause => {
                if self.player.state.state() == PlayState::Stopped {
                    self.play_track_at_cursor();
                } else {
                    self.command(&Command::TogglePause, "toggle");
                }
            }
            Stop => self.command(&Command::Stop, "stop"),
            Next => self.command(&Command::Next, "next"),
            Prev => self.command(&Command::Prev, "prev"),
            SeekForward => self.seek_by(5.0),
            SeekBack => self.seek_by(-5.0),
            SeekForwardBig => self.seek_by(30.0),
            SeekBackBig => self.seek_by(-30.0),
            VolumeUp => self.nudge_volume(KEY_VOLUME_STEP),
            VolumeDown => self.nudge_volume(-KEY_VOLUME_STEP),
            ToggleShuffle => {
                // Asked for as a value, not as a flip: two windows toggling at
                // once race each other into disagreeing.
                let on = !self.player.queue.lock().unwrap().shuffled();
                if self.owns(&format!("set-shuffle {on}")) {
                    self.player.queue.lock().unwrap().set_shuffle(on);
                }
                self.follow_order();
                self.note(if on { "shuffle on" } else { "shuffle off" }.into());
            }
            ShuffleNow => {
                if self.owns("shuffle-now") && self.player.shuffle_now().is_none() {
                    return self.note("nothing to shuffle".into());
                }
                // The list has been reordered; follow it rather than leaving
                // the cursor on an unrelated row.
                self.follow_order();
                self.note("shuffled".into());
            }
            CycleRepeat => {
                let next = self.player.queue.lock().unwrap().repeat().cycle();
                if self.owns(&format!("set-repeat {next}")) {
                    self.player.queue.lock().unwrap().set_repeat(next);
                }
                self.note(format!("repeat {next}"));
            }
            OpenFilter => self.open_filter(),
            FilterQueue => self.open_filter_box(),
            MoveAlbumUp => self.move_album(-1),
            MoveAlbumDown => self.move_album(1),
            ToggleEqPanel => self.panels.eq = !self.panels.eq,
            ChooseCover => self.open_chooser(),
            RetryCover => self.retry_cover(),
            ToggleAlbumPanel => {
                self.panels.album = !self.panels.album;
                if self.panels.album {
                    // Ask straight away rather than waiting for the next track:
                    // opening the panel onto an empty frame looks broken.
                    self.art_uri = None;
                    // Says why a cover is blocky, which is otherwise a mystery.
                    self.note(format!("album \u{2014} {}", self.graphics.name()))
                } else {
                    // Kitty holds uploaded images in the terminal's own
                    // memory; letting the protocol go is what releases one.
                    self.graphics.forget();
                    if self.panels.focus == Focus::Album {
                        self.panels.focus = Focus::Player;
                    }
                    self.note("album closed \u{2014} i to bring it back".into())
                }
            }
            // Just the panel. This used to open the picker whenever there was
            // one to open, which left no way to bring a closed panel back and
            // made the key useless for the thing it is named after.
            TogglePlaylistPanel => {
                self.panels.playlist = !self.panels.playlist;
                if self.panels.playlist {
                    self.panels.focus = Focus::Playlist;
                } else if self.panels.focus == Focus::Playlist {
                    self.panels.focus = Focus::Player;
                }
            }
            OpenPlaylistPicker => {
                if self.over.playlists.is_empty() {
                    self.note("no playlists — set playlist_dir in config.toml".into());
                } else {
                    self.panels.picker = true;
                }
            }
            ToggleVisualizer => {
                self.vis.mode = self.vis.mode.next();
                self.note(format!("visualizer: {}", self.vis.mode.name()));
            }
            PrevVisualizer => {
                self.vis.mode = self.vis.mode.prev();
                self.note(format!("visualizer: {}", self.vis.mode.name()));
            }
            ToggleEqEnabled => {
                self.eq.enabled = !self.eq.enabled;
                self.apply_eq();
                self.note(if self.eq.enabled { "eq on" } else { "eq off" }.into());
            }
            NextEqPreset => self.step_preset(1),
            PrevEqPreset => self.step_preset(-1),
            ToggleAnimations => {
                self.fx.active = !self.fx.active;
                if !self.fx.active {
                    // Drop what is mid-flight rather than leaving a half-drawn
                    // title on screen until the next track.
                    self.fx.running = None;
                    self.look.seek_phase = 0.0;
                }
                let on = self.fx.active;
                self.note(
                    if on {
                        "animations on"
                    } else {
                        "animations off"
                    }
                    .into(),
                );
            }
            NextSeekStyle => {
                self.look.seek_style = self.look.seek_style.next();
                let name = self.look.seek_style.name();
                self.note(format!("seek bar: {name}"));
            }
            NextButtons => {
                let next = self.graphics.buttons_mode().next();
                self.graphics.set_buttons_mode(next);
                // Say what was actually got, not what was asked for: on a
                // terminal with no protocol `auto` and `text` look the same,
                // and a toggle that reports a change nobody can see is worse
                // than one that explains itself.
                let note = match (next, self.graphics.pictures_available()) {
                    (crate::ui::graphics::Buttons::Auto, true) => {
                        "transport buttons: pictures".to_string()
                    }
                    (crate::ui::graphics::Buttons::Auto, false) => {
                        "transport buttons: pictures, but this terminal has no \
                         protocol for them -- still text"
                            .to_string()
                    }
                    (crate::ui::graphics::Buttons::Text, _) => {
                        "transport buttons: text".to_string()
                    }
                };
                self.note(note);
            }
            WidenBars | NarrowBars => {
                let by = if action == WidenBars { 1 } else { -1 };
                let next = self.vis.bars.resized(by);
                if next == self.vis.bars {
                    self.note("visualizer bars are as wide as they go".into());
                } else {
                    self.vis.bars = next;
                    self.note(format!("bar width {}", next.width));
                }
            }
            NextTheme => {
                self.look.index = (self.look.index + 1) % self.look.ids.len();
                let id = self.look.ids[self.look.index].clone();
                let (t, why) = builtin::resolve_named(&id);
                self.note(format!("{} — {why}", t.name));
                self.look.theme = t;
            }
            FocusNext => {
                self.panels.focus = match self.panels.focus {
                    Focus::Player if self.panels.album => Focus::Album,
                    Focus::Player | Focus::Album if self.panels.eq => Focus::Equalizer,
                    Focus::Album => Focus::Playlist,
                    Focus::Player => Focus::Playlist,
                    Focus::Equalizer => Focus::Playlist,
                    Focus::Playlist => Focus::Player,
                };
            }
            CursorUp => self.move_cursor(-1),
            CursorDown => self.move_cursor(1),
            PageUp => self.move_cursor(-10),
            PageDown => self.move_cursor(10),
            Home => self.cursor_to_end(false),
            End => self.cursor_to_end(true),
            Activate => self.play_track_at_cursor(),
        }
    }

    fn seek_by(&mut self, delta: f64) {
        self.command(&Command::SeekBy(delta), &format!("seek {delta}"));
        if let Some(m) = &self.mpris {
            let pos = (self.player.state.position_secs() + delta).max(0.0);
            m.notify(crate::mpris::MprisEvent::Seeked(pos));
        }
    }

    fn step_preset(&mut self, delta: i32) {
        let n = eq::PRESETS.len() as i32;
        self.eq.preset = (((self.eq.preset as i32 + delta) % n + n) % n) as usize;
        self.eq.gains = eq::PRESETS[self.eq.preset].gains;
        self.eq.enabled = true;
        self.apply_eq();
        self.note(format!("eq: {}", eq::PRESETS[self.eq.preset].name));
    }

    fn apply_eq(&self) {
        let rate = self
            .player
            .state
            .sample_rate
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(8_000) as u32;
        self.player.set_eq(EqSettings::build(
            self.eq.enabled,
            self.eq.preamp,
            &self.eq.gains,
            rate,
        ));
    }

    /// Play whatever the cursor is on.
    ///
    /// By name rather than by position. This used to go straight to the local
    /// player, which is why it did nothing at all in a second window: that
    /// window's player is detached and its command receiver dropped, so the
    /// keypress vanished without a word. A position is no use over the wire
    /// either -- the two instances agree on the track, not on where it sits --
    /// so the URI is what gets sent.
    fn play_track_at_cursor(&mut self) {
        let Some(track) = self.cursor_track() else {
            return;
        };
        let uri = self
            .player
            .queue
            .lock()
            .unwrap()
            .tracks()
            .get(track)
            .map(|t| t.uri.to_string());
        match uri {
            Some(uri) => self.command(&Command::PlayIndex(track), &format!("play-uri {uri}")),
            None => self.player.send(Command::PlayIndex(track)),
        }
    }

    /// The track the cursor is on, translating from shown position to storage
    /// index. They differ whenever shuffle is on.
    fn cursor_track(&self) -> Option<usize> {
        self.player
            .queue
            .lock()
            .unwrap()
            .view()
            .get(self.queue.cursor)
            .copied()
    }

    /// Move the cursor by `delta` *visible* tracks.
    ///
    /// Through the rows rather than through the queue, so a folded record is
    /// stepped over instead of into. Ungrouped, every track is visible and
    /// this is the same clamp it always was.
    fn move_cursor(&mut self, delta: i32) {
        let n = self.player.queue.lock().unwrap().len();
        if n == 0 {
            return;
        }
        let next = if self.queue.rows.is_empty() {
            (self.queue.cursor as i64 + delta as i64).clamp(0, n as i64 - 1) as usize
        } else {
            self.queue.rows.step(self.queue.cursor, delta)
        };
        self.set_cursor(next);
    }

    /// The first or last track on show, for `home` and `end`.
    fn cursor_to_end(&mut self, last: bool) {
        let n = self.player.queue.lock().unwrap().len();
        if n == 0 {
            return;
        }
        let i = match self.queue.rows.ends() {
            Some((first, end)) => {
                if last {
                    end
                } else {
                    first
                }
            }
            None => {
                if last {
                    n - 1
                } else {
                    0
                }
            }
        };
        self.set_cursor(i);
    }

    /// Put the cursor back on whatever is playing, after the order moved.
    fn follow_order(&mut self) {
        self.queue.cursor = self.player.queue.lock().unwrap().view_cursor();
        self.queue.scroll = 0;
    }

    /// Move the record the cursor is in, one place up or down the list.
    ///
    /// The first move writes the order down as it currently stands and then
    /// edits that; every move after edits the same list. Arranging by hand has
    /// to start from something, and the something people expect is what they
    /// are already looking at.
    fn move_album(&mut self, delta: i32) {
        let q = self.player.queue.lock().unwrap();
        if !q.grouped_now() {
            drop(q);
            return self.note("records can only be arranged in album order".into());
        }
        let items: Vec<crate::playlist::queue::QueueItem> = q
            .view()
            .iter()
            .filter_map(|&i| q.tracks().get(i).cloned())
            .collect();
        let Some(track) = q.view().get(self.queue.cursor).copied() else {
            drop(q);
            return;
        };
        // Which record the cursor is in, named the way the order names it.
        let Some(here) = crate::playlist::group::keys(&items)
            .get(self.queue.cursor)
            .map(|k| {
                k.as_ref()
                    .map(|k| k.title().to_string())
                    .unwrap_or_default()
            })
        else {
            drop(q);
            return;
        };

        let mut order = q.manual_order().to_vec();
        if order.is_empty() {
            order = crate::playlist::group::titles_in_order(&items);
        }
        let Some(at) = order.iter().position(|t| *t == here) else {
            drop(q);
            return;
        };
        let to = at as i32 + delta;
        if to < 0 || to as usize >= order.len() {
            drop(q);
            return self.note(
                if delta < 0 {
                    "that record is already first"
                } else {
                    "that record is already last"
                }
                .into(),
            );
        }
        order.swap(at, to as usize);
        drop(q);
        if self.owns(&format!("set-album-order {}", order.join("\u{1f}"))) {
            self.player.queue.lock().unwrap().set_manual_order(order);
        }
        // Follow the record: the cursor was on a track, and the track moved.
        let cursor = self
            .player
            .queue
            .lock()
            .unwrap()
            .view_position(track)
            .unwrap_or(self.queue.cursor);
        self.queue.cursor = cursor;
        self.note(format!(
            "moved {} {}",
            if here.is_empty() {
                "the untagged tracks"
            } else {
                &here
            },
            if delta < 0 { "up" } else { "down" }
        ))
    }

    /// Fold a record away, or open it again.
    fn toggle_fold(&mut self, title: &str) {
        if !self.queue.folded.remove(title) {
            self.queue.folded.insert(title.to_string());
        }
        self.queue.fold_gen = self.queue.fold_gen.wrapping_add(1);
    }

    fn set_cursor(&mut self, i: usize) {
        self.queue.cursor = i;
    }

    fn note(&mut self, msg: String) {
        self.status = Some((msg, Instant::now()));
    }

    /// Route a mouse event to whatever is under the pointer.
    ///
    /// Every hit test goes through `regions` and the panels' own geometry
    /// functions, so a click lands on the thing the user can see rather than on
    /// a rect reconstructed by eye.
    fn handle_mouse(&mut self, m: MouseEvent, full: Rect) {
        // Overlays are modal: while one is up nothing behind it can be clicked.
        if self.over.resume.is_some() || self.panels.help {
            return;
        }
        let Some(r) = self.regions(full) else { return };
        let (x, y) = (m.column, m.row);

        // Every state `draw` returns early for needs a branch here, or the
        // panels underneath -- drawn, but stale -- take the click. The cover
        // chooser had no branch at all, which is why clicking under it could
        // start a different track.
        if self.over.settings.is_some() {
            return self.settings_mouse(m, r.area);
        }
        if self.over.chooser.is_some() {
            return;
        }
        if self.panels.picker {
            return self.picker_mouse(m, r.area);
        }
        if self.over.library.is_some() {
            return self.library_mouse(m, r.area);
        }

        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) => match self.edit.drag {
                Some(Drag::Seek) => self.seek_to_x(&r, x),
                Some(Drag::Volume) => self.volume_at_x(&r, x),
                Some(Drag::EqBand(b)) => {
                    if let Some(rect) = r.equalizer {
                        self.set_eq_band(rect, b, y);
                    }
                }
                None => {}
            },
            MouseEventKind::Up(MouseButton::Left) => self.edit.drag = None,

            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                let g = self.player_geometry(&r);
                if let Some(g) = &g {
                    // Over a control, the wheel adjusts that control.
                    if g.controls.volume.is_some_and(|v| hit(v, x, y)) {
                        return self.nudge_volume(if up { VOLUME_STEP } else { -VOLUME_STEP });
                    }
                    if hit(g.seek_row, x, y) {
                        return self.seek_by(if up { 5.0 } else { -5.0 });
                    }
                    if hit(g.visualizer, x, y) {
                        return self.handle(if up {
                            Action::ToggleVisualizer
                        } else {
                            Action::PrevVisualizer
                        });
                    }
                }
                if let Some(rect) = r.album {
                    if hit(rect, x, y) {
                        let g = crate::ui::panels::album::geometry(
                            rect,
                            self.show_cover(),
                            self.cell_aspect(),
                        );
                        if g.is_some_and(|g| hit(g.art, x, y)) {
                            return self.cycle_cover(if up { -1 } else { 1 });
                        }
                        return;
                    }
                }
                if let Some(rect) = r.equalizer {
                    if hit(rect, x, y) {
                        if let Some(g) = equalizer::geometry(rect, eq::PRESETS[self.eq.preset].name)
                        {
                            if let Some(b) = g.band_at(x) {
                                let step = if up { 1.0 } else { -1.0 };
                                return self.nudge_eq_band(b, step);
                            }
                        }
                        return;
                    }
                }
                // Otherwise it scrolls the list, which is what a wheel is for.
                self.move_cursor(if up { -3 } else { 3 });
            }

            MouseEventKind::Down(MouseButton::Left) => {
                let double = self.register_click(x, y);

                // The close mark first: it sits on the panel's own border, so
                // the panel would otherwise swallow the click.
                if let Some(rect) = r.playlist {
                    // The same list the renderer drew, or the hit boxes land
                    // beside the words rather than on them.
                    let words = self.playlist_header();
                    match header::hit(rect, &words, x, y) {
                        Some(header::Item::Close) => {
                            self.panels.playlist = false;
                            self.panels.focus = Focus::Player;
                            return self.note("playlist closed — p to bring it back".into());
                        }
                        Some(header::Item::Settings) => return self.open_settings(Focus::Playlist),
                        Some(header::Item::Filter) => return self.open_filter(),
                        Some(header::Item::Copy) => return self.copy_tagged(),
                        Some(header::Item::Put(_)) => return self.paste_tagged(),
                        Some(header::Item::Move) => return self.move_tagged(),
                        Some(header::Item::Remove) => return self.remove_tagged(),
                        Some(header::Item::Untag) => return self.clear_tags(),
                        // The count names the mode; it is not a button.
                        Some(header::Item::Tagged(_)) => return,
                        None => {}
                    }
                    if hit(rect, x, y) {
                        return self.playlist_click(rect, x, y, double);
                    }
                }
                if let Some(rect) = r.album {
                    match header::hit(rect, header::PLAIN, x, y) {
                        Some(header::Item::Close) => {
                            self.panels.album = false;
                            self.graphics.forget();
                            if self.panels.focus == Focus::Album {
                                self.panels.focus = Focus::Player;
                            }
                            return self.note("album closed \u{2014} i to bring it back".into());
                        }
                        Some(header::Item::Settings) => return self.open_settings(Focus::Album),
                        // Not offered by this panel, so never reported for it.
                        // Spelled out rather than matched with `_`, so the
                        // next word added here has to be thought about.
                        Some(header::Item::Filter) => {}
                        // Only the playlist has rows to tag.
                        Some(
                            header::Item::Tagged(_)
                            | header::Item::Copy
                            | header::Item::Put(_)
                            | header::Item::Move
                            | header::Item::Remove
                            | header::Item::Untag,
                        ) => {}
                        None => {}
                    }
                    if hit(rect, x, y) {
                        self.panels.focus = Focus::Album;
                        // The retry word first: it sits on the detail lines,
                        // which would otherwise swallow the click.
                        if self.album_retry_rect(rect).is_some_and(|r| hit(r, x, y)) {
                            return self.retry_cover();
                        }
                        // Clicking the picture asks for the next one. The
                        // ranking is right most of the time and not all of it,
                        // and this is the correction.
                        let g = crate::ui::panels::album::geometry(
                            rect,
                            self.show_cover(),
                            self.cell_aspect(),
                        );
                        if g.is_some_and(|g| hit(g.art, x, y)) {
                            return self.cycle_cover(1);
                        }
                        return;
                    }
                }
                if let Some(rect) = r.equalizer {
                    match header::hit(rect, header::PLAIN, x, y) {
                        Some(header::Item::Close) => {
                            self.panels.eq = false;
                            self.panels.focus = Focus::Player;
                            return self.note("equalizer closed — alt+g to bring it back".into());
                        }
                        Some(header::Item::Settings) => {
                            return self.open_settings(Focus::Equalizer)
                        }
                        Some(header::Item::Filter) => {}
                        // Only the playlist has rows to tag.
                        Some(
                            header::Item::Tagged(_)
                            | header::Item::Copy
                            | header::Item::Put(_)
                            | header::Item::Move
                            | header::Item::Remove
                            | header::Item::Untag,
                        ) => {}
                        None => {}
                    }
                    if hit(rect, x, y) {
                        return self.equalizer_click(rect, x, y);
                    }
                }
                if hit(r.player, x, y) {
                    self.player_click(&r, x, y);
                }
            }

            // A right click on the player toggles playback: the one gesture
            // worth having without aiming at a two-cell button.
            MouseEventKind::Down(MouseButton::Right) if hit(r.player, x, y) => {
                self.handle(Action::PlayPause)
            }
            _ => {}
        }
    }

    fn equalizer_click(&mut self, rect: Rect, x: u16, y: u16) {
        self.panels.focus = Focus::Equalizer;
        let Some(g) = equalizer::geometry(rect, eq::PRESETS[self.eq.preset].name) else {
            return;
        };
        if hit(g.toggle, x, y) {
            return self.handle(Action::ToggleEqEnabled);
        }
        if hit(g.preset_prev, x, y) {
            return self.handle(Action::PrevEqPreset);
        }
        if hit(g.preset_next, x, y) {
            return self.handle(Action::NextEqPreset);
        }
        if hit(g.sliders, x, y) {
            if let Some(b) = g.band_at(x) {
                self.eq.band = b;
                self.edit.drag = Some(Drag::EqBand(b));
                self.set_eq_band(rect, b, y);
            }
        }
    }

    fn set_eq_band(&mut self, rect: Rect, band: usize, y: u16) {
        let Some(g) = equalizer::geometry(rect, eq::PRESETS[self.eq.preset].name) else {
            return;
        };
        self.eq.gains[band] = g.gain_at(y);
        self.eq.enabled = true;
        self.apply_eq();
    }

    fn nudge_eq_band(&mut self, band: usize, delta: f32) {
        let g = (self.eq.gains[band] + delta).clamp(-eq::MAX_GAIN_DB, eq::MAX_GAIN_DB);
        self.eq.gains[band] = g;
        self.eq.band = band;
        self.eq.enabled = true;
        self.apply_eq();
        self.note(format!("{} {g:+.0} dB", eq::BAND_LABELS[band]));
    }

    fn player_geometry(&self, r: &Regions) -> Option<player::Geometry> {
        let repeat = self.player.queue.lock().unwrap().repeat();
        player::geometry(
            r.player,
            self.player.state.position_secs(),
            self.player.state.duration_secs(),
            repeat,
            self.look.glyphs,
        )
    }

    fn player_click(&mut self, r: &Regions, x: u16, y: u16) {
        self.panels.focus = Focus::Player;
        let Some(g) = self.player_geometry(r) else {
            return;
        };
        let c = &g.controls;

        if hit(c.prev, x, y) {
            return self.handle(Action::Prev);
        }
        if hit(c.play, x, y) {
            return match self.player.state.state() {
                PlayState::Playing => {}
                _ => self.handle(Action::PlayPause),
            };
        }
        if hit(c.pause, x, y) {
            return self.handle(Action::PlayPause);
        }
        if hit(c.stop, x, y) {
            return self.handle(Action::Stop);
        }
        if hit(c.next, x, y) {
            return self.handle(Action::Next);
        }
        if hit(c.shuffle, x, y) {
            return self.handle(Action::ToggleShuffle);
        }
        if hit(c.repeat, x, y) {
            return self.handle(Action::CycleRepeat);
        }
        if c.volume.is_some_and(|v| hit(v, x, y)) {
            self.edit.drag = Some(Drag::Volume);
            return self.volume_at_x(r, x);
        }
        if g.seek.is_some_and(|s| hit(s, x, y)) {
            self.edit.drag = Some(Drag::Seek);
            return self.seek_to_x(r, x);
        }
        if hit(g.visualizer, x, y) {
            self.handle(Action::ToggleVisualizer)
        }
    }

    fn playlist_click(&mut self, rect: Rect, x: u16, y: u16, double: bool) {
        self.panels.focus = Focus::Playlist;
        // The same rect the rows were drawn into, so a click lands on the
        // track it is pointing at rather than the one above it.
        let inner = playlist::list_rect(rect);
        if !hit(inner, x, y) {
            return;
        }
        // The same rows the last frame drew, so a divider does not shift a
        // click onto its neighbour.
        let row = self.queue.scroll + (y - inner.y) as usize;
        let i = match self.queue.rows.rows().get(row) {
            // A heading folds its record away and opens it again. Selecting
            // the first track under it was the other candidate, and it is what
            // clicking that track already does.
            Some(playlist::Row::Section { fold, .. }) => {
                let fold = fold.clone();
                return self.toggle_fold(&fold);
            }
            Some(&playlist::Row::Track(i)) => i,
            None => return,
        };
        if i >= self.player.queue.lock().unwrap().len() {
            return;
        }
        self.set_cursor(i);
        // One click selects, two plays -- the file-manager convention, and it
        // keeps a stray click from interrupting what is already playing.
        if double {
            self.handle(Action::Activate);
        }
    }

    /// The settings list under the pointer.
    ///
    /// A single click acts, unlike the playlist picker where one selects and
    /// two load. These rows are switches: selecting one without flipping it
    /// would be a step that achieves nothing.
    fn settings_mouse(&mut self, m: MouseEvent, area: Rect) {
        let (x, y) = (m.column, m.row);
        let Some(st) = &self.over.settings else {
            return;
        };
        let rows = st.rows.len();
        let scroll = st.scroll;
        match m.kind {
            MouseEventKind::ScrollUp => self.move_settings(-1),
            MouseEventKind::ScrollDown => self.move_settings(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = settings::list_rect(area, rows);
                if !hit(inner, x, y) {
                    // Clicking outside a modal dismisses it.
                    self.over.settings = None;
                    return;
                }
                let i = scroll + (y - inner.y) as usize;
                if i >= rows {
                    return;
                }
                if let Some(st) = &mut self.over.settings {
                    st.cursor = i;
                }
                self.take_setting();
            }
            _ => {}
        }
    }

    fn picker_mouse(&mut self, m: MouseEvent, area: Rect) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollUp => self.move_picker(-3),
            MouseEventKind::ScrollDown => self.move_picker(3),
            MouseEventKind::Down(MouseButton::Left) => {
                let inner = picker::list_rect(area, self.over.playlists.len());
                if !hit(inner, x, y) {
                    // Clicking outside a modal dismisses it.
                    self.panels.picker = false;
                    return;
                }
                let i = self.over.picker_scroll + (y - inner.y) as usize;
                if i >= self.over.playlists.len() {
                    return;
                }
                let double = self.register_click(x, y);
                self.over.picker_cursor = i;
                if double {
                    self.load_selected_playlist();
                }
            }
            _ => {}
        }
    }

    /// Record a click and say whether it completes a double click.
    fn register_click(&mut self, x: u16, y: u16) -> bool {
        let now = Instant::now();
        let double = self.edit.last_click.is_some_and(|(px, py, at)| {
            px == x && py == y && now.duration_since(at) < DOUBLE_CLICK
        });
        // Clear on a double so a third click starts a fresh pair rather than
        // firing again on every click of a rapid run.
        self.edit.last_click = (!double).then_some((x, y, now));
        double
    }

    /// Seek to wherever along the bar the pointer is.
    fn seek_to_x(&mut self, r: &Regions, x: u16) {
        let Some(g) = self.player_geometry(r) else {
            return;
        };
        let Some(bar) = g.seek else { return };
        let dur = self.player.state.duration_secs();
        if dur <= 0.0 || bar.width == 0 {
            return;
        }
        let pos = bar_fraction(bar, x) * dur;
        self.command(&Command::SeekTo(pos), &format!("position {pos}"));
        if let Some(mp) = &self.mpris {
            mp.notify(crate::mpris::MprisEvent::Seeked(pos));
        }
    }

    fn volume_at_x(&mut self, r: &Regions, x: u16) {
        let Some(g) = self.player_geometry(r) else {
            return;
        };
        let Some(v) = g.controls.volume else { return };
        if v.width == 0 {
            return;
        }
        // Snapped, so a drag steps rather than sliding: a bar drawn in half
        // cells cannot show more than twenty positions, and a pointer that
        // reports levels between them makes the fill stutter as it moves.
        self.set_volume(snap(slider_fraction(v, x), VOLUME_STEP));
    }

    fn nudge_volume(&mut self, delta: f32) {
        // To the next position on the step's own grid, so a volume left at
        // 37% by the keyboard is moved to 40% or 35% by the wheel rather than
        // to 42% -- the coarser control should not inherit the finer one's
        // offset and stay off-grid for ever.
        let v = step_toward(self.player.volume(), delta);
        self.set_volume(v);
    }

    fn set_volume(&mut self, v: f32) {
        // Not `command`: volume has no command of its own, and the `Resume`
        // that stood in for one meant every nudge of the volume started a
        // paused player again.
        self.remote(&format!("volume {v}"));
        self.player.set_volume(v);
        self.note(format!("volume {}%", (v * 100.0).round()));
    }

    /// The docked layout, as a pure function of the terminal size and which
    /// panels are showing. `None` when the terminal is too small to draw at all.
    fn regions(&self, full: Rect) -> Option<Regions> {
        if full.height < MIN_HEIGHT || full.width < MIN_WIDTH {
            return None;
        }
        // Never let padding eat so much that the layout stops working.
        let pad_x = clamp_padding(self.panels.padding_x, full.width, MIN_WIDTH);
        let pad_y = clamp_padding(self.panels.padding_y, full.height, MIN_HEIGHT);
        let area = Rect {
            x: full.x + pad_x,
            y: full.y + pad_y,
            width: full.width - pad_x * 2,
            height: full.height - pad_y * 2,
        };

        // Docked windows: the player is always present, the other two toggle.
        // The body plus its border.
        let player_h = crate::ui::panels::player::PANEL_ROWS;
        let eq_h = if self.panels.eq {
            crate::ui::panels::equalizer::PANEL_ROWS
        } else {
            0
        };
        let album_h = if self.panels.album {
            crate::ui::panels::album::PANEL_ROWS
        } else {
            0
        };
        let status_h = 1u16;
        let playlist_h = area
            .height
            .saturating_sub(player_h + album_h + eq_h + status_h);
        // Two of its rows are border and one is the header, so three rows is
        // a panel with nothing in it.
        let show_playlist = self.panels.playlist && playlist_h >= 4;

        let mut constraints = vec![Constraint::Length(player_h)];
        if self.panels.album {
            constraints.push(Constraint::Length(album_h));
        }
        if self.panels.eq {
            constraints.push(Constraint::Length(eq_h));
        }
        if show_playlist {
            constraints.push(Constraint::Length(playlist_h));
        }
        constraints.push(Constraint::Length(status_h));

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let mut idx = 0;
        let player = rows[idx];
        idx += 1;
        // The album sits directly under the player: it is about what is
        // playing, and the equalizer is about how.
        let album = self.panels.album.then(|| {
            let r = rows[idx];
            idx += 1;
            r
        });
        let equalizer = self.panels.eq.then(|| {
            let r = rows[idx];
            idx += 1;
            r
        });
        let playlist = show_playlist.then(|| {
            let r = rows[idx];
            idx += 1;
            r
        });

        Some(Regions {
            area,
            player,
            album,
            equalizer,
            playlist,
            status: rows[idx],
        })
    }

    fn draw(&mut self, full: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Cheap when nothing has changed, and it covers both a new track and
        // the panel simply being opened.
        self.pump_album();
        if full.height < MIN_HEIGHT || full.width < MIN_WIDTH {
            let msg = "terminal too small — 40x8 minimum";
            buf.set_string(0, 0, msg, Style::default().fg(Color::Red));
            return;
        }

        // Paint the whole terminal in the theme background first, then draw
        // inside the padding. Without this the padded columns show whatever the
        // terminal background is, which fights the theme.
        let bg = Style::default().bg(Color::Rgb(
            self.look.theme.bg.r,
            self.look.theme.bg.g,
            self.look.theme.bg.b,
        ));
        for y in full.top()..full.bottom() {
            for x in full.left()..full.right() {
                buf[(x, y)].set_char(' ').set_style(bg);
            }
        }

        let Some(r) = self.regions(full) else { return };
        let area = r.area;
        self.panels.last_area = area;

        // The browser is the window while it is open, but the status bar stays:
        // it is where a note about what was just added appears.
        if self.over.library.is_some() {
            let browse = Rect {
                height: area.height.saturating_sub(r.status.height),
                ..area
            };
            self.draw_library(browse, buf);
            self.draw_status(r.status, buf);
            self.draw_overlays(area, buf);
            return;
        }

        // The fluid mode draws one bar per column, so the analysis has to
        // know how wide the panel is. Taking it here means a resize costs one
        // frame of a stale count rather than a second layout pass.
        if self.vis.mode.uses_fluid() {
            let repeat = self.player.queue.lock().unwrap().repeat();
            if let Some(g) = player::geometry(
                r.player,
                self.player.state.position_secs(),
                self.player.state.duration_secs(),
                repeat,
                self.look.glyphs,
            ) {
                self.vis.fluid_bars =
                    crate::ui::panels::visualizer::fluid_bar_count(g.visualizer.width);
            }
        }

        self.draw_player(r.player, buf);

        if let Some(rect) = r.album {
            self.draw_album(rect, buf);
        }

        if let Some(rect) = r.equalizer {
            EqView {
                theme: &self.look.theme,
                gains: self.eq.gains,
                preamp: self.eq.preamp,
                enabled: self.eq.enabled,
                preset: eq::PRESETS[self.eq.preset].name,
                focused_band: self.eq.band,
                focused: self.panels.focus == Focus::Equalizer,
            }
            .render(rect, buf);
        }

        if let Some(rect) = r.playlist {
            self.draw_playlist(rect, buf);
        }

        self.draw_status(r.status, buf);

        self.draw_overlays(area, buf);
    }

    fn draw_library(&mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        use crate::ui::panels::library::{self as lib, Column, LibraryView};
        let Some(l) = &self.over.library else { return };

        // Scroll is derived at draw time from the cursor and this window's own
        // height, the way every other list here does it -- so two windows of
        // different heights scroll independently off one shared selection.
        let geom = lib::layout(area, l.focus);
        let rows: Vec<Vec<lib::Entry>> = (0..3).map(|c| l.rows(c)).collect();
        let scrolls: Vec<usize> = (0..3)
            .map(|c| lib::clamp_scroll(l.cursor(c), l.scroll(c), geom.bodies[c].height as usize))
            .collect();
        let summary = l.summary();
        let heads = ["ARTISTS", "ALBUMS", "TRACKS"];
        let empties = ["nothing matches", "no records here", "no tracks here"];
        let columns = [0, 1, 2].map(|c| Column {
            head: heads[c],
            rows: &rows[c],
            cursor: l.cursor(c),
            scroll: scrolls[c],
            empty: empties[c],
        });
        LibraryView {
            theme: &self.look.theme,
            search: &l.search,
            typing: l.typing,
            columns,
            focus: l.focus,
            summary: &summary,
            keys: "space add \u{b7} enter play \u{b7} / find \u{b7} esc back",
        }
        .render(area, buf);

        if let Some(l) = &mut self.over.library {
            for (c, s) in scrolls.into_iter().enumerate() {
                l.set_scroll(c, s);
            }
        }
    }

    /// Everything that draws over the top, whichever view is underneath.
    fn draw_overlays(&mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if let Some(text) = &self.edit.typing {
            let rows = [settings::Row::setting(
                "matching",
                format!("{text}\u{2582}"),
            )];
            SettingsView {
                theme: &self.look.theme,
                heading: "FILTER",
                title: "words found in the artist, title, album, year or path",
                rows: &rows,
                cursor: 0,
                scroll: 0,
            }
            .render(area, buf);
            return;
        }
        if let Some(name) = &self.edit.naming {
            let rows = [settings::Row::setting("name", format!("{name}\u{2582}"))];
            SettingsView {
                theme: &self.look.theme,
                heading: "SAVE AS",
                title: "a new playlist, beside the others",
                rows: &rows,
                cursor: 0,
                scroll: 0,
            }
            .render(area, buf);
            return;
        }
        if let Some(st) = &mut self.over.settings {
            let h = settings::list_rect(area, st.rows.len()).height as usize;
            st.scroll = settings::clamp_scroll(st.cursor, st.scroll, h);
            SettingsView {
                theme: &self.look.theme,
                heading: st.kind.heading(),
                title: &st.title,
                rows: &st.rows,
                cursor: st.cursor,
                scroll: st.scroll,
            }
            .render(area, buf);
            return;
        }

        if let Some(c) = &mut self.over.chooser {
            let h = crate::ui::panels::chooser::list_rect(area, c.rows.len()).height as usize;
            c.scroll = crate::ui::panels::chooser::clamp_scroll(c.cursor, c.scroll, h);
            let album = self
                .player
                .current_item()
                .and_then(|i| i.album.clone())
                .unwrap_or_else(|| "this album".into());
            ChooserView {
                theme: &self.look.theme,
                album: &album,
                rows: &c.rows,
                cursor: c.cursor,
                scroll: c.scroll,
            }
            .render(area, buf);
            return;
        }

        if self.panels.picker {
            let inner_h = area
                .height
                .saturating_sub(4)
                .min(self.over.playlists.len() as u16 + 4)
                .max(6)
                .saturating_sub(2) as usize;
            self.over.picker_scroll =
                picker::clamp_scroll(self.over.picker_cursor, self.over.picker_scroll, inner_h);
            PickerView {
                theme: &self.look.theme,
                entries: &self.over.playlists,
                cursor: self.over.picker_cursor,
                scroll: self.over.picker_scroll,
                empty_hint: "no playlists — set playlist_dir in config.toml",
            }
            .render(area, buf);
        }

        if let Some(s) = &self.over.resume {
            ResumeView {
                theme: &self.look.theme,
                session: s,
                now: crate::library::db::now_secs(),
            }
            .render(area, buf);
        }

        if self.panels.help {
            self.draw_help(area, buf);
        }
    }

    fn draw_player(&mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Taken first: it needs `&mut self`, and everything below borrows the
        // player immutably for the rest of the function.
        let dropouts = self.dropouts_now();
        let st = &self.player.state;
        let item = self.player.current_item();
        let (title, subtitle) = match &item {
            Some(i) => (
                track_line(i.artist.as_deref(), i.title.as_deref(), i.album.as_deref())
                    .unwrap_or_else(|| i.uri.to_string()),
                String::new(),
            ),
            None => ("nothing playing".into(), String::new()),
        };

        use std::sync::atomic::Ordering::Relaxed;
        let rate = st.sample_rate.load(Relaxed);
        let depth = st.bit_depth.load(Relaxed);
        let ch = st.channels.load(Relaxed);
        let tech = if rate > 0 {
            let mut parts: Vec<String> = Vec::with_capacity(5);
            let codec = st.codec.load_full();
            if !codec.is_empty() {
                parts.push(codec_label(&codec).to_string());
            }
            let bitrate = st.bitrate_kbps.load(Relaxed);
            if bitrate > 0 {
                parts.push(format!("{bitrate} kbps"));
            }
            parts.push(format!("{:.1} kHz", rate as f64 / 1000.0));
            parts.push(if depth > 0 {
                format!("{depth}-bit")
            } else {
                "float".into()
            });
            parts.push(if ch == 1 { "mono" } else { "stereo" }.into());
            parts.join(" · ")
        } else {
            String::new()
        };

        let (shuffle, repeat) = {
            let q = self.player.queue.lock().unwrap();
            (q.shuffled(), q.repeat())
        };

        let title = match self.fx.running.as_ref() {
            Some(_) => self.fx.title.clone(),
            None => title,
        };

        let empty: [f32; 0] = [];
        let showing = self.vis.mode != VisMode::Off;
        PlayerView {
            theme: &self.look.theme,
            title,
            subtitle,
            tech,
            state: st.state(),
            position: st.position_secs(),
            duration: st.duration_secs(),
            volume: self.player.volume(),
            shuffle,
            repeat,
            bit_perfect: st.bit_perfect.load(Relaxed),
            focused: self.panels.focus == Focus::Player,
            mirroring: self.session.link.is_some(),
            marquee_offset: self.look.marquee,
            vis_mode: self.vis.mode,
            bars: self.vis.bars,
            glyphs: self.look.glyphs,
            seek_phase: self.look.seek_phase,
            seek_style: self.look.seek_style,
            // Ballistic positions rather than the raw analyzer output: the caps
            // and the bar bodies have their own physics.
            bands: if showing {
                self.vis.meters.bars()
            } else {
                &empty
            },
            peaks: if showing {
                self.vis.meters.peaks()
            } else {
                &empty
            },
            wave: if showing && self.vis.mode.needs_waveform() {
                &self.vis.wave
            } else {
                &empty
            },
            underruns: dropouts,
        }
        .render(area, buf);

        // The buttons go on last, as pictures over the plates the panel drew
        // as text. Where the protocol has nothing to say -- no picker, no cell
        // size -- those text plates are what stays, which is the fallback by
        // construction rather than by a second code path.
        let state = st.state();
        let (position, duration) = (st.position_secs(), st.duration_secs());
        let t = &self.look.theme;
        let (bg, plate, plate_lit, ink, ink_lit) = (
            t.bg,
            t.transport_button_bg,
            t.transport_button_active_bg,
            t.transport_button_fg,
            t.transport_button_active_fg,
        );
        if let Some(g) = player::geometry(area, position, duration, repeat, self.look.glyphs) {
            use crate::ui::panels::faces::Button;
            let c = &g.controls;
            let buttons = [
                (Button::Prev, c.prev, false),
                (Button::Play, c.play, state == PlayState::Playing),
                (Button::Pause, c.pause, state == PlayState::Paused),
                (Button::Stop, c.stop, state == PlayState::Stopped),
                (Button::Next, c.next, false),
            ];
            for (which, rect, lit) in buttons {
                let (fg, on) = if lit {
                    (ink_lit, plate_lit)
                } else {
                    (ink, plate)
                };
                let which = crate::ui::graphics::Picture::Button(which);
                if let Some(p) = self.graphics.picture(which, rect, fg, on, bg) {
                    // The placeholder row is written from the first cell and
                    // carries that cell's style, so the text plate's colours
                    // would show wherever the picture did not reach. Panel
                    // colour under every cell first, and nothing else can.
                    let panel = Style::default().bg(Color::Rgb(bg.r, bg.g, bg.b));
                    for y in rect.y..rect.y + rect.height {
                        for x in rect.x..rect.x + rect.width {
                            buf[(x, y)].set_symbol(" ").set_style(panel);
                        }
                    }
                    ratatui_image::Image::new(p).render(rect, buf);
                }
            }
        }
    }

    fn draw_playlist(&mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let q = self.player.queue.lock().unwrap();
        // Shown in play order, not storage order. With shuffle on those differ,
        // Nothing is highlighted until a track is actually loaded.
        let playing = q.current_index().map(|_| q.view_cursor());
        drop(q);
        let grouped = self.session_grouped();
        let q = self.player.queue.lock().unwrap();
        let from = (
            q.revision(),
            grouped,
            self.queue.fold_gen,
            playing,
            self.edit.gen,
        );
        drop(q);

        // Rebuilt only when the queue, the grouping or a fold actually moved:
        // the click handler reads it between frames and must see what was
        // drawn. `playing` is in there because a folded record says whether it
        // holds the track that is playing.
        if self.queue.rows_from != from {
            // Shown in play order, not storage order. With shuffle on those
            // differ, and rendering storage order made shuffle look like it did
            // nothing.
            //
            // Cloned under the same key as the rows rather than every frame:
            // a library queue holds five thousand of these, and copying all of
            // them thirty times a second to draw fifteen was most of what the
            // panel cost.
            let q = self.player.queue.lock().unwrap();
            self.queue.items = q
                .view()
                .iter()
                .filter_map(|&i| q.tracks().get(i).cloned())
                .collect();
            drop(q);
            self.queue.rows = match grouped {
                true => playlist::Rows::grouped(&self.queue.items, &self.queue.folded, playing),
                false => playlist::Rows::flat(self.queue.items.len()),
            };
            if !self.edit.words.trim().is_empty() {
                let mask = crate::playlist::filter::mask(&self.queue.items, &self.edit.words);
                self.queue.rows = std::mem::take(&mut self.queue.rows).matching(&mask);
            }
            self.queue.rows_from = from;
            // A cursor inside a record that has just been folded away has to
            // come back out; there is nothing on screen for it to sit on.
            if let Some(t) = self.queue.rows.nearest_shown(self.queue.cursor) {
                self.queue.cursor = t;
            }
        }

        let visible = playlist::list_rect(area).height as usize;
        let row = self.queue.rows.row_of_track(self.queue.cursor).unwrap_or(0);
        self.queue.scroll = PlaylistView::clamp_scroll(row, self.queue.scroll, visible);
        // Pull a record's heading into view along with its first track -- but
        // never at the cost of pushing the cursor off the bottom, which is
        // what the `min` refuses to do.
        if visible > 1 {
            let anchor = self.queue.rows.anchor_row(self.queue.cursor);
            self.queue.scroll = PlaylistView::clamp_scroll(anchor, self.queue.scroll, visible)
                .min(self.queue.scroll);
        }

        // An asterisk, because adding never writes to disk and a playlist that
        // has been changed and not saved must say so -- otherwise the only way
        // to find out is to look for the tracks tomorrow and not find them.
        let mut name = if self.queue.dirty {
            format!("{} *", self.queue.name)
        } else {
            self.queue.name.clone()
        };
        // The filter in force is part of what the panel is showing, so it
        // goes in the title beside the name rather than only in a note that
        // scrolls away.
        if !self.edit.words.trim().is_empty() {
            name = format!("{name} \u{b7} /{}", self.edit.words.trim());
        }
        // The tag set names tracks; the panel draws view positions. Translated
        // here rather than stored twice, so a reorder cannot leave the two
        // disagreeing about which row is marked.
        let marked: std::collections::HashSet<usize> = {
            let q = self.player.queue.lock().unwrap();
            q.view()
                .iter()
                .enumerate()
                .filter(|(_, t)| self.queue.tagged.contains(t))
                .map(|(slot, _)| slot)
                .collect()
        };
        let words = self.playlist_header();
        PlaylistView {
            theme: &self.look.theme,
            name: &name,
            tagged: &marked,
            items: &self.queue.items,
            rows: &self.queue.rows,
            cursor: self.queue.cursor,
            playing,
            scroll: self.queue.scroll,
            focused: self.panels.focus == Focus::Playlist,
            glyphs: self.look.glyphs,
            header_items: &words,
        }
        .render(area, buf);

        // The playing row's marker as a picture: the play triangle the
        // transport shows, one cell tall, over the chevron the panel drew --
        // which stays where there is no protocol. Its colours are read back
        // from the cell, so a cursor bar over the playing row carries through.
        let solid = |c: Color| match c {
            Color::Rgb(r, g, b) => Some(crate::theme::color::Rgb { r, g, b }),
            _ => None,
        };
        for (x, y) in playlist::marker_cells(area, &self.queue.rows, self.queue.scroll, playing) {
            let cell = &buf[(x, y)];
            let fg = solid(cell.fg).unwrap_or(self.look.theme.row_playing_fg);
            let bg = solid(cell.bg).unwrap_or(self.look.theme.panel_bg);
            let rect = Rect::new(x, y, 1, 1);
            let mark = crate::ui::graphics::Picture::PlayMark;
            if let Some(p) = self.graphics.picture(mark, rect, fg, bg, bg) {
                ratatui_image::Image::new(p).render(rect, buf);
                crate::ui::graphics::mend_unit_placeholder(buf, x, y);
            }
        }
    }

    /// Adopt a probe made before the alternate screen was entered.
    pub fn set_graphics(&mut self, g: crate::ui::graphics::Graphics) {
        self.graphics = g;
    }

    fn draw_album(&mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let item = self.player.current_item();
        let album = self
            .current_uri()
            .zip(self.art.as_ref())
            .and_then(|(uri, w)| w.album_for(&uri));

        // Encoded once per cover and per size, not per frame.
        let show_cover = self.show_cover();
        // Read before the protocol borrows `self.graphics` mutably.
        let cell_aspect = self.cell_aspect();
        let retry = self.retry_look();
        let protocol = match (
            &album,
            crate::ui::panels::album::geometry(area, show_cover, self.cell_aspect()),
        ) {
            (Some(a), Some(g)) => a
                .image
                .as_ref()
                .and_then(|img| self.graphics.protocol(img, g.art)),
            _ => None,
        };

        crate::ui::panels::album::AlbumView {
            theme: &self.look.theme,
            album: album.as_deref(),
            protocol,
            show_cover,
            cell_aspect,
            retry,
            // The queue's own tags, so the panel says something for a track
            // the scan has never seen.
            fallback_album: item.as_ref().and_then(|i| i.album.as_deref()),
            fallback_artist: item.as_ref().and_then(|i| i.artist.as_deref()),
            focused: self.panels.focus == Focus::Album,
        }
        .render(area, buf);
    }

    /// What is playing, addressed the way the index addresses it.
    ///
    /// `None` when nothing is playing, or when mirroring a leader too old to
    /// send its URI.
    fn current_uri(&self) -> Option<String> {
        if self.session.link.is_some() {
            return Some(self.session.uri.clone()).filter(|u| !u.is_empty());
        }
        self.player
            .current_item()
            .map(|i| i.uri.to_string())
            .filter(|u| !u.is_empty())
    }

    /// How tall a terminal cell is relative to its width, as measured if the
    /// terminal would say and assumed otherwise.
    fn cell_aspect(&self) -> f32 {
        self.graphics
            .cell_aspect()
            .unwrap_or(crate::ui::panels::album::CELL_ASPECT)
    }

    /// Whether the album panel is drawing a picture at all.
    fn show_cover(&self) -> bool {
        self.graphics.mode() != crate::ui::graphics::Mode::Off
    }

    /// Where the album panel's clickable retry sits, if it is offering one.
    ///
    /// Built from the same state the panel draws from, so the word and its hit
    /// box are always the same thing.
    fn album_retry_rect(&self, rect: Rect) -> Option<Rect> {
        let item = self.player.current_item();
        let album = self
            .current_uri()
            .zip(self.art.as_ref())
            .and_then(|(uri, w)| w.album_for(&uri));
        crate::ui::panels::album::retry_rect(
            rect,
            self.show_cover(),
            self.cell_aspect(),
            album.as_deref(),
            item.as_ref().and_then(|i| i.album.as_deref()),
            item.as_ref().and_then(|i| i.artist.as_deref()),
        )
    }

    /// Look the current album's cover up again from scratch.
    ///
    /// A recorded miss stands for a week, which is right for a coverless
    /// album and wrong the moment somebody fixes a tag or the archive comes
    /// back up. This is how to say "try now".
    fn retry_cover(&mut self) {
        let Some(uri) = self.current_uri() else {
            return self.note("nothing playing to look up".into());
        };
        let Some(w) = &self.art else {
            return self.note("no library index to look anything up in".into());
        };
        self.panels.album = true;
        self.graphics.forget();
        // Taken before asking, so an answer that arrives immediately -- a
        // cached cover, say -- still counts as having arrived.
        self.retrying = Some(w.serial());
        self.retry_phase = 0.0;
        w.retry(&uri);
        self.note("looking this album up again\u{2026}".into())
    }

    /// What a panel offers to change, and what each row does.
    ///
    /// Rows and actions are built together so a row can never trigger the one
    /// beside it. Every entry either already had a key of its own or is a
    /// setting that had no way to change it at all.
    fn settings_for(&self, panel: Focus, kind: Overlay) -> (String, Vec<(settings::Row, Setting)>) {
        use settings::Row;
        if kind == Overlay::Filter {
            return self.filter_for();
        }
        if kind == Overlay::Joining {
            return self.joining_for();
        }
        if kind == Overlay::Adding {
            return self.adding_for();
        }
        if kind == Overlay::Saving {
            return self.saving_for();
        }
        match panel {
            Focus::Album => (
                "album".into(),
                vec![
                    (
                        Row::setting("cover graphics", self.graphics.mode().name()),
                        Setting::Graphics,
                    ),
                    (
                        Row::setting("fetch cover art", on_off(self.art_fetch())),
                        Setting::FetchArt,
                    ),
                    (Row::action("choose cover\u{2026}"), Setting::ChooseCover),
                    (Row::action("look up again"), Setting::RetryCover),
                ],
            ),
            Focus::Playlist => {
                let q = self.player.queue.lock().unwrap();
                let (shuffle, repeat) = (q.shuffled(), q.repeat().to_string());
                drop(q);
                (
                    self.queue.name.clone(),
                    vec![
                        (Row::setting("shuffle", on_off(shuffle)), Setting::Shuffle),
                        (
                            Row::setting("repeat", repeat.to_lowercase()),
                            Setting::Repeat,
                        ),
                        (
                            Row::action("load a playlist\u{2026}"),
                            Setting::LoadPlaylist,
                        ),
                        (
                            Row::action("save the playlist\u{2026}"),
                            Setting::SavePlaylist,
                        ),
                    ],
                )
            }
            Focus::Equalizer => (
                "equalizer".into(),
                vec![
                    (
                        Row::setting("enabled", on_off(self.eq.enabled)),
                        Setting::EqEnabled,
                    ),
                    (
                        Row::setting("preset", eq::PRESETS[self.eq.preset].name),
                        Setting::EqPreset,
                    ),
                    (Row::action("reset the bands"), Setting::EqReset),
                ],
            ),
            // The transport has no header and so cannot get here.
            Focus::Player => (String::new(), Vec::new()),
        }
    }

    /// What to do about a playlist named on the command line when a session
    /// was already running.
    ///
    /// Asked rather than guessed, and asked here rather than through a flag on
    /// the command that has already been typed: by the time anyone knows there
    /// was a session to join, the moment to have said so has passed.
    fn joining_for(&self) -> (String, Vec<(settings::Row, Setting)>) {
        use settings::Row;
        let name = self
            .session
            .joining
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "that playlist".into());
        (
            format!("{name} was asked for, and a session is already playing"),
            vec![
                (
                    Row::action("join the session, leave it playing"),
                    Setting::JoinSession,
                ),
                (
                    Row::action(format!("load {name} into the session")),
                    Setting::ReplaceQueue,
                ),
            ],
        )
    }

    /// Where to write the playlist that has been edited.
    ///
    /// Overwriting is never assumed. The queue may have been built from a file
    /// curated for years, and turning an afternoon's browsing into a silent
    /// overwrite of it is not a trade anyone offered.
    fn saving_for(&self) -> (String, Vec<(settings::Row, Setting)>) {
        use settings::Row;
        let n = self.player.queue.lock().unwrap().tracks().len();
        let mut rows = vec![];
        if let Some(p) = &self.queue.source {
            let was = crate::playlist::m3u::read_file(p).map(|pl| pl.len()).ok();
            rows.push((
                Row::action(match was {
                    Some(was) => format!("overwrite {} ({was} \u{2192} {n})", self.queue.name),
                    None => format!("overwrite {}", self.queue.name),
                }),
                Setting::SaveOverwrite,
            ));
        }
        rows.push((
            Row::action("save as a new playlist\u{2026}"),
            Setting::SaveAsNew,
        ));
        rows.push((Row::action("cancel"), Setting::SaveCancel));
        (format!("{n} tracks, unsaved"), rows)
    }

    /// Open the save box, if there is anything to save.
    fn save_playlist(&mut self) {
        if self.player.queue.lock().unwrap().tracks().is_empty() {
            return self.note("nothing to save".into());
        }
        self.open_overlay(Focus::Playlist, Overlay::Saving);
    }

    /// Write the queue out as a playlist.
    fn write_playlist(&mut self, path: PathBuf) {
        let uris = {
            let q = self.player.queue.lock().unwrap();
            // The order on screen, not the storage order. They are different
            // the moment anything is grouped, shuffled, or arranged by hand --
            // and saving a list that does not match the one being looked at is
            // the arrangement being silently thrown away.
            q.view()
                .iter()
                .filter_map(|&i| q.tracks().get(i))
                .map(|t| t.uri.clone())
                .collect::<Vec<_>>()
        };
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.queue.name.clone());
        let pl = crate::playlist::m3u::from_uris(&name, uris.into_iter());
        let n = pl.len();
        match crate::playlist::m3u::write_file(
            &pl,
            &path,
            crate::playlist::m3u::WriteStyle::MpdCompatible,
        ) {
            Ok(()) => {
                self.queue.dirty = false;
                self.queue.name = name.clone();
                self.queue.source = Some(path);
                self.rescan_playlists();
                self.note(format!("saved {n} tracks to {name}"));
            }
            Err(e) => self.note(format!("could not save: {e}")),
        }
    }

    /// Re-read the playlist directory, without opening the picker.
    ///
    /// `set_playlists` shows the picker as well, which is right at startup and
    /// wrong after a save -- it would throw an overlay over whatever the user
    /// was doing.
    fn rescan_playlists(&mut self) {
        // Beside the playlist that is loaded, or the configured directory.
        let dir = self
            .queue
            .source
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .or_else(|| crate::paths::playlist_dir().ok());
        if let Some(dir) = dir {
            self.over.playlists = crate::scan_playlist_dir(&dir);
        }
    }

    /// What `space` should do with what it just picked.
    ///
    /// Asked once per playlist rather than once per press: the answer is about
    /// the queue, not about this record, and being asked again for every album
    /// would make filling a playlist unbearable.
    fn adding_for(&self) -> (String, Vec<(settings::Row, Setting)>) {
        use settings::Row;
        let (what, items) = match &self.edit.adding {
            Some((w, i)) => (w.as_str(), i.len()),
            None => ("", 0),
        };
        let here = self.player.queue.lock().unwrap().tracks().len();
        (
            format!("{what} \u{2192} {}, which has {here}", self.queue.name),
            vec![
                (
                    Row::action(format!("add {items} to the end")),
                    Setting::AddAppend,
                ),
                (Row::action("replace what is loaded"), Setting::AddReplace),
                (Row::action("cancel"), Setting::AddCancel),
            ],
        )
    }

    /// `space`: put what is selected into the loaded playlist.
    fn library_add(&mut self, whole_album: bool) {
        let Some(lib) = &self.over.library else {
            return;
        };
        let (what, items) = lib.selection(whole_album);
        if items.is_empty() {
            return self.note("nothing selected".into());
        }
        // The remembered answer is about *this* playlist. If a different one
        // has been loaded since, ask again.
        let remembered = self
            .edit
            .add_mode
            .as_ref()
            .filter(|(name, _)| name == &self.queue.name)
            .map(|(_, m)| *m);
        match remembered {
            Some(mode) => self.apply_add(mode, items, &what),
            None => {
                self.edit.adding = Some((what, items));
                self.open_overlay(Focus::Playlist, Overlay::Adding);
            }
        }
    }

    /// Append or replace, skipping what is already there.
    ///
    /// Nothing is written to disk. The queue changes and is marked unsaved;
    /// the playlist file is untouched until it is saved on purpose.
    fn apply_add(&mut self, mode: Setting, items: Vec<QueueItem>, what: &str) {
        let replace = mode == Setting::AddReplace;
        let mut added = 0usize;
        let mut already = 0usize;

        // Not `owns`, which forwards a request as a side effect -- there is a
        // real one to send below, and it needs its answer.
        if self.session.link.is_none() {
            let mut q = self.player.queue.lock().unwrap();
            if replace {
                added = items.len();
                q.set_tracks(items);
            } else {
                // Already here is already here, whichever spelling it uses.
                let have: std::collections::HashSet<String> =
                    q.tracks().iter().map(|t| t.uri.to_string()).collect();
                for item in items {
                    if have.contains(&item.uri.to_string()) {
                        already += 1;
                        continue;
                    }
                    q.push(item);
                    added += 1;
                }
            }
        } else {
            // A window following the session sends the URIs rather than
            // touching a queue it does not own.
            let uris: Vec<String> = items.iter().map(|t| t.uri.to_string()).collect();
            let verb = if replace { "set-queue" } else { "enqueue" };
            let Ok(json) = serde_json::to_string(&uris) else {
                return self.note("could not send that".into());
            };
            // The count comes from the session rather than from hope. An
            // instance older than this one does not know the verb, and saying
            // "added 20" when nothing was added is the worst of both.
            let reply = self
                .session
                .link
                .as_ref()
                .and_then(|m| m.ask(&format!("{verb} {json}")));
            match reply.as_deref().map(str::trim) {
                Some(r) => match r.parse::<usize>() {
                    Ok(n) => added = n,
                    Err(_) => {
                        return self.note(format!(
                            "the session would not take them: {}",
                            r.trim_start_matches("error: ")
                        ))
                    }
                },
                None => return self.note("the session did not answer".into()),
            }
        }

        self.queue.dirty = true;
        let name = self.queue.name.clone();
        self.note(match (replace, already) {
            (true, _) => format!("{what} \u{2192} {name}, replacing what was there"),
            (false, 0) => format!("added {added} to {name}"),
            (false, n) => format!("added {added} to {name} \u{b7} {n} already there"),
        });
    }

    /// How the playlist is ordered, and which way round.
    ///
    /// One switch, not two: the dividers are a consequence of the album order
    /// rather than a setting of their own, because breaking a list that is not
    /// sorted by album puts a heading every second row.
    fn filter_for(&self) -> (String, Vec<(settings::Row, Setting)>) {
        use settings::Row;
        let q = self.player.queue.lock().unwrap();
        let group = q.grouping();
        let shuffled = q.shuffled();
        let arranged = !q.manual_order().is_empty();
        // Nothing to group by is worth saying out loud. A queue built by
        // pointing at a directory carries no tags at all, and album order over
        // it would silently do nothing.
        let tagged = q.tracks().iter().any(|t| t.album.is_some());
        drop(q);

        let order = if !tagged {
            "no album data".to_string()
        } else if shuffled {
            match group {
                Some(_) => "album by year (shuffled)".into(),
                None => "shuffled".into(),
            }
        } else {
            match group {
                Some(_) => "album by year".into(),
                None => "playlist order".into(),
            }
        };
        let mut rows = vec![
            (Row::setting("order", order), Setting::GroupOrder),
            (
                Row::setting(
                    "direction",
                    match (group, arranged) {
                        // A hand-made arrangement is not going in any
                        // direction, so saying "oldest first" under it would
                        // be a lie about what the list is doing.
                        (Some(_), true) => "by hand",
                        (Some(true), false) => "newest first",
                        (Some(false), false) => "oldest first",
                        (None, _) => "\u{2014}",
                    },
                ),
                Setting::GroupDirection,
            ),
        ];
        if arranged {
            rows.push((
                Row::action("forget my arrangement"),
                Setting::ClearAlbumOrder,
            ));
        }
        (self.queue.name.clone(), rows)
    }

    /// Turn album order on, or back off again.
    ///
    /// Reordering the queue is felt rather than merely seen: the track keeps
    /// playing, but what follows it changes. The note says so.
    fn cycle_grouping(&mut self) {
        let q = self.player.queue.lock().unwrap();
        if !q.tracks().iter().any(|t| t.album.is_some()) {
            drop(q);
            return self.note("nothing in this queue is indexed \u{2014} run a scan".into());
        }
        let shuffled = q.shuffled();
        drop(q);
        // Asked of the session, which may be another window's.
        let on = !self.session_grouped();

        let descending = self.queue.group_desc;
        let want = match (on, descending) {
            (false, _) => "set-group off".to_string(),
            (true, true) => "set-group album desc".to_string(),
            (true, false) => "set-group album".to_string(),
        };
        if self.owns(&want) {
            self.player
                .queue
                .lock()
                .unwrap()
                .set_grouping(on.then_some(descending));
        }
        // The cursor is a track and the reorder moved it, so follow it rather
        // than leaving the highlight on whatever landed in its place.
        self.follow_order();
        self.note(match (on, shuffled) {
            (true, true) => "album order \u{2014} shuffle is on top of it".into(),
            (true, false) => "album order, oldest first \u{2014} this is what plays next".into(),
            (false, _) => "back to the playlist's own order".into(),
        })
    }

    /// Turn the albums round. Off means there is nothing to turn.
    fn flip_grouping(&mut self) {
        if !self.session_grouped() {
            return self.note("nothing to reverse until the albums are in order".into());
        }
        self.queue.group_desc = !self.queue.group_desc;
        let desc = self.queue.group_desc;
        // A direction over a hand-made arrangement means nothing, so asking
        // for one puts the records back in the order the years give them.
        if self.owns("set-album-order") {
            self.player
                .queue
                .lock()
                .unwrap()
                .set_manual_order(Vec::new());
        }
        let want = format!("set-group album{}", if desc { " desc" } else { "" });
        if self.owns(&want) {
            self.player.queue.lock().unwrap().set_grouping(Some(desc));
        }
        self.follow_order();
        self.note(if self.queue.group_desc {
            "newest records first".into()
        } else {
            "oldest records first".into()
        })
    }

    fn art_fetch(&self) -> bool {
        self.art_fetch.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Open a panel's settings, or close them if they are already open.
    fn open_settings(&mut self, panel: Focus) {
        self.open_overlay(panel, Overlay::Settings)
    }

    /// Open the playlist's ordering, or close it if it is already open.
    fn open_filter(&mut self) {
        self.panels.playlist = true;
        self.panels.focus = Focus::Playlist;
        self.open_overlay(Focus::Playlist, Overlay::Filter)
    }

    /// The one overlay, showing whichever list was asked for.
    ///
    /// Keyed on the pair: asking for the list that is already up closes it,
    /// and asking for the other one switches to it rather than closing.
    fn open_overlay(&mut self, panel: Focus, kind: Overlay) {
        if self
            .over
            .settings
            .as_ref()
            .is_some_and(|s| s.panel == panel && s.kind == kind)
        {
            self.over.settings = None;
            return;
        }
        let (title, built) = self.settings_for(panel, kind);
        if built.is_empty() {
            return self.note("nothing to change here".into());
        }
        let (rows, items) = built.into_iter().unzip();
        self.over.settings = Some(SettingsState {
            panel,
            kind,
            title,
            rows,
            items,
            cursor: 0,
            scroll: 0,
        });
    }

    fn move_settings(&mut self, delta: i32) {
        let Some(s) = &mut self.over.settings else {
            return;
        };
        let last = s.rows.len().saturating_sub(1) as i32;
        s.cursor = (s.cursor as i32 + delta).clamp(0, last) as usize;
    }

    /// Rebuild the rows in place, so the values shown are the values that are.
    fn refresh_settings(&mut self) {
        let Some(s) = &self.over.settings else { return };
        let (title, built) = self.settings_for(s.panel, s.kind);
        let (rows, items) = built.into_iter().unzip();
        if let Some(s) = &mut self.over.settings {
            s.title = title;
            s.rows = rows;
            s.items = items;
        }
    }

    /// Act on the row the cursor is over.
    fn take_setting(&mut self) {
        let Some(s) = &self.over.settings else { return };
        let Some(item) = s.items.get(s.cursor).copied() else {
            return;
        };
        match item {
            Setting::Graphics => self.cycle_graphics(),
            Setting::FetchArt => self.toggle_art_fetch(),
            // The answer is kept for this playlist, so the next `space` goes
            // straight in. Cancelling remembers nothing.
            Setting::AddAppend | Setting::AddReplace => {
                self.over.settings = None;
                let Some((what, items)) = self.edit.adding.take() else {
                    return;
                };
                self.edit.add_mode = Some((self.queue.name.clone(), item));
                return self.apply_add(item, items, &what);
            }
            Setting::AddCancel => {
                self.edit.adding = None;
                self.over.settings = None;
                return;
            }
            Setting::SavePlaylist => {
                self.over.settings = None;
                return self.save_playlist();
            }
            Setting::SaveOverwrite => {
                self.over.settings = None;
                let Some(p) = self.queue.source.clone() else {
                    return;
                };
                return self.write_playlist(p);
            }
            Setting::SaveAsNew => {
                self.over.settings = None;
                self.edit.naming = Some(String::new());
                return;
            }
            Setting::SaveCancel => {
                self.over.settings = None;
                return;
            }
            // The rows that lead somewhere close the settings behind them,
            // rather than stacking one overlay on another.
            Setting::ChooseCover => {
                self.over.settings = None;
                return self.open_chooser();
            }
            Setting::RetryCover => {
                self.over.settings = None;
                return self.retry_cover();
            }
            Setting::LoadPlaylist => {
                self.over.settings = None;
                return self.handle(Action::OpenPlaylistPicker);
            }
            Setting::Shuffle => self.handle(Action::ToggleShuffle),
            Setting::Repeat => self.handle(Action::CycleRepeat),
            Setting::JoinSession => {
                let name = self.queue.name.clone();
                self.session.joining = None;
                self.over.settings = None;
                return self.note(format!("joined \u{2014} {name} is still playing"));
            }
            Setting::ReplaceQueue => {
                let Some(path) = self.session.joining.take() else {
                    self.over.settings = None;
                    return;
                };
                self.over.settings = None;
                return self.load_playlist_into_session(&path);
            }
            Setting::GroupOrder => self.cycle_grouping(),
            Setting::GroupDirection => self.flip_grouping(),
            Setting::ClearAlbumOrder => {
                if self.owns("set-album-order") {
                    self.player
                        .queue
                        .lock()
                        .unwrap()
                        .set_manual_order(Vec::new());
                }
                self.follow_order();
                self.note("back to year order".into());
            }
            Setting::EqEnabled => self.handle(Action::ToggleEqEnabled),
            Setting::EqPreset => self.handle(Action::NextEqPreset),
            Setting::EqReset => {
                self.eq.gains = [0.0; 10];
                self.eq.preamp = 0.0;
                self.eq.preset = 0;
                self.apply_eq();
                self.note("eq: bands reset".into());
            }
        }
        self.refresh_settings();
    }

    /// Step through how covers are drawn, and remember the choice.
    fn cycle_graphics(&mut self) {
        use crate::ui::graphics::Mode;
        let next = match self.graphics.mode() {
            Mode::Auto => Mode::Kitty,
            Mode::Kitty => Mode::Blocks,
            Mode::Blocks => Mode::Off,
            Mode::Off => Mode::Auto,
        };
        let took = self.graphics.set_mode(next);
        if took {
            self.note(format!("covers: {}", self.graphics.name()));
        } else {
            // Detection can only happen before the alternate screen, so `auto`
            // cannot go and look now. Saying so beats appearing to do nothing.
            self.note("covers: auto \u{2014} detected on the next start".into());
        }
    }

    fn toggle_art_fetch(&mut self) {
        let on = !self.art_fetch();
        self.art_fetch
            .store(on, std::sync::atomic::Ordering::Relaxed);
        self.note(if on {
            "cover art archive: on \u{2014} artist and album names are sent to musicbrainz".into()
        } else {
            "cover art archive: off".to_string()
        });
    }

    /// Open the cover chooser for what is playing.
    fn open_chooser(&mut self) {
        if self.over.chooser.is_some() {
            self.over.chooser = None;
            return;
        }
        let Some(uri) = self.current_uri() else {
            return self.note("nothing playing to choose a cover for".into());
        };
        let Some(album) = self.art.as_ref().and_then(|w| w.album_for(&uri)) else {
            return self.note("still looking this album up".into());
        };

        // Images already here first, then releases the archive offered. One
        // list, because from the user's side it is one question.
        let mut rows: Vec<chooser::Row> = album
            .labels
            .iter()
            .map(|label| chooser::Row {
                label: label.clone(),
                note: "on disk".into(),
                remote: false,
            })
            .collect();
        let local = rows.len();
        rows.extend(album.offers.iter().map(|r| chooser::Row {
            label: r.describe(),
            // The similarity, because it is the reason this was not taken
            // automatically and the user is being asked to overrule it.
            note: format!("{}%", (r.similarity * 100.0).round() as u32),
            remote: true,
        }));

        if rows.is_empty() {
            return self.note("no covers and no releases to choose from".into());
        }
        self.panels.album = true;
        self.over.chooser = Some(Chooser {
            cursor: album.choice.min(rows.len() - 1),
            uri,
            rows,
            local,
            scroll: 0,
        });
    }

    fn move_chooser(&mut self, delta: i32) {
        let Some(c) = &mut self.over.chooser else {
            return;
        };
        let last = c.rows.len().saturating_sub(1) as i32;
        c.cursor = (c.cursor as i32 + delta).clamp(0, last) as usize;
    }

    /// Use whatever the cursor is on.
    fn take_chosen_cover(&mut self) {
        let Some(c) = self.over.chooser.take() else {
            return;
        };
        let Some(w) = &self.art else { return };
        self.graphics.forget();

        if c.cursor < c.local {
            // An image already here: step the album's own choice to it.
            let Some(current) = w.album_for(&c.uri).map(|a| a.choice) else {
                return;
            };
            w.cycle(&c.uri, c.cursor as i32 - current as i32);
            self.note(format!("cover: {}", c.rows[c.cursor].label))
        } else {
            w.choose_release(&c.uri, c.cursor - c.local);
            self.note(format!("fetching {}", c.rows[c.cursor].label))
        }
    }

    /// Show a different one of this album's covers.
    fn cycle_cover(&mut self, delta: i32) {
        let Some(uri) = self.current_uri() else {
            return;
        };
        let Some(w) = &self.art else { return };
        match w.album_for(&uri).map(|a| a.choices) {
            Some(n) if n > 1 => {
                w.cycle(&uri, delta);
                // The protocol is built from the old picture; letting it go is
                // what makes the new one appear.
                self.graphics.forget();
            }
            _ => self.note("no other cover for this album".into()),
        }
    }

    /// Ask the art worker about the current track, once per track.
    ///
    /// Only while the panel is open: there is no point resolving covers nobody
    /// is looking at, and it keeps a closed panel entirely free.
    fn pump_album(&mut self) {
        if !self.panels.album {
            return;
        }
        let Some(w) = &self.art else { return };
        let uri = self.current_uri();
        if uri == self.art_uri {
            return;
        }
        if let Some(u) = &uri {
            w.look_up(u);
        }
        self.art_uri = uri;
    }

    fn draw_status(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let t = &self.look.theme;
        let bg = Color::Rgb(t.status_bg.r, t.status_bg.g, t.status_bg.b);
        for x in 0..area.width {
            buf[(area.x + x, area.y)]
                .set_char(' ')
                .set_style(Style::default().bg(bg));
        }

        // The right-hand indicators are always shown, lit or dim. A transient
        // message must not be the only way to learn what mode you are in --
        // that is state, not news, and the visualizer had exactly that problem:
        // its name flashed for three seconds after `w` and then vanished.
        let (shuffled, repeat) = {
            let q = self.player.queue.lock().unwrap();
            (q.shuffled(), q.repeat())
        };
        let segments = status_indicators(self.vis.mode, shuffled, repeat);
        let ind_w = indicator_width(&segments);

        let recent = self
            .status
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(3));
        let text = match recent {
            Some((msg, _)) => msg.clone(),
            None => {
                // `? help` leads, because this line is truncated from the
                // right to fit: the way round it used to be, the one hint
                // that tells you where every other key is documented was the
                // first thing a narrow terminal dropped.
                "? help · space play · z/b prev/next · S shuffle now · p playlist · alt+e choose · q quit".into()
            }
        };

        let left_w = area.width.saturating_sub(ind_w + 3) as usize;
        buf.set_string(
            area.x + 1,
            area.y,
            crate::ui::panels::player::truncate(&text, left_w),
            Style::default()
                .fg(Color::Rgb(t.status_fg.r, t.status_fg.g, t.status_fg.b))
                .bg(bg),
        );

        if area.width > ind_w + 4 {
            let on = Color::Rgb(
                t.transport_toggle_on_fg.r,
                t.transport_toggle_on_fg.g,
                t.transport_toggle_on_fg.b,
            );
            let off = Color::Rgb(
                t.transport_toggle_off_fg.r,
                t.transport_toggle_off_fg.g,
                t.transport_toggle_off_fg.b,
            );
            let dim = Color::Rgb(t.dim.r, t.dim.g, t.dim.b);

            // Drawn segment by segment so each one is lit by its own state.
            // A single colour over the whole string lit SHUF whenever repeat
            // was on, which said the wrong thing.
            let mut x = area.x + area.width - ind_w - 1;
            for (i, (label, lit)) in segments.iter().enumerate() {
                if i > 0 {
                    buf.set_string(x, area.y, SEPARATOR, Style::default().fg(dim).bg(bg));
                    x += SEPARATOR.chars().count() as u16;
                }
                buf.set_string(
                    x,
                    area.y,
                    label,
                    Style::default().fg(if *lit { on } else { off }).bg(bg),
                );
                x += label.chars().count() as u16;
            }
        }
    }

    fn draw_help(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let t = &self.look.theme;
        let fg = Color::Rgb(t.fg.r, t.fg.g, t.fg.b);
        let key = Color::Rgb(t.accent.r, t.accent.g, t.accent.b);
        let head = Color::Rgb(t.warn.r, t.warn.g, t.warn.b);

        // Two columns, because the key list alone is longer than most
        // terminals are tall and used to be silently clipped.
        let w = area.width.min(80);
        let h = area.height.min(38);
        let rect = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        Clear.render(rect, buf);

        let heading = |g: &str| {
            ratatui::text::Line::from(ratatui::text::Span::styled(
                format!("  {g}"),
                Style::default().fg(head).add_modifier(Modifier::BOLD),
            ))
        };
        let entry = |k: &str, label: &str, pad: usize| {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(format!("  {k:<pad$}"), Style::default().fg(key)),
                ratatui::text::Span::styled(label.to_string(), Style::default().fg(fg)),
            ])
        };

        let mut keys: Vec<ratatui::text::Line> = Vec::new();
        let mut group = "";
        for b in keymap::BINDINGS {
            if b.group != group {
                group = b.group;
                keys.push(heading(group));
            }
            keys.push(entry(b.keys, b.label, 14));
        }

        let mut mouse: Vec<ratatui::text::Line> = Vec::new();
        group = "";
        for m in keymap::MOUSE {
            if m.group != group {
                group = m.group;
                mouse.push(heading(group));
            }
            mouse.push(entry(m.gesture, m.label, 21));
        }

        // Clamped here rather than where the key is handled, because only the
        // draw knows how tall the box came out and how many lines went in it.
        let inner_h = h.saturating_sub(2);
        let over = (keys.len() as u16).saturating_sub(inner_h);
        let at = self.panels.help_scroll.min(over);
        let title = if over == 0 {
            " HELP ".to_string()
        } else if at == 0 {
            " HELP \u{2014} more below ".to_string()
        } else if at == over {
            " HELP \u{2014} the end ".to_string()
        } else {
            " HELP \u{2014} more below ".to_string()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Rgb(
                t.border_focused.r,
                t.border_focused.g,
                t.border_focused.b,
            )))
            // Styled rather than inherited: an untitled `title` takes the
            // block's border colour, which is chrome and reads as chrome. The
            // other overlays all name themselves in `header_fg`, and this is
            // the one you open when you cannot find something.
            .title(ratatui::text::Span::styled(
                title,
                Style::default()
                    .fg(Color::Rgb(t.header_fg.r, t.header_fg.g, t.header_fg.b))
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(t.bg.r, t.bg.g, t.bg.b)));
        let inner = block.inner(rect);
        block.render(rect, buf);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);
        Paragraph::new(keys)
            .wrap(Wrap { trim: false })
            .scroll((at, 0))
            .render(cols[0], buf);
        Paragraph::new(mouse)
            .wrap(Wrap { trim: false })
            .render(cols[1], buf);
    }
}
