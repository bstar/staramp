//! Album details and cover art, resolved off the UI thread.
//!
//! Everything here happens on one worker thread that owns its own read-only
//! database handle. That is not tidiness: the library lives on a removable
//! volume that genuinely disappears, and a `stat` on a dead mount blocks for as
//! long as the kernel feels like. A frame must never be able to wait on one.
//!
//! The UI asks by URI and reads whatever the worker last published. There is no
//! blocking call in either direction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use crossbeam_channel::{bounded, Receiver, Sender};

use super::cover;
use super::db::{AlbumDetail, Db};
use super::remote::Fetcher;

/// Where a cover came from, so the panel can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// A picture inside the audio file itself.
    Embedded,
    /// An image beside the audio, named like a cover.
    Sidecar,
    /// An image in a `Covers/`-style subdirectory.
    Subdir,
    /// Fetched from the Cover Art Archive for this album.
    Remote,
    /// Fetched for the record the *song* originally came from, when the album
    /// itself could not be placed.
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogTarget {
    Artist,
    Album,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Embedded => "embedded",
            Source::Sidecar => "folder",
            Source::Subdir => "subfolder",
            Source::Remote => "cover art archive",
            Source::Original => "original release",
        }
    }
}

/// What the panel draws.
#[derive(Debug, Clone)]
pub struct Album {
    pub uri: String,
    pub detail: Option<AlbumDetail>,
    /// What it came from, named the way the candidate names it: a
    /// library-relative path for a sidecar image, a cache path for a fetched
    /// one.
    pub art: Option<String>,
    /// Decoded and shrunk, ready for the panel to sample.
    ///
    /// Decoding happens here rather than in the panel because a cover can be a
    /// 3000px scan and a frame has 16 milliseconds. Shrinking to
    /// [`THUMB_MAX`] costs a few hundred kilobytes and makes drawing a matter
    /// of sampling.
    pub image: Option<Arc<image::RgbImage>>,
    pub source: Option<Source>,
    /// Which candidate is showing, and how many there are. The panel says so
    /// when there is more than one, because an alternative nobody knows about
    /// might as well not exist.
    pub choice: usize,
    pub choices: usize,
    /// What every candidate is called, for the chooser.
    pub labels: Vec<String>,
    /// Releases the archive offered that were not alike enough to take
    /// automatically. Read from the cache, so this costs no request.
    pub offers: Vec<super::remote::Release>,
}

/// Longest edge kept when a cover is decoded.
///
/// Far more than a terminal can show -- a full-screen panel on a 4K display is
/// perhaps 200 cells -- but small enough that the decode is quick and the
/// memory is nothing. It also leaves headroom for a graphics protocol, which
/// draws real pixels rather than sampling per cell.
const THUMB_MAX: u32 = 512;

impl Album {
    fn unknown(uri: String) -> Self {
        Self {
            uri,
            detail: None,
            art: None,
            image: None,
            source: None,
            choice: 0,
            choices: 0,
            labels: Vec::new(),
            offers: Vec::new(),
        }
    }
}

/// What the UI asks the worker for.
enum Request {
    /// Resolve this track's album.
    Look(String),
    /// Move to another of this album's candidate covers.
    Cycle(String, i32),
    /// Fetch the cover for one of the offered releases, chosen by hand.
    Release(String, usize),
    /// Look this album up again, ignoring what was remembered about it.
    Retry(String),
    /// Materialise the original candidate, then hand it to the configured
    /// desktop viewer.
    OpenImage(String, Vec<String>),
    /// Resolve and open the best external discography page.
    OpenCatalog(String, CatalogTarget),
}

/// Handle the UI holds. Cheap to clone the published value out of.
pub struct Watcher {
    tx: Sender<Request>,
    current: Arc<ArcSwapOption<Album>>,
    /// How many answers have been published.
    ///
    /// A retry republishes the same URI, so the UI cannot tell a new answer
    /// from the old one by looking at it. Counting is what lets it know when
    /// the lookup it asked for has come back.
    serial: Arc<AtomicU64>,
    outcomes: Receiver<String>,
}

impl Watcher {
    /// Start the worker. Returns `None` when there is no index to read, which
    /// is not an error -- staramp runs perfectly well before a first scan.
    pub fn spawn(
        index: PathBuf,
        vfs: Arc<crate::vfs::Vfs>,
        fetch: Arc<AtomicBool>,
    ) -> Option<Self> {
        if !index.is_file() {
            return None;
        }
        // A depth of one: only the newest request matters, and the worker
        // drains the rest. Bounded so a stuck lookup cannot grow a queue.
        let (tx, rx) = bounded::<Request>(8);
        let current = Arc::new(ArcSwapOption::from(None));
        let (outcome_tx, outcomes) = bounded::<String>(8);
        let published = Arc::clone(&current);
        let serial = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&serial);

        std::thread::Builder::new()
            .name("staramp-art".into())
            .spawn(move || run(index, vfs, fetch, rx, published, counted, outcome_tx))
            .ok()?;

        Some(Self {
            tx,
            current,
            serial,
            outcomes,
        })
    }

    /// Ask for a URI. Dropped rather than queued if the worker is busy: the
    /// next track change will ask again, and a stale request is worth nothing.
    pub fn look_up(&self, uri: &str) {
        let _ = self.tx.try_send(Request::Look(uri.to_string()));
    }

    /// Show a different one of this album's covers.
    ///
    /// The choice is remembered for the album, so it survives the next track
    /// and the next run.
    pub fn cycle(&self, uri: &str, delta: i32) {
        let _ = self.tx.try_send(Request::Cycle(uri.to_string(), delta));
    }

    /// Fetch the cover for the release at `index` in this album's offers.
    pub fn choose_release(&self, uri: &str, index: usize) {
        let _ = self.tx.try_send(Request::Release(uri.to_string(), index));
    }

    /// How many answers have been published so far.
    ///
    /// Take this before asking for a retry and watch for it to change: that is
    /// the lookup finishing, whether it found anything or not.
    pub fn serial(&self) -> u64 {
        self.serial.load(Ordering::Acquire)
    }

    /// Look this album up again from scratch.
    pub fn retry(&self, uri: &str) {
        let _ = self.tx.try_send(Request::Retry(uri.to_string()));
    }

    pub fn open_image(&self, uri: &str, viewer: &[String]) -> bool {
        self.tx
            .try_send(Request::OpenImage(uri.to_string(), viewer.to_vec()))
            .is_ok()
    }

    pub fn open_catalog(&self, uri: &str, target: CatalogTarget) -> bool {
        self.tx
            .try_send(Request::OpenCatalog(uri.to_string(), target))
            .is_ok()
    }

    /// Status from an explicit open action. Drained by the UI once per frame.
    pub fn outcome(&self) -> Option<String> {
        self.outcomes.try_recv().ok()
    }

    /// The most recent result, if it is for `uri`.
    ///
    /// Matching on the URI is what stops the panel showing the previous
    /// album's cover for the fraction of a second before the worker catches up.
    pub fn album_for(&self, uri: &str) -> Option<Arc<Album>> {
        self.current.load_full().filter(|a| a.uri == uri)
    }
}

/// Albums kept decoded.
///
/// Keyed on the directory, because that is what an album is on disk and what
/// the cover belongs to: every track on a record hits the same entry. Small
/// because playing an album is a walk through one directory, and stepping back
/// a track -- the case this exists for -- never leaves it.
const CACHE_ALBUMS: usize = 8;

/// One album's worth of resolved state, kept so cycling costs a decode and
/// stepping to the next track costs nothing.
struct Entry {
    dir_id: i64,
    candidates: Vec<cover::Candidate>,
    choice: usize,
    /// False when the archive could not be reached rather than having nothing.
    /// Such a result must not be cached, or one bad minute at MusicBrainz
    /// strands the album for the rest of the session.
    settled: bool,
    /// Releases that were close but not close enough to take unasked.
    offers: Vec<super::remote::Release>,
    /// Set when the cover belongs to one song rather than to the folder, which
    /// is what happens on a compilation: reusing it for the next track would
    /// show the wrong record.
    for_uri: Option<String>,
    /// The embedded picture, read once when the album was gathered.
    ///
    /// `present` decodes the chosen candidate on every request, so without
    /// this the audio file was reopened and its tags walked again at every
    /// track change within the album.
    embedded: Option<Arc<Vec<u8>>>,
}

fn run(
    index: PathBuf,
    vfs: Arc<crate::vfs::Vfs>,
    fetch: Arc<AtomicBool>,
    rx: Receiver<Request>,
    current: Arc<ArcSwapOption<Album>>,
    serial: Arc<AtomicU64>,
    outcomes: Sender<String>,
) {
    let db = match Db::open_readonly(&index) {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("album lookups unavailable: {e}");
            return;
        }
    };

    // The cache directory holds fetched covers and remembered choices, so it
    // is wanted even when fetching is off.
    let cache_dir = crate::paths::cache_dir().ok();
    // Built whether or not fetching is on, since the setting can be turned on
    // while the player is running; `fetch` is consulted per lookup.
    let mut fetcher = cache_dir.as_deref().and_then(Fetcher::new);

    // Most-recent-first, so the front is the album being played.
    let mut cache: Vec<Entry> = Vec::with_capacity(CACHE_ALBUMS);
    let mut catalog_cache: HashMap<(CatalogTarget, String), Option<String>> = HashMap::new();

    while let Ok(mut request) = rx.recv() {
        // Skipping to the newest request: with a held-down next key, every
        // intermediate track is work nobody will see. A cycle is a deliberate
        // act and is never skipped.
        // A cycle, a chosen release and a retry are all deliberate acts and
        // are never skipped; only a plain lookup is.
        while matches!(request, Request::Look(_)) {
            match rx.try_recv() {
                Ok(newer) => request = newer,
                Err(_) => break,
            }
        }
        let (uri, delta, release, retry, open, catalog) = match request {
            Request::Look(uri) => (uri, 0, None, false, None, None),
            Request::Cycle(uri, d) => (uri, d, None, false, None, None),
            Request::Release(uri, i) => (uri, 0, Some(i), false, None, None),
            Request::Retry(uri) => (uri, 0, None, true, None, None),
            Request::OpenImage(uri, viewer) => (uri, 0, None, false, Some(viewer), None),
            Request::OpenCatalog(uri, target) => (uri, 0, None, false, None, Some(target)),
        };

        let detail = match db.album_for_uri(&uri) {
            Ok(Some(d)) => d,
            Ok(None) => {
                current.store(Some(Arc::new(Album::unknown(uri))));
                if open.is_some() || catalog.is_some() {
                    let _ = outcomes.try_send("this track is not in the library index".into());
                }
                continue;
            }
            Err(e) => {
                // A vanished mount reaches us as a database error rather than
                // a hang, because the handle is already open. Say so and carry
                // on showing nothing.
                tracing::debug!("album lookup for {uri}: {e}");
                current.store(Some(Arc::new(Album::unknown(uri))));
                if open.is_some() || catalog.is_some() {
                    let _ = outcomes.try_send(format!("cannot read album metadata: {e}"));
                }
                continue;
            }
        };

        // An explicit retry drops everything remembered about this album:
        // the recorded miss, the backoff, and the resolved entry in hand.
        if retry {
            if let Some(f) = fetcher.as_mut() {
                f.forget(&detail);
            }
            cache.retain(|e| e.dir_id != detail.dir_id);
        }

        let at = cache.iter().position(|e| {
            e.dir_id == detail.dir_id && e.for_uri.as_deref().is_none_or(|u| u == uri)
        });
        let mut entry = match at {
            Some(i) => cache.remove(i),
            None => gather(
                &db,
                &vfs,
                fetcher.as_mut(),
                cache_dir.as_deref(),
                &detail,
                &uri,
                fetch.load(Ordering::Relaxed),
            ),
        };

        // A release chosen by hand. Fetch its cover, then rebuild the entry so
        // the new file takes its place at the front of the candidates.
        if let Some(index) = release {
            match (fetcher.as_mut(), entry.offers.get(index).cloned()) {
                (Some(f), Some(r)) => match f.cover_for(&detail, &r) {
                    Some(path) => {
                        entry.candidates.insert(0, cover::Candidate::Remote(path));
                        entry.choice = 0;
                        entry.settled = true;
                        save_choice(cache_dir.as_deref(), &detail, &entry.candidates[0]);
                    }
                    None => tracing::info!("the chosen release has no art in the archive"),
                },
                _ => tracing::debug!("no such release to choose"),
            }
        }

        if delta != 0 && !entry.candidates.is_empty() {
            let n = entry.candidates.len() as i32;
            entry.choice = (((entry.choice as i32 + delta) % n + n) % n) as usize;
            save_choice(
                cache_dir.as_deref(),
                &detail,
                &entry.candidates[entry.choice],
            );
        }

        if let Some(viewer) = open {
            let result = entry
                .candidates
                .get(entry.choice)
                .ok_or_else(|| "this album has no image to open".to_string())
                .and_then(|candidate| {
                    original_path(
                        &vfs,
                        cache_dir.as_deref(),
                        &detail,
                        candidate,
                        entry.embedded.as_deref(),
                    )
                })
                .and_then(|path| launch_image(&viewer, &path));
            let _ = outcomes.try_send(match result {
                Ok(()) => "opened full-resolution artwork".into(),
                Err(e) => format!("cannot open artwork: {e}"),
            });
        }

        if let Some(target) = catalog {
            let url = catalog_url(&detail, target, fetcher.as_mut(), &mut catalog_cache);
            let message = match url {
                Some(url) => match webbrowser::open(&url) {
                    Ok(()) => match target {
                        CatalogTarget::Artist => "opened artist database".into(),
                        CatalogTarget::Album => "opened album database".into(),
                    },
                    Err(e) => format!("cannot open music database: {e}"),
                },
                None => "not enough album information for a database search".into(),
            };
            let _ = outcomes.try_send(message);
        }

        let album = present(&vfs, &detail, &entry, uri);
        // An unsettled entry is one the archive could not be asked about. Kept
        // out of the cache so the next track change tries again.
        if entry.settled {
            cache.insert(0, entry);
            cache.truncate(CACHE_ALBUMS);
        }
        current.store(Some(Arc::new(album)));
        serial.fetch_add(1, Ordering::Release);
    }
}

fn original_path(
    vfs: &crate::vfs::Vfs,
    cache_dir: Option<&Path>,
    detail: &AlbumDetail,
    candidate: &cover::Candidate,
    embedded: Option<&Vec<u8>>,
) -> Result<PathBuf, String> {
    match candidate {
        cover::Candidate::File(rel) => {
            if let Some(path) = vfs.local_path(rel) {
                return Ok(path);
            }
            let bytes = vfs.read(rel).map_err(|e| e.to_string())?;
            cache_original(
                cache_dir,
                &bytes,
                Path::new(rel).extension().and_then(|e| e.to_str()),
            )
        }
        cover::Candidate::Embedded => {
            let reread;
            let bytes = match embedded {
                Some(bytes) => bytes.as_slice(),
                None => {
                    reread = cover::embedded(vfs, &detail.file_rel)
                        .ok_or_else(|| "embedded image is no longer readable".to_string())?;
                    &reread
                }
            };
            cache_original(cache_dir, bytes, guessed_extension(bytes))
        }
        cover::Candidate::Remote(path) | cover::Candidate::Original(path) => Ok(path.clone()),
    }
}

fn guessed_extension(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Bmp => Some("bmp"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::WebP => Some("webp"),
        _ => Some("jpg"),
    }
}

fn cache_original(
    cache_dir: Option<&Path>,
    bytes: &[u8],
    extension: Option<&str>,
) -> Result<PathBuf, String> {
    let dir = cache_dir
        .ok_or_else(|| "no cache directory is available".to_string())?
        .join("art/view");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = extension.unwrap_or("jpg").trim_start_matches('.');
    let path = dir.join(format!("{}.{}", blake3::hash(bytes).to_hex(), ext));
    if !path.is_file() {
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn launch_image(viewer: &[String], path: &Path) -> Result<(), String> {
    let mut command = if let Some(program) = viewer.first().filter(|p| !p.is_empty()) {
        let mut command = Command::new(program);
        command.args(&viewer[1..]);
        command
    } else {
        #[cfg(target_os = "macos")]
        let command = Command::new("open");
        #[cfg(not(target_os = "macos"))]
        let command = Command::new("xdg-open");
        command
    };
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn catalog_url(
    detail: &AlbumDetail,
    target: CatalogTarget,
    mut fetcher: Option<&mut Fetcher>,
    cache: &mut HashMap<(CatalogTarget, String), Option<String>>,
) -> Option<String> {
    let (entity, id) = match target {
        CatalogTarget::Artist => ("artist", detail.mb_artist_id.as_deref()),
        CatalogTarget::Album => ("release", detail.mb_release_id.as_deref()),
    };
    let id = id.filter(|id| valid_uuid(id));
    let exact_metal = id.and_then(|id| {
        let key = (target, id.to_string());
        if !cache.contains_key(&key) {
            let found = fetcher
                .as_deref_mut()
                .and_then(|f| f.metal_archives_url(entity, id));
            cache.insert(key.clone(), found);
        }
        cache.get(&key).cloned().flatten()
    });
    if let Some(url) = exact_metal {
        return Some(url);
    }

    let metal = detail.genre.as_deref().is_some_and(is_metal_genre);
    let artist = detail.artist.as_deref().filter(|s| !s.trim().is_empty());
    let album = detail.album.as_deref().filter(|s| !s.trim().is_empty());
    let genre_unknown = detail.genre.as_deref().is_none_or(|g| g.trim().is_empty());
    if metal || genre_unknown {
        if let (Some(artist), Some(album)) = (artist, album) {
            let key = (target, format!("search:{artist}\n{album}"));
            if !cache.contains_key(&key) {
                let found = fetcher.and_then(|f| {
                    f.discover_metal_archives_url(artist, album, target == CatalogTarget::Album)
                });
                cache.insert(key.clone(), found);
            }
            if let Some(url) = cache.get(&key).cloned().flatten() {
                return Some(url);
            }
        }
    }
    if metal {
        let (query, kind) = match target {
            CatalogTarget::Artist => (artist?.to_string(), "band_name"),
            CatalogTarget::Album => (album?.to_string(), "album_title"),
        };
        return Some(format!(
            "https://www.metal-archives.com/search?searchString={}&type={kind}",
            super::remote::urlencode(&query)
        ));
    }

    if let Some(id) = id {
        return Some(format!("https://musicbrainz.org/{entity}/{id}"));
    }
    let (query, kind) = match target {
        CatalogTarget::Artist => (artist?.to_string(), "artist"),
        CatalogTarget::Album => (
            format!(
                "release:\"{}\" AND artist:\"{}\"",
                album?,
                artist.unwrap_or("")
            ),
            "release",
        ),
    };
    Some(format!(
        "https://musicbrainz.org/search?query={}&type={kind}&method=indexed",
        super::remote::urlencode(&query)
    ))
}

/// Genre tags in real libraries are neither controlled nor consistently
/// separated. Match the family names that identify metal without classifying
/// broad neighbours such as plain rock, hard rock, folk or industrial as
/// metal on their own.
fn is_metal_genre(genre: &str) -> bool {
    let lower = genre.to_lowercase();
    if lower.contains("metal") || lower.contains("металл") {
        return true;
    }
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.iter().any(|word| {
        matches!(
            *word,
            "nwobhm"
                | "djent"
                | "thrash"
                | "doom"
                | "sludge"
                | "grindcore"
                | "deathcore"
                | "mathcore"
                | "goregrind"
                | "deathgrind"
                | "cybergrind"
                | "blackgaze"
                | "deathrock"
        )
    })
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(i, c)| {
            (matches!(i, 8 | 13 | 18 | 23) && c == '-')
                || (!matches!(i, 8 | 13 | 18 | 23) && c.is_ascii_hexdigit())
        })
}

/// Work out an album's candidate covers, fetching if the files offer none.
fn gather(
    db: &Db,
    vfs: &crate::vfs::Vfs,
    fetcher: Option<&mut Fetcher>,
    cache_dir: Option<&std::path::Path>,
    detail: &AlbumDetail,
    uri: &str,
    fetch: bool,
) -> Entry {
    let started = std::time::Instant::now();
    let found = cover::candidates(db, vfs, detail);
    let embedded = found.embedded.map(Arc::new);
    let mut list = found.list;
    let mut settled = true;

    let mut offers = Vec::new();
    let mut per_song = false;
    if found.wants_remote && fetch {
        if let Some(f) = fetcher {
            match f.cover(detail) {
                Ok(Some(path)) => list.insert(0, cover::Candidate::Remote(path)),
                Ok(None) => {
                    // The album could not be placed. The artist and the song
                    // title usually still can be, so ask what record the song
                    // originally came from -- which is the only thing that
                    // works for a box-set disc or a compilation.
                    match f.song_cover(detail) {
                        Ok(Some(path)) => {
                            list.insert(0, cover::Candidate::Original(path));
                            per_song = true;
                        }
                        // Nothing taken automatically. Whatever came close is
                        // worth offering, and the lookup already wrote it down.
                        Ok(None) => offers = f.offers(detail),
                        Err(()) => settled = false,
                    }
                }
                // Could not ask. Not an answer, so do not remember it as one.
                Err(()) => settled = false,
            }
        }
    }

    // A choice the user made earlier wins over the ranking, which is the whole
    // point of having made it.
    let choice = load_choice(cache_dir, detail)
        .and_then(|id| list.iter().position(|c| c.id() == id))
        .unwrap_or(0);

    tracing::debug!(
        "{} candidate(s) for {}: {} in {}ms",
        list.len(),
        detail.album.as_deref().unwrap_or("?"),
        list.get(choice).map(|c| c.label()).unwrap_or_default(),
        started.elapsed().as_millis()
    );

    Entry {
        dir_id: detail.dir_id,
        candidates: list,
        choice,
        settled,
        offers,
        // A cover found for one song is that song's, not the folder's. On a
        // compilation the next track is a different record entirely.
        for_uri: per_song.then(|| uri.to_string()),
        embedded,
    }
}

/// Turn a resolved entry into what the panel draws.
fn present(vfs: &crate::vfs::Vfs, detail: &AlbumDetail, entry: &Entry, uri: String) -> Album {
    let chosen = entry.candidates.get(entry.choice);
    let image = chosen.and_then(|c| load(vfs, detail, c, entry.embedded.as_deref()));
    let art = match chosen {
        Some(cover::Candidate::File(rel)) => Some(rel.clone()),
        Some(cover::Candidate::Remote(p)) | Some(cover::Candidate::Original(p)) => {
            Some(p.display().to_string())
        }
        _ => None,
    };
    Album {
        uri,
        detail: Some(detail.clone()),
        art,
        source: image
            .is_some()
            .then(|| chosen.map(|c| c.source()))
            .flatten(),
        image,
        choice: entry.choice,
        choices: entry.candidates.len(),
        labels: entry.candidates.iter().map(|c| c.label()).collect(),
        offers: entry.offers.clone(),
    }
}

/// Decode one candidate.
fn load(
    vfs: &crate::vfs::Vfs,
    detail: &AlbumDetail,
    candidate: &cover::Candidate,
    embedded: Option<&Vec<u8>>,
) -> Option<Arc<image::RgbImage>> {
    match candidate {
        cover::Candidate::Embedded => {
            // Gathered with the candidate list. The fallback is for an entry
            // that somehow has the candidate without the bytes, and costs what
            // this whole field exists to avoid.
            let reread;
            let bytes = match embedded {
                Some(b) => b,
                None => {
                    reread = cover::embedded(vfs, &detail.file_rel)?;
                    &reread
                }
            };
            shrink(
                crate::util::image::decode_limited(bytes, crate::util::image::MAX_DIMENSION).ok(),
            )
        }
        // A library image, wherever the library is.
        cover::Candidate::File(rel) => match vfs.read(rel) {
            Ok(bytes) => shrink(
                crate::util::image::decode_limited(&bytes, crate::util::image::MAX_DIMENSION).ok(),
            ),
            Err(e) => {
                tracing::debug!("cover {rel}: {e}");
                None
            }
        },
        // Fetched covers are always in the local cache, whatever the library
        // is doing.
        cover::Candidate::Remote(p) | cover::Candidate::Original(p) => decode(p),
    }
}

/// Where an album's remembered choice is kept.
///
/// Keyed the same way fetched covers are -- on the album rather than its path
/// -- so a rescan or a remount does not lose it.
fn choice_path(cache_dir: Option<&std::path::Path>, detail: &AlbumDetail) -> Option<PathBuf> {
    let dir = cache_dir?.join("art");
    let key = super::remote::album_key(
        detail.artist.as_deref().unwrap_or(""),
        detail.album.as_deref().unwrap_or(""),
        detail.year,
    );
    Some(dir.join(format!("{key}.pick")))
}

fn load_choice(cache_dir: Option<&std::path::Path>, detail: &AlbumDetail) -> Option<String> {
    let path = choice_path(cache_dir, detail)?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

fn save_choice(
    cache_dir: Option<&std::path::Path>,
    detail: &AlbumDetail,
    candidate: &cover::Candidate,
) {
    let Some(path) = choice_path(cache_dir, detail) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, candidate.id());
}

/// Read a cover and shrink it.
///
/// Failure is silent and returns `None`: a corrupt or truncated JPEG in a
/// music folder is common enough, and it is not worth a message on the status
/// line every time a track changes.
fn decode(path: &std::path::Path) -> Option<Arc<image::RgbImage>> {
    match crate::util::image::open_limited(path, crate::util::image::MAX_DIMENSION) {
        Ok(i) => shrink(Some(i)),
        Err(e) => {
            tracing::debug!("cover {}: {e}", path.display());
            None
        }
    }
}

/// Bring a decoded cover down to something a panel can hold.
fn shrink(img: Option<image::DynamicImage>) -> Option<Arc<image::RgbImage>> {
    let img = img?;
    let (w, h) = (img.width(), img.height());
    let img = if w.max(h) > THUMB_MAX {
        // Triangle rather than Lanczos: the result is being sampled down to
        // cells anyway, and this is on a worker that a track change is waiting
        // on.
        img.resize(THUMB_MAX, THUMB_MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some(Arc::new(img.to_rgb8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(genre: Option<&str>) -> AlbumDetail {
        AlbumDetail {
            album: Some("Hliðskjálf".into()),
            artist: Some("Björk".into()),
            year: Some(1999),
            codec: Some("flac".into()),
            genre: genre.map(str::to_string),
            mb_release_id: Some("11111111-1111-1111-1111-111111111111".into()),
            mb_artist_id: Some("22222222-2222-2222-2222-222222222222".into()),
            track_count: 8,
            total_ms: 0,
            dir_id: 1,
            file_rel: "album/01.flac".into(),
            track_title: None,
            track_artist: None,
        }
    }

    #[test]
    fn metal_genres_use_metal_archives_search_without_scraping() {
        let mut cache = HashMap::new();
        let url = catalog_url(
            &detail(Some("Atmospheric Black Metal")),
            CatalogTarget::Album,
            None,
            &mut cache,
        )
        .unwrap();
        assert!(url.starts_with("https://www.metal-archives.com/search?"));
        assert!(url.contains("Hli%C3%B0skj%C3%A1lf"));
        assert!(url.ends_with("type=album_title"));
    }

    #[test]
    fn non_metal_tags_use_exact_musicbrainz_ids() {
        let mut cache = HashMap::new();
        assert_eq!(
            catalog_url(
                &detail(Some("Art Pop")),
                CatalogTarget::Artist,
                None,
                &mut cache
            )
            .as_deref(),
            Some("https://musicbrainz.org/artist/22222222-2222-2222-2222-222222222222")
        );
    }

    #[test]
    fn extracted_originals_keep_the_exact_bytes_and_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"original image bytes";
        let first = cache_original(Some(dir.path()), bytes, Some("png")).unwrap();
        let second = cache_original(Some(dir.path()), bytes, Some("png")).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), bytes);
    }

    #[test]
    fn only_uuid_shaped_tag_values_can_enter_exact_urls() {
        assert!(valid_uuid("11111111-1111-1111-1111-111111111111"));
        assert!(!valid_uuid("../../somewhere-that-is-not-an-id"));
    }

    #[test]
    fn metal_family_genres_cover_subgenres_without_claiming_classic_rock() {
        for genre in [
            "Power Metal",
            "Alternative/Indie Rock/Metal",
            "Металл",
            "NWOBHM",
            "Djent",
            "Funeral Doom",
            "Technical Deathcore",
            "Atmospheric Sludge",
            "Goregrind",
        ] {
            assert!(is_metal_genre(genre), "{genre}");
        }
        for genre in ["Classic Rock", "Hard Rock", "Latin", "Power Pop", "Folk"] {
            assert!(!is_metal_genre(genre), "{genre}");
        }
    }
}
