pub mod album;
pub mod chooser;
pub mod equalizer;
pub mod faces;
pub mod files;
pub mod header;
pub mod history;
pub mod library;
pub mod picker;
pub mod player;
pub mod playlist;
pub mod resume;
pub mod visualizer;

// The panel chrome itself -- the solid double-line frame, the settings overlay -- is
// shared. Re-exported under the names the panels already use it by, since
// where the code lives is not something a call site should have to know.
pub use starkit::chrome::{frame, overlay, scrollbar, settings};

/// One key per scrollbar a panel draws, shared with `App`'s single
/// `starkit::chrome::scrollbar::Scrollbars<Bar>` so a press or drag on any of
/// them is answered from the one place that owns the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    Playlist,
    /// The library's three columns, `ARTISTS`/`ALBUMS`/`TRACKS` by index --
    /// see [`library::ARTISTS`] and its neighbours.
    Library(usize),
    History,
    Files,
    Chooser,
    Picker,
}
