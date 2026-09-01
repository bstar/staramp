//! Drawing real pixels in a terminal, where the terminal will have it.
//!
//! Half blocks put two pixels in a cell, which is enough to tell one album
//! cover from another and not enough to read the title on it. Kitty's graphics
//! protocol puts the actual image there instead.
//!
//! The awkward part is the probe. Asking a terminal what it supports means
//! writing an escape sequence and reading the reply off stdin, and that cannot
//! be done once the app owns the keyboard -- the reply would arrive as
//! keystrokes. So the probe happens before the alternate screen, at startup,
//! and its answer is carried in.

use std::sync::Arc;

use ratatui::layout::{Rect, Size};
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

/// What the user asked for, from `[ui] graphics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Ask the terminal and believe it.
    #[default]
    Auto,
    /// Insist on kitty. Over ssh and inside multiplexers the probe sometimes
    /// says no when the answer is yes.
    Kitty,
    /// Never use a protocol; draw covers as half blocks.
    Blocks,
    /// Draw no cover at all.
    Off,
}

impl Mode {
    /// The name this mode has in `config.toml`, which is also the one worth
    /// showing: a setting should be listed by the word you would type.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Kitty => "kitty",
            Mode::Blocks => "blocks",
            Mode::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "kitty" => Mode::Kitty,
            "blocks" | "halfblocks" => Mode::Blocks,
            "off" | "none" => Mode::Off,
            _ => Mode::Auto,
        }
    }
}

/// A built protocol and what it was built for.
///
/// Encoding an image is not free, and the panel redraws thirty times a second.
/// Rebuilding only when the picture or the space it goes in actually changes is
/// the difference between this being usable and not.
struct Cached {
    /// The decoded cover this was built from.
    ///
    /// The image itself, not a name for it. Keying on the track's URI looked
    /// equivalent and was not: cycling to another of an album's covers leaves
    /// the track, and the area, exactly as they were, so every cover after the
    /// first was drawn as the one before it while the caption underneath said
    /// otherwise.
    ///
    /// Held rather than compared by address alone, so a freed image's address
    /// cannot be reused by the next one and mistaken for it.
    image: Arc<image::RgbImage>,
    size: Size,
    protocol: Protocol,
}

/// Is what was built last time still what is being asked for?
///
/// The picture, by identity, and the area it was built for. Split out because
/// it is the whole of the caching decision and the only part of it that can be
/// tested without a terminal to encode into.
fn is_fresh(
    cached: Option<(&Arc<image::RgbImage>, Size)>,
    img: &Arc<image::RgbImage>,
    size: Size,
) -> bool {
    cached.is_some_and(|(had, at)| Arc::ptr_eq(had, img) && at == size)
}

pub struct Graphics {
    mode: Mode,
    /// What the terminal said it could do, kept so the setting can be changed
    /// back without asking again -- which cannot be done once the alternate
    /// screen is up, since the reply would arrive as keystrokes.
    probed: Option<Picker>,
    /// `None` when the terminal has no protocol, or the user asked for none.
    picker: Option<Picker>,
    cached: Option<Cached>,
}

impl Graphics {
    /// Ask the terminal what it can do.
    ///
    /// **Must be called before the alternate screen is entered**, and before
    /// raw mode: the query's reply comes back on stdin, and once the app is
    /// reading keys it would be read as one.
    ///
    /// Never fails. A terminal that does not answer, or answers badly, simply
    /// gets half blocks.
    pub fn probe(mode: Mode) -> Self {
        let probed = match mode {
            // Never asked, so there is nothing to go back to later.
            Mode::Off | Mode::Blocks => None,
            // Only ask something that will answer.
            //
            // Not squeamishness about the cost: a terminal that answers the
            // query does it in single-digit milliseconds, and one that answers
            // *anything* -- even "no graphics here" -- costs nothing either,
            // because the reply ends the probe. The danger is the thing that
            // answers nothing at all, which is not a terminal so much as a pty
            // with nobody behind it: a bare `script`, some CI harnesses. There
            // the probe's reader thread is still sitting on stdin when it gives
            // up, and it eats the backend's own cursor-position reply, and the
            // first frame never comes. Measured: 0.11s to draw when the far end
            // answers anything, and no frame at all when it answers nothing.
            //
            // So the question is not "can this terminal draw pixels" but "is
            // anyone listening", and the environment is read for that.
            Mode::Auto if !looks_capable() => {
                tracing::debug!("graphics: nothing in the environment suggests a protocol");
                None
            }
            _ => match Picker::from_query_stdio_with_options(QueryStdioOptions {
                // A quarter of a second. The library's own default is two, and
                // a terminal that is going to answer does so in single-digit
                // milliseconds -- the wait is only ever paid by one that will
                // not answer at all, and paying two seconds of it before the
                // first frame is worse than drawing half blocks.
                timeout: std::time::Duration::from_millis(250),
                ..Default::default()
            }) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::debug!("no graphics protocol: {e}");
                    None
                }
            },
        };
        let mut g = Self {
            mode,
            probed,
            picker: None,
            cached: None,
        };
        g.apply();
        if let Some(p) = &g.picker {
            tracing::info!("graphics: {:?}", p.protocol_type());
        }
        g
    }

    /// Change how covers are drawn.
    ///
    /// Returns false when the choice cannot be honoured for the rest of this
    /// run: `auto` re-detects nothing, because detection can only happen
    /// before the alternate screen. Saying so is better than appearing to do
    /// nothing.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        self.mode = mode;
        self.apply();
        !(mode == Mode::Auto && self.probed.is_none())
    }

    /// Point the picker at whatever the current mode asks for.
    fn apply(&mut self) {
        // The built image belongs to the old renderer.
        self.cached = None;
        self.picker = match self.mode {
            Mode::Off | Mode::Blocks => None,
            // Half blocks are drawn by the panel itself, which knows the
            // theme; there is nothing for a picker to add.
            Mode::Auto => self
                .probed
                .clone()
                .filter(|p| p.protocol_type() != ProtocolType::Halfblocks),
            Mode::Kitty => {
                // Being told to use kitty is the point of this setting, so it
                // is honoured even when nothing was detected -- over ssh and
                // inside a multiplexer the outer terminal cannot be seen from
                // here. The font size comes from the probe when there was one.
                let mut p = self.probed.clone().unwrap_or_else(Picker::halfblocks);
                p.set_protocol_type(ProtocolType::Kitty);
                Some(p)
            }
        };
    }

    /// Half blocks with no probe at all, for tests and for `graphics = "off"`.
    pub fn disabled() -> Self {
        Self {
            mode: Mode::Off,
            probed: None,
            picker: None,
            cached: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// How tall a terminal cell is relative to its width.
    ///
    /// The usual assumption is two, and it is only ever approximately true:
    /// kitty reports its real cell size, and at a typical font that is nearer
    /// 2.1 or 2.4. It matters because it decides how many columns make a
    /// square, and getting it wrong leaves a cover letterboxed inside the
    /// space reserved for it rather than filling it.
    ///
    /// `None` when nothing has measured it, in which case two is as good a
    /// guess as any.
    pub fn cell_aspect(&self) -> Option<f32> {
        let f = self.picker.as_ref()?.font_size();
        (f.width > 0 && f.height > 0).then(|| f.height as f32 / f.width as f32)
    }

    /// What is actually in use, for the help overlay.
    pub fn name(&self) -> &'static str {
        match self.picker.as_ref().map(|p| p.protocol_type()) {
            Some(ProtocolType::Kitty) => "kitty",
            Some(ProtocolType::Sixel) => "sixel",
            Some(ProtocolType::Iterm2) => "iterm2",
            _ if self.mode == Mode::Off => "off",
            _ => "half blocks",
        }
    }

    /// A protocol for this image at this size, building one if need be.
    ///
    /// `None` means "draw it yourself": either there is no protocol, or
    /// encoding failed, and in both cases half blocks are the right answer
    /// rather than an empty panel.
    pub fn protocol(&mut self, img: &Arc<image::RgbImage>, area: Rect) -> Option<&Protocol> {
        let picker = self.picker.as_ref()?;
        let size = Size::new(area.width, area.height);
        if area.width == 0 || area.height == 0 {
            return None;
        }

        let fresh = is_fresh(self.cached.as_ref().map(|c| (&c.image, c.size)), img, size);
        if !fresh {
            let dynamic = image::DynamicImage::ImageRgb8((**img).clone());
            match picker.new_protocol(dynamic, size, Resize::Fit(None)) {
                Ok(protocol) => {
                    self.cached = Some(Cached {
                        image: Arc::clone(img),
                        size,
                        protocol,
                    })
                }
                Err(e) => {
                    tracing::debug!("encoding a cover failed: {e}");
                    self.cached = None;
                }
            }
        }
        self.cached.as_ref().map(|c| &c.protocol)
    }

    /// Forget the built image.
    ///
    /// Kitty keeps uploaded images in the terminal's own memory keyed by id;
    /// dropping the protocol is what releases one.
    pub fn forget(&mut self) {
        self.cached = None;
    }
}

/// Is there a terminal on the other end that will answer a question?
///
/// Two ways to be sure. Either the environment names a terminal that speaks a
/// graphics protocol -- which is the local case, where these variables are
/// actually set -- or this is an ssh session, where a real terminal is by
/// definition attached and only `TERM` survives the hop.
///
/// That second clause is the whole point. `TERM_PROGRAM` and
/// `WEZTERM_EXECUTABLE` are local variables that ssh does not forward, and a
/// terminal whose terminfo is not installed on the far end gets `TERM`
/// normalised to `xterm-256color` -- which is how Ghostty and WezTerm, both of
/// which speak the kitty protocol, ended up drawing half blocks over ssh
/// without ever being asked. Being on the far end of an ssh connection is
/// itself the evidence that somebody is listening, so ask.
fn looks_capable() -> bool {
    use std::env::var;
    let term = var("TERM").unwrap_or_default().to_ascii_lowercase();
    let program = var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();

    term.contains("kitty")
    || term.contains("ghostty")
    || term.contains("wezterm")
    || term.contains("sixel")
    || term.starts_with("foot")
    || term.starts_with("contour")
    || matches!(program.as_str(), "wezterm" | "iterm.app" | "ghostty")
    || var("KITTY_WINDOW_ID").is_ok()
    || var("GHOSTTY_RESOURCES_DIR").is_ok()
    || var("WEZTERM_EXECUTABLE").is_ok()
    || var("KONSOLE_VERSION").is_ok()
    // Inside a multiplexer the outer terminal is invisible from here, so
    // ask: the passthrough support is the whole reason the query exists.
    || var("TMUX").is_ok()
    // And over ssh, where the outer terminal is invisible for the same
    // reason and `TERM` has very likely been flattened on the way.
    || var("SSH_TTY").is_ok()
    || var("SSH_CONNECTION").is_ok()
}

/// Probe, but only if this is really a terminal.
///
/// Piping staramp's output somewhere is not a normal thing to do, but a query
/// written into a pipe waits for a reply that will never come. Infallible on
/// purpose: no capability check is worth refusing to start over.
pub fn probe_if_tty(mode: Mode) -> Graphics {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Graphics::disabled();
    }
    let g = Graphics::probe(mode);
    drain_stdin();
    g
}

/// Throw away whatever the terminal said back.
///
/// A capability query is a conversation, and terminals are not obliged to
/// answer only the questions asked -- a reply that arrives late, or one the
/// probe did not consume, is still sitting in the input buffer afterwards.
/// Left there it is read as keystrokes, and the first thing that reads it is
/// the backend's own cursor-position query, which then fails and takes startup
/// with it.
fn drain_stdin() {
    use crossterm::event::{poll, read};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use std::time::Duration;

    if enable_raw_mode().is_err() {
        return;
    }
    // A bounded number of reads: a terminal that talks forever must not be
    // able to hold startup open.
    for _ in 0..64 {
        match poll(Duration::from_millis(20)) {
            Ok(true) => {
                if read().is_err() {
                    break;
                }
            }
            _ => break,
        }
    }
    let _ = disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_name_parses_back_to_itself() {
        // The name is written into config.toml and read back on the next run,
        // so a mode that cannot survive the round trip is a setting that
        // silently resets.
        for m in [Mode::Auto, Mode::Kitty, Mode::Blocks, Mode::Off] {
            assert_eq!(Mode::parse(m.name()), m, "{} did not round trip", m.name());
        }
    }

    #[test]
    fn the_mode_names_are_the_ones_documented_in_the_config() {
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse("kitty"), Mode::Kitty);
        assert_eq!(Mode::parse("blocks"), Mode::Blocks);
        assert_eq!(Mode::parse("off"), Mode::Off);
        // A typo falls back to detection rather than to nothing: an unreadable
        // setting should not cost the user their covers.
        assert_eq!(Mode::parse("KITTEN"), Mode::Auto);
        assert_eq!(Mode::parse(""), Mode::Auto);
    }

    #[test]
    fn without_a_protocol_the_panel_is_told_to_draw_it_itself() {
        let mut g = Graphics::disabled();
        let img = Arc::new(image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3])));
        assert!(g.protocol(&img, Rect::new(0, 0, 12, 6)).is_none());
        assert_eq!(g.name(), "off");
    }

    #[test]
    fn a_different_cover_is_a_different_picture_even_at_the_same_size() {
        // The bug this exists for: the encoded image used to be kept against
        // the *track*, so cycling to another of an album's covers -- same
        // track, same panel, same size -- served the one before it while the
        // caption underneath named the new one.
        let one = Arc::new(image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3])));
        let two = Arc::new(image::RgbImage::from_pixel(4, 4, image::Rgb([9, 9, 9])));
        let size = Size::new(12, 6);

        assert!(!is_fresh(None, &one, size), "nothing built yet");
        assert!(is_fresh(Some((&one, size)), &one, size), "the same cover");
        assert!(
            !is_fresh(Some((&one, size)), &two, size),
            "another cover of the same album, at the same size"
        );
        assert!(
            !is_fresh(Some((&one, size)), &one, Size::new(20, 10)),
            "the same cover in a resized panel"
        );

        // Two images that happen to be identical are still two images: the
        // test is which one the panel was handed, not what is in it.
        let same_pixels = Arc::new(image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3])));
        assert!(!is_fresh(Some((&one, size)), &same_pixels, size));
    }
}
