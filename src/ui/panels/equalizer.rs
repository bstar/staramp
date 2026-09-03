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

pub struct EqView<'a> {
    pub theme: &'a Theme,
    pub profile: &'a Profile,
    pub enabled: bool,
    pub selected: usize,
    pub scroll: usize,
    pub focused: bool,
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
        buf.set_string(
            inner.x + 1,
            inner.y + 1,
            "  #  CH  TYPE        FREQUENCY       GAIN       Q/BW",
            Style::default().fg(rgb(t.dim)),
        );
        let height = inner.height.saturating_sub(2) as usize;
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
                inner.y + 2 + row as u16,
                format_stage(index, stage, inner.width as usize),
                style,
            );
        }
    }
}

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
