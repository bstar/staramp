//! Resolving a cue sheet's `FILE` references to real files on disk.
//!
//! In the reference library 164 of 1,123 sheets have at least one `FILE` that
//! does not literally exist. Naively dropping them loses ~90 real albums; naively
//! repairing them invents ~64 broken ones. Both a resolution ladder and explicit
//! drop rules are needed, and the drop rules are the subtle half.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use super::model::CueSheet;

/// Extensions a `FILE` may have been encoded to. EAC and friends routinely write
/// `FILE "x.wav"` into a sheet that ships alongside `x.flac`.
const AUDIO_EXTS: &[&str] = &[
    "flac", "ape", "wv", "wav", "mpc", "m4a", "mp3", "ogg", "dsf", "dff", "tta", "tak", "wma",
    "aiff", "aif",
];

/// How a `FILE` reference was matched, for reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    CaseInsensitive,
    UnicodeNormalised,
    ExtensionSwapped,
}

#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub path: PathBuf,
    pub how: MatchKind,
}

/// What should happen to a whole sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// Every `FILE` resolved. Index it.
    Index,
    /// A multi-FILE sheet where nothing resolves.
    ///
    /// These are EAC per-track cues: one `FILE "NN - title.wav"` per track, whose
    /// wavs were encoded to individual files that the scanner already indexes
    /// separately. Repairing them would duplicate the entire album; the correct
    /// action is to ignore the sheet.
    SkipPerTrackCue,
    /// A sheet in an archival subdirectory whose parent already holds a working
    /// cue for the same album.
    ///
    /// 64 of these exist, nearly all as `.../Technical/album (wav).cue`
    /// referencing a long-deleted wav while `../album.cue` + `.flac` sit one
    /// level up. Searching parent directories to "fix" them would create a
    /// duplicate album with wrong boundaries.
    SkipArchival,
    /// Nothing resolved and none of the above explains it.
    Orphaned,
}

/// The outcome of resolving one sheet.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub disposition: Disposition,
    /// One entry per `FILE` stanza, in order. `None` where that stanza did not
    /// resolve.
    pub files: Vec<Option<ResolvedFile>>,
    /// One `FILE` per track, rather than one disc image. Decides whether the
    /// backing files should be hidden once the virtual tracks are indexed.
    pub per_track: bool,
}

impl Resolution {
    pub fn is_indexable(&self) -> bool {
        self.disposition == Disposition::Index
    }

    /// Should the backing audio files be hidden in favour of the virtual tracks?
    ///
    /// Yes for a disc image: otherwise one 70-minute track appears next to the
    /// thirteen carved out of it. No for a per-track cue: there the backing file
    /// is the track, and playlists reference it both ways.
    pub fn suppresses_backing_files(&self) -> bool {
        self.is_indexable() && !self.per_track
    }
}

/// Resolve every `FILE` in a sheet and decide what to do with it.
///
/// `cue_path` is the sheet itself; resolution is confined to its own directory.
pub fn resolve(sheet: &CueSheet, cue_path: &Path) -> Resolution {
    let dir = cue_path.parent().unwrap_or(Path::new("."));
    let listing = DirListing::read(dir);

    let files: Vec<Option<ResolvedFile>> = sheet
        .files
        .iter()
        .map(|f| resolve_one(&listing, &f.name))
        .collect();

    let resolved = files.iter().filter(|f| f.is_some()).count();
    let per_track = is_per_track_cue(sheet);

    let disposition = if resolved > 0 {
        // Partial resolution still indexes what it can, rather than discarding a
        // whole album because one stanza is broken.
        Disposition::Index
    } else if per_track {
        // Nothing resolved and it is per-track: an EAC leftover pointing at wavs
        // that were encoded and deleted. The encodes are indexed on their own.
        Disposition::SkipPerTrackCue
    } else if is_archival(cue_path) {
        Disposition::SkipArchival
    } else {
        Disposition::Orphaned
    };

    Resolution {
        disposition,
        files,
        per_track,
    }
}

/// Is this a per-track cue rather than a disc-image cue?
///
/// A disc image has one `FILE` holding many tracks. A per-track cue has many
/// `FILE`s holding one track each.
///
/// This is *not* on its own a reason to skip a sheet. MPD indexes per-track cues
/// whose files exist, and the reference library's playlists reference twelve of
/// them by `<path>.cue/trackNNNN` — dropping those would break the playlists that
/// are the whole reason for MPD URI compatibility. The flag matters for a
/// different decision: whether indexing the virtual tracks should suppress the
/// plain files underneath them. For a disc image it should, or one 70-minute
/// track appears alongside its own thirteen. For a per-track cue it must not,
/// because there the plain file *is* the track and playlists address it both
/// ways.
pub fn is_per_track_cue(sheet: &CueSheet) -> bool {
    sheet.files.len() > 1 && sheet.files.iter().all(|f| f.tracks.len() == 1)
}

/// The four-step ladder. Order matters: cheapest and most certain first.
fn resolve_one(listing: &DirListing, name: &str) -> Option<ResolvedFile> {
    // A `FILE` may carry a path prefix; only the final component is meaningful,
    // since resolution never leaves the cue's own directory.
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if name.is_empty() {
        return None;
    }

    // 1. Exact.
    if let Some(p) = listing.exact(name) {
        return Some(ResolvedFile {
            path: p,
            how: MatchKind::Exact,
        });
    }
    // 2. Case-insensitive.
    if let Some(p) = listing.case_insensitive(name) {
        return Some(ResolvedFile {
            path: p,
            how: MatchKind::CaseInsensitive,
        });
    }
    // 3. Unicode normalisation. Recovers `Hattin` vs `Hattïn`, where the sheet
    //    and the filename disagree only on composed vs decomposed form.
    if let Some(p) = listing.normalised(name) {
        return Some(ResolvedFile {
            path: p,
            how: MatchKind::UnicodeNormalised,
        });
    }
    // 4. Same stem, different audio extension. Recovers 24 albums whose sheets
    //    say `.wav` but which ship as `.flac`, `.ape` or `.wv`.
    if let Some(p) = listing.extension_swapped(name) {
        return Some(ResolvedFile {
            path: p,
            how: MatchKind::ExtensionSwapped,
        });
    }
    None
}

/// Is this sheet sitting in an archival subdirectory next to a working one?
fn is_archival(cue_path: &Path) -> bool {
    const ARCHIVAL_DIRS: &[&str] = &["technical", "artwork", "scans", "covers", "eac"];

    let Some(dir) = cue_path.parent() else {
        return false;
    };
    let in_archival_dir = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| ARCHIVAL_DIRS.contains(&n.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    if !in_archival_dir {
        return false;
    }

    // Only archival if the parent directory actually has a cue of its own —
    // otherwise this is the album's only sheet and dropping it loses the album.
    let Some(parent) = dir.parent() else {
        return false;
    };
    std::fs::read_dir(parent)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("cue"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// One directory read, indexed several ways.
///
/// Cue sheets reference at most a handful of files but a per-`FILE` `read_dir`
/// on a 1.1 TB spinning disk is a real cost during a full scan.
struct DirListing {
    dir: PathBuf,
    names: Vec<String>,
    by_lower: HashMap<String, usize>,
    by_nfc: HashMap<String, usize>,
    /// Lowercased stem -> indices of audio files with that stem.
    by_stem: HashMap<String, Vec<usize>>,
}

impl DirListing {
    fn read(dir: &Path) -> Self {
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    names.push(n.to_owned());
                }
            }
        }

        let mut by_lower = HashMap::new();
        let mut by_nfc = HashMap::new();
        let mut by_stem: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, n) in names.iter().enumerate() {
            by_lower.entry(n.to_lowercase()).or_insert(i);
            by_nfc.entry(n.nfc().collect::<String>()).or_insert(i);

            let path = Path::new(n);
            let is_audio = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if is_audio {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    by_stem.entry(stem.to_lowercase()).or_default().push(i);
                }
            }
        }

        Self {
            dir: dir.to_path_buf(),
            names,
            by_lower,
            by_nfc,
            by_stem,
        }
    }

    fn at(&self, i: usize) -> PathBuf {
        self.dir.join(&self.names[i])
    }

    fn exact(&self, name: &str) -> Option<PathBuf> {
        let p = self.dir.join(name);
        p.is_file().then_some(p)
    }

    fn case_insensitive(&self, name: &str) -> Option<PathBuf> {
        self.by_lower.get(&name.to_lowercase()).map(|&i| self.at(i))
    }

    fn normalised(&self, name: &str) -> Option<PathBuf> {
        let want: String = name.nfc().collect();
        if let Some(&i) = self.by_nfc.get(&want) {
            return Some(self.at(i));
        }
        // Also try the other direction, comparing case-insensitively.
        let want_lower = want.to_lowercase();
        self.names
            .iter()
            .position(|n| n.nfc().collect::<String>().to_lowercase() == want_lower)
            .map(|i| self.at(i))
    }

    fn extension_swapped(&self, name: &str) -> Option<PathBuf> {
        let stem = Path::new(name).file_stem()?.to_str()?.to_lowercase();
        let candidates = self.by_stem.get(&stem)?;
        // Prefer lossless, and prefer them in the order most likely to be the
        // real encode of an EAC rip.
        for want in [
            "flac", "ape", "wv", "tta", "tak", "wav", "mpc", "m4a", "mp3",
        ] {
            for &i in candidates {
                let ext = Path::new(&self.names[i])
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase());
                if ext.as_deref() == Some(want) {
                    return Some(self.at(i));
                }
            }
        }
        candidates.first().map(|&i| self.at(i))
    }
}
