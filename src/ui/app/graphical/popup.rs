//! Native layout for the existing settings and track actions. Keep the data
//! and commands shared with ASCII, but never truncate them into terminal cells.
use super::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const FONT: u16 = 12;
const ADVANCE: usize = 8; // Leave room for the terminal's monospace font.
const LINE: u16 = 20;

pub(super) fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0;
    for c in text.chars() {
        let n = c.width().unwrap_or(0);
        if c == '\n' {
            lines.push(std::mem::take(&mut line));
            width = 0;
        } else if width + n > columns.max(1) && !line.is_empty() {
            // Prefer word boundaries; split long paths/tokens only when they
            // cannot fit on a line. Preserve every character of the value.
            let tail = line
                .char_indices()
                .rev()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(at, ch)| line.split_off(at + ch.len_utf8()));
            lines.push(std::mem::take(&mut line));
            line = tail.unwrap_or_default();
            width = line.width();
            if width + n > columns.max(1) {
                lines.push(std::mem::take(&mut line));
                width = 0;
            }
        }
        if c != '\n' {
            line.push(c);
            width += n;
        }
    }
    lines.push(line);
    lines
}

impl Rack<'_> {
    pub(super) fn resume_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        let Some(session) = &self.app.over.resume else {
            self.resume_scroll = 0;
            self.resume_fresh = false;
            return false;
        };
        self.overlay = true;
        self.app.bars.begin_frame();
        let width = 760.min(s.width - 32);
        let inner = width - 40;
        let mut rows = Vec::new();
        if !session.artist.is_empty() {
            rows.push((session.artist.clone(), c.accent.clone(), true));
        }
        rows.push((
            if session.title.is_empty() {
                session.uri.clone()
            } else {
                session.title.clone()
            },
            c.ink.clone(),
            true,
        ));
        rows.push((String::new(), c.dim.clone(), false));
        rows.push((session.describe_context(), c.ink.clone(), false));
        rows.push((
            format!(
                "Position {} / {}",
                crate::ui::digits::clock(session.position),
                crate::ui::digits::clock(session.duration)
            ),
            c.accent.clone(),
            false,
        ));
        rows.push((
            format!(
                "Saved {}",
                crate::session::age(session.saved_at, crate::library::db::now_secs())
            ),
            c.dim.clone(),
            false,
        ));
        if let Some(view) = session.describe_view() {
            rows.push((view, c.dim.clone(), false));
        }
        if !self.app.over.resume_playlist_available {
            rows.push((
                "The saved playlist is unavailable. Resume will use the library.".into(),
                c.accent.clone(),
                false,
            ));
        }
        let lines = rows
            .iter()
            .flat_map(|(text, color, bold)| {
                wrap(text, usize::from(inner) / ADVANCE)
                    .into_iter()
                    .map(move |line| (line, color, *bold))
            })
            .collect::<Vec<_>>();
        let header = 52;
        let footer = 86;
        let height = (lines.len() * usize::from(LINE) + usize::from(header + footer))
            .min(usize::from(s.height - 32)) as u16;
        let visible = usize::from((height - header - footer) / LINE);
        self.resume_scroll = self.resume_scroll.min(lines.len().saturating_sub(visible));
        let x = (s.width - width) / 2;
        let y = (s.height - height) / 2;
        s.hits.clear();
        self.frame(s, R::new(x, y, width, height), c, self.radius, false);
        s.hits.push(HitRegion {
            rect: R::new(x, y, width, height),
            action: "resume:body".into(),
        });
        classic::label(
            s,
            R::new(x + 20, y + 10, inner, 28),
            "RESUME SESSION",
            &c.accent,
            FONT,
            true,
        );
        s.fill(R::new(x + 20, y + 43, inner, 1), &c.shadow, 0);
        for (i, (text, color, bold)) in lines
            .iter()
            .skip(self.resume_scroll)
            .take(visible)
            .enumerate()
        {
            classic::label(
                s,
                R::new(x + 20, y + header + i as u16 * LINE, inner, LINE),
                text,
                color,
                FONT,
                *bold,
            );
        }
        let hint = if lines.len() > visible {
            "↑↓ scroll · Tab switch · Enter choose · N / Esc start fresh"
        } else {
            "Tab switch · Enter choose · N / Esc start fresh"
        };
        classic::label(
            s,
            R::new(x + 20, y + height - 76, inner, 20),
            hint,
            &c.dim,
            11,
            false,
        );
        for (left, text, action, selected) in [
            (
                width - 358,
                "Start fresh",
                "resume:fresh",
                self.resume_fresh,
            ),
            (width - 180, "Resume", "resume:accept", !self.resume_fresh),
        ] {
            let rect = R::new(x + left, y + height - 48, 160, 30);
            self.button(s, rect, c, text, action, FONT, selected);
        }
        true
    }

    pub(super) fn resume_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.app.over.resume.is_none()
            || !self
                .surface
                .as_ref()
                .is_some_and(|s| s.hits.iter().any(|h| h.action == "resume:body"))
        {
            return false;
        }
        match code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                self.resume_fresh = !self.resume_fresh
            }
            KeyCode::Up => self.resume_scroll = self.resume_scroll.saturating_sub(1),
            KeyCode::Down => self.resume_scroll = self.resume_scroll.saturating_add(1),
            KeyCode::PageUp => self.resume_scroll = self.resume_scroll.saturating_sub(8),
            KeyCode::PageDown => self.resume_scroll = self.resume_scroll.saturating_add(8),
            KeyCode::Home => self.resume_scroll = 0,
            KeyCode::End => self.resume_scroll = usize::MAX,
            KeyCode::Enter => {
                let answer = if self.resume_fresh {
                    KeyCode::Char('n')
                } else {
                    KeyCode::Enter
                };
                self.app.answer_resume(KeyEvent::new(answer, modifiers));
            }
            _ => return false,
        }
        true
    }

    pub(super) fn resume_pointer(
        &mut self,
        action: &str,
        button: u8,
        x: u16,
        y: u16,
        pixel: Option<[u32; 2]>,
    ) -> bool {
        if self.app.over.resume.is_none() {
            return false;
        }
        let Some(surface) = &self.surface else {
            return false;
        };
        if !surface.hits.iter().any(|h| h.action == "resume:body") {
            return false;
        }
        let [px, py] = pixel.unwrap_or([
            u32::from(x) * self.viewport.width / u32::from(self.viewport.columns.max(1)),
            u32::from(y) * self.viewport.height / u32::from(self.viewport.rows.max(1)),
        ]);
        let hit = surface.hit(px as u16, py as u16).map(|h| h.action.clone());
        self.skin.hovered = hit.clone();
        match action {
            "scroll_up" => self.resume_scroll = self.resume_scroll.saturating_sub(3),
            "scroll_down" => self.resume_scroll = self.resume_scroll.saturating_add(3),
            "down" if button == 0 => match hit.as_deref() {
                Some("resume:accept") => {
                    self.app
                        .answer_resume(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
                Some("resume:fresh") => {
                    self.app
                        .answer_resume(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
                }
                _ => {}
            },
            _ => {}
        }
        true
    }

    /// A real editable filter, independent of the terminal settings grid.
    /// KIT owns text editing and horizontal scrolling; AMP owns filter meaning.
    pub(super) fn filter_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        if self.app.edit.typing.is_none() {
            return false;
        }
        self.overlay = true;
        self.app.bars.begin_frame();
        let width = 620.min(s.width - 32);
        let height = 238;
        let x = (s.width - width) / 2;
        let y = (s.height - height) / 2;
        s.hits.clear();
        s.hits.push(HitRegion {
            rect: R::new(0, 0, s.width, s.height),
            action: "filter:dismiss".into(),
        });
        self.frame(s, R::new(x, y, width, height), c, self.radius, false);
        s.hits.push(HitRegion {
            rect: R::new(x, y, width, height),
            action: "filter:body".into(),
        });
        classic::label(
            s,
            R::new(x + 20, y + 12, width - 80, 28),
            "FILTER PLAYLIST",
            &c.accent,
            FONT,
            true,
        );
        native::close_button(
            s,
            R::new(x + width - 44, y + 11, 30, 28),
            c,
            "filter:close",
            self.skin.hovered.as_deref() == Some("filter:close"),
        );
        classic::label(
            s,
            R::new(x + 20, y + 50, width - 40, 24),
            "Match words in artist, title, album, year or path.",
            &c.dim,
            FONT,
            false,
        );
        let field = R::new(x + 20, y + 86, width - 40, 36);
        s.fill(field, &c.inset, 2);
        s.nodes.push(Primitive::Border {
            rect: field,
            color: c.accent.clone(),
            radius: 2,
        });
        s.hits.push(HitRegion {
            rect: field,
            action: "filter:field".into(),
        });
        let area = Rect::new(0, 0, (field.width - 20) / ADVANCE as u16, 1);
        let mut buffer = starkit::ratatui::buffer::Buffer::empty(area);
        let input = self.app.edit.typing.as_mut().unwrap();
        let caret = input.render(area, &mut buffer, Style::default());
        if input.is_empty() {
            classic::label(
                s,
                R::new(field.x + 10, field.y + 8, field.width - 20, 20),
                "Type words to filter…",
                &c.dim,
                FONT,
                false,
            );
        } else {
            for column in 0..area.width {
                let symbol = buffer[(column, 0)].symbol();
                if symbol.trim().is_empty() {
                    continue;
                }
                classic::label(
                    s,
                    R::new(
                        field.x + 10 + column * ADVANCE as u16,
                        field.y + 8,
                        symbol.width().max(1) as u16 * ADVANCE as u16,
                        20,
                    ),
                    symbol,
                    &c.ink,
                    FONT,
                    false,
                );
            }
        }
        if let Some((column, _)) = caret {
            s.fill(
                R::new(field.x + 10 + column * ADVANCE as u16, field.y + 9, 1, 18),
                &c.accent,
                0,
            );
        }
        classic::label(
            s,
            R::new(x + 20, y + 132, width - 40, 24),
            format!(
                "{} of {} tracks · updates as you type",
                self.app.queue.matched_count,
                self.app.queue.items.len()
            ),
            if self.app.queue.matched_count == 0 {
                &c.accent
            } else {
                &c.dim
            },
            FONT,
            false,
        );
        classic::label(
            s,
            R::new(x + 20, y + 160, width - 40, 20),
            "Enter apply · Esc cancel · Ctrl+U clear",
            &c.dim,
            11,
            false,
        );
        for (left, text, action, button_width) in [
            (20, "Clear", "filter:clear", 86),
            (width - 226, "Cancel", "filter:cancel", 96),
            (width - 122, "Apply", "filter:apply", 102),
        ] {
            self.button(
                s,
                R::new(x + left, y + 190, button_width, 30),
                c,
                text,
                action,
                FONT,
                false,
            );
        }
        true
    }

    pub(super) fn filter_pointer(
        &mut self,
        action: &str,
        button: u8,
        x: u16,
        y: u16,
        pixel: Option<[u32; 2]>,
    ) -> bool {
        if self.app.edit.typing.is_none() {
            return false;
        }
        let Some(s) = &self.surface else {
            return false;
        };
        if !s.hits.iter().any(|h| h.action == "filter:body") {
            return false;
        }
        let [px, py] = pixel.unwrap_or([
            u32::from(x) * self.viewport.width / u32::from(self.viewport.columns.max(1)),
            u32::from(y) * self.viewport.height / u32::from(self.viewport.rows.max(1)),
        ]);
        let hit = s.hit(px as u16, py as u16).map(|h| h.action.clone());
        self.skin.hovered = hit.clone();
        if action == "down" && button == 0 {
            tracing::debug!(
                action = hit.as_deref().unwrap_or(""),
                "Graphical filter action"
            );
            match hit.as_deref() {
                Some("filter:apply") => {
                    self.app
                        .filter_type(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
                Some("filter:clear") => {
                    self.app.edit.typing.as_mut().unwrap().clear();
                    self.app.filter_changed();
                }
                Some("filter:cancel" | "filter:close" | "filter:dismiss") => {
                    self.app
                        .filter_type(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                }
                _ => {}
            }
        }
        true
    }

    /// Track actions are a compact context menu; destination/confirmation
    /// flows keep the roomier dialog layout below. Actions still use the same
    /// source row indices and dispatch as the ASCII menu.
    fn track_context_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        let Some(state) = &self.app.over.playlist_add else {
            return false;
        };
        let regular = matches!(state.step, PlaylistAddStep::Menu);
        if !regular && !matches!(state.step, PlaylistAddStep::ImportAlbum) {
            return false;
        }
        let (_, _, source_rows) = App::playlist_add_rows(state);
        let rows = if regular {
            [
                "Add to playlist",
                "Remove from playlist",
                "Create Gravity playlist",
                "Create artist playlist",
                "Create library timeline",
                "Why this track?",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        } else {
            source_rows
                .into_iter()
                .map(|r| {
                    let mut chars = r.label.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().to_string() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect()
        };
        let cursor = state.cursor;
        let width = ((rows.iter().map(|r| r.width()).max().unwrap_or(0) * ADVANCE + 56)
            .clamp(280, 400) as u16)
            .min(s.width - 12);
        let row_height = 30;
        let height = rows.len() as u16 * row_height + if regular { 18 } else { 0 } + 12;
        let [px, py] = self.context_anchor.unwrap_or([
            u32::from(s.width / 2) * u32::from(self.density),
            u32::from(s.height / 2) * u32::from(self.density),
        ]);
        let anchor_x = (px / u32::from(self.density)).min(u32::from(s.width)) as u16;
        let anchor_y = (py / u32::from(self.density)).min(u32::from(s.height)) as u16;
        let x = if u32::from(anchor_x) + 2 + u32::from(width) <= u32::from(s.width - 6) {
            anchor_x + 2
        } else {
            anchor_x.saturating_sub(width + 2)
        }
        .clamp(6, s.width - width - 6);
        let y = if u32::from(anchor_y) + u32::from(height) <= u32::from(s.height - 6) {
            anchor_y
        } else {
            anchor_y.saturating_sub(height)
        }
        .clamp(6, s.height - height - 6);
        let rect = R::new(x, y, width, height);
        let radius = self.radius.min(6);
        s.fill(R::new(x + 2, y + 3, width, height), &c.shadow, radius);
        s.fill(rect, &c.panel, radius);
        s.nodes.push(Primitive::Border {
            rect,
            color: native::hex(self.palette().border),
            radius,
        });
        s.hits.clear();
        s.hits.push(HitRegion {
            rect: R::new(0, 0, s.width, s.height),
            action: "popup:dismiss".into(),
        });
        s.hits.push(HitRegion {
            rect,
            action: "popup:body".into(),
        });
        let mut row_y = y + 6;
        for (index, text) in rows.iter().enumerate() {
            if regular && matches!(index, 2 | 5) {
                s.fill(R::new(x + 12, row_y + 4, width - 24, 1), &c.raised, 0);
                row_y += 9;
            }
            let row = R::new(x + 5, row_y, width - 10, row_height);
            if index == cursor {
                s.fill(row, &native::hex(self.palette().selected), 3);
            }
            classic::label(
                s,
                R::new(x + 16, row_y + 5, width - 50, LINE),
                text,
                &c.ink,
                FONT,
                false,
            );
            if regular && matches!(index, 0 | 4) {
                classic::label(
                    s,
                    R::new(x + width - 28, row_y + 5, 16, LINE),
                    "›",
                    &c.dim,
                    FONT,
                    true,
                );
            }
            s.hits.push(HitRegion {
                rect: row,
                action: format!("popup:row:{index}"),
            });
            row_y += row_height;
        }
        self.overlay = true;
        self.app.bars.begin_frame();
        true
    }

    pub(super) fn list_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        // Text-entry dialogs take precedence over the list that opened them.
        if self.app.edit.eq_value.is_some()
            || self.app.edit.auth.is_some()
            || self.app.edit.typing.is_some()
            || self.app.edit.naming.is_some()
        {
            return false;
        }
        if self.track_context_popup(s, c) {
            return true;
        }
        let (heading, title, rows, cursor, mut scroll) =
            if let Some(state) = &self.app.over.playlist_add {
                let (heading, title, rows) = App::playlist_add_rows(state);
                (heading, title, rows, state.cursor, state.scroll)
            } else if let Some(state) = &self.app.over.settings {
                (
                    state.kind.heading(),
                    state.title.clone(),
                    state.rows.clone(),
                    state.cursor,
                    state.scroll,
                )
            } else {
                return false;
            };
        self.overlay = true;
        self.app.bars.begin_frame();
        let longest = rows
            .iter()
            .map(|r| r.label.width() + r.value.width() + 5)
            .chain([title.width(), 54])
            .max()
            .unwrap_or(54);
        let width = ((longest * ADVANCE + 48).min(900) as u16).min(s.width - 32);
        let inner = width - 40;
        let label_width = (inner * 2 / 5).saturating_sub(12);
        let value_width = inner - label_width - 20;
        let title = wrap(&title, usize::from(inner) / ADVANCE);
        let header = 42 + title.len() as u16 * LINE + 12;
        let available = s.height.saturating_sub(header + 76);
        let wrapped: Vec<_> = rows
            .iter()
            .map(|row| {
                let labels = wrap(
                    &row.label,
                    usize::from(if row.value.is_empty() {
                        inner
                    } else {
                        label_width
                    }) / ADVANCE,
                );
                let values = if row.value.is_empty() {
                    Vec::new()
                } else {
                    wrap(&row.value, usize::from(value_width) / ADVANCE)
                };
                let height = labels.len().max(values.len()) as u16 * LINE + 12;
                (labels, values, height)
            })
            .collect();
        scroll = scroll.min(cursor).min(rows.len().saturating_sub(1));
        while scroll < cursor
            && wrapped[scroll..=cursor]
                .iter()
                .map(|r| u32::from(r.2))
                .sum::<u32>()
                > u32::from(available)
        {
            scroll += 1;
        }
        let mut used = 0;
        let mut end = scroll;
        while end < wrapped.len() && used + wrapped[end].2 <= available {
            used += wrapped[end].2;
            end += 1;
        }
        if let Some(state) = &mut self.app.over.playlist_add {
            state.scroll = scroll;
        } else if let Some(state) = &mut self.app.over.settings {
            state.scroll = scroll;
        }
        let height = header + used + 44;
        let x = (s.width - width) / 2;
        let y = (s.height - height) / 2;
        s.hits.clear();
        s.hits.push(HitRegion {
            rect: R::new(0, 0, s.width, s.height),
            action: "popup:dismiss".into(),
        });
        let rect = R::new(x, y, width, height);
        self.frame(s, rect, c, self.radius, true);
        s.hits.push(HitRegion {
            rect,
            action: "popup:body".into(),
        });
        classic::label(
            s,
            R::new(x + 20, y + 10, width - 70, 26),
            heading,
            &c.accent,
            FONT,
            true,
        );
        native::close_button(
            s,
            R::new(x + width - 44, y + 9, 30, 28),
            c,
            "popup:close",
            self.skin.hovered.as_deref() == Some("popup:close"),
        );
        for (i, line) in title.iter().enumerate() {
            classic::label(
                s,
                R::new(x + 20, y + 42 + i as u16 * LINE, inner, LINE),
                line,
                &c.dim,
                FONT,
                false,
            );
        }
        let mut row_y = y + header;
        for (i, (labels, values, height)) in wrapped.iter().enumerate().take(end).skip(scroll) {
            let rect = R::new(x + 12, row_y, width - 24, *height);
            if i == cursor {
                s.fill(rect, &native::hex(self.palette().selected), 3);
            }
            for (line, text) in labels.iter().enumerate() {
                classic::label(
                    s,
                    R::new(
                        x + 20,
                        row_y + 6 + line as u16 * LINE,
                        if values.is_empty() {
                            inner
                        } else {
                            label_width
                        },
                        LINE,
                    ),
                    text,
                    &c.ink,
                    FONT,
                    false,
                );
            }
            for (line, text) in values.iter().enumerate() {
                classic::label(
                    s,
                    R::new(
                        x + 20 + label_width + 20,
                        row_y + 6 + line as u16 * LINE,
                        value_width,
                        LINE,
                    ),
                    text,
                    &c.accent,
                    FONT,
                    false,
                );
            }
            s.hits.push(HitRegion {
                rect,
                action: format!("popup:row:{i}"),
            });
            row_y += height;
        }
        let footer = if scroll > 0 || end < rows.len() {
            format!(
                "{}–{} / {} · scroll or ↑↓ · Enter choose · Esc close",
                scroll + 1,
                end,
                rows.len()
            )
        } else {
            "Click to choose · Enter choose · Esc close".into()
        };
        classic::label(
            s,
            R::new(x + 20, row_y + 10, inner, 24),
            footer,
            &c.dim,
            11,
            false,
        );
        true
    }

    pub(super) fn popup_pointer(
        &mut self,
        action: &str,
        button: u8,
        x: u16,
        y: u16,
        pixel: Option<[u32; 2]>,
    ) -> bool {
        let Some(s) = &self.surface else { return false };
        if !s.hits.iter().any(|h| h.action == "popup:body") {
            return false;
        }
        let [px, py] = pixel.unwrap_or([
            u32::from(x) * self.viewport.width / u32::from(self.viewport.columns.max(1)),
            u32::from(y) * self.viewport.height / u32::from(self.viewport.rows.max(1)),
        ]);
        let hit = s.hit(px as u16, py as u16).map(|h| h.action.clone());
        if action == "move" {
            self.skin.hovered = hit.clone();
            if let Some(state) = &mut self.app.over.playlist_add {
                if matches!(
                    state.step,
                    PlaylistAddStep::Menu | PlaylistAddStep::ImportAlbum
                ) {
                    if let Some(index) = hit
                        .as_deref()
                        .and_then(|h| h.strip_prefix("popup:row:"))
                        .and_then(|n| n.parse().ok())
                    {
                        state.cursor = index;
                    }
                }
            }
        } else if action == "scroll_up" || action == "scroll_down" {
            let delta = if action == "scroll_up" { -1 } else { 1 };
            if self.app.over.playlist_add.is_some() {
                self.app.move_playlist_add(delta);
            } else {
                self.app.move_settings(delta);
            }
        } else if action == "down" && button == 0 {
            if let Some(index) = hit
                .as_deref()
                .and_then(|h| h.strip_prefix("popup:row:"))
                .and_then(|n| n.parse().ok())
            {
                tracing::debug!(index, "Graphical popup row activated");
                if let Some(state) = &mut self.app.over.playlist_add {
                    state.cursor = index;
                    self.app.take_playlist_add();
                } else if let Some(state) = &mut self.app.over.settings {
                    state.cursor = index;
                    self.app.take_setting();
                }
            } else if matches!(hit.as_deref(), Some("popup:dismiss" | "popup:close")) {
                self.app.handle(Action::CloseOverlay);
            }
        }
        true
    }
}
