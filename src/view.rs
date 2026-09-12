//! The part of the screen several windows share.
//!
//! Not the whole screen. What a terminal is -- its size, its colours, whether
//! it can draw pixels, how wide a cell is -- belongs to the window it is, and
//! two windows of different heights scroll differently even when they agree
//! about everything else. What they share is what is *being looked at*: which
//! track the cursor is on, which records are folded shut, which panels are
//! open.
//!
//! Held behind one mutex and stamped with a revision. Every window reconciles
//! against it the same way, whether it owns the session or is following
//! another instance over the socket -- the local case is a lock and the remote
//! case is a request, and neither is allowed to be the special one.

use std::sync::{Arc, Mutex};

/// What every window showing this session should agree about.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct View {
    /// What the queue is called, for the panel title. Shared so a window that
    /// joined a session shows the playlist's name rather than a word about its
    /// own relationship to the session.
    pub playlist_name: String,
    /// The file it was loaded from, so whichever instance ends up writing the
    /// session file can name it. Empty when the queue came from the library.
    pub playlist_path: String,
    /// The track the cursor is on, by URI.
    ///
    /// Not a position. Two windows agree on the track; they do not agree on
    /// where it sits, because that moves with a reorder and with a fold.
    pub cursor: String,
    /// Records folded shut, by album title, lower case.
    pub folded: Vec<String>,
    pub show_album: bool,
    pub show_equalizer: bool,
    pub show_playlist: bool,
    pub show_scrobbler: bool,
    /// Bumped on every change, so a window can tell at a glance whether what it
    /// is showing is still current.
    pub revision: u64,
}

impl View {
    /// Is this a different view from `other`, ignoring the revision?
    ///
    /// The revision is bookkeeping about the change rather than part of it;
    /// comparing it would make every view differ from every other.
    pub fn differs(&self, other: &View) -> bool {
        self.cursor != other.cursor
            || self.playlist_name != other.playlist_name
            || self.playlist_path != other.playlist_path
            || self.folded != other.folded
            || self.show_album != other.show_album
            || self.show_equalizer != other.show_equalizer
            || self.show_playlist != other.show_playlist
            || self.show_scrobbler != other.show_scrobbler
    }
}

/// The shared view, as the instance that owns the session holds it.
pub type Shared = Arc<Mutex<View>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(View::default()))
}

/// Replace the view, keeping the revision moving.
///
/// A no-op when nothing actually changed, so a window republishing what it
/// already published does not make every other window think there is news.
pub fn publish(shared: &Shared, next: &View) -> u64 {
    let mut held = shared.lock().unwrap();
    if held.differs(next) {
        let revision = held.revision + 1;
        *held = next.clone();
        held.revision = revision;
    }
    held.revision
}

/// How much of the view is shared, from `[session] share`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Share {
    /// Cursor, folds and open panels move together.
    #[default]
    View,
    /// Only the music does. Each window keeps its own place in the list.
    Playback,
}

impl Share {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "playback" | "music" | "off" => Share::Playback,
            _ => Share::View,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Share::View => "view",
            Share::Playback => "playback",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_view() -> View {
        View {
            playlist_name: "1995".into(),
            playlist_path: "/music/1995.m3u".into(),
            cursor: "Angra/Rebirth/02.flac".into(),
            folded: vec!["holy land".into()],
            show_album: true,
            show_equalizer: false,
            show_playlist: true,
            show_scrobbler: false,
            revision: 7,
        }
    }

    #[test]
    fn the_revision_is_bookkeeping_not_part_of_the_view() {
        let a = a_view();
        let mut b = a.clone();
        b.revision = 99;
        assert!(!a.differs(&b), "the same view, counted differently");
        b.cursor = "somewhere/else.flac".into();
        assert!(a.differs(&b));
    }

    #[test]
    fn publishing_the_same_view_is_not_news() {
        let s = shared();
        let v = a_view();
        let first = publish(&s, &v);
        assert_eq!(first, 1, "the first publication is a change");
        assert_eq!(publish(&s, &v), first, "and the second is not");

        let mut moved = v.clone();
        moved.cursor = "elsewhere.flac".into();
        assert_eq!(publish(&s, &moved), first + 1);
        assert_eq!(s.lock().unwrap().cursor, "elsewhere.flac");
    }

    #[test]
    fn a_fold_is_a_change_and_so_is_a_panel() {
        let s = shared();
        let v = a_view();
        let at = publish(&s, &v);

        let mut folded = v.clone();
        folded.folded.push("chained".into());
        assert_eq!(publish(&s, &folded), at + 1);

        let mut panel = folded.clone();
        panel.show_equalizer = true;
        assert_eq!(publish(&s, &panel), at + 2);
    }

    #[test]
    fn the_sharing_setting_reads_what_people_write() {
        assert_eq!(Share::parse("view"), Share::View);
        assert_eq!(Share::parse("playback"), Share::Playback);
        assert_eq!(Share::parse("off"), Share::Playback);
        // Anything unrecognised keeps the default rather than turning it off.
        assert_eq!(Share::parse("nonsense"), Share::View);
        assert_eq!(Share::parse(""), Share::View);
        assert_eq!(Share::View.name(), "view");
    }

    #[test]
    fn it_round_trips_through_the_wire() {
        let v = a_view();
        let back: View = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(back, v);
    }
}
