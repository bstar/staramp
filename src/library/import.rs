//! Review and safely admit album directories from a local staging library.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use unicode_normalization::UnicodeNormalization;

use super::{browse, db::Db, infer, scan};
use crate::playlist::queue::QueueItem;

#[derive(Debug, Clone)]
pub struct Album {
    pub artist: String,
    pub title: String,
    pub source_rel: PathBuf,
    pub date_seconds: Option<i64>,
    pub tracks: Vec<QueueItem>,
    pub artist_candidates: Vec<PathBuf>,
    pub existing: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    Cancel,
    Beside,
    Replace,
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub destination: PathBuf,
    pub tracks: usize,
}

/// Refresh the staging index and return physical album folders oldest-first.
pub fn discover(staging: &Path, library: &Path) -> Result<Vec<Album>> {
    validate_roots(staging, library, None)?;
    if fs::read_dir(staging)?.next().is_none() {
        return Ok(Vec::new());
    }
    let index = crate::paths::import_index_file()?;
    let mut db = Db::open(&index)?;
    scan::scan(&mut db, staging, &scan::ScanOptions::default())?;
    let model = browse::Model::load(&db)?;
    let dates = dates(&db)?;
    let artist_dirs = top_dirs(library)?;
    let permanent = permanent_albums()?;

    let mut out = Vec::with_capacity(model.albums.len());
    let mut claimed = HashSet::new();
    for album in &model.albums {
        let tracks = &model.tracks[album.tracks.start as usize..album.tracks.end as usize];
        let Some(first) = tracks.first() else {
            continue;
        };
        let rel = PathBuf::from(infer::album_dir(&first.dir));
        if rel.as_os_str().is_empty() || !claimed.insert(rel.clone()) {
            continue;
        }
        let artist = model.artists[album.artist as usize].name.to_string();
        let mut album_dates: Vec<i64> = tracks
            .iter()
            .filter_map(|track| dates.get(&track.item.uri.to_string()).copied().flatten())
            .collect();
        album_dates.sort_unstable();
        let date_seconds = album_dates.get(album_dates.len() / 2).copied();
        let candidates = artist_candidates(&artist, &artist_dirs);
        let album_key = normal(&album.title);
        let artist_key = normal(&artist);
        let existing = permanent
            .iter()
            .filter(|(a, title, _)| normal(a) == artist_key && normal(title) == album_key)
            .map(|(_, _, path)| path.clone())
            .collect();
        out.push(Album {
            artist,
            title: album.title.to_string(),
            source_rel: rel,
            date_seconds,
            tracks: tracks.iter().map(|track| track.item.clone()).collect(),
            artist_candidates: candidates,
            existing,
        });
    }
    out.sort_by(|a, b| {
        a.date_seconds
            .cmp(&b.date_seconds)
            .then_with(|| normal(&a.artist).cmp(&normal(&b.artist)))
            .then_with(|| normal(&a.title).cmp(&normal(&b.title)))
    });
    Ok(out)
}

/// Copy, verify and index one album. The staging source is removed only after
/// the permanent index can see the new files.
pub fn import(
    staging: &Path,
    library: &Path,
    quarantine: &Path,
    album: &Album,
    artist_dir: &Path,
    conflict: Conflict,
) -> Result<Imported> {
    validate_roots(staging, library, Some(quarantine))?;
    anyhow::ensure!(conflict != Conflict::Cancel, "import cancelled");
    let source = contained(staging, &staging.join(&album.source_rel))?;
    anyhow::ensure!(
        source.is_dir(),
        "staging album disappeared: {}",
        source.display()
    );

    let requested_artist_dir = if artist_dir.is_absolute() {
        artist_dir.to_path_buf()
    } else {
        library.join(artist_dir)
    };
    let artist_dir = contained_or_new_child(library, &requested_artist_dir)?;
    fs::create_dir_all(&artist_dir)?;
    let folder = album
        .source_rel
        .file_name()
        .context("album source has no folder name")?;
    let mut destination = artist_dir.join(folder);
    if conflict == Conflict::Replace {
        if let Some(existing) = album.existing.first() {
            destination = contained(library, &library.join(existing))?;
        }
    }
    if destination.exists() {
        match conflict {
            Conflict::Cancel => unreachable!(),
            Conflict::Replace => {}
            Conflict::Beside => destination = beside_path(&destination),
        }
    }

    let stamp = super::db::now_secs();
    let partial = artist_dir.join(format!(
        ".staramp-import-{stamp}-{}.partial",
        std::process::id()
    ));
    anyhow::ensure!(
        !partial.exists(),
        "unfinished import already exists: {}",
        partial.display()
    );
    if let Err(error) = copy_verified(&source, &partial) {
        let _ = fs::remove_dir_all(&partial);
        return Err(error).context("copying staged album");
    }

    let mut backup = None;
    if destination.exists() {
        anyhow::ensure!(conflict == Conflict::Replace, "destination already exists");
        let replacement_dir = quarantine.join("replaced");
        fs::create_dir_all(&replacement_dir)?;
        let target = unique_path(&replacement_dir.join(folder));
        move_verified(&destination, &target).context("quarantining replaced album")?;
        backup = Some(target);
    }
    if let Err(error) = fs::rename(&partial, &destination) {
        if let Some(old) = backup.as_ref() {
            let _ = move_verified(old, &destination);
        }
        let _ = fs::remove_dir_all(&partial);
        return Err(error).context("installing verified album");
    }

    let scan_result = (|| -> Result<()> {
        let mut db = Db::open(&crate::paths::index_file()?)?;
        scan::scan(&mut db, library, &scan::ScanOptions::default())?;
        let prefix = destination
            .strip_prefix(library)?
            .to_string_lossy()
            .replace('\\', "/");
        let found: i64 = db.conn.query_row(
            "SELECT count(*) FROM track WHERE uri = ?1 OR uri LIKE ?2",
            rusqlite::params![prefix, format!("{prefix}/%")],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            found > 0,
            "the permanent scan did not index the imported album"
        );
        Ok(())
    })();
    if let Err(error) = scan_result {
        let failed = unique_path(&quarantine.join("failed-imports").join(folder));
        if let Some(parent) = failed.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = move_verified(&destination, &failed);
        if let Some(old) = backup.as_ref() {
            let _ = move_verified(old, &destination);
        }
        return Err(error).context("refreshing the permanent library index");
    }

    fs::remove_dir_all(&source)
        .with_context(|| format!("removing verified staging copy {}", source.display()))?;
    Ok(Imported {
        destination,
        tracks: album.tracks.len(),
    })
}

pub fn reject(staging: &Path, library: &Path, quarantine: &Path, album: &Album) -> Result<PathBuf> {
    validate_roots(staging, library, Some(quarantine))?;
    let source = contained(staging, &staging.join(&album.source_rel))?;
    let destination = unique_path(&quarantine.join(&album.source_rel));
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    move_verified(&source, &destination)?;
    Ok(destination)
}

fn permanent_albums() -> Result<Vec<(String, String, PathBuf)>> {
    let path = crate::paths::index_file()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = Db::open_readonly(&path)?;
    let model = browse::Model::load(&db)?;
    Ok(model
        .albums
        .iter()
        .map(|album| {
            let artist = model.artists[album.artist as usize].name.to_string();
            let first = &model.tracks[album.tracks.start as usize];
            (
                artist,
                album.title.to_string(),
                PathBuf::from(infer::album_dir(&first.dir)),
            )
        })
        .collect())
}

fn dates(db: &Db) -> Result<HashMap<String, Option<i64>>> {
    let mut stmt = db.conn.prepare(
        "SELECT t.uri, COALESCE(f.created_ns, f.mtime_ns) / 1000000000
           FROM track t JOIN file f ON f.id = t.file_id",
    )?;
    let dates = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .flatten()
        .collect();
    Ok(dates)
}

fn top_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(fs::read_dir(root)?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect())
}

fn artist_candidates(artist: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let wanted = normal(artist);
    let mut scored: Vec<(usize, PathBuf)> = dirs
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy();
            let candidate = normal(&name);
            let score = if candidate == wanted {
                0
            } else {
                levenshtein(&candidate, &wanted)
            };
            (score <= wanted.len().max(candidate.len()) / 3 + 1).then(|| (score, path.clone()))
        })
        .collect();
    scored.sort_by_key(|(score, path)| (*score, path.to_string_lossy().to_lowercase()));
    scored.into_iter().take(5).map(|(_, path)| path).collect()
}

/// Whether an existing directory already represents this artist after the
/// same case, punctuation, spacing and accent normalization used for ranking.
pub(crate) fn artist_dir_is_exact(artist: &str, path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| normal(&name.to_string_lossy()) == normal(artist))
}

/// Resolve a typed artist name to the library's established directory when a
/// normalized equivalent already exists; otherwise propose a new child.
pub(crate) fn artist_destination(root: &Path, typed: &str) -> Result<PathBuf> {
    let typed = typed.trim();
    anyhow::ensure!(!typed.is_empty(), "artist directory cannot be empty");
    anyhow::ensure!(
        typed != "." && typed != ".." && !typed.chars().any(|c| matches!(c, '/' | '\\' | '\0')),
        "artist directory must be one name, not a path"
    );
    if let Some(existing) = top_dirs(root)?
        .into_iter()
        .find(|path| artist_dir_is_exact(typed, path))
    {
        return Ok(existing);
    }
    Ok(root.join(typed))
}

fn normal(value: &str) -> String {
    value
        .nfkd()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let mut costs: Vec<usize> = (0..=b.chars().count()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut corner = i;
        costs[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let upper = costs[j + 1];
            costs[j + 1] = if ca == cb {
                corner
            } else {
                1 + corner.min(upper).min(costs[j])
            };
            corner = upper;
        }
    }
    costs[b.chars().count()]
}

fn validate_roots(staging: &Path, library: &Path, quarantine: Option<&Path>) -> Result<()> {
    anyhow::ensure!(
        staging.is_dir(),
        "staging root is not a directory: {}",
        staging.display()
    );
    anyhow::ensure!(
        library.is_dir(),
        "library root is not a directory: {}",
        library.display()
    );
    let staging = staging.canonicalize()?;
    let library = library.canonicalize()?;
    anyhow::ensure!(
        !staging.starts_with(&library) && !library.starts_with(&staging),
        "staging and library roots must be separate"
    );
    if let Some(quarantine) = quarantine {
        fs::create_dir_all(quarantine)?;
        let quarantine = quarantine.canonicalize()?;
        anyhow::ensure!(
            !quarantine.starts_with(&staging) && !quarantine.starts_with(&library),
            "quarantine must be outside staging and the permanent library"
        );
    }
    Ok(())
}

fn contained(root: &Path, candidate: &Path) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let candidate = candidate.canonicalize()?;
    anyhow::ensure!(
        candidate.starts_with(&root),
        "path escapes its configured root"
    );
    Ok(candidate)
}

/// Validate an immediate destination directory which may not exist yet.
///
/// `canonicalize` is the right containment check for existing paths, but a
/// brand-new artist has no directory to canonicalize. Resolve its existing
/// parent instead, then append exactly one ordinary file-name component.
fn contained_or_new_child(root: &Path, candidate: &Path) -> Result<PathBuf> {
    if candidate.exists() {
        return contained(root, candidate);
    }
    let name = candidate
        .file_name()
        .filter(|name| *name != "." && *name != "..")
        .context("new artist directory has no safe name")?;
    let parent = candidate
        .parent()
        .context("new artist directory has no parent")?;
    let parent = contained(root, parent)?;
    Ok(parent.join(name))
}

fn beside_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().unwrap_or_default().to_string_lossy();
    for n in 2.. {
        let candidate = parent.join(format!("{stem} [{n}]"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    beside_path(path)
}

fn move_verified(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_verified(from, to)?;
    fs::remove_dir_all(from)?;
    Ok(())
}

fn copy_verified(from: &Path, to: &Path) -> Result<()> {
    anyhow::ensure!(
        !from.symlink_metadata()?.file_type().is_symlink(),
        "refusing to follow symlink {}",
        from.display()
    );
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_verified(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    let mut input = fs::File::open(from)?;
    let mut output = fs::File::create(to)?;
    let mut source_hash = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        source_hash.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    let mut copied = fs::File::open(to)?;
    let mut destination_hash = blake3::Hasher::new();
    loop {
        let read = copied.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        destination_hash.update(&buffer[..read]);
    }
    anyhow::ensure!(
        source_hash.finalize() == destination_hash.finalize(),
        "checksum mismatch copying {}",
        from.display()
    );
    fs::set_permissions(to, fs::metadata(from)?.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_artist_spellings_are_suggested_but_rank_after_exact() {
        let dirs = vec![
            PathBuf::from("/music/Guns N' Roses"),
            PathBuf::from("/music/Guns and Roses"),
        ];
        let found = artist_candidates("guns n roses", &dirs);
        assert_eq!(found[0], dirs[0]);
    }

    #[test]
    fn existing_artist_treatment_wins_over_case_spacing_and_punctuation() {
        assert!(artist_dir_is_exact(
            "Düne Dain",
            Path::new("/music/DÜNE-DAIN")
        ));
        assert!(!artist_dir_is_exact(
            "Düne Dain",
            Path::new("/music/Terra Atlantica")
        ));
    }

    #[test]
    fn typed_artist_reuses_the_existing_normalized_directory() {
        let root = std::env::temp_dir().join(format!(
            "staramp-typed-artist-{}-{}",
            std::process::id(),
            crate::library::db::now_secs()
        ));
        fs::create_dir_all(root.join("DÜNE-DAIN")).unwrap();
        assert_eq!(
            artist_destination(&root, "Düne Dain").unwrap(),
            root.join("DÜNE-DAIN")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn beside_never_overwrites() {
        let root = std::env::temp_dir().join(format!("staramp-import-{}", std::process::id()));
        let path = root.join("Album");
        fs::create_dir_all(&path).unwrap();
        assert_eq!(beside_path(&path), root.join("Album [2]"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_artist_directory_is_validated_through_its_existing_library_parent() {
        let root = std::env::temp_dir().join(format!(
            "staramp-new-import-{}-{}",
            std::process::id(),
            crate::library::db::now_secs()
        ));
        fs::create_dir_all(&root).unwrap();
        // The validator answers with the canonical parent, and on macOS the
        // temp dir is reached through a symlink (/var -> /private/var).
        let root = root.canonicalize().unwrap();
        let wanted = root.join("Dünedain");
        assert_eq!(contained_or_new_child(&root, &wanted).unwrap(), wanted);
        assert!(!wanted.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
