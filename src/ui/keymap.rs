//! One table describing every action, its keys and its help text.
//!
//! Single source of truth on purpose. The reference implementation dispatches
//! keys in hand-written handlers *and* lists them in a separate registry *and*
//! documents them in a third place, which is how `r` ended up meaning four
//! different things. Here the help overlay and the dispatcher read the same
//! table, so they cannot drift.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Quit,
    Help,
    PlayPause,
    Stop,
    Next,
    Prev,
    SeekForward,
    SeekBack,
    SeekForwardBig,
    SeekBackBig,
    VolumeUp,
    VolumeDown,
    ToggleShuffle,
    ShuffleNow,
    CycleRepeat,
    ToggleEqPanel,
    ToggleAlbumPanel,
    ChooseCover,
    RetryCover,
    TogglePlaylistPanel,
    ToggleHistoryPanel,
    OpenPlaylistPicker,
    ToggleEqEnabled,
    NextEqPreset,
    PrevEqPreset,
    EqBandPrev,
    EqBandNext,
    EqGainUp,
    EqGainDown,
    EqGainUpBig,
    EqGainDownBig,
    FocusPrev,
    FocusPlayer,
    FocusPlaylist,
    FocusEqualizer,
    FocusAlbum,
    FocusHistory,
    OpenPanelSettings,
    CursorUpBig,
    CursorDownBig,
    CursorUp,
    CursorDown,
    PageUp,
    PageDown,
    Home,
    End,
    Activate,
    FocusNext,
    NextTheme,
    ToggleVisualizer,
    NextSeekStyle,
    ToggleAnimations,
    WidenBars,
    NarrowBars,
    NextButtons,
    PrevVisualizer,
    OpenFilter,
    FilterQueue,
    OpenLibrary,
    LibraryLeft,
    LibraryRight,
    LibrarySearch,
    LibraryAdd,
    LibraryAddAlbum,
    SavePlaylist,
    TagRow,
    ClearTags,
    CopyTagged,
    PasteTagged,
    MoveTagged,
    RemoveTagged,
    MoveAlbumUp,
    MoveAlbumDown,
    CloseOverlay,
}

/// What the mouse does.
///
/// Same single-table rule as the key bindings: the help overlay reads this, so
/// a gesture cannot be implemented and left undocumented in a second place.
pub struct MouseHelp {
    pub gesture: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

pub const MOUSE: &[MouseHelp] = &[
    MouseHelp {
        gesture: "wheel",
        label: "scroll the list",
        group: "playlist",
    },
    MouseHelp {
        gesture: "click",
        label: "select a track",
        group: "playlist",
    },
    MouseHelp {
        gesture: "double click",
        label: "play it",
        group: "playlist",
    },
    MouseHelp {
        gesture: "click `filter`",
        label: "order the playlist",
        group: "playlist",
    },
    MouseHelp {
        gesture: "click the transport",
        label: "transport",
        group: "player",
    },
    MouseHelp {
        gesture: "click SHUF / REP",
        label: "toggle",
        group: "player",
    },
    MouseHelp {
        gesture: "click or drag bar",
        label: "seek",
        group: "player",
    },
    MouseHelp {
        gesture: "wheel over bar",
        label: "seek 5s",
        group: "player",
    },
    MouseHelp {
        gesture: "click or drag VOL",
        label: "set volume",
        group: "player",
    },
    MouseHelp {
        gesture: "wheel over VOL",
        label: "volume 5%",
        group: "player",
    },
    MouseHelp {
        gesture: "click analyzer",
        label: "next visualization",
        group: "player",
    },
    MouseHelp {
        gesture: "right click",
        label: "play or pause",
        group: "player",
    },
    MouseHelp {
        gesture: "click [ON ]",
        label: "enable or bypass",
        group: "equalizer",
    },
    MouseHelp {
        gesture: "click the chevrons",
        label: "change preset",
        group: "equalizer",
    },
    MouseHelp {
        gesture: "click or drag a band",
        label: "set its gain",
        group: "equalizer",
    },
    MouseHelp {
        gesture: "wheel over a band",
        label: "1 dB",
        group: "equalizer",
    },
    MouseHelp {
        gesture: "click outside",
        label: "close the picker",
        group: "overlays",
    },
    MouseHelp {
        gesture: "click `close`",
        label: "close a panel",
        group: "overlays",
    },
    MouseHelp {
        gesture: "click `settings`",
        label: "what that panel controls",
        group: "overlays",
    },
];

pub struct Binding {
    pub action: Action,
    pub keys: &'static str,
    pub label: &'static str,
    pub group: &'static str,
}

/// Displayed in the help overlay, in this order.
pub const BINDINGS: &[Binding] = &[
    Binding {
        action: Action::PlayPause,
        keys: "space / c",
        label: "play or pause",
        group: "transport",
    },
    Binding {
        action: Action::Stop,
        keys: "v",
        label: "stop",
        group: "transport",
    },
    Binding {
        action: Action::Prev,
        keys: "z",
        label: "previous track",
        group: "transport",
    },
    Binding {
        action: Action::Next,
        keys: "b",
        label: "next track",
        group: "transport",
    },
    Binding {
        action: Action::VolumeUp,
        keys: "ctrl+up",
        label: "volume up",
        group: "transport",
    },
    Binding {
        action: Action::VolumeDown,
        keys: "ctrl+down",
        label: "volume down",
        group: "transport",
    },
    Binding {
        action: Action::ToggleShuffle,
        keys: "s",
        label: "shuffle",
        group: "transport",
    },
    Binding {
        action: Action::CycleRepeat,
        keys: "r",
        label: "repeat off/all/one",
        group: "transport",
    },
    Binding {
        action: Action::ToggleVisualizer,
        keys: "w",
        label: "next visualizer",
        group: "visualizer",
    },
    Binding {
        action: Action::PrevVisualizer,
        keys: "W",
        label: "previous one",
        group: "visualizer",
    },
    Binding {
        action: Action::WidenBars,
        keys: "+ / -",
        label: "bar width",
        group: "visualizer",
    },
    Binding {
        action: Action::SeekBack,
        keys: "left",
        label: "seek back 5s",
        group: "progress bar",
    },
    Binding {
        action: Action::SeekForward,
        keys: "right",
        label: "seek forward 5s",
        group: "progress bar",
    },
    Binding {
        action: Action::SeekBackBig,
        keys: "shift+left",
        label: "seek back 30s",
        group: "progress bar",
    },
    Binding {
        action: Action::SeekForwardBig,
        keys: "shift+right",
        label: "seek forward 30s",
        group: "progress bar",
    },
    Binding {
        action: Action::NextSeekStyle,
        keys: "d",
        label: "seek bar style",
        group: "progress bar",
    },
    Binding {
        action: Action::NextButtons,
        keys: "o",
        label: "button style",
        group: "progress bar",
    },
    Binding {
        action: Action::CursorUp,
        keys: "up / k",
        label: "move up",
        group: "playlist",
    },
    Binding {
        action: Action::CursorDownBig,
        keys: "shift+up/down",
        label: "ten rows",
        group: "playlist",
    },
    Binding {
        action: Action::CursorDown,
        keys: "down / j",
        label: "move down",
        group: "playlist",
    },
    Binding {
        action: Action::PageUp,
        keys: "pgup",
        label: "page up",
        group: "playlist",
    },
    Binding {
        action: Action::PageDown,
        keys: "pgdn",
        label: "page down",
        group: "playlist",
    },
    Binding {
        action: Action::Home,
        keys: "home / g",
        label: "first track",
        group: "playlist",
    },
    Binding {
        action: Action::End,
        keys: "end / G",
        label: "last track",
        group: "playlist",
    },
    Binding {
        action: Action::Activate,
        keys: "enter",
        label: "play selected",
        group: "playlist",
    },
    Binding {
        action: Action::OpenFilter,
        keys: "f",
        label: "order the playlist",
        group: "playlist",
    },
    Binding {
        action: Action::FilterQueue,
        keys: "/",
        label: "filter the playlist",
        group: "playlist",
    },
    Binding {
        action: Action::MoveAlbumUp,
        keys: "alt+up",
        label: "record up",
        group: "playlist",
    },
    Binding {
        action: Action::MoveAlbumDown,
        keys: "alt+down",
        label: "record down",
        group: "playlist",
    },
    Binding {
        action: Action::TagRow,
        keys: "t",
        label: "tag this row",
        group: "tagging",
    },
    Binding {
        action: Action::ClearTags,
        keys: "T",
        label: "clear every tag",
        group: "tagging",
    },
    Binding {
        action: Action::CopyTagged,
        keys: "y",
        label: "copy",
        group: "tagging",
    },
    Binding {
        action: Action::PasteTagged,
        keys: "u",
        label: "put them here",
        group: "tagging",
    },
    Binding {
        action: Action::MoveTagged,
        keys: "m",
        label: "move here",
        group: "tagging",
    },
    Binding {
        action: Action::RemoveTagged,
        keys: "del / D",
        label: "remove them",
        group: "tagging",
    },
    Binding {
        action: Action::ToggleEqPanel,
        keys: "alt+g",
        label: "show/hide equalizer",
        group: "windows",
    },
    Binding {
        action: Action::ToggleAlbumPanel,
        keys: "i",
        label: "album info on/off",
        group: "windows",
    },
    Binding {
        action: Action::ChooseCover,
        keys: "alt+i",
        label: "choose a cover",
        group: "windows",
    },
    Binding {
        action: Action::RetryCover,
        keys: "alt+r",
        label: "look the cover up",
        group: "windows",
    },
    Binding {
        action: Action::TogglePlaylistPanel,
        keys: "p",
        label: "playlist on/off",
        group: "windows",
    },
    Binding {
        action: Action::ToggleHistoryPanel,
        keys: "alt+s",
        label: "listening history",
        group: "windows",
    },
    Binding {
        action: Action::OpenPlaylistPicker,
        keys: "alt+e",
        label: "choose a playlist",
        group: "windows",
    },
    Binding {
        action: Action::FocusNext,
        keys: "tab",
        label: "next pane",
        group: "windows",
    },
    Binding {
        action: Action::FocusPrev,
        keys: "shift+tab",
        label: "previous pane",
        group: "windows",
    },
    Binding {
        action: Action::FocusPlayer,
        keys: "alt+1..5",
        label: "focus a pane",
        group: "windows",
    },
    Binding {
        action: Action::NextTheme,
        keys: "alt+t",
        label: "next theme",
        group: "appearance",
    },
    Binding {
        action: Action::ToggleAnimations,
        keys: "a",
        label: "animations on/off",
        group: "appearance",
    },
    Binding {
        action: Action::ToggleEqEnabled,
        keys: "e",
        label: "equalizer on/off",
        group: "equalizer",
    },
    Binding {
        action: Action::NextEqPreset,
        keys: "]",
        label: "next preset",
        group: "equalizer",
    },
    Binding {
        action: Action::EqBandNext,
        keys: "left/right",
        label: "choose a band",
        group: "equalizer",
    },
    Binding {
        action: Action::EqGainUp,
        keys: "up/down",
        label: "band gain, 1 dB",
        group: "equalizer",
    },
    Binding {
        action: Action::EqGainUpBig,
        keys: "shift+up/down",
        label: "band gain, 10 dB",
        group: "equalizer",
    },
    Binding {
        action: Action::PrevEqPreset,
        keys: "[",
        label: "previous preset",
        group: "equalizer",
    },
    Binding {
        action: Action::OpenLibrary,
        keys: "l",
        label: "browse the library",
        group: "library",
    },
    Binding {
        action: Action::LibrarySearch,
        keys: "/",
        label: "search it",
        group: "library",
    },
    Binding {
        action: Action::LibraryAdd,
        keys: "space",
        label: "add the selection",
        group: "library",
    },
    Binding {
        action: Action::LibraryAddAlbum,
        keys: "a",
        label: "add the record",
        group: "library",
    },
    Binding {
        action: Action::SavePlaylist,
        keys: "ctrl+s",
        label: "save the playlist",
        group: "library",
    },
    Binding {
        action: Action::LibraryLeft,
        keys: "\u{2190} / \u{2192}",
        label: "change column",
        group: "library",
    },
    Binding {
        action: Action::CloseOverlay,
        keys: "esc",
        label: "close what is open",
        group: "general",
    },
    Binding {
        action: Action::Help,
        keys: "? / F1",
        label: "this help",
        group: "general",
    },
    Binding {
        action: Action::Quit,
        keys: "q",
        label: "quit",
        group: "general",
    },
];

/// Keys that mean something else while the library browser is open.
///
/// A second table rather than a flag threaded through [`resolve`], because the
/// two keys the browser most needs are already taken by the transport and
/// cannot be shared: `space` is play/pause under *every* modifier, and the
/// arrows seek. One table means one meaning, so a second meaning needs a second
/// table.
///
/// Anything this does not claim falls through to [`resolve`], which is what
/// keeps `z`, `b` and `s` working while you browse -- the music carries on
/// behind the browser, and being unable to skip a track without leaving would
/// be absurd.
pub fn library(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    if ctrl || alt {
        return None;
    }
    Some(match k.code {
        Char(' ') => Action::LibraryAdd,
        Char('a') => Action::LibraryAddAlbum,
        Char('/') => Action::LibrarySearch,
        Left | Char('h') => Action::LibraryLeft,
        Right | Char('l') => Action::LibraryRight,
        _ => return None,
    })
}

/// Which docked panel a key is being offered to first.
///
/// Mirrors `ui::app::Focus`, and is its own type so `keymap` does not depend
/// on `app` -- the table is the thing everything else is written against, and
/// it should not need the application to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Player,
    Playlist,
    Equalizer,
    Album,
    History,
}

/// The focused panel's own bindings, tried before [`resolve`].
///
/// `None` means the module does not want this key and the global table should
/// have it -- which is what keeps every existing binding working from
/// everywhere. A module only ever *adds* meaning to keys it names.
///
/// Two rules hold across all four, and the tests enforce them:
///
/// **A bare arrow moves one unit and a shifted one moves ten.** Already true
/// of seeking before this existed, so it is a convention being extended
/// rather than imposed.
///
/// **`hjkl` navigates and never adjusts a value.** No module below binds
/// them: `j`/`k` reach the playlist cursor through the global table, which is
/// what they mean everywhere, and a module that wants a second axis takes the
/// arrows. The equalizer is the case that proves it -- selecting a band *is*
/// navigation and would be `h`/`l`, except `l` opens the library and taking
/// that away from the panel you would reach for it from is a bad trade.
pub fn module(m: Module, k: KeyEvent) -> Option<Action> {
    match m {
        Module::Player => player(k),
        Module::Playlist => playlist(k),
        Module::Equalizer => equalizer(k),
        Module::Album => album(k),
        Module::History => history(k),
    }
}

fn history(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    (!k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && k.code == Enter)
        .then_some(Action::OpenPanelSettings)
}

/// Seeking, at the two sizes.
///
/// Already in the global table and repeated here so the player's own bindings
/// are in one place and cannot be silently claimed by a future module.
fn player(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    Some(match (k.code, shift) {
        (Left, false) => Action::SeekBack,
        (Right, false) => Action::SeekForward,
        (Left, true) => Action::SeekBackBig,
        (Right, true) => Action::SeekForwardBig,
        _ => return None,
    })
}

/// The cursor, one row or ten.
///
/// `j`/`k` are deliberately absent: they are in the global table and reach
/// here through it, so they keep working from every panel rather than only
/// from this one.
fn playlist(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    Some(match (k.code, shift) {
        (Up, true) => Action::CursorUpBig,
        (Down, true) => Action::CursorDownBig,
        _ => return None,
    })
}

/// Band selection and gain.
///
/// The band is on the arrows rather than on `h`/`l` because `l` opens the
/// library; the gain is on the arrows because a gain is a value and `hjkl`
/// does not touch values.
fn equalizer(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    Some(match (k.code, shift) {
        (Left, _) => Action::EqBandPrev,
        (Right, _) => Action::EqBandNext,
        (Up, false) => Action::EqGainUp,
        (Down, false) => Action::EqGainDown,
        (Up, true) => Action::EqGainUpBig,
        (Down, true) => Action::EqGainDownBig,
        _ => return None,
    })
}

/// The cover: choose one, or open the panel's settings.
fn album(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    Some(match k.code {
        Char('c') => Action::ChooseCover,
        Char('r') => Action::RetryCover,
        _ => return None,
    })
}

/// Map a key event to an action.
pub fn resolve(k: KeyEvent) -> Option<Action> {
    use KeyCode::*;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

    Some(match (k.code, ctrl, shift, alt) {
        // Not Esc. Terminals encode Alt+key as ESC followed by the key, so
        // binding Esc to quit means every Alt binding is one dropped byte away
        // from closing the player. Esc only dismisses overlays.
        (Char('q'), false, _, false) => Action::Quit,
        (Esc, ..) => Action::CloseOverlay,
        (Char('?'), ..) | (F(1), ..) => Action::Help,

        (Char(' '), ..) | (Char('c'), false, _, false) => Action::PlayPause,
        (Char('v'), false, _, false) => Action::Stop,
        (Char('z'), false, _, false) => Action::Prev,
        (Char('b'), false, _, false) => Action::Next,
        (Char('x'), false, _, false) => Action::PlayPause,

        (Left, false, true, false) => Action::SeekBackBig,
        (Right, false, true, false) => Action::SeekForwardBig,
        (Left, false, false, false) => Action::SeekBack,
        (Right, false, false, false) => Action::SeekForward,

        (Up, true, ..) => Action::VolumeUp,
        (Down, true, ..) => Action::VolumeDown,

        (Char('s'), true, ..) => Action::SavePlaylist,
        (Char('s'), false, _, false) => Action::ToggleShuffle,
        (Char('S'), ..) => Action::ShuffleNow,
        (Char('r'), false, _, false) => Action::CycleRepeat,
        (Char('e'), false, _, false) => Action::ToggleEqEnabled,
        (Char(']'), ..) => Action::NextEqPreset,
        (Char('['), ..) => Action::PrevEqPreset,
        (Char('w'), false, _, false) => Action::ToggleVisualizer,
        (Char('W'), ..) => Action::PrevVisualizer,
        (Char('d'), false, _, false) => Action::NextSeekStyle,
        (Char('a'), false, _, false) => Action::ToggleAnimations,
        (Char('t'), false, _, false) => Action::TagRow,
        (Char('T'), ..) => Action::ClearTags,
        (Char('y'), false, _, false) => Action::CopyTagged,
        (Char('u'), false, _, false) => Action::PasteTagged,
        (Char('m'), false, _, false) => Action::MoveTagged,
        (Delete, ..) | (Char('D'), ..) => Action::RemoveTagged,
        (Char('f'), false, _, false) => Action::OpenFilter,
        (Char('/'), false, _, false) => Action::FilterQueue,
        (Char('o'), false, _, false) => Action::NextButtons,
        (Char('l'), false, _, false) => Action::OpenLibrary,
        (Char('+'), ..) | (Char('='), ..) => Action::WidenBars,
        (Char('-'), ..) | (Char('_'), ..) => Action::NarrowBars,

        (Char('g'), _, _, true) => Action::ToggleEqPanel,
        (Char('i'), false, _, false) => Action::ToggleAlbumPanel,
        (Char('i'), _, _, true) => Action::ChooseCover,
        (Char('r'), _, _, true) => Action::RetryCover,
        (Char('t'), _, _, true) => Action::NextTheme,
        (Char('p'), false, _, false) => Action::TogglePlaylistPanel,
        (Char('e'), _, _, true) => Action::OpenPlaylistPicker,
        (Char('s'), _, _, true) => Action::ToggleHistoryPanel,
        (Tab, ..) => Action::FocusNext,
        (BackTab, ..) => Action::FocusPrev,
        // Straight to a panel, opening it if it is shut. Alt because the bare
        // digits belong to whatever a panel wants them for, and ctrl+digit is
        // eaten by a good many terminals before it reaches us.
        (Char('1'), false, _, true) => Action::FocusPlayer,
        (Char('2'), false, _, true) => Action::FocusPlaylist,
        (Char('3'), false, _, true) => Action::FocusEqualizer,
        (Char('4'), false, _, true) => Action::FocusAlbum,
        (Char('5'), false, _, true) => Action::FocusHistory,

        (Up, false, _, true) => Action::MoveAlbumUp,
        (Down, false, _, true) => Action::MoveAlbumDown,
        (Up, false, ..) | (Char('k'), false, _, false) => Action::CursorUp,
        (Down, false, ..) | (Char('j'), false, _, false) => Action::CursorDown,
        (PageUp, ..) => Action::PageUp,
        (PageDown, ..) => Action::PageDown,
        (Home, ..) | (Char('g'), false, false, false) => Action::Home,
        (End, ..) | (Char('G'), ..) => Action::End,
        (Enter, ..) => Action::Activate,

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_appearance_keys_resolve() {
        use super::*;
        let plain = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(resolve(plain('d')), Some(Action::NextSeekStyle));
        assert_eq!(resolve(plain('/')), Some(Action::FilterQueue));
        assert_eq!(resolve(plain('o')), Some(Action::NextButtons));
        assert_eq!(resolve(plain('a')), Some(Action::ToggleAnimations));
        assert_eq!(resolve(plain('+')), Some(Action::WidenBars));
        assert_eq!(resolve(plain('=')), Some(Action::WidenBars));
        assert_eq!(resolve(plain('-')), Some(Action::NarrowBars));
    }

    #[test]
    fn every_binding_group_is_listed_once() {
        // The help overlay writes a heading whenever the group changes, so a
        // group split across the table prints its heading twice.
        use super::BINDINGS;
        let mut seen: Vec<&str> = Vec::new();
        let mut last = "";
        for b in BINDINGS {
            if b.group != last {
                assert!(
                    !seen.contains(&b.group),
                    "{} appears in two places in the table",
                    b.group
                );
                seen.push(b.group);
                last = b.group;
            }
        }
    }

    #[test]
    fn the_playlist_panel_and_its_picker_have_separate_keys() {
        // `p` used to reach the picker whenever there was one to reach, which
        // made it useless for the thing it is named after.
        use super::*;
        let plain = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let alt = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
        assert_eq!(resolve(plain('p')), Some(Action::TogglePlaylistPanel));
        assert_eq!(resolve(alt('e')), Some(Action::OpenPlaylistPicker));
    }

    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn winamp_transport_letters_work() {
        // z x c v b, exactly as the original.
        assert_eq!(resolve(key('z')), Some(Action::Prev));
        assert_eq!(resolve(key('x')), Some(Action::PlayPause));
        assert_eq!(resolve(key('c')), Some(Action::PlayPause));
        assert_eq!(resolve(key('v')), Some(Action::Stop));
        assert_eq!(resolve(key('b')), Some(Action::Next));
    }

    #[test]
    fn shuffle_has_a_toggle_and_a_go_now() {
        assert_eq!(resolve(key('s')), Some(Action::ToggleShuffle));
        assert_eq!(
            resolve(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Some(Action::ShuffleNow)
        );
    }

    #[test]
    fn space_is_play_pause_because_everyone_expects_it() {
        assert_eq!(resolve(key(' ')), Some(Action::PlayPause));
    }

    #[test]
    fn shift_changes_the_seek_step() {
        let plain = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        let shifted = KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT);
        assert_eq!(resolve(plain), Some(Action::SeekForward));
        assert_eq!(resolve(shifted), Some(Action::SeekForwardBig));
    }

    #[test]
    fn every_binding_in_the_help_table_actually_resolves() {
        // The whole reason for one table: help and dispatch cannot disagree.
        let actions: Vec<Action> = BINDINGS.iter().map(|b| b.action).collect();
        for a in [
            Action::Quit,
            Action::PlayPause,
            Action::Next,
            Action::Prev,
            Action::ToggleShuffle,
            Action::CycleRepeat,
            Action::Help,
        ] {
            assert!(actions.contains(&a), "{a:?} missing from the help table");
        }
    }

    /// The help's key column is 35 wide, of which 16 are the key pad, so a
    /// label past 19 characters wraps and costs a second line.
    ///
    /// Eight of them did, which is most of why the overlay had grown to 67
    /// lines against an inner height of 36 and had silently stopped showing
    /// everything after `windows`.
    fn ev(c: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(c, m)
    }

    const MODULES: [Module; 4] = [
        Module::Player,
        Module::Playlist,
        Module::Equalizer,
        Module::Album,
    ];

    /// The rule the whole scheme rests on.
    ///
    /// `hjkl` moves a cursor and never changes a value. Written as a test
    /// rather than left as a convention because it is the one a future module
    /// is most likely to break -- reaching for `j`/`k` as "down/up" on a gain
    /// is the obvious thing to do and the wrong thing to do.
    #[test]
    fn no_module_binds_hjkl() {
        for m in MODULES {
            for c in ['h', 'j', 'k', 'l'] {
                let got = module(m, ev(KeyCode::Char(c), KeyModifiers::NONE));
                assert!(
                    got.is_none(),
                    "{m:?} claims {c:?} as {got:?}; hjkl navigates and the \
                     global table is what carries it"
                );
            }
        }
    }

    /// A module only ever adds meaning; it never swallows a key it has no use
    /// for, or the global bindings would stop working panel by panel.
    #[test]
    fn a_module_declines_what_it_does_not_want() {
        for m in MODULES {
            for c in ['t', 'y', 'u', 'q', ' '] {
                assert_eq!(
                    module(m, ev(KeyCode::Char(c), KeyModifiers::NONE)),
                    None,
                    "{m:?} swallowed {c:?}"
                );
            }
            // Nor anything with ctrl or alt on it: those are the global
            // layer's, and volume is ctrl+up.
            for m2 in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
                assert_eq!(module(m, ev(KeyCode::Up, m2)), None, "{m:?}");
            }
        }
    }

    /// Bare is one unit, shifted is ten, in every module that has both.
    #[test]
    fn shift_is_the_coarse_modifier_everywhere() {
        let bare = |m, c| module(m, ev(c, KeyModifiers::NONE));
        let shifted = |m, c| module(m, ev(c, KeyModifiers::SHIFT));

        assert_eq!(bare(Module::Player, KeyCode::Left), Some(Action::SeekBack));
        assert_eq!(
            shifted(Module::Player, KeyCode::Left),
            Some(Action::SeekBackBig)
        );
        assert_eq!(
            shifted(Module::Playlist, KeyCode::Down),
            Some(Action::CursorDownBig)
        );
        assert_eq!(bare(Module::Equalizer, KeyCode::Up), Some(Action::EqGainUp));
        assert_eq!(
            shifted(Module::Equalizer, KeyCode::Up),
            Some(Action::EqGainUpBig)
        );
    }

    /// The equalizer is the module where both rules bite at once.
    #[test]
    fn the_equalizer_puts_bands_on_the_arrows_not_on_hl() {
        // Selecting a band is navigation, so it would be `h`/`l` -- except
        // `l` opens the library, and taking that away from a panel you would
        // reach for it from is the worse trade.
        assert_eq!(
            module(Module::Equalizer, ev(KeyCode::Left, KeyModifiers::NONE)),
            Some(Action::EqBandPrev)
        );
        assert_eq!(
            resolve(ev(KeyCode::Char('l'), KeyModifiers::NONE)),
            Some(Action::OpenLibrary),
            "the library key is what the band keys are avoiding"
        );
    }

    /// The playlist keeps `j`/`k` through the global table rather than
    /// claiming them, so they work from every panel and not just this one.
    #[test]
    fn the_cursor_keys_stay_global() {
        assert_eq!(
            resolve(ev(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(Action::CursorDown)
        );
        assert_eq!(
            module(
                Module::Equalizer,
                ev(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            None,
            "the equalizer must not shadow the playlist cursor"
        );
    }

    /// The promise the whole layering makes.
    ///
    /// A module gets first refusal, not exclusive rights. With the equalizer
    /// focused, every global binding still resolves -- which is the reason
    /// this was built as a fallback rather than as a replacement, and nothing
    /// else checks it.
    #[test]
    fn global_keys_still_work_from_inside_a_module() {
        for m in MODULES {
            for (c, want) in [
                (' ', Action::PlayPause),
                ('t', Action::TagRow),
                ('e', Action::ToggleEqEnabled),
                ('l', Action::OpenLibrary),
                ('j', Action::CursorDown),
                ('k', Action::CursorUp),
            ] {
                let k = ev(KeyCode::Char(c), KeyModifiers::NONE);
                let got = module(m, k).or_else(|| resolve(k));
                assert_eq!(got, Some(want), "{m:?} broke {c:?}");
            }
            // And the transport, which has to work from anywhere at all.
            let k = ev(KeyCode::Up, KeyModifiers::CONTROL);
            assert_eq!(module(m, k).or_else(|| resolve(k)), Some(Action::VolumeUp));
        }
    }

    #[test]
    fn focus_moves_both_ways_and_lands_directly() {
        assert_eq!(
            resolve(ev(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::FocusNext)
        );
        assert_eq!(
            resolve(ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::FocusPrev)
        );
        for (c, want) in [
            ('1', Action::FocusPlayer),
            ('2', Action::FocusPlaylist),
            ('3', Action::FocusEqualizer),
            ('4', Action::FocusAlbum),
        ] {
            assert_eq!(
                resolve(ev(KeyCode::Char(c), KeyModifiers::ALT)),
                Some(want),
                "alt+{c}"
            );
        }
    }

    #[test]
    fn no_help_label_is_long_enough_to_wrap() {
        for b in BINDINGS {
            assert!(
                b.label.chars().count() <= 19,
                "{:?}'s label wraps the help column: {:?} ({} chars)",
                b.action,
                b.label,
                b.label.chars().count()
            );
        }
    }

    #[test]
    fn every_mouse_group_is_listed_once_too() {
        // The help writes a heading on a group change for this table as well,
        // so a split group prints its heading twice -- and nothing checked it.
        let mut seen: Vec<&str> = Vec::new();
        let mut last = "";
        for m in MOUSE {
            if m.group != last {
                assert!(!seen.contains(&m.group), "{} appears twice", m.group);
                seen.push(m.group);
                last = m.group;
            }
        }
    }

    #[test]
    fn no_binding_is_listed_twice_with_conflicting_labels() {
        let mut seen = std::collections::HashMap::new();
        for b in BINDINGS {
            if let Some(prev) = seen.insert(b.action, b.label) {
                panic!("{:?} listed twice: {prev:?} and {:?}", b.action, b.label);
            }
        }
    }

    #[test]
    fn escape_does_not_quit_because_alt_keys_start_with_it() {
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(resolve(esc), Some(Action::CloseOverlay));
        assert_ne!(resolve(esc), Some(Action::Quit));
    }

    #[test]
    fn alt_bindings_resolve_to_their_own_actions() {
        let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
        assert_eq!(resolve(alt('g')), Some(Action::ToggleEqPanel));
        assert_eq!(resolve(alt('e')), Some(Action::OpenPlaylistPicker));
    }

    #[test]
    fn unbound_keys_are_ignored_rather_than_guessed() {
        assert_eq!(resolve(key('~')), None);
        assert_eq!(resolve(key('%')), None);
    }
}
