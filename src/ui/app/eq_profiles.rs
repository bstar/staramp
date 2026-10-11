//! EQ profile library and recoverable working edits, shared by both presentations.
use super::*;
use crate::audio::dsp::apo::{Filter, Profile};
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) enum Confirmation {
    Delete(usize),
    Revert(usize),
}
impl Confirmation {
    pub fn index(self) -> usize {
        match self {
            Self::Delete(i) | Self::Revert(i) => i,
        }
    }
    pub fn reverting(self) -> bool {
        matches!(self, Self::Revert(_))
    }
}

#[derive(Default)]
pub(super) struct Library {
    pub query: String,
    pub cursor: usize,
    pub scroll: usize,
    pub confirmation: Option<Confirmation>,
    pub error: String,
}
impl Library {
    pub fn matches(&self, eq: &EqState) -> Vec<usize> {
        let words = self.query.to_lowercase();
        let mut rows: Vec<_> = eq
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                words
                    .split_whitespace()
                    .all(|word| p.name.to_lowercase().contains(word))
            })
            .map(|(index, _)| index)
            .collect();
        rows.sort_by_key(|&index| (eq.bundled(index), eq.profiles[index].name.to_lowercase()));
        rows
    }
    pub fn selected(&self, eq: &EqState) -> Option<usize> {
        self.matches(eq).get(self.cursor).copied()
    }
}

#[derive(Serialize, Deserialize)]
struct Draft {
    profile: Profile,
    base: Option<Profile>,
}
#[derive(Serialize, Deserialize)]
struct Drafts {
    version: u8,
    active: String,
    drafts: Vec<Draft>,
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || name.chars().count() > 128
    {
        bail!("Use a name of 1–128 characters, without slashes or control characters.");
    }
    Ok(())
}

impl EqState {
    pub(super) fn bundled(&self, index: usize) -> bool {
        let name = &self.profiles[index].name;
        !self.managed_names.contains(&name.to_lowercase())
            && eq::PRESETS
                .iter()
                .any(|preset| preset.name.eq_ignore_ascii_case(name))
    }
    pub(super) fn modified(&self, index: usize) -> bool {
        let profile = &self.profiles[index];
        self.saved_profiles.get(&profile.name.to_lowercase()) != Some(profile)
    }
    pub(super) fn profile_status(&self, index: usize) -> &'static str {
        if self.modified(index) {
            "Unsaved edits"
        } else if self.bundled(index) {
            "Built-in"
        } else if self
            .managed_names
            .contains(&self.profiles[index].name.to_lowercase())
        {
            "Saved"
        } else {
            "Unsaved profile"
        }
    }
    pub(super) fn select_first_band(&mut self) {
        self.stage = self
            .active()
            .stages
            .iter()
            .position(|s| !matches!(s.filter, Filter::Preamp { .. }))
            .unwrap_or(0);
        self.scroll = 0;
    }
    pub(super) fn band_number(&self, stage: usize) -> usize {
        self.active()
            .stages
            .iter()
            .take(stage + 1)
            .filter(|s| !matches!(s.filter, Filter::Preamp { .. }))
            .count()
    }
    fn draft_file() -> Result<PathBuf> {
        Ok(crate::paths::equalizer_dir()?.join("working-edits.json"))
    }
    pub(super) fn persist_drafts(&self) -> Result<()> {
        self.persist_drafts_at(&Self::draft_file()?)
    }
    fn persist_drafts_at(&self, path: &Path) -> Result<()> {
        let drafts = self
            .profiles
            .iter()
            .enumerate()
            .filter(|(i, _)| self.modified(*i))
            .map(|(_, p)| Draft {
                profile: p.clone(),
                base: self.saved_profiles.get(&p.name.to_lowercase()).cloned(),
            })
            .collect();
        let state = Drafts {
            version: 1,
            active: self.active().name.clone(),
            drafts,
        };
        std::fs::create_dir_all(path.parent().context("EQ drafts directory")?)?;
        starkit::fs::write_private(path, &serde_json::to_vec(&state)?)?;
        Ok(())
    }
    pub(super) fn restore_drafts(&mut self) {
        if let Ok(path) = Self::draft_file() {
            self.restore_drafts_from(&path);
        }
    }
    fn restore_drafts_from(&mut self, path: &Path) {
        let load = || -> Result<Drafts> {
            if std::fs::metadata(path)?.len() > 4 * 1024 * 1024 {
                bail!("EQ drafts exceed size limit");
            }
            let data: Drafts = serde_json::from_slice(&std::fs::read(path)?)?;
            if data.version != 1 || data.drafts.len() > 256 {
                bail!("unsupported EQ drafts");
            }
            let mut names = std::collections::BTreeSet::new();
            for draft in &data.drafts {
                if !names.insert(draft.profile.name.to_lowercase()) {
                    bail!("duplicate EQ draft name");
                }
                valid_name(&draft.profile.name)?;
                draft.profile.validate()?;
                if let Some(base) = &draft.base {
                    if base.name != draft.profile.name {
                        bail!("EQ draft baseline name differs");
                    }
                    base.validate()?;
                }
            }
            Ok(data)
        };
        let Ok(data) = load() else { return };
        for draft in data.drafts {
            let key = draft.profile.name.to_lowercase();
            if let Some(base) = draft.base {
                self.saved_profiles.insert(key, base);
            } else {
                self.saved_profiles.remove(&key);
            }
            if let Some(at) = self
                .profiles
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(&draft.profile.name))
            {
                self.profiles[at] = draft.profile;
            } else {
                self.profiles.push(draft.profile);
            }
        }
        if let Some(at) = self
            .profiles
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(&data.active))
        {
            self.profile = at;
        }
    }
    pub(super) fn remember_saved(&mut self, index: usize) {
        let p = self.profiles[index].clone();
        self.managed_names.insert(p.name.to_lowercase());
        self.saved_profiles.insert(p.name.to_lowercase(), p);
    }
    fn managed_source(&self, index: usize) -> Result<PathBuf> {
        let txt = self.profiles[index].managed_path()?;
        if !txt.exists() && txt.with_extension("apo").exists() {
            Ok(txt.with_extension("apo"))
        } else {
            Ok(txt)
        }
    }
    fn check_saved_source(&self, index: usize) -> Result<PathBuf> {
        let path = self.managed_source(index)?;
        if Profile::parse_file(&path).ok().as_ref()
            != self
                .saved_profiles
                .get(&self.profiles[index].name.to_lowercase())
        {
            bail!("Saved file changed elsewhere. Use Save As to keep both versions.");
        }
        Ok(path)
    }
    pub(super) fn unique_name(&self, base: &str) -> String {
        let mut number = 1;
        loop {
            let name = if number == 1 {
                base.to_owned()
            } else {
                format!("{base} {number}")
            };
            let exists = self
                .profiles
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&name));
            let path = Profile {
                name: name.clone(),
                stages: vec![],
            }
            .managed_path();
            if !exists && path.is_ok_and(|p| !p.exists()) {
                return name;
            }
            number += 1;
        }
    }
}

impl App {
    pub(super) fn open_eq_profiles(&mut self) {
        self.over.settings = None;
        let mut library = Library::default();
        library.cursor = library
            .matches(&self.eq)
            .iter()
            .position(|i| *i == self.eq.profile)
            .unwrap_or(0);
        self.edit.eq_profiles = Some(library);
    }
    pub(super) fn select_eq_profile(&mut self, index: usize) {
        if index >= self.eq.profiles.len() {
            return;
        }
        self.eq.profile = index;
        self.eq.select_first_band();
        self.eq.enabled = true;
        self.apply_eq();
        if let Err(e) = self.eq.persist_drafts() {
            self.note(format!("Could not retain EQ edits: {e}"));
        } else {
            self.note(format!("Loaded {}", self.eq.active().name));
        }
    }
    pub(super) fn save_eq_profile(&mut self) {
        if self.eq.bundled(self.eq.profile) {
            self.begin_eq_name(EqField::SaveAsProfile);
            return;
        }
        let save = || -> Result<()> {
            self.eq.active().validate()?;
            if self.eq.active().stages.is_empty() {
                bail!("Add a filter before saving this profile.");
            }
            let path = self.eq.managed_source(self.eq.profile)?;
            if path.exists() {
                self.eq.check_saved_source(self.eq.profile)?;
                self.eq.active().save_managed_at(&path)?;
            } else {
                self.eq.active().save_new_managed()?;
            }
            Ok(())
        };
        match save() {
            Ok(()) => {
                self.eq.remember_saved(self.eq.profile);
                let result = self.eq.persist_drafts();
                self.note(match result {
                    Ok(()) => format!("Saved {}", self.eq.active().name),
                    Err(e) => format!("Profile saved; could not update draft recovery: {e}"),
                });
            }
            Err(e) => {
                if let Some(lib) = &mut self.edit.eq_profiles {
                    lib.error = e.to_string();
                }
                self.note(format!("Could not save EQ profile: {e}"));
            }
        }
    }
    pub(super) fn begin_eq_name(&mut self, field: EqField) {
        self.begin_eq_name_for(field, self.eq.profile);
    }
    fn begin_eq_name_for(&mut self, field: EqField, target: usize) {
        let value = match field {
            EqField::NewProfile => self.eq.unique_name("New profile"),
            EqField::SaveAsProfile => self
                .eq
                .unique_name(&format!("{} copy", self.eq.profiles[target].name)),
            _ => self.eq.profiles[target].name.clone(),
        };
        if matches!(field, EqField::RenameProfile) && self.eq.bundled(target) {
            self.note(
                "Built-in profiles stay intact. Choose Save As to name your own version.".into(),
            );
            return;
        }
        self.over.settings = None;
        self.edit.eq_profiles = None;
        self.edit.eq_value = Some(EqInput {
            field,
            value,
            target,
            error: String::new(),
        });
    }
    pub(super) fn commit_eq_name(
        &mut self,
        field: EqField,
        name: &str,
        target: usize,
    ) -> Result<()> {
        valid_name(name)?;
        let renaming = matches!(field, EqField::RenameProfile);
        if renaming && name == self.eq.profiles[target].name {
            return Ok(());
        }
        if self
            .eq
            .profiles
            .iter()
            .enumerate()
            .any(|(i, p)| (!renaming || i != target) && p.name.eq_ignore_ascii_case(name))
        {
            bail!("A profile already has that name. Choose a different name.");
        }
        let mut profile = if matches!(field, EqField::NewProfile) {
            Profile {
                name: name.into(),
                stages: vec![
                    crate::audio::dsp::apo::Stage {
                        enabled: true,
                        channels: crate::audio::dsp::apo::ChannelMask::ALL,
                        filter: Filter::Preamp { gain_db: 0. },
                    },
                    crate::audio::dsp::apo::Stage {
                        enabled: true,
                        channels: crate::audio::dsp::apo::ChannelMask::ALL,
                        filter: Filter::Biquad {
                            kind: crate::audio::dsp::apo::BiquadKind::Peaking,
                            frequency: 1000.,
                            gain_db: 0.,
                            width: crate::audio::dsp::apo::Width::Q(1.),
                            corner_frequency: false,
                        },
                    },
                ],
            }
        } else {
            self.eq.profiles[target].clone()
        };
        profile.name = name.into();
        profile.validate()?;
        let old = self.eq.profiles[target].clone();
        let old_path = if renaming && self.eq.managed_names.contains(&old.name.to_lowercase()) {
            Some(self.eq.check_saved_source(target)?)
        } else {
            None
        };
        let new_path = profile.save_new_managed()?;
        if renaming {
            if let Some(old_path) = old_path {
                if let Err(e) = std::fs::remove_file(old_path) {
                    let _ = std::fs::remove_file(new_path);
                    return Err(e).context("Could not rename the saved profile");
                }
            }
            self.eq.saved_profiles.remove(&old.name.to_lowercase());
            self.eq.managed_names.remove(&old.name.to_lowercase());
            self.eq.profiles[target] = profile;
        } else {
            self.eq.profiles.push(profile);
            self.eq.profile = self.eq.profiles.len() - 1;
        }
        self.eq
            .remember_saved(if renaming { target } else { self.eq.profile });
        self.eq.select_first_band();
        self.apply_eq();
        self.eq.persist_drafts()?;
        self.note(format!("Saved {}", self.eq.active().name));
        Ok(())
    }
    pub(super) fn eq_library_action(&mut self, action: &str) {
        let index = self
            .edit
            .eq_profiles
            .as_ref()
            .and_then(|lib| lib.selected(&self.eq));
        match action {
            "close" | "dismiss" => self.edit.eq_profiles = None,
            "load" => {
                if let Some(i) = index {
                    self.select_eq_profile(i);
                    self.edit.eq_profiles = None;
                }
            }
            "save" => self.save_eq_profile(),
            "save-as" => self.begin_eq_name(EqField::SaveAsProfile),
            "new" => self.begin_eq_name(EqField::NewProfile),
            "rename" => {
                if let Some(i) = index {
                    if self.eq.bundled(i) {
                        self.note(
                            "Built-in profiles cannot be renamed. Load it and choose Save As."
                                .into(),
                        );
                    } else {
                        self.begin_eq_name_for(EqField::RenameProfile, i);
                    }
                }
            }
            "revert" => {
                if let (Some(i), Some(lib)) = (index, &mut self.edit.eq_profiles) {
                    if self.eq.modified(i)
                        && self
                            .eq
                            .saved_profiles
                            .contains_key(&self.eq.profiles[i].name.to_lowercase())
                    {
                        lib.confirmation = Some(Confirmation::Revert(i));
                    } else {
                        lib.error =
                            "There are no saved edits to revert to for this profile.".into();
                    }
                }
            }
            "confirm-revert" => {
                if let Some(Confirmation::Revert(i)) = self
                    .edit
                    .eq_profiles
                    .as_ref()
                    .and_then(|lib| lib.confirmation)
                {
                    if let Some(saved) = self
                        .eq
                        .saved_profiles
                        .get(&self.eq.profiles[i].name.to_lowercase())
                        .cloned()
                    {
                        self.eq.profiles[i] = saved;
                        self.eq.select_first_band();
                        self.apply_eq();
                        if let Err(e) = self.eq.persist_drafts() {
                            self.note(format!("Reverted; draft recovery update failed: {e}"));
                        }
                        if let Some(lib) = &mut self.edit.eq_profiles {
                            lib.confirmation = None;
                        }
                    }
                }
            }
            "delete" => {
                if let (Some(i), Some(lib)) = (index, &mut self.edit.eq_profiles) {
                    if !self.eq.bundled(i) {
                        lib.confirmation = Some(Confirmation::Delete(i));
                    } else {
                        lib.error = "Built-in profiles cannot be deleted.".into();
                    }
                }
            }
            "cancel-delete" => {
                if let Some(lib) = &mut self.edit.eq_profiles {
                    lib.confirmation = None;
                }
            }
            "confirm-delete" => {
                if let Some(Confirmation::Delete(i)) = self
                    .edit
                    .eq_profiles
                    .as_ref()
                    .and_then(|lib| lib.confirmation)
                {
                    if let Err(e) = self.delete_eq_profile(i) {
                        if let Some(lib) = &mut self.edit.eq_profiles {
                            lib.error = e.to_string();
                        }
                    } else if let Some(lib) = &mut self.edit.eq_profiles {
                        lib.confirmation = None;
                        lib.cursor = 0;
                    }
                }
            }
            "import" => {
                self.edit.eq_profiles = None;
                self.open_eq_files(crate::ui::panels::files::Purpose::ImportEq);
            }
            "export" => {
                self.edit.eq_profiles = None;
                self.open_eq_files(crate::ui::panels::files::Purpose::ExportEq);
            }
            _ => {}
        }
    }
    fn delete_eq_profile(&mut self, index: usize) -> Result<()> {
        if self.eq.bundled(index) {
            bail!("Built-in profiles cannot be deleted.");
        }
        let name = self.eq.profiles[index].name.clone();
        if self.eq.managed_names.contains(&name.to_lowercase()) {
            std::fs::remove_file(self.eq.check_saved_source(index)?)?;
        }
        self.eq.saved_profiles.remove(&name.to_lowercase());
        self.eq.managed_names.remove(&name.to_lowercase());
        if let Some(preset) = eq::PRESETS
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(&name))
        {
            let profile = Profile::legacy(preset.name, 0., &preset.gains);
            self.eq
                .saved_profiles
                .insert(name.to_lowercase(), profile.clone());
            self.eq.profiles[index] = profile;
        } else {
            self.eq.profiles.remove(index);
            if self.eq.profile > index {
                self.eq.profile -= 1;
            }
            self.eq.profile = self.eq.profile.min(self.eq.profiles.len() - 1);
        }
        self.eq.select_first_band();
        self.apply_eq();
        self.eq.persist_drafts()?;
        self.note(format!("Deleted {name}"));
        Ok(())
    }
    pub(super) fn eq_library_key(&mut self, key: KeyEvent) -> bool {
        let Some(lib) = &mut self.edit.eq_profiles else {
            return false;
        };
        if let Some(confirm) = lib.confirmation {
            match key.code {
                KeyCode::Enter => self.eq_library_action(if confirm.reverting() {
                    "confirm-revert"
                } else {
                    "confirm-delete"
                }),
                KeyCode::Esc => self.eq_library_action("cancel-delete"),
                _ => {}
            }
            return true;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.edit.eq_profiles = None,
            KeyCode::Enter => self.eq_library_action("load"),
            KeyCode::Up => lib.cursor = lib.cursor.saturating_sub(1),
            KeyCode::Down => {
                lib.cursor = (lib.cursor + 1).min(lib.matches(&self.eq).len().saturating_sub(1))
            }
            KeyCode::PageUp => lib.cursor = lib.cursor.saturating_sub(8),
            KeyCode::PageDown => {
                lib.cursor = (lib.cursor + 8).min(lib.matches(&self.eq).len().saturating_sub(1))
            }
            KeyCode::Home => lib.cursor = 0,
            KeyCode::End => lib.cursor = lib.matches(&self.eq).len().saturating_sub(1),
            KeyCode::Delete => self.eq_library_action("delete"),
            KeyCode::Char('s' | 'S') if ctrl => {
                self.eq_library_action(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    "save-as"
                } else {
                    "save"
                })
            }
            KeyCode::Char('n') if ctrl => self.eq_library_action("new"),
            KeyCode::Char('r') if ctrl => self.eq_library_action("rename"),
            KeyCode::Char('u') if ctrl => {
                lib.query.clear();
                lib.cursor = 0;
                lib.scroll = 0;
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                lib.query.push(c);
                lib.cursor = 0;
                lib.scroll = 0;
            }
            KeyCode::Backspace => {
                lib.query.pop();
                lib.cursor = 0;
                lib.scroll = 0;
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> EqState {
        let profile = Profile::legacy("Flat", 0., &[0.; 10]);
        let compiled = crate::audio::dsp::eq::EqSettings::from_profile(true, &profile, 44100);
        EqState {
            enabled: true,
            profiles: vec![profile.clone()],
            profile: 0,
            saved_profiles: [("flat".into(), profile)].into(),
            managed_names: Default::default(),
            stage: 1,
            scroll: 0,
            compiled,
            compiled_rate: 44100,
        }
    }
    #[test]
    fn drafts_restore_working_edits_without_changing_the_saved_curve() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("working-edits.json");
        let mut eq = state();
        let saved = eq.active().clone();
        if let Filter::Biquad { gain_db, .. } = &mut eq.active_mut().stages[1].filter {
            *gain_db = -3.8;
        }
        assert!(eq.modified(0));
        assert!(eq.bundled(0));
        eq.persist_drafts_at(&file).unwrap();
        assert_eq!(eq.saved_profiles["flat"], saved);
        let mut restored = state();
        restored.restore_drafts_from(&file);
        assert_eq!(restored.active(), eq.active());
        assert_eq!(restored.saved_profiles["flat"], saved);
        assert!(restored.modified(0));
        assert!(!dir.path().join("Flat.txt").exists());
    }
    #[test]
    fn invalid_or_future_drafts_do_not_partially_replace_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("working-edits.json");
        let mut eq = state();
        let original = eq.active().clone();
        for text in [
            "not json",
            r#"{"version":2,"active":"Flat","drafts":[]}"#,
            r#"{"version":1,"active":"Bad","drafts":[{"profile":{"name":"../Bad","stages":[]},"base":null}]}"#,
        ] {
            std::fs::write(&file, text).unwrap();
            eq.restore_drafts_from(&file);
            assert_eq!(eq.active(), &original);
        }
    }
    #[test]
    fn profile_search_is_case_insensitive_and_saved_profiles_precede_builtins() {
        let mut eq = state();
        eq.profiles
            .push(Profile::legacy("Studio Headphones", 0., &[0.; 10]));
        eq.remember_saved(1);
        let lib = Library {
            query: "HEAD studio".into(),
            ..Library::default()
        };
        assert_eq!(lib.matches(&eq), [1]);
        assert_eq!(Library::default().matches(&eq), [1, 0]);
        assert_eq!(eq.profile, 0, "searching must not load or change playback");
    }
    proptest::proptest! {
        #[test]
        fn accepted_names_remain_plain_file_names(name in ".{0,180}") {
            if valid_name(&name).is_ok() {
                proptest::prop_assert!(!name.contains(['/', '\\']));
                proptest::prop_assert!(!name.chars().any(char::is_control));
                proptest::prop_assert!(!name.is_empty() && name != "." && name != "..");
                proptest::prop_assert!(name.chars().count() <= 128);
            }
        }
    }
}
