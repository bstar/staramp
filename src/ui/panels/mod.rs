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

// The panel chrome itself -- the corner gradient, the settings overlay -- is
// shared. Re-exported under the names the panels already use it by, since
// where the code lives is not something a call site should have to know.
pub use starkit::chrome::{frame, overlay, settings};
