//! The "resume where you left off?" prompt.
//!
//! Shown rather than done silently: dropping straight into the middle of a
//! track on launch is startling, and there is no way to decline it afterwards.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::session::{self, Session};
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::panels::player::truncate;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

pub struct ResumeView<'a> {
    pub theme: &'a Theme,
    pub session: &'a Session,
    pub now: i64,
}

impl<'a> Widget for ResumeView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let w = area.width.saturating_sub(4).clamp(28, 64);
        let h = 7u16.min(area.height);
        let rect = Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        Clear.render(rect, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(t.border_focused)))
            .title(Span::styled(
                " RESUME ",
                Style::default()
                    .fg(rgb(t.header_fg))
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(
                Line::from(Span::styled(
                    " enter resume · n start fresh ",
                    Style::default().fg(rgb(t.dim)),
                ))
                .right_aligned(),
            )
            .style(Style::default().bg(rgb(t.panel_bg)));

        let inner = block.inner(rect);
        block.render(rect, buf);
        if inner.height == 0 {
            return;
        }
        let width = inner.width.saturating_sub(2) as usize;

        let mut y = inner.y;
        let put = |s: String, style: Style, buf: &mut Buffer, y: &mut u16| {
            if *y < inner.y + inner.height {
                buf.set_string(inner.x + 1, *y, truncate(&s, width), style);
                *y += 1;
            }
        };

        put(
            self.session.describe(),
            Style::default()
                .fg(rgb(t.row_playing_fg))
                .add_modifier(Modifier::BOLD),
            buf,
            &mut y,
        );
        put(
            self.session.describe_context(),
            Style::default().fg(rgb(t.row_meta_fg)),
            buf,
            &mut y,
        );
        put(
            format!("saved {}", session::age(self.session.saved_at, self.now)),
            Style::default().fg(rgb(t.dim)),
            buf,
            &mut y,
        );

        // Say so before doing it: a list that comes back with four records
        // folded away, unannounced, reads as a list that has lost them.
        if let Some(view) = self.session.describe_view() {
            put(view, Style::default().fg(rgb(t.dim)), buf, &mut y);
        }

        if !self.session.playlist_available() {
            put(
                "that playlist is gone — will resume from the library".into(),
                Style::default().fg(rgb(t.warn)),
                buf,
                &mut y,
            );
        }
    }
}
