//! A filesystem browser used by import and export operations.
//!
//! It uses the library browser's full-screen shape and navigation rather than
//! introducing a path prompt that behaves unlike the rest of the player.

use std::path::{Path, PathBuf};

use starkit::chrome::rgb;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};

use crate::theme::Theme;

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
        let title = match self.browser.purpose {
            Purpose::ImportEq => "import apo profile",
            Purpose::ExportEq => "export apo profile",
        };
        let hint = match self.browser.purpose {
            Purpose::ImportEq => {
                "j/k move \u{b7} l/enter open or import \u{b7} h parent \u{b7} esc close"
            }
            Purpose::ExportEq => {
                "j/k move \u{b7} l/enter directory \u{b7} s save \u{b7} h parent \u{b7} esc close"
            }
        };
        let body = super::frame::frame(
            area,
            buf,
            &super::frame::Frame {
                theme: t,
                focused: true,
                title,
                detail: None,
                heading: false,
                badge: None,
                footer: Some(hint),
                words: super::frame::NO_WORDS,
            },
        );
        if body.height == 0 || body.width == 0 {
            return;
        }
        buf.set_string(
            body.x,
            body.y,
            self.browser.directory.display().to_string(),
            Style::default().fg(rgb(t.dim)),
        );
        // The directory line, then a blank row, then the list; the export
        // form also keeps one row at the bottom for the file name it will
        // write, now that the hint itself lives on the footer.
        let extra_bottom = u16::from(self.browser.purpose == Purpose::ExportEq);
        let list_y = body.y + 2;
        let height = body.height.saturating_sub(2 + extra_bottom) as usize;
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
                body.x,
                list_y + row as u16,
                crate::ui::panels::player::truncate(&line, body.width as usize),
                style,
            );
        }
        if self.browser.purpose == Purpose::ExportEq {
            buf.set_string(
                body.x,
                body.y + body.height - 1,
                format!("file: {}.txt", self.save_name),
                Style::default().fg(rgb(t.eq_band_value)),
            );
        }
    }
}
