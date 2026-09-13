//! Decoding pictures that arrived with somebody else's music.
//!
//! Moved to starkit: every image either application decodes came from outside
//! it, and the header limits that stop a few kilobytes claiming to be
//! 10000x10000 are the same limits wherever the bytes came from.
//! Re-exported here so the art cache keeps its existing spelling.

pub use starkit::graphics::{decode_limited, open_limited, MAX_DIMENSION};
