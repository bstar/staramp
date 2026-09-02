//! The Winamp main window.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

use crate::audio::player::PlayState;
use crate::playlist::queue::RepeatMode;
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::digits;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

pub struct PlayerView<'a> {
    pub theme: &'a Theme,
    pub title: String,
    pub subtitle: String,
    pub tech: String,
    pub state: PlayState,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: crate::playlist::queue::RepeatMode,
    pub bit_perfect: bool,
    pub focused: bool,
    /// True when this instance is mirroring another rather than playing.
    pub mirroring: bool,
    pub marquee_offset: usize,
    pub bands: &'a [f32],
    pub peaks: &'a [f32],
    pub wave: &'a [f32],
    pub vis_mode: crate::vis::mode::VisMode,
    pub bars: super::visualizer::BarLayout,
    pub glyphs: Glyphs,
    /// Where the seek highlight has travelled to, 0 to 1. Zero holds it off.
    pub seek_phase: f32,
    pub seek_style: SeekStyle,
    pub underruns: u64,
}

/// The transport button faces.
///
/// Switchable because the good glyphs are not universally available. A face
/// carries ink and nothing else -- no padding spaces. The faces in a set are
/// therefore *not* all the same width, and do not need to be: the plate is a
/// fixed size, so the buttons form an even row and are the same size to click
/// whatever they hold, and the face is centred in it.
///
/// They used to be padded to a common width with a trailing space, which is
/// what made `play` and `stop` sit hard against the left of their plates: the
/// centring saw a two-cell face where only one cell had ink in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    pub prev: &'static str,
    pub play: &'static str,
    pub pause: &'static str,
    pub stop: &'static str,
    pub next: &'static str,
}

impl Glyphs {
    /// One cell per face, so every one of them centres exactly.
    ///
    /// The width is the whole design. A face centres in its plate only when
    /// the two have the same parity, so a set of mixed one- and two-cell faces
    /// cannot centre all of them at any plate width: this set used to pair the
    /// triangles -- `prev` and `next` were two cells, `play` and `stop` one --
    /// and those two sat half a cell left of centre with nothing to be done
    /// about it. Uniform faces make the question go away.
    ///
    /// U+00AB and U+00BB for the skips, which are Latin-1 and therefore in
    /// every font that exists; U+25B6 and U+25A0 for play and stop, the
    /// full-size triangle and square a monospace font draws at cap height.
    ///
    /// U+23F8 for pause is a deliberate exception, chosen on how it looks.
    /// It carries **default emoji presentation**: `unicode-width` reports one
    /// cell and a terminal is entitled to draw two, in colour, and a face a
    /// cell wider than the layout believes shifts every button after it. It
    /// renders as text in the terminals this was checked in, and `block` and
    /// `ascii` are there for one where it does not. Nothing else in any set
    /// may do this -- `no_face_risks_being_drawn_as_an_emoji` holds the line,
    /// and names this one codepoint as the only exception.
    pub const UNICODE: Self = Self {
        prev: "\u{00ab}",
        play: "\u{25b6}",
        pause: "\u{23f8}",
        stop: "\u{25a0}",
        next: "\u{00bb}",
    };

    /// Nerd Font private-use icons, from the Material Design set.
    ///
    /// Useless without a patched font, which is why they are not the default.
    /// Worth knowing before choosing them: patched fonts draw these inside the
    /// cell rather than filling it, so they come out noticeably smaller than
    /// the text beside them. `unicode` is the larger-looking set.
    pub const NERD: Self = Self {
        prev: "\u{f04ae}",
        play: "\u{f040a}",
        pause: "\u{f03e4}",
        stop: "\u{f04db}",
        next: "\u{f04ad}",
    };

    /// Characters a monospace font is certain to draw itself, at the size it
    /// draws its letters.
    ///
    /// This exists because the sets above are not reliably that size. A
    /// terminal font that lacks U+25B6, U+25AE and their neighbours falls back
    /// to another font for them, and the substitute is drawn to its own
    /// metrics -- which is why the controls came out visibly smaller than the
    /// `SHUF` label beside them however large a codepoint was chosen. ASCII
    /// and the block elements are in every monospace font, so they are drawn
    /// by the same font at the same scale as the text.
    pub const BLOCK: Self = Self {
        prev: "<<",
        play: ">",
        pause: "\u{258c}\u{258c}",
        stop: "\u{2588}\u{2588}",
        next: ">>",
    };

    /// For terminals and fonts that can manage neither.
    pub const ASCII: Self = Self {
        prev: "|<",
        play: ">",
        pause: "||",
        stop: "[]",
        next: ">|",
    };

    /// The play face, for a marker column a single cell wide.
    pub fn play_mark(&self) -> &'static str {
        self.play
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "unicode" | "auto" => Self::UNICODE,
            "nerd" | "nerdfont" | "nerd-font" => Self::NERD,
            "block" | "big" => Self::BLOCK,
            "ascii" | "plain" => Self::ASCII,
            _ => return None,
        })
    }

    /// Every set, for tests that must hold across all of them.
    pub const ALL: [Self; 4] = [Self::UNICODE, Self::BLOCK, Self::NERD, Self::ASCII];

    pub fn faces(&self) -> [&'static str; 5] {
        [self.prev, self.play, self.pause, self.stop, self.next]
    }

    /// Cells each button occupies.
    ///
    /// Derived from the set's own widest face, so the padding is the same in
    /// every set and the plate's parity follows the face's: `nerd`, whose
    /// faces are one cell, gets an odd plate and centres them exactly, where a
    /// fixed even width would have left every one of them half a cell out.
    pub fn button_width(&self) -> u16 {
        self.face_width_max() + BUTTON_PAD * 2
    }

    /// Rows each button occupies.
    pub fn button_height(&self) -> u16 {
        BUTTON_H
    }

    fn face_width_max(&self) -> u16 {
        self.faces()
            .iter()
            .map(|f| face_width(f))
            .max()
            .unwrap_or(1)
    }
}

impl Default for Glyphs {
    fn default() -> Self {
        Self::UNICODE
    }
}

const SHUFFLE: &str = "SHUF";

/// Blank cells either side of a button's face.
///
/// The buttons are drawn as plates rather than bare glyphs, because how large
/// a glyph looks is the font's decision and not one a terminal program can
/// override -- a font without the shape substitutes another font's, at that
/// font's size. The plate is the one dimension staramp does control.
const BUTTON_PAD: u16 = 1;

/// The plate's width for a two-cell face, in cells.
///
/// A cell is about twice as tall as it is wide -- 8 by 17 pixels in the font
/// this was measured in -- so 4 cells is 32 pixels, and the plate is 34 tall
/// (see [`BUTTON_H`]): square to within a pixel. Not the width itself, which
/// [`Glyphs::button_width`] derives; this is what that comes to for the
/// default set, and what the squareness rests on.
const BUTTON_W: u16 = 4;

/// The plate's height, in rows.
///
/// Three rows occupied, but nothing like three rows tall: the face has the
/// middle one to itself and the outer two are only fractionally painted --
/// see [`PLATE_EDGE`].
///
/// It has to be odd. A face is one row and can only be centred in an odd
/// number of them, so the alternative was a two-row plate with every glyph
/// half a row out.
const BUTTON_H: u16 = 3;

/// The cell proportions the plate is squared against.
///
/// A program cannot ask what size its terminal draws a cell, so this is the
/// common case: about 8 by 17 pixels, a little over one to two. Fonts vary by
/// a few percent either side of it and the plate is square across that range,
/// which is as good as this can be made from inside.
const CELL_W_PX: u16 = 8;
const CELL_H_PX: u16 = 17;

/// Eighths of each outer row to paint, so a plate `cells` wide comes out
/// square.
///
/// This is the plate's real height control, and the reason its height does not
/// follow from its row count. The block elements divide a cell into eighths,
/// so the plate can stand at any of `CELL_H + 2 x (CELL_H x n/8)` pixels
/// rather than only at whole rows -- and the row count merely has to be odd,
/// so the face has a middle row to sit on.
///
/// Solving that for a height equal to the width gives
/// `n = (cells x CELL_W - CELL_H) x 4 / CELL_H`. Three cells wants a quarter
/// each side: 24 by 25.5. Whole rows would have been 51, and halves 34.
const fn plate_edge(cells: u16) -> u16 {
    let target = cells * CELL_W_PX;
    if target <= CELL_H_PX {
        return 0;
    }
    // Rounded, not truncated: a plate a sixteenth too short beats one an
    // eighth too tall.
    let n = ((target - CELL_H_PX) * 4 + CELL_H_PX / 2) / CELL_H_PX;
    if n > 8 {
        8
    } else {
        n
    }
}

/// Lower *n* eighths of a cell, indexed by `n`.
///
/// The top of the plate is drawn with one of these -- plate colour over the
/// panel -- and the bottom is *cut out* with its complement, panel colour over
/// a plate background. Two directions because the block elements are not
/// symmetric: every eighth exists downwards, but upwards there is only a half
/// and a one-eighth, so the upper edge has to be the part that is left unpainted.
const LOWER_EIGHTHS: [&str; 9] = [
    " ", "\u{2581}", "\u{2582}", "\u{2583}", "\u{2584}", "\u{2585}", "\u{2586}", "\u{2587}",
    "\u{2588}",
];

/// Cells a label occupies.
///
/// Display width, not a character count: a face may carry a variation
/// selector, which is a character that takes no space.
fn face_width(s: &str) -> u16 {
    use unicode_width::UnicodeWidthStr;
    s.width() as u16
}

/// The panel's interactive geometry.
///
/// A pure function of the panel rect and the two values whose *width* moves
/// things around: the clock labels either side of the seek bar, and the repeat
/// button's label. The renderer places things with it and the mouse handler
/// hit-tests with it, so the two cannot drift -- which is the whole reason it
/// exists rather than the arithmetic living inline in `render`.
pub struct Geometry {
    pub inner: Rect,
    pub clock: Rect,
    pub visualizer: Rect,
    pub title: Rect,
    pub tech: Rect,
    pub seek_row: Rect,
    /// The seek bar's track, between the two clock labels.
    pub seek: Option<Rect>,
    pub controls: Controls,
}

/// The transport row.
pub struct Controls {
    pub row: Rect,
    pub prev: Rect,
    pub play: Rect,
    pub pause: Rect,
    pub stop: Rect,
    pub next: Rect,
    pub shuffle: Rect,
    pub repeat: Rect,
    /// The slider cells only, without the `VOL` label or the percentage.
    pub volume: Option<Rect>,
}

/// `None` when the panel is too short to have a body at all.
pub fn geometry(
    area: Rect,
    position: f64,
    duration: f64,
    repeat: RepeatMode,
    glyphs: Glyphs,
) -> Option<Geometry> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if inner.height < BODY_ROWS {
        return None;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(ANALYZER_ROWS), // clock and analyzer
            Constraint::Length(1),             // title
            Constraint::Length(1),             // album and format
            Constraint::Length(1),             // seek bar
            // The transport. Its plates are three rows and the face sits on
            // the middle one, so the blank row that used to separate this from
            // the seek bar is now the top of the buttons themselves.
            Constraint::Length(BUTTON_H),
        ])
        .split(inner);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(10)])
        .split(rows[0]);

    Some(Geometry {
        inner,
        clock: top[0],
        visualizer: top[1],
        title: rows[1],
        tech: rows[2],
        seek_row: rows[3],
        seek: seek_track(rows[3], position, duration),
        controls: controls(rows[4], repeat, glyphs),
    })
}

/// The clock labels shown either side of the seek bar.
fn seek_labels(position: f64, duration: f64) -> (String, String) {
    (
        digits::clock_padded(position),
        if duration > 0.0 {
            digits::clock_padded(duration)
        } else {
            "--:--".into()
        },
    )
}

fn seek_track(row: Rect, position: f64, duration: f64) -> Option<Rect> {
    if row.width < 16 {
        return None;
    }
    let (left, right) = seek_labels(position, duration);
    let (lw, rw) = (left.len() as u16, right.len() as u16);
    // Two blank columns each side rather than one: the inner pair carries the
    // end caps a style may have, and the outer pair keeps the bar from
    // crowding the clocks.
    let width = row.width.saturating_sub(lw + rw + SEEK_MARGIN * 2 + 2);
    (width > 0).then(|| Rect {
        x: row.x + lw + SEEK_MARGIN + 1,
        y: row.y,
        width,
        height: 1,
    })
}

/// Blank columns between a clock label and the bar.
const SEEK_MARGIN: u16 = 3;

/// Rows the analyzer gets.
///
/// Four rather than three because each carries two colours -- a cell's
/// foreground and its background -- so this is what sets how many steps of the
/// theme's sixteen-step ramp a bar can actually show. Four rows is eight.
const ANALYZER_ROWS: u16 = 4;

/// Rows the panel body needs: the analyzer's, four single rows, and one blank
/// between the seek bar and the transport.
pub const BODY_ROWS: u16 = ANALYZER_ROWS + 3 + BUTTON_H;

/// The whole panel, border included.
pub const PANEL_ROWS: u16 = BODY_ROWS + 2;

pub fn repeat_label(repeat: RepeatMode) -> &'static str {
    match repeat {
        RepeatMode::Off => "REP",
        RepeatMode::All => "REP:ALL",
        RepeatMode::One => "REP:1",
    }
}

/// Lay the transport row out left to right, exactly as `render_controls` draws
/// it: each button takes its label's width and is followed by one space, and a
/// button that would not fit takes no room at all.
fn controls(row: Rect, repeat: RepeatMode, glyphs: Glyphs) -> Controls {
    // The volume slider is right-aligned and laid out first, because the
    // transport buttons have to stop short of it. They used to be drawn under
    // it and then painted over at narrow widths, which left a click landing on
    // a button nobody could see.
    // Everything that is not a plate is a single row, and sits on the same
    // row the faces do -- the middle of the three -- so the transport reads as
    // one line of controls rather than a row of buttons with labels adrift
    // above them.
    let mid = row.y + row.height / 2;

    let vol_w = VOLUME_WIDTH;
    let volume = (row.width > vol_w + 8).then(|| Rect {
        x: row.x + row.width - vol_w - 6 + 4,
        y: mid,
        width: vol_w - 4,
        height: 1,
    });

    let right = match volume {
        // `VOL ` sits four cells left of the slider.
        Some(v) => v.x - 4,
        None => row.x + row.width,
    };
    let mut x = row.x + 1;
    // A button that would not fit takes no room, matching how the renderer
    // skips it -- so the two stay in step at any panel width.
    let take = |x: &mut u16, w: u16, h: u16| -> Rect {
        let y = if h == 1 { mid } else { row.y };
        if *x + w > right {
            return Rect::new((*x).min(right), y, 0, h);
        }
        let r = Rect::new(*x, y, w, h);
        *x += w + 1;
        r
    };

    // Widths come from the faces themselves, so a change of glyph cannot
    // leave the hit rects pointing at where a button used to be.
    let w = glyphs.button_width();
    let h = glyphs.button_height();
    let prev = take(&mut x, w, h);
    let play = take(&mut x, w, h);
    let pause = take(&mut x, w, h);
    let stop = take(&mut x, w, h);
    let next = take(&mut x, w, h);
    x += 1;
    let shuffle = take(&mut x, face_width(SHUFFLE), 1);
    let rep = take(&mut x, face_width(repeat_label(repeat)), 1);

    Controls {
        row,
        prev,
        play,
        pause,
        stop,
        next,
        shuffle,
        repeat: rep,
        volume,
    }
}

/// Total cells the volume control occupies, label and readout included.
const VOLUME_WIDTH: u16 = 12;

impl<'a> Widget for PlayerView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let border = if self.focused {
            t.border_focused
        } else {
            t.border
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(border)))
            .title(Span::styled(
                // Letter-spaced, with the slash spaced along with the rest:
                // pulling it tight against its neighbours would make the seam
                // read as a typo in one word rather than the join between two.
                if self.mirroring {
                    "\u{2550} S T A R / A M P \u{b7} mirror "
                } else {
                    "\u{2550} S T A R / A M P "
                },
                Style::default()
                    .fg(rgb(t.titlebar_active_fg))
                    .add_modifier(Modifier::BOLD),
            ))
            // Only claim anything once audio is actually flowing. A
            // "bit-perfect" badge that is really just a default is worse than
            // no badge at all.
            .title_top(
                Line::from(match self.state {
                    PlayState::Stopped => Span::styled("", Style::default()),
                    _ if self.bit_perfect => Span::styled(
                        concat!(" bit-perfect", " \u{2550}"),
                        Style::default().fg(rgb(t.ok)),
                    ),
                    _ => Span::styled(
                        concat!(" resampled", " \u{2550}"),
                        Style::default().fg(rgb(t.warn)),
                    ),
                })
                .right_aligned(),
            )
            .style(Style::default().bg(rgb(t.bg)));

        block.render(area, buf);
        // Corners, but no close mark: closing the transport would leave
        // nothing to play with.
        super::frame::render_corners(area, buf, t);
        let Some(g) = geometry(area, self.position, self.duration, self.repeat, self.glyphs) else {
            return;
        };

        // ---- clock and analyzer ----
        let shown = if self.state == PlayState::Stopped {
            "-:--".to_string()
        } else {
            digits::clock(self.position)
        };
        let art = digits::render(&shown);
        // Sat on the analyzer's floor rather than its ceiling: the bars grow
        // from the bottom, and the digits are three rows in a taller box.
        let clock_top = g.clock.y + g.clock.height.saturating_sub(art.len() as u16);
        for (i, line) in art.iter().enumerate() {
            buf.set_string(
                g.clock.x + 1,
                clock_top + i as u16,
                line,
                Style::default().fg(rgb(t.time_digit_fg)).bg(rgb(t.bg)),
            );
        }

        super::visualizer::render(
            g.visualizer,
            buf,
            t,
            self.vis_mode,
            &super::visualizer::Frame {
                bands: self.bands,
                peaks: self.peaks,
                wave: self.wave,
            },
            self.bars,
        );

        // ---- track title, with a marquee for long ones ----
        let width = g.title.width.saturating_sub(2) as usize;
        let title = marquee(&self.title, width, self.marquee_offset);
        let title_colour = match self.state {
            PlayState::Playing => t.marquee_fg,
            PlayState::Paused => t.marquee_paused_fg,
            PlayState::Stopped => t.marquee_stopped_fg,
        };
        buf.set_string(
            g.title.x + 1,
            g.title.y,
            &title,
            Style::default()
                .fg(rgb(title_colour))
                .add_modifier(Modifier::BOLD),
        );

        // ---- album / technical line ----
        //
        // The tech string is right-aligned and takes its width first, with the
        // album given whatever is left. Concatenating the two and truncating
        // the result loses the format and bitrate on any long album title,
        // which is exactly when they are still worth knowing.
        {
            use unicode_width::UnicodeWidthStr;
            let row = g.tech;
            let width = row.width.saturating_sub(2) as usize;
            let tech = truncate(&self.tech, width);
            let tech_w = tech.width();
            let style = Style::default().fg(rgb(t.row_meta_fg));

            // Two spaces of gap, so the album never abuts the tech string.
            let sub_w = width.saturating_sub(tech_w + 2);
            if sub_w > 0 {
                buf.set_string(row.x + 1, row.y, truncate(&self.subtitle, sub_w), style);
            }
            if tech_w > 0 {
                let x = row.x + 1 + (width - tech_w) as u16;
                buf.set_string(x, row.y, &tech, style);
            }
        }

        // ---- seek bar ----
        render_seek(
            &g,
            buf,
            t,
            self.position,
            self.duration,
            self.seek_phase,
            self.seek_style,
        );

        // ---- transport, toggles, volume ----
        render_controls(
            &g.controls,
            buf,
            t,
            self.state,
            self.volume,
            self.shuffle,
            self.repeat,
            self.underruns,
            self.glyphs,
        );
    }
}

/// How the seek bar is drawn.
///
/// Selectable for the same reason the transport faces are: the bar is made of
/// characters, and which characters a font draws well is not something a
/// terminal program gets to decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekStyle {
    pub fill: SeekFill,
    /// Drawn in the blank column either side of the bar, if the style has any.
    pub caps: Option<(char, char)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFill {
    /// A horizontal rule, drawn with the given character.
    ///
    /// No glyph is both part-height and part-width, so the boundary cell is
    /// *shaded* between the groove and the fill instead of part-drawn. That
    /// reads as a smooth edge and costs no height, where the eighth-width
    /// blocks buy their smoothness by filling the whole cell.
    ///
    /// Box drawing sits on the cell's middle, where the clock digits either
    /// side of the bar have their weight, so a rule lines up with them. A
    /// block element would not: those anchor to a cell edge.
    Rule(char),
    /// Full-height, filling by eighths of a cell.
    Blocks,
    /// One character per cell, so this one steps whole cells.
    Ansi,
}

/// The shade ramp, lightest first.
///
/// The four characters every ANSI art of the era was built from. Two colours
/// per cell is all a terminal offers, so density is the only gradient
/// available -- which is exactly why the artists used them.
const SHADES: [char; 4] = ['\u{2591}', '\u{2592}', '\u{2593}', '\u{2588}'];

/// How dense the fill is at `along`, 0 to 1 across the bar.
///
/// Never the lightest shade: that one belongs to the track, and a filled cell
/// drawn with it is a filled cell nobody can see. The gradient therefore runs
/// across the three heavier shades.
fn fill_shade(along: f32) -> usize {
    let span = SHADES.len() - 2;
    1 + ((along * span as f32).round() as usize).min(span)
}

/// The rules a bar can be drawn with, thinnest first.
///
/// Heavy is a single stroke; double is two, which reads as roughly twice the
/// weight while staying on the cell's middle. There is nothing between them
/// and nothing heavier: a cell offers no thicker centred horizontal, so above
/// this the only step is [`SeekFill::Blocks`], which fills the cell.
const RULE_HEAVY: char = '\u{2501}';
const RULE_DOUBLE: char = '\u{2550}';

/// Eighth-width blocks, index 0 empty to 8 full.
const EIGHTHS_WIDE: [char; 9] = [
    ' ', '\u{258f}', '\u{258e}', '\u{258d}', '\u{258c}', '\u{258b}', '\u{258a}', '\u{2589}',
    '\u{2588}',
];

impl SeekStyle {
    /// A single stroke: the lightest bar.
    pub const THIN: Self = Self {
        fill: SeekFill::Rule(RULE_HEAVY),
        caps: None,
    };

    /// Twice the weight of [`Self::THIN`] and still centred, so there is
    /// enough of it to see the highlight move along.
    pub const BAR: Self = Self {
        fill: SeekFill::Rule(RULE_DOUBLE),
        caps: None,
    };

    /// Full height, for anyone who wants the heavier bar.
    pub const BLOCKS: Self = Self {
        fill: SeekFill::Blocks,
        caps: None,
    };

    pub const ANSI: Self = Self {
        fill: SeekFill::Ansi,
        caps: Some(('[', ']')),
    };

    pub const ALL: [Self; 4] = [Self::THIN, Self::BAR, Self::BLOCKS, Self::ANSI];

    pub fn name(&self) -> &'static str {
        match self.fill {
            SeekFill::Rule(RULE_DOUBLE) => "bar",
            SeekFill::Rule(_) => "thin",
            SeekFill::Blocks => "blocks",
            SeekFill::Ansi => "ascii",
        }
    }

    /// The next style in [`Self::ALL`], wrapping.
    pub fn next(&self) -> Self {
        let i = Self::ALL.iter().position(|s| s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            // `nerd` was a half-height bar closed with a patched font's
            // rounded caps. Those caps are a full cell tall, so beside a
            // half-height bar they sat above it as two loose marks rather than
            // closing anything. Kept as an alias so the setting still loads.
            "thin" => Self::THIN,
            "bar" | "nerd" | "nerdfont" | "nerd-font" => Self::BAR,
            "blocks" | "block" => Self::BLOCKS,
            "ansi" | "ascii" | "plain" | "auto" => Self::ANSI,
            _ => return None,
        })
    }

    /// The groove character, drawn in the track colour.
    fn groove(&self) -> char {
        match self.fill {
            SeekFill::Rule(c) => c,
            SeekFill::Blocks => ' ',
            SeekFill::Ansi => '-',
        }
    }

    /// The character for a cell that is entirely played.
    ///
    /// The same as [`Self::groove`] for the thin style, which distinguishes
    /// the two by colour rather than by shape.
    fn full(&self) -> char {
        match self.fill {
            SeekFill::Rule(c) => c,
            SeekFill::Blocks => EIGHTHS_WIDE[8],
            SeekFill::Ansi => '=',
        }
    }
}

impl Default for SeekStyle {
    fn default() -> Self {
        Self::ANSI
    }
}

/// Seconds for the highlight to travel the bar once.
const SHEEN_PERIOD: f32 = 3.5;

/// How much of the bar the highlight covers.
const SHEEN_WIDTH: f32 = 0.26;

fn render_seek(
    g: &Geometry,
    buf: &mut Buffer,
    t: &Theme,
    pos: f64,
    dur: f64,
    phase: f32,
    style: SeekStyle,
) {
    let area = g.seek_row;
    let Some(bar) = g.seek else { return };
    let (left, right) = seek_labels(pos, dur);
    let label = Style::default().fg(rgb(t.seek_label_fg));

    buf.set_string(area.x + 1, area.y, &left, label);
    buf.set_string(
        area.x + area.width - right.len() as u16 - 1,
        area.y,
        &right,
        label,
    );

    let frac = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Position in eighths of a cell, so the boundary can be part of one.
    let eighths = (frac * bar.width as f64 * 8.0).round() as u32;
    let track = t.seek_track_fg;
    let bg = rgb(t.bg);

    if let Some((l, r)) = style.caps {
        // The clocks' colour, not the groove's: the caps mark where the bar
        // begins and ends, which is information, and in the groove colour they
        // read as the dimmest thing on the row.
        let cap = Style::default().fg(rgb(t.seek_label_fg)).bg(bg);
        if bar.x > area.x {
            buf[(bar.x - 1, bar.y)].set_char(l).set_style(cap);
        }
        if bar.x + bar.width < area.x + area.width {
            buf[(bar.x + bar.width, bar.y)].set_char(r).set_style(cap);
        }
    }

    for i in 0..bar.width {
        let cell = (eighths as i64 - i as i64 * 8).clamp(0, 8) as usize;
        let along = i as f32 / bar.width.max(1) as f32;

        // The played part runs from the fill colour down into the background,
        // so the bar has depth without a bright edge chasing the playhead.
        let lit = sheen(along, frac as f32, phase);
        let mut played = t.seek_filled_fg.mix(t.bg, (along * 0.6) as f64);
        if let Some(l) = lit {
            // Toward the thumb colour, which is the brightest the theme has,
            // rather than back to the fill: crossing a bar that darkens as it
            // goes, a highlight that only restores the fill colour is a
            // change nobody notices.
            played = played.mix(t.seek_thumb_fg, l as f64);
        }

        let (ch, fg, cell_bg) = match style.fill {
            // Nothing is both part-height and part-width, so the edge is
            // shaded between groove and fill rather than part-drawn.
            SeekFill::Rule(c) => {
                let blend = cell as f64 / 8.0;
                (c, track.mix(played, blend), bg)
            }
            SeekFill::Blocks if cell == 0 => (' ', t.bg, rgb(track)),
            SeekFill::Blocks => (EIGHTHS_WIDE[cell], played, rgb(track)),
            SeekFill::Ansi => {
                if cell == 0 {
                    ('-', track, bg)
                } else {
                    ('=', played, bg)
                }
            }
        };
        buf[(bar.x + i, bar.y)]
            .set_char(ch)
            .set_style(Style::default().fg(rgb(fg)).bg(cell_bg));
    }
}

/// How brightly the travelling highlight lands at `along`, if at all.
///
/// It sweeps only the played portion and fades at both ends of its own width,
/// so it reads as a sheen crossing the bar rather than a block sliding along
/// it. `None` when this position is untouched, which keeps the common case
/// free of arithmetic.
fn sheen(along: f32, played: f32, phase: f32) -> Option<f32> {
    if phase <= 0.0 || played <= 0.0 {
        return None;
    }
    // Never past the playhead. The sweep runs a little beyond the played part
    // so it can leave cleanly, which means its trailing edge would otherwise
    // reach into the groove. No renderer draws it there today -- they all
    // decide the groove before asking -- but a function that says a groove
    // cell is lit is a trap for the next one that does.
    if along > played {
        return None;
    }
    // The highlight travels the played part, entering and leaving cleanly.
    let centre = phase * (played + SHEEN_WIDTH * 2.0) - SHEEN_WIDTH;
    let d = (along - centre).abs() / SHEEN_WIDTH;
    (d < 1.0).then(|| {
        let f = 1.0 - d;
        // Squared, so the edges fall away rather than ending in a line, and
        // taken to the full range at the peak: a highlight worth having is one
        // that reaches the brightest the theme allows.
        f * f
    })
}

#[allow(clippy::too_many_arguments)]
fn render_controls(
    c: &Controls,
    buf: &mut Buffer,
    t: &Theme,
    state: PlayState,
    volume: f32,
    shuffle: bool,
    repeat: RepeatMode,
    underruns: u64,
    glyphs: Glyphs,
) {
    let area = c.row;
    let active = Style::default().fg(rgb(t.transport_button_active_fg));
    let idle = Style::default().fg(rgb(t.transport_button_fg));
    let on = Style::default().fg(rgb(t.transport_toggle_on_fg));
    let off = Style::default().fg(rgb(t.transport_toggle_off_fg));

    // Each button draws into the rect the layout gave it, so what the mouse
    // hits and what the eye sees are the same cells by construction.
    // Each button is a plate: the face centred on a filled background, so it
    // reads as a control at whatever size the font draws the glyph.
    let put = |r: Rect, s: &str, fg: Style, lit: bool, buf: &mut Buffer| {
        if r.width == 0 {
            return;
        }
        let plate = if lit {
            rgb(t.transport_button_active_bg)
        } else {
            rgb(t.transport_button_bg)
        };
        // The middle row is solid; the outer two are half blocks facing it,
        // so the plate stands two rows tall while the face still has a whole
        // row of its own to be centred on. Their *background* is the panel's,
        // not the plate's -- that is the half that is meant to disappear.
        let mid = r.y + r.height / 2;
        let behind = rgb(t.bg);
        for dy in 0..r.height {
            let y = r.y + dy;
            let edge = plate_edge(r.width) as usize;
            let (ch, style) = if y == mid {
                (" ", Style::default().bg(plate))
            } else if y < mid {
                (LOWER_EIGHTHS[edge], Style::default().fg(plate).bg(behind))
            } else {
                // Cut out rather than drawn -- see `LOWER_EIGHTHS`.
                (
                    LOWER_EIGHTHS[8 - edge],
                    Style::default().fg(behind).bg(plate),
                )
            };
            for dx in 0..r.width {
                buf[(r.x + dx, y)].set_symbol(ch).set_style(style);
            }
        }
        // Centred both ways: the plate's middle row, and the face's own width
        // -- which is its ink, since a face carries no padding -- centred
        // across the plate's.
        let inset = (r.width.saturating_sub(face_width(s))) / 2;
        buf.set_string(r.x + inset, mid, s, fg.bg(plate));
    };

    let playing = state == PlayState::Playing;
    let paused = state == PlayState::Paused;
    let stopped = state == PlayState::Stopped;
    put(c.prev, glyphs.prev, idle, false, buf);
    put(
        c.play,
        glyphs.play,
        if playing { active } else { idle },
        playing,
        buf,
    );
    put(
        c.pause,
        glyphs.pause,
        if paused { active } else { idle },
        paused,
        buf,
    );
    put(
        c.stop,
        glyphs.stop,
        if stopped { active } else { idle },
        stopped,
        buf,
    );
    put(c.next, glyphs.next, idle, false, buf);

    // The toggles are labels, not buttons, so they take no plate.
    let label = |r: Rect, s: &str, st: Style, buf: &mut Buffer| {
        if r.width > 0 {
            buf.set_string(r.x, r.y, s, st);
        }
    };
    label(c.shuffle, SHUFFLE, if shuffle { on } else { off }, buf);
    label(
        c.repeat,
        repeat_label(repeat),
        if repeat == RepeatMode::Off { off } else { on },
        buf,
    );

    // Underruns are shown rather than swallowed: silent dropouts are how a
    // player earns a reputation for crackling.
    if underruns > 0 {
        // Named for what it sounds like rather than for what ALSA calls it.
        // `xrun` is the driver's word; what the listener heard was a gap.
        let msg = if underruns == 1 {
            "1 dropout".to_string()
        } else {
            format!("{underruns} dropouts")
        };
        let w = msg.chars().count() as u16;
        if area.width > w + 24 {
            buf.set_string(
                area.x + area.width - w - 22,
                area.y,
                &msg,
                Style::default().fg(rgb(t.error)),
            );
        }
    }

    // Volume, right-aligned.
    if let Some(v) = c.volume {
        buf.set_string(v.x - 4, v.y, "VOL ", Style::default().fg(rgb(t.dim)));
        // Quarter-cell precision from the shade ramp, on top of the ramp's own
        // gradient: the leading cell holds a lighter shade than its position
        // calls for when the level falls between two cells.
        let steps = SHADES.len() as i32;
        let filled = (volume * v.width as f32 * steps as f32).round() as i32;
        for i in 0..v.width {
            let along = i as f32 / (v.width - 1).max(1) as f32;
            let step = (filled - i as i32 * steps).clamp(0, steps);
            let (ch, colour) = if step == 0 {
                (SHADES[0], t.volume_track_fg)
            } else {
                // Density by position, so the bar reads as a ramp rather than
                // a block -- the shading gradient a BBS would have drawn, and
                // the only gradient two colours to a cell can carry. A cell
                // only partly reached is held below its position's density.
                // Capped by how much of this cell the level has reached, so
                // the leading cell steps up through the shades as it fills.
                let reached = fill_shade(along);
                let ch = SHADES[reached.min(step as usize).max(1)];
                let colour = t
                    .volume_filled_fg
                    .mix(t.volume_thumb_fg, (along * 0.5) as f64);
                (ch, colour)
            };
            buf[(v.x + i, v.y)]
                .set_char(ch)
                .set_style(Style::default().fg(rgb(colour)));
        }
        buf.set_string(
            v.x + v.width,
            v.y,
            format!("{:>3}", (volume * 100.0).round() as u32),
            Style::default().fg(rgb(t.dim)),
        );
    }
}

/// Scroll a string that does not fit, looping with a separator.
pub fn marquee(s: &str, width: usize, offset: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_string();
    }
    let padded = format!("{s}   ***   ");
    let chars: Vec<char> = padded.chars().collect();
    let start = offset % chars.len();
    chars.iter().cycle().skip(start).take(width).collect()
}

/// Truncate to a display width, with an ellipsis.
pub fn truncate(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

pub fn paragraph_placeholder<'a>(text: &'a str, t: &Theme) -> Paragraph<'a> {
    Paragraph::new(text).style(Style::default().fg(rgb(t.empty_fg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render the panel and return its rows as plain text.
    fn draw(subtitle: &str, tech: &str, width: u16) -> Vec<String> {
        use crate::theme::builtin;
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, width, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let empty: [f32; 0] = [];
        PlayerView {
            theme: &theme,
            title: "Nova Era".into(),
            subtitle: subtitle.into(),
            tech: tech.into(),
            state: PlayState::Playing,
            position: 30.0,
            duration: 300.0,
            volume: 0.8,
            shuffle: false,
            repeat: crate::playlist::queue::RepeatMode::Off,
            bit_perfect: true,
            focused: true,
            mirroring: false,
            marquee_offset: 0,
            bands: &empty,
            peaks: &empty,
            wave: &empty,
            vis_mode: crate::vis::mode::VisMode::Off,
            bars: crate::ui::panels::visualizer::BarLayout::default(),
            glyphs: Glyphs::default(),
            seek_phase: 0.0,
            seek_style: SeekStyle::default(),
            underruns: 0,
        }
        .render(area, &mut buf);
        (0..area.height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn face_width_measures_columns_rather_than_counting_characters() {
        // A face may carry a zero-width character. Counting would reserve a
        // column too many and shift every button after it out of its hit rect.
        assert_eq!(face_width("\u{25b6}\u{fe0e} "), 2, "VS15 takes no column");
        assert_eq!(face_width("ab"), 2);
    }

    /// Draw just the seek bar at a given fraction and return its cells.
    fn seek_bar(frac: f64, width: u16, phase: f32, style: SeekStyle) -> Vec<(String, Color)> {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, width, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(
            area,
            frac * 300.0,
            300.0,
            RepeatMode::Off,
            Glyphs::default(),
        )
        .expect("panel has a body");
        render_seek(&g, &mut buf, &theme, frac * 300.0, 300.0, phase, style);
        let bar = g.seek.expect("bar fits");
        (0..bar.width)
            .map(|i| {
                let c = &buf[(bar.x + i, bar.y)];
                (c.symbol().to_string(), c.style().fg.unwrap())
            })
            .collect()
    }

    fn seek_cells_styled(frac: f64, width: u16, phase: f32, style: SeekStyle) -> String {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, width, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(
            area,
            frac * 300.0,
            300.0,
            RepeatMode::Off,
            Glyphs::default(),
        )
        .expect("panel has a body");
        render_seek(&g, &mut buf, &theme, frac * 300.0, 300.0, phase, style);
        let bar = g.seek.expect("bar fits");
        (0..bar.width)
            .map(|i| buf[(bar.x + i, bar.y)].symbol().to_string())
            .collect()
    }

    /// Cells of the bar that are not plain groove, by colour or by glyph.
    fn seek_played(frac: f64, style: SeekStyle) -> usize {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(
            area,
            frac * 300.0,
            300.0,
            RepeatMode::Off,
            Glyphs::default(),
        )
        .expect("panel has a body");
        render_seek(&g, &mut buf, &theme, frac * 300.0, 300.0, 0.0, style);
        let bar = g.seek.expect("bar fits");
        // The thin style shades rather than swapping glyphs, so "played" is a
        // question about colour, not about which character is there.
        let groove = rgb(theme.seek_track_fg);
        (0..bar.width)
            .filter(|i| {
                let c = &buf[(bar.x + i, bar.y)];
                c.style().fg != Some(groove) && c.symbol() != "-" && c.symbol() != " "
            })
            .count()
    }

    #[test]
    fn every_seek_style_draws_a_bar_that_grows() {
        for style in SeekStyle::ALL {
            assert_eq!(seek_played(0.0, style), 0, "{style:?} drew fill at zero");
            assert!(
                seek_played(0.5, style) > seek_played(0.25, style),
                "{style:?} did not grow"
            );
            assert!(
                seek_played(1.0, style) > seek_played(0.5, style),
                "{style:?} did not reach the end"
            );
        }
    }

    #[test]
    fn only_the_block_style_draws_a_partly_filled_cell() {
        // What the sub-cell fill buys: a boundary cell that is part full,
        // rather than a cell that snaps on whole as soon as any of it is
        // played. ASCII cannot do it -- that is the stated cost of the style,
        // and worth holding so it is not mistaken for a bug.
        // Swept across fractions rather than fixed at one: a fraction can
        // land exactly on a cell boundary, where even a sub-cell style has
        // nothing part-filled to draw.
        let partial = |style: SeekStyle| {
            // A prime number of samples, so they do not all land on cell
            // boundaries the way a round divisor of the bar width would.
            (0..37).any(|i| {
                seek_cells_styled(i as f64 / 37.0, 60, 0.0, style)
                    .chars()
                    .any(|c| c != style.groove() && c != style.full())
            })
        };
        assert!(partial(SeekStyle::BLOCKS), "no partial cell in blocks");
        assert!(!partial(SeekStyle::ANSI), "ascii has no partial forms");
    }

    /// The volume slider's cells at a given level.
    fn volume_cells(level: f32) -> String {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 80, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(area, 0.0, 300.0, RepeatMode::Off, Glyphs::default()).unwrap();
        render_controls(
            &g.controls,
            &mut buf,
            &theme,
            PlayState::Playing,
            level,
            false,
            RepeatMode::Off,
            0,
            Glyphs::default(),
        );
        let v = g.controls.volume.unwrap();
        (0..v.width)
            .map(|i| buf[(v.x + i, v.y)].symbol().to_string())
            .collect()
    }

    /// Not an assertion -- run with `--nocapture` to look at the ramp.
    #[test]
    fn preview_the_volume_ramp() {
        for step in 0..=8 {
            let level = step as f32 / 8.0;
            println!("{:>3}%  {}", (level * 100.0) as u32, volume_cells(level));
        }
    }

    #[test]
    fn the_volume_is_the_accent_rather_than_a_status_colour() {
        // It is a control, not a state: green would read as "ok", which is a
        // thing volume never reports. Nor the border's colour -- the frame is
        // deliberately dim, and a control has to be readable.
        for name in ["cosmic", "catppuccin-mocha", "nord", "gruvbox-dark"] {
            let theme = crate::theme::builtin::load(name).unwrap();
            assert_eq!(
                theme.volume_filled_fg, theme.accent,
                "{name}: the volume is not the accent"
            );
            assert_ne!(theme.volume_filled_fg, theme.border_focused);
        }
    }

    #[test]
    fn the_volume_ramp_gets_denser_toward_full() {
        // The 1990s shading gradient: two colours per cell is all a terminal
        // offers, so density is the gradient.
        let full = volume_cells(1.0);
        let weight = |c: char| SHADES.iter().position(|s| *s == c).unwrap_or(0);
        let first = weight(full.chars().next().unwrap());
        let last = weight(full.chars().last().unwrap());
        assert!(last > first, "the ramp does not build: {full:?}");
        // Monotonic: no cell lighter than the one before it.
        let mut prev = 0;
        for c in full.chars() {
            let w = weight(c);
            assert!(w >= prev, "the ramp dips: {full:?}");
            prev = w;
        }
    }

    #[test]
    fn an_unset_volume_cell_is_the_lightest_shade() {
        // The same dithered block throughout, so the unfilled part reads as
        // the track rather than as a different widget.
        let quiet = volume_cells(0.0);
        assert!(
            quiet.chars().all(|c| c == SHADES[0]),
            "silence is not all track: {quiet:?}"
        );
    }

    #[test]
    fn the_volume_resolves_below_a_whole_cell() {
        // Four shades per cell, so an eight-cell slider carries 32 steps.
        assert_ne!(
            volume_cells(0.50),
            volume_cells(0.53),
            "a sub-cell change drew nothing new"
        );
    }

    #[test]
    fn the_caps_are_brighter_than_the_groove() {
        // They say where the bar starts and stops, which is worth reading; in
        // the groove colour they were the dimmest thing on the row.
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 80, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(area, 0.0, 300.0, RepeatMode::Off, Glyphs::default()).unwrap();
        render_seek(&g, &mut buf, &theme, 0.0, 300.0, 0.0, SeekStyle::ANSI);
        let bar = g.seek.unwrap();
        let cap = buf[(bar.x - 1, bar.y)].style().fg.unwrap();
        assert_ne!(cap, rgb(theme.seek_track_fg), "the caps match the groove");
        assert_eq!(cap, rgb(theme.seek_label_fg), "the caps match the clocks");
    }

    #[test]
    fn caps_sit_outside_the_bar_and_take_none_of_its_width() {
        // Otherwise adding them would move every position a click maps through.
        let plain = geometry(
            Rect::new(0, 0, 80, PANEL_ROWS),
            0.0,
            300.0,
            RepeatMode::Off,
            Glyphs::default(),
        )
        .unwrap();
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let bar = plain.seek.unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, PANEL_ROWS));
        render_seek(&plain, &mut buf, &theme, 0.0, 300.0, 0.0, SeekStyle::ANSI);
        assert_eq!(buf[(bar.x - 1, bar.y)].symbol(), "[");
        assert_eq!(buf[(bar.x + bar.width, bar.y)].symbol(), "]");
    }

    #[test]
    fn cycling_seek_styles_visits_every_one_and_returns() {
        let mut style = SeekStyle::default();
        let mut seen = vec![style.name()];
        for _ in 1..SeekStyle::ALL.len() {
            style = style.next();
            assert!(
                !seen.contains(&style.name()),
                "{} came round twice",
                style.name()
            );
            seen.push(style.name());
        }
        assert_eq!(style.next(), SeekStyle::default(), "did not wrap");
        assert_eq!(seen.len(), SeekStyle::ALL.len());
    }

    #[test]
    fn every_seek_style_has_its_own_name_and_parses_back() {
        for style in SeekStyle::ALL {
            assert_eq!(
                SeekStyle::parse(style.name()),
                Some(style),
                "{} does not round-trip",
                style.name()
            );
        }
    }

    #[test]
    fn the_rule_styles_sit_on_the_cell_middle() {
        // What keeps the bar level with the clock digits either side of it.
        // Block elements anchor to a cell edge and would sit above or below
        // the text; box drawing is centred.
        for style in [SeekStyle::THIN, SeekStyle::BAR] {
            let SeekFill::Rule(c) = style.fill else {
                panic!("{style:?} is not a rule");
            };
            assert!(
                ('\u{2500}'..='\u{257f}').contains(&c),
                "{c:?} is not box drawing, so it will not be centred"
            );
        }
    }

    #[test]
    fn seek_styles_resolve_by_name() {
        assert_eq!(SeekStyle::parse("thin"), Some(SeekStyle::THIN));
        assert_eq!(SeekStyle::parse("bar"), Some(SeekStyle::BAR));
        assert_eq!(SeekStyle::parse("blocks"), Some(SeekStyle::BLOCKS));
        assert_eq!(SeekStyle::parse("NERD"), Some(SeekStyle::BAR));
        assert_eq!(SeekStyle::parse("auto"), Some(SeekStyle::default()));
        assert_eq!(SeekStyle::parse("ascii"), Some(SeekStyle::ANSI));
        assert_eq!(SeekStyle::parse("nope"), None);
        assert_eq!(SeekStyle::default(), SeekStyle::ANSI);
    }

    fn seek_cells(frac: f64, width: u16, phase: f32) -> String {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, width, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let g = geometry(
            area,
            frac * 300.0,
            300.0,
            RepeatMode::Off,
            Glyphs::default(),
        )
        .expect("panel has a body");
        render_seek(
            &g,
            &mut buf,
            &theme,
            frac * 300.0,
            300.0,
            phase,
            SeekStyle::default(),
        );
        let bar = g.seek.expect("bar fits");
        (0..bar.width)
            .map(|i| buf[(bar.x + i, bar.y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn the_seek_bar_advances_inside_a_cell() {
        // Two positions a fraction of a cell apart must not draw the same bar,
        // or an hour-long track looks stopped between jumps. The thin style
        // shades its boundary cell rather than swapping a glyph, so this is a
        // question about colour.
        for style in [SeekStyle::THIN, SeekStyle::BLOCKS] {
            let a = seek_bar(0.500, 80, 0.0, style);
            let b = seek_bar(0.505, 80, 0.0, style);
            assert_ne!(a, b, "{style:?}: a sub-cell step drew nothing new");
            assert_eq!(a[..30], b[..30], "{style:?}: cells behind it moved");
        }
    }

    #[test]
    fn the_seek_bar_is_solid_from_end_to_end() {
        // Every cell belongs to the bar: no gap where played meets groove, so
        // it reads as one object rather than two.
        for style in SeekStyle::ALL {
            let cells = seek_bar(0.5, 60, 0.0, style);
            assert!(!cells.is_empty(), "{style:?} drew no bar");
            let colours: std::collections::BTreeSet<String> =
                cells.iter().map(|(_, c)| format!("{c:?}")).collect();
            assert!(
                colours.len() > 1,
                "{style:?} drew the whole bar in one colour"
            );
        }
    }

    #[test]
    fn an_empty_and_a_full_bar_are_what_they_say() {
        for style in SeekStyle::ALL {
            assert_eq!(seek_played(0.0, style), 0, "{style:?} filled at zero");
            let width = seek_bar(1.0, 60, 0.0, style).len();
            assert_eq!(
                seek_played(1.0, style),
                width,
                "{style:?} did not fill at the end"
            );
        }
    }

    #[test]
    fn the_highlight_never_moves_the_end_of_the_bar() {
        // The bar must not appear to seek while the track sits still. The
        // ascii style does change its characters as the highlight passes, so
        // what has to hold is the *extent*: how much of the bar is played.
        for style in SeekStyle::ALL {
            let still = seek_played(0.6, style);
            for phase in [0.1f32, 0.35, 0.7, 0.95] {
                let bar = seek_bar(0.6, 60, phase, style);
                let groove = rgb(crate::theme::builtin::load("cosmic").unwrap().seek_track_fg);
                let played = bar
                    .iter()
                    .filter(|(sym, fg)| *fg != groove && sym != "-" && sym != " ")
                    .count();
                assert_eq!(played, still, "{style:?} changed length at phase {phase}");
            }
        }
    }

    #[test]
    fn a_held_phase_stops_every_style_animating() {
        // What the animations switch has to guarantee: phase zero is still,
        // for the character animation as much as the colour one.
        for style in SeekStyle::ALL {
            let a = seek_bar(0.8, 60, 0.0, style);
            let b = seek_bar(0.8, 60, 0.0, style);
            assert_eq!(a, b, "{style:?} is not deterministic at rest");
            // And a bar at rest carries none of the highlight's brightening.
            assert!(sheen(0.4, 0.8, 0.0).is_none());
        }
    }

    #[test]
    fn the_highlight_is_visible_in_every_style() {
        // The point of the sweep. Whatever the style, some cell of the bar has
        // to look different as the highlight passes, or the animation is only
        // there in principle.
        for style in SeekStyle::ALL {
            let rest = seek_bar(0.8, 60, 0.0, style);
            let moved = (1..12).any(|i| seek_bar(0.8, 60, i as f32 / 12.0, style) != rest);
            assert!(moved, "{style:?} looks the same throughout the sweep");
        }
    }

    #[test]
    fn no_style_changes_its_characters_as_the_highlight_passes() {
        // The sweep is colour, and for ansi also weight -- never the glyph.
        // A bar whose characters change under a moving highlight reads as
        // static that the eye follows instead of a level it can measure.
        for style in SeekStyle::ALL {
            let still = seek_cells_styled(0.8, 60, 0.0, style);
            for i in 1..12 {
                assert_eq!(
                    seek_cells_styled(0.8, 60, i as f32 / 12.0, style),
                    still,
                    "{style:?} changed its characters"
                );
            }
        }
    }

    #[test]
    fn the_sweep_changes_nothing_but_colour() {
        // Not the glyph, and not the weight either: bold reflows a character
        // in most terminal fonts, so the cells at each end of the highlight
        // appeared to change size as it crossed them.
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        for style in SeekStyle::ALL {
            for i in 0..12 {
                let area = Rect::new(0, 0, 60, PANEL_ROWS);
                let mut buf = Buffer::empty(area);
                let g = geometry(area, 240.0, 300.0, RepeatMode::Off, Glyphs::default()).unwrap();
                render_seek(&g, &mut buf, &theme, 240.0, 300.0, i as f32 / 12.0, style);
                let bar = g.seek.unwrap();
                for x in 0..bar.width {
                    assert_eq!(
                        buf[(bar.x + x, bar.y)].style().add_modifier,
                        Modifier::empty(),
                        "{style:?} set a modifier on the bar"
                    );
                }
            }
        }
    }

    #[test]
    fn the_highlight_stays_within_the_played_part() {
        // Sweeping the groove would suggest audio that is not there.
        for phase in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            for along in [0.0f32, 0.3, 0.6, 0.9] {
                if let Some(lit) = sheen(along, 0.5, phase) {
                    assert!(lit > 0.0 && lit <= 1.0, "out of range: {lit}");
                    assert!(
                        along <= 0.5 + SHEEN_WIDTH,
                        "lit the groove at {along} (played 0.5)"
                    );
                }
            }
        }
        assert!(
            sheen(0.2, 0.0, 0.5).is_none(),
            "nothing played, nothing lit"
        );
        assert!(sheen(0.2, 0.5, 0.0).is_none(), "held still, nothing lit");
    }

    #[test]
    fn glyph_sets_resolve_by_name() {
        assert_eq!(Glyphs::parse("unicode"), Some(Glyphs::UNICODE));
        assert_eq!(Glyphs::parse("NERD"), Some(Glyphs::NERD));
        assert_eq!(Glyphs::parse("nerd-font"), Some(Glyphs::NERD));
        assert_eq!(Glyphs::parse("block"), Some(Glyphs::BLOCK));
        assert_eq!(Glyphs::parse("ascii"), Some(Glyphs::ASCII));
        assert_eq!(Glyphs::parse("nonsense"), None);
        assert_eq!(Glyphs::default(), Glyphs::UNICODE);
    }

    /// The plate is a fixed size, so the buttons are the same size to click
    /// whatever they hold. The *faces* are deliberately not uniform any more.
    #[test]
    fn every_transport_button_is_the_same_size() {
        for set in Glyphs::ALL {
            assert_eq!(set.button_height(), BUTTON_H, "plate height in {set:?}");
            // Two clear cells either side of the widest face, in every set.
            let slack = set.button_width() - set.face_width_max();
            assert_eq!(slack, BUTTON_PAD * 2, "padding in {set:?}");
            // And the widest face lands dead centre, because the plate's
            // parity follows it.
            assert_eq!(slack % 2, 0, "{set:?} cannot centre its widest face");
        }
    }

    /// The plate's real dimensions.
    ///
    /// A cell is roughly twice as tall as it is wide -- 8 by 17 pixels where
    /// this was measured -- so the default set's three cells are 24 px across,
    /// and a quarter-cell edge either side of the face's row makes it
    /// 17 + 2 x 4.25 = 25.5 px tall. Square to within 6%.
    ///
    /// The height does not follow from the row count and cannot be read off
    /// it: three occupied rows would be 51 px, and this stands at half that.
    /// [`PLATE_EDGE`] is what sets it.
    ///
    /// Pinned because these are visual properties nothing else would catch.
    /// The buttons drew as 32-by-17 lozenges for a long time, then as 16-by-17
    /// plates with the glyph jammed against the edge, then as 48-by-51 slabs,
    /// and every test passed throughout.
    #[test]
    fn the_plate_is_the_size_it_was_chosen_to_be() {
        let px = |cells: u16| -> (f32, f32) {
            let w = (cells * CELL_W_PX) as f32;
            // Occupied rows are not height: only `plate_edge` eighths of each
            // outer row are painted.
            let h = CELL_H_PX as f32 * (1.0 + 2.0 * plate_edge(cells) as f32 / 8.0);
            (w, h)
        };

        // The default set: three cells, a quarter each side.
        assert_eq!(plate_edge(3), 2);
        let (w, h) = px(3);
        assert!(
            (w - 24.0).abs() < 0.01 && (h - 25.5).abs() < 0.01,
            "{w}x{h}"
        );

        // Every set is square, whatever width its faces need, because the
        // edge is derived from that width.
        for set in Glyphs::ALL {
            let (w, h) = px(set.button_width());
            let ratio = w / h;
            assert!(
                (0.9..=1.1).contains(&ratio),
                "{set:?} stands {w}x{h} px, ratio {ratio:.2}"
            );
        }

        assert_eq!(
            Glyphs::UNICODE.button_width(),
            3,
            "three cells: one face plus padding"
        );
        // The two-cell sets need a wider plate, and at this height it is still
        // within tolerance -- checked for every set in the loop above.
        assert_eq!(Glyphs::BLOCK.button_width(), BUTTON_W);
    }

    /// What the uniform faces buy: every button, not merely the widest, sits
    /// dead centre. A face centres only when it and the plate share parity, so
    /// this holds exactly when a set's faces are all the same width.
    #[test]
    fn every_face_of_the_default_set_centres_exactly() {
        for set in [Glyphs::UNICODE, Glyphs::NERD] {
            let w = set.button_width();
            for face in set.faces() {
                let slack = w - face_width(face);
                assert_eq!(
                    slack % 2,
                    0,
                    "{face:?} cannot centre in a {w}-cell plate in {set:?}"
                );
            }
        }
    }

    /// A face must never carry default emoji presentation, bar one.
    ///
    /// `unicode-width` reports one cell for the media control pictographs and
    /// a terminal is entitled to draw two, in colour. A face a cell wider than
    /// the layout believes shifts every button after it, and nothing else in
    /// the suite would see it -- the widths all agree, on this machine.
    ///
    /// U+23F8 is allowed because it was chosen knowing this, and is named
    /// rather than excluded by a wider range so that reaching for any of its
    /// neighbours is still a test failure and a fresh decision.
    #[test]
    fn no_face_risks_being_drawn_as_an_emoji() {
        // The pictograph run that carries Emoji_Presentation=Yes.
        let risky = '\u{23e9}'..='\u{23fa}';
        const ALLOWED: char = '\u{23f8}';
        for set in Glyphs::ALL {
            for face in set.faces() {
                for ch in face.chars() {
                    assert!(
                        !risky.contains(&ch) || ch == ALLOWED,
                        "{face:?} in {set:?} uses U+{:04X}, which has default \
                         emoji presentation and may be drawn double width",
                        ch as u32
                    );
                }
            }
        }
    }

    /// The whole point of the three-row plate: an odd height has a middle row,
    /// so a one-row face is centred in it rather than half a row out.
    #[test]
    fn the_plate_has_a_middle_row_to_centre_the_face_on() {
        assert_eq!(BUTTON_H % 2, 1, "an even-height plate cannot centre a row");
    }

    #[test]
    fn the_transport_faces_are_all_different() {
        // Padding one to width must not turn it into another.
        for set in Glyphs::ALL {
            let faces = set.faces();
            for (i, a) in faces.iter().enumerate() {
                for b in &faces[i + 1..] {
                    assert_ne!(a, b, "two buttons look the same in {set:?}");
                }
            }
        }
    }

    #[test]
    fn every_button_face_is_one_cell_per_character() {
        // The layout assumes it. A glyph that a terminal renders double width
        // -- which is what an emoji-presentation triangle would do -- shifts
        // every button after it out of its own hit rect.
        use unicode_width::UnicodeWidthStr;
        let mut all: Vec<&str> = vec![SHUFFLE];
        for set in Glyphs::ALL {
            all.extend(set.faces());
        }
        for face in all {
            assert_eq!(
                face.width(),
                face_width(face) as usize,
                "{face:?} is not one cell per character"
            );
        }
    }

    /// The point of the shared geometry: a click lands on the glyph you see.
    #[test]
    fn the_hit_rects_sit_on_the_glyphs_the_renderer_draws() {
        use crate::playlist::queue::RepeatMode;
        for width in [40u16, 60, 80, 120] {
            let theme = crate::theme::builtin::load("cosmic").unwrap();
            let area = Rect::new(0, 0, width, PANEL_ROWS);
            let mut buf = Buffer::empty(area);
            let empty: [f32; 0] = [];
            PlayerView {
                theme: &theme,
                title: "Nova Era".into(),
                subtitle: String::new(),
                tech: String::new(),
                state: PlayState::Playing,
                position: 30.0,
                duration: 300.0,
                volume: 1.0,
                shuffle: true,
                repeat: RepeatMode::All,
                bit_perfect: true,
                focused: true,
                mirroring: false,
                marquee_offset: 0,
                bands: &empty,
                peaks: &empty,
                wave: &empty,
                vis_mode: crate::vis::mode::VisMode::Off,
                bars: crate::ui::panels::visualizer::BarLayout::default(),
                glyphs: Glyphs::default(),
                seek_phase: 0.0,
                seek_style: SeekStyle::default(),
                underruns: 0,
            }
            .render(area, &mut buf);

            let g = geometry(area, 30.0, 300.0, RepeatMode::All, Glyphs::default())
                .expect("panel has a body");
            // The face sits on the plate's middle row, so that is the row to
            // read it back from.
            let at = |r: Rect| -> String {
                let y = r.y + r.height / 2;
                (0..r.width)
                    .map(|i| buf[(r.x + i, y)].symbol().to_string())
                    .collect()
            };
            let c = &g.controls;
            // Each button is a padded plate, so the face sits inside the
            // rect rather than filling it -- but it must still be *in* the
            // rect the mouse tests against.
            let g2 = Glyphs::default();
            for (rect, face) in [
                (c.prev, g2.prev),
                (c.play, g2.play),
                (c.pause, g2.pause),
                (c.stop, g2.stop),
                (c.next, g2.next),
            ] {
                // A button with no room takes a zero-width rect rather than
                // one drawn over the volume slider.
                if rect.width == 0 {
                    assert!(width < 60, "the transport should fit at width {width}");
                    continue;
                }
                assert_eq!(rect.width, g2.button_width(), "width {width}");
                assert_eq!(
                    at(rect).trim(),
                    face.trim(),
                    "face missing from its button at width {width}"
                );
            }
            // A control that does not fit gets a zero-width rect rather than
            // one sitting under the volume slider. Narrow panels run out of
            // room for the toggles before the transport.
            //
            // The transport is five three-cell plates and their gaps, 20
            // cells, so the toggles need 54 columns to appear.
            for (rect, face) in [(c.shuffle, SHUFFLE), (c.repeat, "REP:ALL")] {
                if rect.width > 0 {
                    assert_eq!(at(rect), face, "width {width}");
                } else {
                    assert!(width < 54, "{face} should fit at width {width}");
                }
            }

            let vol = c.volume.expect("volume fits at these widths");
            assert!(
                at(vol).chars().all(|ch| SHADES.contains(&ch)),
                "volume rect is not the slider at width {width}: {:?}",
                at(vol)
            );

            // A tenth played: the bar starts filled and ends in groove.
            let seek = g.seek.expect("seek bar fits");
            let drawn = at(seek);
            let style = SeekStyle::default();
            assert!(
                drawn
                    .chars()
                    .all(|c| c == style.groove() || c == style.full()),
                "the default bar is drawn from its own two characters: {drawn}"
            );
        }
    }

    #[test]
    fn the_format_and_bitrate_survive_a_long_album_title() {
        // Concatenating album and tech and truncating the result dropped the
        // technical line entirely on any long album name -- which is exactly
        // the case where it is still the only thing the row can tell you.
        let tech = "FLAC · 1006 kbps · 44.1 kHz · 16-bit · stereo";
        let rows = draw(
            "Rebirth (Deluxe Edition, Remastered 2021, Disc One of Two)",
            tech,
            80,
        );
        let line = rows
            .iter()
            .find(|r| r.contains("FLAC"))
            .unwrap_or_else(|| panic!("the tech line was truncated away:\n{}", rows.join("\n")));
        assert!(line.contains("1006 kbps"), "bitrate missing: {line}");
        // The border is the last cell on the row; the tech string ends against it.
        let inside = line.trim_end_matches(['║', ' ']);
        assert!(inside.ends_with("stereo"), "not right-aligned: {line}");
    }

    #[test]
    fn a_narrow_panel_keeps_the_tech_line_rather_than_the_album() {
        let rows = draw("Some Album", "FLAC · 44.1 kHz", 30);
        assert!(
            rows.iter().any(|r| r.contains("FLAC")),
            "{}",
            rows.join("\n")
        );
    }

    #[test]
    fn short_titles_are_left_alone() {
        assert_eq!(marquee("Short", 20, 0), "Short");
        assert_eq!(marquee("Short", 20, 7), "Short", "offset is irrelevant");
    }

    #[test]
    fn long_titles_scroll_and_wrap_around() {
        let s = "A Very Long Track Title That Does Not Fit At All";
        let a = marquee(s, 10, 0);
        let b = marquee(s, 10, 1);
        assert_eq!(a.chars().count(), 10);
        assert_eq!(b.chars().count(), 10);
        assert_ne!(a, b, "it should actually move");
    }

    #[test]
    fn truncate_respects_display_width_not_byte_length() {
        assert_eq!(truncate("hello", 10), "hello");
        let t = truncate("hello world", 8);
        assert!(t.ends_with('…'));
        use unicode_width::UnicodeWidthStr;
        assert!(t.width() <= 8);
    }

    #[test]
    fn truncate_handles_wide_characters_without_overflowing() {
        // A CJK title is two cells per character; counting chars would overrun
        // the column and corrupt the row.
        use unicode_width::UnicodeWidthStr;
        let s = "君の名は。星を追う子ども";
        for w in [4, 7, 10, 13] {
            assert!(truncate(s, w).width() <= w, "width {w} overflowed");
        }
    }

    #[test]
    fn zero_width_produces_nothing_rather_than_panicking() {
        assert_eq!(truncate("anything", 0), "");
        assert_eq!(marquee("anything", 0, 3), "");
    }
    #[test]
    fn a_dropout_is_named_for_what_it_sounded_like() {
        // `xrun` is the driver's word for it. What the listener heard was a
        // gap, and the indicator should say the thing they can recognise.
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        for (n, want) in [(1u64, "1 dropout"), (2, "2 dropouts"), (42, "42 dropouts")] {
            let area = Rect::new(0, 0, 96, 12);
            let mut buf = Buffer::empty(area);
            let empty: [f32; 0] = [];
            PlayerView {
                theme: &theme,
                title: "Nova Era".into(),
                subtitle: "Angra".into(),
                tech: "flac".into(),
                state: PlayState::Playing,
                position: 30.0,
                duration: 300.0,
                volume: 0.8,
                shuffle: false,
                repeat: crate::playlist::queue::RepeatMode::Off,
                bit_perfect: true,
                focused: true,
                mirroring: false,
                marquee_offset: 0,
                bands: &empty,
                peaks: &empty,
                wave: &empty,
                vis_mode: crate::vis::mode::VisMode::Off,
                bars: crate::ui::panels::visualizer::BarLayout::default(),
                glyphs: Glyphs::default(),
                seek_phase: 0.0,
                seek_style: SeekStyle::default(),
                underruns: n,
            }
            .render(area, &mut buf);
            let all: String = (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .map(|(x, y)| buf[(x, y)].symbol().to_string())
                .collect();
            assert!(all.contains(want), "{n} dropouts drew neither, got {all:?}");
            assert!(!all.contains("xrun"), "the driver's word survived");
        }
    }
}
