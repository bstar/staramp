//! Compact listening-history panel with network-delivery status.

use starkit::chrome::rgb;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::widgets::Widget;

use crate::activity::Snapshot;
use crate::theme::Theme;

pub const PANEL_ROWS: u16 = 8;
pub const VISIBLE_ROWS: usize = 5;

pub struct HistoryView<'a> {
    pub theme: &'a Theme,
    pub snapshot: &'a Snapshot,
    pub focused: bool,
    /// First recent listen shown; zero is the newest.
    pub scroll: usize,
}

fn provider_summary(snapshot: &Snapshot) -> String {
    let mut parts: Vec<String> = snapshot
        .providers
        .iter()
        .filter(|p| p.configured)
        .map(|p| {
            format!(
                "{} {}",
                p.provider.label(),
                if p.enabled { "ON" } else { "OFF" }
            )
        })
        .collect();
    if snapshot.pending > 0 {
        parts.push(format!("{} pending", snapshot.pending));
    }
    if parts.is_empty() {
        "LOCAL ACTIVITY".into()
    } else {
        parts.join(" · ")
    }
}

impl Widget for HistoryView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        super::frame::frame(
            area,
            buf,
            &super::frame::Frame {
                theme: t,
                focused: self.focused,
                title: "activity",
                detail: None,
                heading: false,
                badge: None,
                footer: None,
                words: super::header::PLAIN,
            },
        );
        let inner = super::header::body(area);
        let header = super::header::rect(area);
        let status_x = header.x.saturating_add(1);
        let status_right = super::header::slots(area, super::header::PLAIN)
            .first()
            .map(|(_, r)| r.x.saturating_sub(2))
            .unwrap_or_else(|| header.x.saturating_add(header.width).saturating_sub(1));
        let status_width = status_right.saturating_sub(status_x) as usize;
        if header.height > 0 && status_width > 0 {
            buf.set_string(
                status_x,
                header.y,
                super::player::truncate(&provider_summary(self.snapshot), status_width),
                Style::default()
                    .fg(rgb(t.row_meta_fg))
                    .add_modifier(Modifier::BOLD),
            );
        }
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        for (i, row) in self
            .snapshot
            .recent
            .iter()
            .skip(self.scroll)
            .take(VISIBLE_ROWS)
            .enumerate()
        {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let state = row.state();
            let state_w = state.chars().count() as u16;
            let name_w = inner.width.saturating_sub(state_w + 4) as usize;
            buf.set_string(
                inner.x + 1,
                y,
                super::player::truncate(&row.name(), name_w),
                Style::default().fg(rgb(t.row_fg)),
            );
            if inner.width > state_w + 2 {
                buf.set_string(
                    inner.x + inner.width - state_w - 1,
                    y,
                    state,
                    Style::default().fg(rgb(if row.errors > 0 {
                        t.error
                    } else {
                        t.row_meta_fg
                    })),
                );
            }
        }

        let visible = inner.height.min(VISIBLE_ROWS as u16);
        let list = Rect {
            height: visible,
            ..inner
        };
        super::scrollbar::render(
            super::scrollbar::track(area, list),
            buf,
            t,
            super::scrollbar::rows(self.scroll, self.snapshot.recent.len(), visible),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{Provider, ProviderStatus};

    #[test]
    fn unconfigured_services_are_not_advertised_as_off() {
        let snapshot = Snapshot {
            providers: vec![
                ProviderStatus {
                    provider: Provider::Lastfm,
                    enabled: false,
                    configured: false,
                    username: String::new(),
                },
                ProviderStatus {
                    provider: Provider::Listenbrainz,
                    enabled: false,
                    configured: true,
                    username: "listener".into(),
                },
            ],
            ..Snapshot::default()
        };
        assert_eq!(provider_summary(&snapshot), "ListenBrainz OFF");
    }

    #[test]
    fn no_configured_service_leaves_local_history_only() {
        let snapshot = Snapshot {
            providers: vec![ProviderStatus {
                provider: Provider::Lastfm,
                enabled: true,
                configured: false,
                username: String::new(),
            }],
            ..Snapshot::default()
        };
        assert_eq!(provider_summary(&snapshot), "LOCAL ACTIVITY");
    }
}
