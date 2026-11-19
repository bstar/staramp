//! Changing one setting in `config.toml` without disturbing the rest of it.
//!
//! Moved to starcore; both applications have a hand-edited config file and the
//! same reason not to round-trip it through serde.

pub use starcore::config::edit::*;
