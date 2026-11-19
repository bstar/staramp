//! Terminal setup and teardown.
//!
//! Moved to starcore. The panic hook in particular is not something to keep two
//! copies of: a TUI that panics without restoring the terminal leaves the user
//! blind-typing `reset`.

pub use starcore::term::*;
