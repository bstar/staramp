//! The "resume where you left off?" prompt.
//!
//! Shown rather than done silently: dropping straight into the middle of a
//! track on launch is startling, and there is no way to decline it afterwards.

use super::overlay::{self, Anchor, Overlay};
use starkit::chrome::rgb;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::widgets::Widget;

use crate::session::{self, Session};
use crate::theme::Theme;
use crate::ui::panels::player::truncate;

pub struct ResumeView<'a> {
    pub theme: &'a Theme,
    pub session: &'a Session,
    pub now: i64,
}

/// Where the box lands, so a click can be tested against it.
pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (28, 64), 7, 7, Anchor::Centre)
}

impl<'a> Widget for ResumeView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let r = rect(area);
        let inner = overlay::render(
            r,
            buf,
            &Overlay {
                theme: t,
                title: "resume",
                detail: None,
                footer: Some("enter resume \u{b7} n start fresh"),
            },
        );
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
