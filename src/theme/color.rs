//! Colour, and the perceptual maths the derivation chain needs.
//!
//! Moved to starkit: both applications derive a palette from the same eight
//! colours, and doing it twice would mean two answers. Re-exported here so the
//! forty-odd `crate::theme::color::Rgb` in this crate keep working.

pub use starkit::theme::color::*;
