//! Changing one setting in `config.toml` without disturbing the rest of it.
//!
//! Moved to starkit; both applications have a hand-edited config file and the
//! same reason not to round-trip it through serde.

pub use starkit::config::edit::*;
