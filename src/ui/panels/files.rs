//! A filesystem browser used by import and export operations.
//!
//! It uses the library browser's full-screen shape and navigation rather than
//! introducing a path prompt that behaves unlike the rest of the player.

use std::path::{Path, PathBuf};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    ImportEq,
    ExportEq,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
}

pub struct Browser {
    pub purpose: Purpose,
    pub directory: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub scroll: usize,
    pub confirm: Option<PathBuf>,
}

impl Browser {
    pub fn new(purpose: Purpose, directory: PathBuf) -> Self {
        let mut out = Self {
            purpose,
            directory,
            entries: Vec::new(),
            cursor: 0,
            scroll: 0,
            confirm: None,
        };
        out.refresh();
        out
    }

    pub fn refresh(&mut self) {
        self.entries = std::fs::read_dir(&self.directory)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let directory = path.is_dir();
                let apo = matches!(
                    path.extension()
                        .and_then(|v| v.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("txt" | "apo")
                );
                (directory || apo).then_some(Entry { path, directory })
            })
            .collect();
        self.entries.sort_by(|a, b| {
            (
                !a.directory,
                a.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase()),
            )
                .cmp(&(
                    !b.directory,
                    b.path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_lowercase()),
                ))
        });
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.scroll = 0;
        self.confirm = None;
    }

    pub fn move_by(&mut self, delta: i32) {
        let last = self.entries.len().saturating_sub(1) as i32;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
        self.confirm = None;
    }

    pub fn parent(&mut self) {
        if let Some(parent) = self.directory.parent() {
            self.directory = parent.to_path_buf();
            self.cursor = 0;
            self.refresh();
        }
    }

    pub fn enter_directory(&mut self) -> bool {
        let Some(entry) = self.entries.get(self.cursor) else {
            return false;
        };
        if !entry.directory {
            return false;
        }
        self.directory = entry.path.clone();
        self.cursor = 0;
        self.refresh();
        true
    }

    pub fn selected_file(&self) -> Option<&Path> {
        self.entries
            .get(self.cursor)
            .filter(|e| !e.directory)
            .map(|e| e.path.as_path())
    }
}

pub struct FileView<'a> {
    pub theme: &'a Theme,
    pub browser: &'a Browser,
    pub save_name: &'a str,
}

impl FileView<'_> {
    pub fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        buf.set_style(area, Style::default().bg(rgb(t.bg)).fg(rgb(t.row_fg)));
        let title = match self.browser.purpose {
            Purpose::ImportEq => "IMPORT APO PROFILE",
            Purpose::ExportEq => "EXPORT APO PROFILE",
        };
        buf.set_string(
            area.x + 2,
            area.y,
            title,
            Style::default()
                .fg(rgb(t.header_fg))
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            area.x + 2,
            area.y + 1,
            self.browser.directory.display().to_string(),
            Style::default().fg(rgb(t.dim)),
        );
        let body_y = area.y + 3;
        let height = area.height.saturating_sub(5) as usize;
        let scroll = crate::ui::panels::picker::clamp_scroll(
            self.browser.cursor,
            self.browser.scroll,
            height,
        );
        for row in 0..height {
            let index = scroll + row;
            let Some(entry) = self.browser.entries.get(index) else {
                break;
            };
            let selected = index == self.browser.cursor;
            let style = if selected {
                Style::default()
                    .fg(rgb(t.row_selected_fg))
                    .bg(rgb(t.row_selected_bg))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(rgb(t.row_fg))
            };
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            let line = format!("{} {}", if entry.directory { "▸" } else { " " }, name);
            buf.set_string(
                area.x + 2,
                body_y + row as u16,
                crate::ui::panels::player::truncate(&line, area.width.saturating_sub(4) as usize),
                style,
            );
        }
        let hint = match self.browser.purpose {
            Purpose::ImportEq => "j/k move · l/enter open or import · h parent · esc close",
            Purpose::ExportEq => "j/k move · l/enter directory · s save · h parent · esc close",
        };
        buf.set_string(
            area.x + 2,
            area.y + area.height.saturating_sub(2),
            hint,
            Style::default().fg(rgb(t.dim)),
        );
        if self.browser.purpose == Purpose::ExportEq {
            buf.set_string(
                area.x + 2,
                area.y + area.height.saturating_sub(1),
                format!("file: {}.txt", self.save_name),
                Style::default().fg(rgb(t.eq_band_value)),
            );
        }
    }
}
