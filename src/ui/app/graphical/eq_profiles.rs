//! Native presentation for EQ profile management and exact-value forms.
use super::*;

impl Rack<'_> {
    pub(super) fn eq_value_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        let Some(input) = &self.app.edit.eq_value else {
            return false;
        };
        let (heading, hint) = match input.field {
            EqField::SaveAsProfile => (
                "SAVE EQ PROFILE AS",
                "Name a saved copy of the current curve.",
            ),
            EqField::NewProfile => (
                "NEW EQ PROFILE",
                "Start a named profile with a neutral peaking filter.",
            ),
            EqField::RenameProfile => (
                "RENAME EQ PROFILE",
                "Choose a new name for this saved profile.",
            ),
            EqField::Frequency => ("FREQUENCY", "Enter frequency in Hz."),
            EqField::Gain => ("GAIN", "Enter gain in dB."),
            EqField::Width => (
                "FILTER WIDTH",
                "Enter Q, bandwidth or slope for this filter.",
            ),
            EqField::GraphicPoints => (
                "GRAPHIC EQ POINTS",
                "Enter frequency gain; frequency gain; …",
            ),
            EqField::IirCoefficients => (
                "FILTER COEFFICIENTS",
                "Enter the b coefficients, followed by the a coefficients.",
            ),
        };
        let value = input.value.clone();
        let error = input.error.clone();
        let width = 760.min(s.width - 32);
        let lines = super::popup::wrap(&format!("{value}_"), usize::from(width - 72) / 8);
        let visible = (lines.len().min(6) as u16).min((s.height.saturating_sub(276) / 20).max(1));
        let height = (244 + visible * 20).min(s.height - 32);
        let x = (s.width - width) / 2;
        let y = (s.height - height) / 2;
        self.overlay = true;
        self.app.bars.begin_frame();
        s.hits.clear();
        self.frame(s, R::new(x, y, width, height), c, self.radius, false);
        s.hits.push(HitRegion {
            rect: R::new(x, y, width, height),
            action: "eq-form:body".into(),
        });
        classic::label(
            s,
            R::new(x + 20, y + 12, width - 64, 28),
            heading,
            &c.accent,
            12,
            true,
        );
        native::close_button(
            s,
            R::new(x + width - 44, y + 10, 30, 28),
            c,
            "eq-form:cancel",
            false,
        );
        classic::label(
            s,
            R::new(x + 20, y + 48, width - 40, 24),
            hint,
            &c.dim,
            12,
            false,
        );
        let field = R::new(x + 20, y + 84, width - 40, visible * 20 + 16);
        self.frame(s, field, c, 0, true);
        for (i, line) in lines.iter().take(usize::from(visible)).enumerate() {
            classic::label(
                s,
                R::new(
                    field.x + 12,
                    field.y + 8 + i as u16 * 20,
                    field.width - 24,
                    20,
                ),
                line,
                &c.ink,
                12,
                false,
            );
        }
        if !error.is_empty() {
            for (i, line) in super::popup::wrap(&error, usize::from(width - 40) / 8)
                .iter()
                .take(2)
                .enumerate()
            {
                classic::label(
                    s,
                    R::new(
                        x + 20,
                        field.y + field.height + 10 + i as u16 * 20,
                        width - 40,
                        20,
                    ),
                    line,
                    &c.accent,
                    11,
                    false,
                );
            }
        }
        classic::label(
            s,
            R::new(x + 20, y + height - 76, width - 40, 24),
            "Ctrl+U clear · Enter apply · Esc cancel",
            &c.dim,
            11,
            false,
        );
        self.button(
            s,
            R::new(x + width - 228, y + height - 42, 96, 29),
            c,
            "CANCEL",
            "eq-form:cancel",
            12,
            false,
        );
        self.button(
            s,
            R::new(x + width - 120, y + height - 42, 100, 29),
            c,
            if matches!(
                input.field,
                EqField::SaveAsProfile | EqField::NewProfile | EqField::RenameProfile
            ) {
                "SAVE"
            } else {
                "APPLY"
            },
            "eq-form:accept",
            12,
            false,
        );
        true
    }

    pub(super) fn eq_library_popup(&mut self, s: &mut Surface, c: &Colors) -> bool {
        let Some(lib) = &self.app.edit.eq_profiles else {
            return false;
        };
        let indices = lib.matches(&self.app.eq);
        let cursor = lib.cursor.min(indices.len().saturating_sub(1));
        let query = lib.query.clone();
        let error = lib.error.clone();
        let confirmation = lib.confirmation;
        let width = 920.min(s.width - 32);
        let height = 530.min(s.height - 32);
        let visible = usize::from((height - 218) / 32);
        let scroll = lib
            .scroll
            .min(cursor)
            .max(cursor.saturating_sub(visible.saturating_sub(1)));
        let x = (s.width - width) / 2;
        let y = (s.height - height) / 2;
        let list_width = (width * 3 / 5).saturating_sub(36);
        let detail_x = x + list_width + 44;
        let detail_width = width - list_width - 64;
        let active_name = self.app.eq.active().name.clone();
        let active_status = self.app.eq.profile_status(self.app.eq.profile);
        self.overlay = true;
        self.app.bars.begin_frame();
        s.hits.clear();
        self.frame(s, R::new(x, y, width, height), c, self.radius, false);
        s.hits.push(HitRegion {
            rect: R::new(0, 0, s.width, s.height),
            action: "eq-library:dismiss".into(),
        });
        s.hits.push(HitRegion {
            rect: R::new(x, y, width, height),
            action: "eq-library:body".into(),
        });
        classic::label(
            s,
            R::new(x + 20, y + 12, width - 70, 28),
            "EQ PROFILES",
            &c.accent,
            12,
            true,
        );
        native::close_button(
            s,
            R::new(x + width - 44, y + 10, 30, 28),
            c,
            "eq-library:close",
            false,
        );
        classic::label(
            s,
            R::new(x + 20, y + 44, width - 40, 24),
            format!("Current: {active_name} · {active_status}"),
            &c.dim,
            11,
            false,
        );
        self.frame(s, R::new(x + 20, y + 82, width - 40, 32), c, 0, true);
        classic::label(
            s,
            R::new(x + 32, y + 85, width - 64, 26),
            format!("Search: {query}_"),
            &c.ink,
            12,
            false,
        );
        s.fill(
            R::new(x + list_width + 32, y + 130, 1, height - 234),
            &c.shadow,
            0,
        );
        if indices.is_empty() {
            classic::label(
                s,
                R::new(x + 20, y + 136, list_width, 48),
                "No matching profiles",
                &c.dim,
                12,
                false,
            );
        }
        for (row, &index) in indices.iter().enumerate().skip(scroll).take(visible) {
            let rect = R::new(
                x + 12,
                y + 130 + (row - scroll) as u16 * 32,
                list_width + 12,
                32,
            );
            if row == cursor {
                s.fill(rect, &native::hex(self.palette().selected), 2);
            }
            let prefix = if index == self.app.eq.profile {
                "› "
            } else {
                "  "
            };
            let marker = if self.app.eq.modified(index) {
                " *"
            } else {
                ""
            };
            let name = format!("{prefix}{}{marker}", self.app.eq.profiles[index].name);
            classic::label(
                s,
                R::new(rect.x + 8, rect.y, rect.width - 16, 32),
                name,
                if index == self.app.eq.profile {
                    &c.accent
                } else {
                    &c.ink
                },
                12,
                row == cursor,
            );
            s.hits.push(HitRegion {
                rect,
                action: format!("eq-library:row:{row}"),
            });
        }
        if let Some(&index) = indices.get(cursor) {
            let profile = &self.app.eq.profiles[index];
            let bands = profile
                .stages
                .iter()
                .filter(|st| !matches!(st.filter, Filter::Preamp { .. }))
                .count();
            let enabled = profile
                .stages
                .iter()
                .filter(|st| st.enabled && !matches!(st.filter, Filter::Preamp { .. }))
                .count();
            let preamp: f64 = profile
                .stages
                .iter()
                .filter_map(|st| {
                    if let Filter::Preamp { gain_db } = st.filter {
                        Some(gain_db)
                    } else {
                        None
                    }
                })
                .sum();
            let details = [
                profile.name.clone(),
                self.app.eq.profile_status(index).to_owned(),
                format!("{bands} bands · {enabled} enabled"),
                format!("Preamp {preamp:+.1} dB"),
                if self.app.eq.bundled(index) {
                    "Built-in profiles stay intact. Save As creates your own version.".into()
                } else {
                    "Load this profile to edit it. Save updates its saved curve.".into()
                },
            ];
            let mut row_y = y + 132;
            for (i, text) in details.iter().enumerate() {
                for line in super::popup::wrap(text, usize::from(detail_width) / 8)
                    .iter()
                    .take(3)
                {
                    if row_y + 20 > y + height - 128 {
                        break;
                    }
                    classic::label(
                        s,
                        R::new(detail_x, row_y, detail_width, 20),
                        line,
                        if i == 0 { &c.accent } else { &c.dim },
                        12,
                        i == 0,
                    );
                    row_y += 20;
                }
                row_y += 8;
            }
        }
        let note = if !error.is_empty() {
            error
        } else {
            format!(
                "{} profiles · unsaved drafts are retained when switching",
                indices.len()
            )
        };
        classic::label(
            s,
            R::new(x + 20, y + height - 114, width - 40, 22),
            note,
            &c.dim,
            10,
            false,
        );
        for (offset, caption, action, button_width) in [
            (20, "NEW", "new", 60),
            (88, "RENAME", "rename", 80),
            (176, "DELETE", "delete", 80),
            (width - 340, "SAVE", "save", 64),
            (width - 268, "SAVE AS", "save-as", 88),
            (width - 172, "LOAD", "load", 152),
        ] {
            self.button(
                s,
                R::new(x + offset, y + height - 80, button_width, 29),
                c,
                caption,
                &format!("eq-library:{action}"),
                12,
                false,
            );
        }
        self.button(
            s,
            R::new(x + 20, y + height - 42, 80, 29),
            c,
            "IMPORT",
            "eq-library:import",
            12,
            false,
        );
        self.button(
            s,
            R::new(x + 108, y + height - 42, 80, 29),
            c,
            "EXPORT",
            "eq-library:export",
            12,
            false,
        );
        self.button(
            s,
            R::new(x + 196, y + height - 42, 80, 29),
            c,
            "REVERT",
            "eq-library:revert",
            12,
            false,
        );
        classic::label(
            s,
            R::new(x + 300, y + height - 42, width - 320, 29),
            "Enter load · Ctrl+S save · Esc close",
            &c.dim,
            10,
            false,
        );
        if let Some(lib) = &mut self.app.edit.eq_profiles {
            lib.scroll = scroll;
            lib.cursor = cursor;
        }
        if let Some(confirm) = confirmation {
            let index = confirm.index();
            let confirm_width = width - 48;
            let confirm_height = 180;
            let cx = x + 24;
            let cy = y + (height - confirm_height) / 2;
            s.hits.clear();
            self.frame(
                s,
                R::new(cx, cy, confirm_width, confirm_height),
                c,
                self.radius,
                false,
            );
            s.hits.push(HitRegion {
                rect: R::new(0, 0, s.width, s.height),
                action: "eq-library:body".into(),
            });
            classic::label(
                s,
                R::new(cx + 20, cy + 14, confirm_width - 40, 28),
                if confirm.reverting() {
                    "DISCARD UNSAVED EQ EDITS?"
                } else {
                    "DELETE EQ PROFILE?"
                },
                &c.accent,
                12,
                true,
            );
            let name = &self.app.eq.profiles[index].name;
            classic::label(
                s,
                R::new(cx + 20, cy + 50, confirm_width - 40, 26),
                name,
                &c.ink,
                12,
                true,
            );
            classic::label(
                s,
                R::new(cx + 20, cy + 80, confirm_width - 40, 26),
                if confirm.reverting() {
                    "Restores the saved curve and removes this draft."
                } else {
                    "Deletes the saved profile and its unsaved draft."
                },
                &c.dim,
                11,
                false,
            );
            self.button(
                s,
                R::new(cx + confirm_width - 228, cy + 130, 96, 29),
                c,
                "CANCEL",
                "eq-library:cancel-delete",
                12,
                false,
            );
            self.button(
                s,
                R::new(cx + confirm_width - 120, cy + 130, 100, 29),
                c,
                if confirm.reverting() {
                    "REVERT"
                } else {
                    "DELETE"
                },
                if confirm.reverting() {
                    "eq-library:confirm-revert"
                } else {
                    "eq-library:confirm-delete"
                },
                12,
                false,
            );
        }
        true
    }

    pub(super) fn eq_profile_pointer(
        &mut self,
        action: &str,
        button: u8,
        x: u16,
        y: u16,
        pixel: Option<[u32; 2]>,
    ) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        if !surface
            .hits
            .iter()
            .any(|h| h.action == "eq-form:body" || h.action == "eq-library:body")
        {
            return false;
        }
        let [px, py] = pixel.unwrap_or([
            u32::from(x) * self.viewport.width / u32::from(self.viewport.columns.max(1)),
            u32::from(y) * self.viewport.height / u32::from(self.viewport.rows.max(1)),
        ]);
        let hit = surface.hit(px as u16, py as u16).map(|h| h.action.clone());
        if action == "down" && button == 0 {
            if let Some(hit) = hit {
                if let Some(command) = hit.strip_prefix("eq-form:") {
                    match command {
                        "accept" => {
                            self.app
                                .eq_value_type(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                        "cancel" => {
                            self.app
                                .eq_value_type(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                        }
                        _ => {}
                    }
                } else if let Some(row) = hit
                    .strip_prefix("eq-library:row:")
                    .and_then(|v| v.parse().ok())
                {
                    if let Some(lib) = &mut self.app.edit.eq_profiles {
                        lib.cursor = row;
                        lib.error.clear();
                    }
                } else if let Some(command) = hit.strip_prefix("eq-library:") {
                    self.app.eq_library_action(command);
                }
            }
        } else if matches!(action, "scroll_up" | "scroll_down")
            && self.app.edit.eq_profiles.is_some()
        {
            self.app.eq_library_key(KeyEvent::new(
                if action == "scroll_up" {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                },
                KeyModifiers::NONE,
            ));
        } else if action == "move" {
            self.skin.hovered = hit;
        }
        true
    }
}
