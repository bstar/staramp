//! MPRIS, where there is no MPRIS.
//!
//! MPRIS is D-Bus, and D-Bus is a Linux desktop convention: macOS has nothing
//! that answers on `org.mpris.MediaPlayer2` and never will. The real module
//! (`mpris.rs`) already degrades gracefully at runtime when no session bus is
//! present, so this stub is not about avoiding a crash -- it is about not
//! linking zbus, tokio and a D-Bus protocol implementation into a binary that
//! provably cannot use them.
//!
//! The shape is the real module's, so every call site compiles unchanged: a
//! player that finds no bus and one built where no bus can exist take exactly
//! the same `Option::None` path.

use std::sync::Arc;

use crate::audio::player::Player;

pub const BUS_NAME: &str = "staramp";

pub enum MprisEvent {
    TrackChanged,
    StateChanged,
    Seeked(f64),
    Quit,
}

pub struct MprisHandle {
    _private: (),
}

impl MprisHandle {
    pub fn notify(&self, _e: MprisEvent) {}
}

/// Always `None`: there is no bus to publish on.
pub fn spawn(_player: Arc<Player>) -> Option<MprisHandle> {
    None
}

/// Always false, so `spawn` is never even reached.
pub fn session_bus_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    /// The same assertion the real module makes, so the name cannot drift on
    /// one platform without the other noticing.
    #[test]
    fn the_bus_name_is_what_status_bars_match_on() {
        assert_eq!(super::BUS_NAME, "staramp");
    }

    #[test]
    fn there_is_never_a_session_bus() {
        assert!(!super::session_bus_available());
    }
}
