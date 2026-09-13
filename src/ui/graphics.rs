//! Drawing real pixels in a terminal, where the terminal will have it.
//!
//! The machinery is `starkit::graphics` -- probing, the modes, the cache, the
//! half-block fallback -- and what is left here is the part that is about this
//! player: which pictures it draws, and the setting that says whether the
//! transport buttons are pictures at all.

use ratatui::layout::Rect;
use ratatui_image::protocol::Protocol;

pub use starkit::graphics::{
    halfblocks, mend_unit_placeholder, placeholder, Graphics, ImageId, Mode,
};

use crate::theme::color::Rgb;
use crate::ui::panels::faces::Button;

/// Whether the transport buttons are drawn as pictures.
///
/// The pictures are the point -- they are the one way two terminals set to
/// different fonts show the same buttons -- so `Auto` is the default and
/// takes them wherever a protocol exists. `Text` is for seeing the fallback
/// without changing terminals, and for a terminal that claims a protocol it
/// does not really have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buttons {
    /// Pictures where the terminal can show them, ASCII where it cannot.
    Auto,
    /// The ASCII faces, whatever the terminal can do.
    Text,
}

impl Buttons {
    pub fn name(self) -> &'static str {
        match self {
            Buttons::Auto => "auto",
            Buttons::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "text" | "ascii" => Buttons::Text,
            _ => Buttons::Auto,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Buttons::Auto => Buttons::Text,
            Buttons::Text => Buttons::Auto,
        }
    }
}

/// The pictures this player rasterises for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Picture {
    /// A transport button: a plate with its icon.
    Button(Button),
    /// The playlist's playing-row marker: the play triangle alone, one cell.
    PlayMark,
}

/// Probe, but only if this is really a terminal.
pub fn probe_if_tty(mode: Mode) -> Graphics {
    Graphics::probe_if_tty(mode)
}

/// Log what the buttons and the covers will actually do.
///
/// Called once the picker is settled, so it reports the decision rather than
/// the inputs. The buttons are passed in because the setting belongs to the
/// application rather than to the terminal.
pub fn log_capabilities(g: &Graphics, buttons: Buttons) {
    g.log_capabilities();
    tracing::debug!("graphics: buttons {}", buttons.name());
}

/// This player's own pictures, over the shared rasteriser.
pub trait GraphicsExt {
    /// A picture the size of its cells, in the colours given, drawing and
    /// transmitting it the first time it is asked for. `plate` is the button
    /// plate's colour, and unused by the marker.
    ///
    /// `None` when there is no protocol, or the terminal never said how big a
    /// cell is -- in which case there is nothing to size the image to, and the
    /// caller draws the text face instead.
    fn picture(
        &mut self,
        which: Picture,
        area: Rect,
        fg: Rgb,
        plate: Rgb,
        bg: Rgb,
    ) -> Option<&Protocol>;
}

impl GraphicsExt for Graphics {
    fn picture(
        &mut self,
        which: Picture,
        area: Rect,
        fg: Rgb,
        plate: Rgb,
        bg: Rgb,
    ) -> Option<&Protocol> {
        // The colours are in the identity rather than the theme's name, so a
        // theme change simply misses and rebuilds.
        let tuple = |c: Rgb| (c.r, c.g, c.b);
        let id = ImageId::of(&(which, tuple(fg), tuple(plate), tuple(bg)));
        self.raster(id, area, |w, h| match which {
            Picture::Button(b) => crate::ui::panels::faces::raster(b, w, h, fg, plate, bg),
            Picture::PlayMark => crate::ui::panels::faces::mark(w, h, fg, bg),
        })
    }
}
