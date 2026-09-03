//! The ordered parametric equalizer window.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::audio::dsp::apo::{BiquadKind, Filter, Profile, Stage, Width};
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Rows the response curve takes when there is room for it.
///
/// Three, which at four braille dots to a row is twelve vertical steps over
/// the +/-12 dB scale -- one step per decibel, which is the resolution the
/// numbers beside it claim.
pub const CURVE_ROWS: u16 = 3;

/// The dB the curve spans, either side of flat.
///
/// Fixed rather than fitted to the profile. A scale that rescales itself
/// makes a gentle profile look as dramatic as a savage one, and the whole
/// point of the picture is to see at a glance how much is being done.
const CURVE_DB: f64 = 12.0;

/// The frequency range plotted, in Hz.
///
/// Log-spaced, because hearing is: linear would spend four fifths of the
/// width above 4 kHz where almost nothing in a profile happens.
const CURVE_LO_HZ: f64 = 20.0;
const CURVE_HI_HZ: f64 = 20_000.0;

/// Rows needed before the curve is worth drawing at all.
///
/// Below this the list is short enough that taking three rows from it costs
/// more than the picture gives back.
const CURVE_MIN_LIST_ROWS: u16 = 4;

pub struct EqView<'a> {
    pub theme: &'a Theme,
    pub profile: &'a Profile,
    pub enabled: bool,
    pub selected: usize,
    pub scroll: usize,
    pub focused: bool,
    /// The chain as the audio path compiled it, for the response curve.
    ///
    /// The compiled form rather than the profile, so the picture is drawn
    /// from the coefficients actually being run: a curve derived separately
    /// from the profile would be a second implementation of the maths, and
    /// the two would agree only until one was edited.
    pub compiled: &'a crate::audio::dsp::eq::EqSettings,
    pub sample_rate: u32,
}

pub struct Geometry {
    pub toggle: Rect,
    pub preset_prev: Rect,
    pub preset_next: Rect,
    pub rows: Rect,
}

pub const PANEL_ROWS: u16 = super::header::ROWS + 8;

pub fn geometry(area: Rect, profile: &str) -> Option<Geometry> {
    let inner = super::header::body(area);
    if inner.height < 3 || inner.width < 40 {
        return None;
    }
    let name = profile.chars().count() as u16;
    Some(Geometry {
        toggle: Rect::new(inner.x + 1, inner.y, 5, 1),
        preset_prev: Rect::new(inner.x + 7, inner.y, 1, 1),
        preset_next: Rect::new(inner.x + 10 + name, inner.y, 1, 1),
        rows: Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(2),
        ),
    })
}

impl Widget for EqView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(if self.focused {
                t.border_focused
            } else {
                t.border
            })))
            .title(Span::styled(
                "═ EQUALIZER ",
                Style::default().fg(rgb(t.header_fg)),
            ))
            .style(Style::default().bg(rgb(t.bg)));
        let inner = super::header::body(area);
        block.render(area, buf);
        super::frame::render_corners(area, buf, t, self.focused);
        super::header::render(area, super::header::PLAIN, buf, t);
        if inner.height == 0 || inner.width < 40 {
            return;
        }
        buf.set_string(
            inner.x + 1,
            inner.y,
            if self.enabled { "[ON ]" } else { "[OFF]" },
            Style::default()
                .fg(rgb(if self.enabled {
                    t.eq_enabled_fg
                } else {
                    t.eq_disabled_fg
                }))
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            inner.x + 7,
            inner.y,
            format!("‹ {} ›", self.profile.name),
            Style::default().fg(rgb(t.eq_band_value)),
        );
        if inner.height < 3 {
            return;
        }

        // The curve takes rows from the list, so it only appears when the
        // list can spare them.
        let curve = curve_rows(inner.height);
        if curve > 0 {
            render_curve(
                Rect::new(inner.x, inner.y + 1, inner.width, curve),
                buf,
                t,
                self.compiled,
                self.sample_rate,
                self.enabled,
            );
        }

        buf.set_string(
            inner.x + 1,
            inner.y + 1 + curve,
            "  #  CH  TYPE        FREQUENCY       GAIN       Q/BW",
            Style::default().fg(rgb(t.dim)),
        );
        let height = inner.height.saturating_sub(2 + curve) as usize;
        let scroll = clamp_scroll(self.selected, self.scroll, height);
        for row in 0..height {
            let index = scroll + row;
            let Some(stage) = self.profile.stages.get(index) else {
                break;
            };
            let selected = self.focused && index == self.selected;
            let style = if selected {
                Style::default()
                    .fg(rgb(t.row_selected_fg))
                    .bg(rgb(t.row_selected_bg))
                    .add_modifier(Modifier::BOLD)
            } else if stage.enabled {
                Style::default().fg(rgb(t.row_fg))
            } else {
                Style::default().fg(rgb(t.dim))
            };
            buf.set_string(
                inner.x,
                inner.y + 2 + curve + row as u16,
                format_stage(index, stage, inner.width as usize),
                style,
            );
        }
    }
}

/// How many rows the curve gets in a panel of this height.
///
/// Zero when the list would be left with too little to be useful -- a
/// picture that costs the thing it describes is a poor trade.
pub fn curve_rows(inner_height: u16) -> u16 {
    let after = inner_height.saturating_sub(2 + CURVE_ROWS);
    if after >= CURVE_MIN_LIST_ROWS {
        CURVE_ROWS
    } else {
        0
    }
}

/// The chain's magnitude response, as a braille trace over a log-frequency
/// axis.
///
/// Braille rather than block characters: four dots to a cell vertically and
/// two across gives the curve eight times the resolution the row count
/// suggests, which is what makes a three-row plot readable at all. The
/// oscilloscope draws the same way.
fn render_curve(
    area: Rect,
    buf: &mut Buffer,
    t: &Theme,
    eq: &crate::audio::dsp::eq::EqSettings,
    sample_rate: u32,
    enabled: bool,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    // Two dot columns per cell, four dot rows.
    let cols = area.width as usize * 2;
    let rows = area.height as usize * 4;

    // Sample the response once per dot column, log-spaced.
    let ratio = (CURVE_HI_HZ / CURVE_LO_HZ).ln();
    let db: Vec<f64> = (0..cols)
        .map(|i| {
            let t = i as f64 / (cols - 1).max(1) as f64;
            let f = CURVE_LO_HZ * (t * ratio).exp();
            eq.magnitude_db_at(f, sample_rate)
        })
        .collect();

    // dB to a dot row, flat in the middle, clamped at the edges.
    let dot_of = |v: f64| {
        let norm = (v / CURVE_DB).clamp(-1.0, 1.0);
        let y = (1.0 - norm) * 0.5 * (rows - 1) as f64;
        y.round() as usize
    };
    let zero = dot_of(0.0);

    let mut mask = vec![0u8; area.width as usize * area.height as usize];
    let mut set = |cx: usize, dot_x: usize, dot_y: usize| {
        let cell_y = dot_y / 4;
        if cx >= area.width as usize || cell_y >= area.height as usize {
            return;
        }
        mask[cell_y * area.width as usize + cx] |= BRAILLE_BIT[dot_y % 4][dot_x];
    };

    for i in 0..cols {
        let (cx, dot_x) = (i / 2, i % 2);
        let y = dot_of(db[i]);
        // Join to the previous sample rather than plotting points, so a steep
        // filter is a line instead of a dotted stipple.
        let prev = if i == 0 { y } else { dot_of(db[i - 1]) };
        for step in y.min(prev)..=y.max(prev) {
            set(cx, dot_x, step);
        }
    }

    let trace = rgb(if enabled {
        t.eq_enabled_fg
    } else {
        t.eq_disabled_fg
    });
    for cy in 0..area.height as usize {
        for cx in 0..area.width as usize {
            let bits = mask[cy * area.width as usize + cx];
            let (ch, style) = if bits == 0 {
                // The zero line, so an empty profile reads as "nothing is
                // being done" rather than as a panel that failed to draw.
                //
                // Drawn in braille rather than with a box-drawing rule: a
                // rule sits in the middle of its cell while the trace sits on
                // a dot row, so the two met at a visible step wherever the
                // curve crossed zero.
                if cy == zero / 4 {
                    let dots = BRAILLE_BIT[zero % 4][0] | BRAILLE_BIT[zero % 4][1];
                    (
                        char::from_u32(0x2800 + dots as u32).unwrap_or(' '),
                        Style::default().fg(rgb(t.dim)),
                    )
                } else {
                    continue;
                }
            } else {
                (
                    char::from_u32(0x2800 + bits as u32).unwrap_or(' '),
                    Style::default().fg(trace),
                )
            };
            buf[(area.x + cx as u16, area.y + cy as u16)]
                .set_char(ch)
                .set_style(style.bg(rgb(t.bg)));
        }
    }
}

/// Braille dot bits, by row then column. The same layout the oscilloscope
/// uses.
const BRAILLE_BIT: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

fn format_stage(index: usize, stage: &Stage, width: usize) -> String {
    let on = if stage.enabled { '●' } else { '○' };
    let ch = if stage.channels == crate::audio::dsp::apo::ChannelMask::ALL {
        "all".into()
    } else {
        format!("{:x}", stage.channels.0)
    };
    let body = match &stage.filter {
        Filter::Preamp { gain_db } => format!("PREAMP                         {gain_db:+.3} dB"),
        Filter::Biquad {
            kind,
            frequency,
            gain_db,
            width,
            ..
        } => {
            let kind = match kind {
                BiquadKind::Peaking => "PK",
                BiquadKind::LowPass => "LP",
                BiquadKind::HighPass => "HP",
                BiquadKind::BandPass => "BP",
                BiquadKind::LowShelf => "LS",
                BiquadKind::HighShelf => "HS",
                BiquadKind::Notch => "NO",
                BiquadKind::AllPass => "AP",
            };
            let amount = match width {
                Width::Q(v) => format!("Q {v:.4}"),
                Width::Bandwidth(v) => format!("BW {v:.4}"),
                Width::Slope(v) => format!("S {v:.2}"),
            };
            format!("{kind:<10} {frequency:>9.2} Hz {gain_db:>+8.3} dB {amount}")
        }
        Filter::Iir { numerator, .. } => format!(
            "IIR         order {:>2}     {} coefficients",
            numerator.len() - 1,
            numerator.len() * 2
        ),
        Filter::GraphicEq { points } => format!("GraphicEQ   {:>5} points", points.len()),
    };
    crate::ui::panels::player::truncate(&format!("{on} {:>2} {ch:<3} {body}", index + 1), width)
}

pub fn clamp_scroll(selected: usize, scroll: usize, height: usize) -> usize {
    super::picker::clamp_scroll(selected, scroll, height)
}

#[cfg(test)]
mod curve_tests {
    use super::*;
    use crate::audio::dsp::apo::{BiquadKind, Filter, Profile, Stage, Width};
    use crate::audio::dsp::eq::EqSettings;

    fn peaking(f: f64, gain: f64, q: f64) -> Stage {
        Stage {
            enabled: true,
            channels: crate::audio::dsp::apo::ChannelMask::ALL,
            filter: Filter::Biquad {
                kind: BiquadKind::Peaking,
                frequency: f,
                gain_db: gain,
                width: Width::Q(q),
                corner_frequency: false,
            },
        }
    }

    /// Render just the curve and return its rows.
    fn curve(stages: Vec<Stage>, enabled: bool, w: u16, h: u16) -> Vec<String> {
        let theme = crate::theme::builtin::load("cosmic").unwrap();
        let profile = Profile {
            name: "t".into(),
            stages,
        };
        let eq = EqSettings::from_profile(enabled, &profile, 44_100);
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render_curve(area, &mut buf, &theme, &eq, 44_100, enabled);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// The picture has to follow the filters.
    #[test]
    fn a_boost_rises_and_a_cut_falls() {
        let rows = curve(vec![peaking(1_000.0, 9.0, 1.4)], true, 46, 3);
        // 1 kHz is a little past the middle of a 20 Hz to 20 kHz log axis.
        let ink = |row: &str, from: usize, to: usize| {
            row.chars()
                .skip(from)
                .take(to - from)
                .filter(|c| *c != ' ' && *c != '\u{2800}')
                .count()
        };
        assert!(
            ink(&rows[0], 18, 30) > 0,
            "the boost does not rise:\n{rows:#?}"
        );

        let rows = curve(vec![peaking(1_000.0, -9.0, 1.4)], true, 46, 3);
        assert!(
            ink(&rows[2], 18, 30) > 0,
            "the cut does not fall:\n{rows:#?}"
        );
    }

    /// A bypassed chain is flat, whatever the profile holds.
    #[test]
    fn a_disabled_equalizer_draws_a_flat_line() {
        let stages = vec![peaking(1_000.0, 12.0, 1.4)];
        let rows = curve(stages, false, 46, 3);
        assert!(
            rows[0].chars().all(|c| c == ' '),
            "the top row is not empty:\n{rows:#?}"
        );
        assert!(
            rows[1].chars().all(|c| c != ' '),
            "the zero line is broken:\n{rows:#?}"
        );
    }

    /// The curve gives its rows back when the list needs them.
    #[test]
    fn the_curve_yields_to_the_list_in_a_short_panel() {
        // Tall enough for a curve and a usable list.
        assert_eq!(curve_rows(3 + CURVE_ROWS + CURVE_MIN_LIST_ROWS), CURVE_ROWS);
        // Not tall enough: the list keeps everything.
        assert_eq!(curve_rows(2 + CURVE_ROWS), 0);
        assert_eq!(curve_rows(4), 0);
    }

    /// Not an assertion -- run with `--nocapture` to look at the plot.
    #[test]
    fn preview_the_response_curve() {
        println!("\n  flat:");
        for r in curve(vec![], true, 46, 3) {
            println!("    |{r}|");
        }
        println!("  +9 dB at 1 kHz, -6 dB at 120 Hz:");
        for r in curve(
            vec![peaking(1_000.0, 9.0, 1.4), peaking(120.0, -6.0, 1.0)],
            true,
            46,
            3,
        ) {
            println!("    |{r}|");
        }
        println!("  a narrow notch at 3 kHz:");
        for r in curve(vec![peaking(3_000.0, -12.0, 8.0)], true, 46, 3) {
            println!("    |{r}|");
        }
    }
}
