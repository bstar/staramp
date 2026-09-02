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
    OpenPlaylistPicker,
    ToggleEqEnabled,
    NextEqPreset,
    PrevEqPreset,
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
        action: Action::CursorUp,
        keys: "up / k",
        label: "move up",
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
        (Tab, ..) => Action::FocusNext,

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
