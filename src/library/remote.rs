//! Cover art from the Cover Art Archive, for the records that have none.
//!
//! Forty-three percent of the album directories in the reference library have
//! no picture anywhere -- not in the tags, not beside the audio, not in a
//! `Covers/` folder. For those the only remaining source is somebody else's,
//! and the Cover Art Archive is the one that is free, unauthenticated and
//! attached to MusicBrainz's release identifiers.
//!
//! It is **off by default**. Looking a cover up means sending an artist and an
//! album name to a third party, and that is the user's decision to make, not
//! a default to be helpful about.
//!
//! Five things here were established against the live services rather than
//! from the documentation, and each of them was a bug first:
//!
//! 1. **Lucene terms must be quoted.** `artist:"Blind Guardian"` matches;
//!    `artist:Blind Guardian` matches nothing, because the second word becomes
//!    a separate free-text term.
//! 2. **The rate limit is real.** MusicBrainz asks for no more than one
//!    request a second and the archive starts returning 429 as soon as you
//!    ignore that.
//! 3. **A generic User-Agent is refused.** It has to name the application and
//!    a way to reach whoever runs it.
//! 4. **404 is an answer, not a failure.** It means this release has no art,
//!    and it should be remembered as such -- otherwise every track change on a
//!    coverless album repeats two network requests forever.
//! 5. **An exact title match is not good enough.** Quoting the album as one
//!    phrase -- the fix for (1) -- turns out to be too strict for real rips.
//!    `Legend of the Forgotten Reign - Chapter 3` and MusicBrainz's
//!    `Legend of the Forgotten Reign, Chapter 3` differ by a comma and match
//!    nothing. See [`Fetcher::release_id`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::db::AlbumDetail;

const USER_AGENT: &str = concat!(
    "staramp/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/bstar/staramp )"
);

/// Minimum spacing between requests.
///
/// MusicBrainz asks for one per second. The extra hundred milliseconds is
/// there because the limit is enforced on their clock, not ours.
const SPACING: Duration = Duration::from_millis(1100);

/// How long a "this album has no art" answer stands.
///
/// A week: long enough that a coverless library is not re-asking constantly,
/// short enough that art added to the archive shows up without the user having
/// to know there is a cache to clear.
const MISS_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How alike two titles must be before a release is used without asking.
///
/// This is a comparison of the titles themselves, not MusicBrainz's relevance
/// score, because the score does not catch the failure that matters. Searching
/// Cinderella's `Monster Ballads` returns `Best Ballads` at a score of 90-odd:
/// highly relevant, wrong record. Comparing the words gives 0.5 and it is
/// refused. `Powerplant (VICP-61808)` against `Powerplant` gives 1.0 once the
/// catalogue number is stripped, and is taken.
const AUTO_ACCEPT: f32 = 0.9;

/// How alike they must be to be worth offering as a manual choice.
///
/// Below this a release is not the same record by any reading, and putting it
/// in front of someone to reject is not help.
pub const OFFER: f32 = 0.6;

/// How many releases a search asks for.
///
/// Enough that a record with several pressings offers a real choice, few
/// enough that the chooser is a list rather than a haystack.
const SEARCH_LIMIT: usize = 8;

/// How many recordings a song search asks for.
///
/// Much larger than the release search, because MusicBrainz returns one row
/// per *recording* and a well-covered song has scores of them -- Extreme's
/// "More Than Words" has 111 -- each carrying only one or two of the releases
/// it appears on. At twenty-five results the studio album does not appear at
/// all; at a hundred it does.
const RECORDING_LIMIT: usize = 100;

/// How well a recording must match before its releases are considered.
const RECORDING_SCORE: u64 = 90;

/// How many releases the automatic path will try before giving up.
///
/// Each is a request, and the ones past the first few are increasingly
/// unlikely; the chooser is there for the rest.
const AUTO_TRIES: usize = 4;

/// Extra attempts when the server says it is merely busy.
///
/// MusicBrainz flaps rather than failing: during a bad spell the identical
/// query alternates between "currently busy" and a perfectly good answer,
/// roughly half and half -- measured at exactly six of twelve on an identical
/// query. Four extra attempts at widening intervals turn a coin toss into
/// something that lands about ninety-seven times in a hundred.
const BUSY_RETRIES: u32 = 4;

/// How long to wait after the first busy answer, doubling each time.
///
/// On top of the ordinary request spacing, and capped so the last attempts do
/// not drift minutes apart. Worst case adds eleven seconds to a lookup, on a
/// worker thread where no frame is waiting on it.
const BUSY_PAUSE: Duration = Duration::from_secs(1);
const BUSY_PAUSE_MAX: Duration = Duration::from_secs(4);

/// How long to stand off after being rate limited, doubling each time.
const BACKOFF_START: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

pub struct Fetcher {
    agent: ureq::Agent,
    /// `cache/art`, holding one file per album identity.
    dir: PathBuf,
    last_request: Option<Instant>,
    /// Set when the service asked us to stop; nothing is sent before it.
    quiet_until: Option<Instant>,
    backoff: Duration,
}

impl Fetcher {
    /// `None` if the cache directory cannot be made, since there is no point
    /// fetching what cannot be kept.
    pub fn new(cache_dir: &Path) -> Option<Self> {
        let dir = cache_dir.join("art");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("no art cache at {}: {e}", dir.display());
            return None;
        }
        Some(Self {
            agent: agent(),
            dir,
            last_request: None,
            quiet_until: None,
            backoff: BACKOFF_START,
        })
    }

    /// A cover for this album, from the cache or from the network.
    ///
    /// `Ok(None)` means asked and answered: this album has no art. `Err(())`
    /// means it could not be asked -- the service was unwell, or we are still
    /// backing off -- which is a different thing entirely, and the caller must
    /// not remember it as a settled answer.
    ///
    /// Blocking, and deliberately so: this runs on the art worker, which
    /// exists precisely so that slow things happen somewhere a frame is not
    /// waiting on them.
    pub fn cover(&mut self, detail: &AlbumDetail) -> Result<Option<PathBuf>, ()> {
        let (artist, album) = match (detail.artist.as_deref(), detail.album.as_deref()) {
            (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => (a, b),
            // Nothing to search on. A settled answer: no query would help.
            _ => return Ok(None),
        };
        let key = album_key(artist, album, detail.year);
        let hit = self.dir.join(format!("{key}.jpg"));
        if hit.is_file() {
            return Ok(Some(hit));
        }
        if self.recently_missed(&key) {
            return Ok(None);
        }
        if self.quiet_until.is_some_and(|t| Instant::now() < t) {
            return Err(());
        }

        let candidates = match self.releases(artist, album) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("musicbrainz: {e}");
                return Err(());
            }
        };

        // Only releases that really look like this record are taken without
        // being asked about. The rest are kept for the chooser.
        for release in candidates
            .iter()
            .filter(|r| r.similarity >= AUTO_ACCEPT)
            .take(AUTO_TRIES)
        {
            match self.art_for(release) {
                Ok(Some(bytes)) => match std::fs::write(&hit, &bytes) {
                    Ok(()) => {
                        tracing::info!("fetched a cover for {artist} - {album}");
                        return Ok(Some(hit));
                    }
                    Err(e) => {
                        tracing::debug!("caching {}: {e}", hit.display());
                        return Err(());
                    }
                },
                // This pressing has no art; the next one might.
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!("cover art archive: {e}");
                    return Err(());
                }
            }
        }

        // Nothing could be taken on its own. Remember what was on offer, so
        // the chooser can show it later without asking the service again, and
        // so a coverless album does not repeat this on every track change.
        self.remember_miss(&key, &candidates);
        Ok(None)
    }

    /// Fetch the cover for a release the user chose themselves.
    ///
    /// No similarity check: they looked at the list and decided, and that
    /// outranks any comparison this could make.
    pub fn cover_for(&mut self, detail: &AlbumDetail, release: &Release) -> Option<PathBuf> {
        let key = album_key(
            detail.artist.as_deref().unwrap_or(""),
            detail.album.as_deref().unwrap_or(""),
            detail.year,
        );
        let hit = self.dir.join(format!("{key}.jpg"));
        match self.art_for(release) {
            Ok(Some(bytes)) => match std::fs::write(&hit, &bytes) {
                Ok(()) => Some(hit),
                Err(e) => {
                    tracing::debug!("caching {}: {e}", hit.display());
                    None
                }
            },
            Ok(None) => {
                tracing::debug!("no art for the chosen release {}", release.mbid);
                None
            }
            Err(e) => {
                tracing::debug!("cover art archive: {e}");
                None
            }
        }
    }

    /// The record a song originally came from.
    ///
    /// The fallback for everything the album search cannot place: box-set
    /// discs, compilations, and rips whose album tag names something the
    /// archive has never heard of. The artist and the song title are usually
    /// right even when the album is not, so this asks what the song is on
    /// rather than what the folder claims.
    ///
    /// Keyed on the recording rather than the album, so on a compilation each
    /// track resolves to its own record.
    pub fn song_cover(&mut self, detail: &AlbumDetail) -> Result<Option<PathBuf>, ()> {
        let artist = detail
            .track_artist
            .as_deref()
            .or(detail.artist.as_deref())
            .unwrap_or("");
        let title = detail.track_title.as_deref().unwrap_or("");
        if artist.is_empty() || title.is_empty() {
            return Ok(None);
        }

        // A separate namespace from the album keys: the same blake3 over
        // different things must not be able to collide.
        let key = format!("s{}", album_key(artist, title, None));
        let hit = self.dir.join(format!("{key}.jpg"));
        if hit.is_file() {
            return Ok(Some(hit));
        }
        if self.recently_missed(&key) {
            return Ok(None);
        }
        if self.quiet_until.is_some_and(|t| Instant::now() < t) {
            return Err(());
        }

        let releases = match self.recording_albums(artist, title) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("musicbrainz recording: {e}");
                return Err(());
            }
        };
        for release in releases.iter().take(AUTO_TRIES) {
            match self.art_for(release) {
                Ok(Some(bytes)) => match std::fs::write(&hit, &bytes) {
                    Ok(()) => {
                        tracing::info!("{artist} - {title}: cover from {}", release.title);
                        return Ok(Some(hit));
                    }
                    Err(e) => {
                        tracing::debug!("caching {}: {e}", hit.display());
                        return Err(());
                    }
                },
                Ok(None) => continue,
                Err(e) => {
                    tracing::debug!("cover art archive: {e}");
                    return Err(());
                }
            }
        }
        self.remember_miss(&key, &releases);
        Ok(None)
    }

    /// The studio albums a song appears on, earliest first.
    fn recording_albums(&mut self, artist: &str, title: &str) -> Result<Vec<Release>, ureq::Error> {
        let query = format!(
            "artist:{} AND recording:{}",
            quote_term(artist),
            quote_term(title)
        );
        let url = format!(
            "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit={RECORDING_LIMIT}",
            urlencode(&query)
        );
        Ok(match self.get(&url)? {
            Some(body) => parse_recording_albums(&body),
            None => Vec::new(),
        })
    }

    /// Forget that this album, and this song, had no cover.
    ///
    /// A recorded miss is what stops a coverless album repeating two network
    /// requests on every track change, and it stands for a week. That is the
    /// right default and the wrong answer when somebody has just fixed a tag,
    /// or when the service was having the sort of evening MusicBrainz has been
    /// having. Asking again is an explicit act, so it also clears the backoff:
    /// being told to try now means now.
    pub fn forget(&mut self, detail: &AlbumDetail) {
        let album = album_key(
            detail.artist.as_deref().unwrap_or(""),
            detail.album.as_deref().unwrap_or(""),
            detail.year,
        );
        let song = format!(
            "s{}",
            album_key(
                detail
                    .track_artist
                    .as_deref()
                    .or(detail.artist.as_deref())
                    .unwrap_or(""),
                detail.track_title.as_deref().unwrap_or(""),
                None,
            )
        );
        for key in [album, song] {
            let _ = std::fs::remove_file(self.miss_path(&key));
        }
        self.quiet_until = None;
        self.backoff = BACKOFF_START;
    }

    /// The releases that were on offer for this album last time it was looked
    /// up, for the chooser. Read from the cache; never asks the network.
    pub fn offers(&self, detail: &AlbumDetail) -> Vec<Release> {
        let key = album_key(
            detail.artist.as_deref().unwrap_or(""),
            detail.album.as_deref().unwrap_or(""),
            detail.year,
        );
        std::fs::read_to_string(self.miss_path(&key))
            .map(|s| s.lines().filter_map(parse_offer).collect())
            .unwrap_or_default()
    }

    /// Every release that might be this album, best match first.
    ///
    /// Two searches, because one is not enough. The exact phrase is tried
    /// first and is worth trying: when a rip's tags happen to match, the answer
    /// is unambiguous. But they usually do not. A real example:
    ///
    /// ```text
    /// tagged:       Legend of the Forgotten Reign - Chapter 3: The Way Of The Light
    /// musicbrainz:  Legend of the Forgotten Reign, Chapter 3: The Way of the Light
    /// ```
    ///
    /// One comma, and the phrase query returns nothing at all for a release
    /// the service plainly has. So a loosened word search follows, and the
    /// results are then judged on [`similarity`] rather than on MusicBrainz's
    /// relevance score -- the score says how well a row answers the query, not
    /// whether it is the same record.
    fn releases(&mut self, artist: &str, album: &str) -> Result<Vec<Release>, ureq::Error> {
        let quoted = quote_term(artist);
        let mut found = self.search(&format!(
            "artist:{quoted} AND release:{}",
            quote_term(album)
        ))?;

        let terms = loose_terms(album);
        if found.is_empty() && !terms.is_empty() {
            found = self.search(&format!("artist:{quoted} AND release:({terms})"))?;
        }

        let mut out: Vec<Release> = Vec::new();
        for mut r in found {
            r.similarity = similarity(album, &r.title);
            if r.similarity < OFFER || out.iter().any(|e| e.mbid == r.mbid) {
                continue;
            }
            out.push(r);
        }
        // Most like the tagged title first; that is the order the chooser
        // shows and the order the automatic path tries.
        out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        Ok(out)
    }

    /// One search.
    fn search(&mut self, query: &str) -> Result<Vec<Release>, ureq::Error> {
        let url = format!(
            "https://musicbrainz.org/ws/2/release?query={}&fmt=json&limit={SEARCH_LIMIT}",
            urlencode(query)
        );
        Ok(match self.get(&url)? {
            Some(body) => parse_releases(&body),
            None => Vec::new(),
        })
    }

    /// The front cover for one release, or `None` when there is none.
    ///
    /// Falls back to the release *group* when the specific pressing has no
    /// art. Measured across twelve coverless albums here, that alone takes the
    /// hit rate from nine to eleven: the archive frequently holds art for a
    /// record without holding it for the particular pressing a rip came from.
    fn art_for(&mut self, release: &Release) -> Result<Option<Vec<u8>>, ureq::Error> {
        let url = format!(
            "https://coverartarchive.org/release/{}/front-500",
            release.mbid
        );
        if let Some(bytes) = self.get(&url)? {
            return Ok(Some(bytes));
        }
        let Some(group) = release.group.as_deref() else {
            return Ok(None);
        };
        let url = format!("https://coverartarchive.org/release-group/{group}/front-500");
        self.get(&url)
    }

    /// One request, spaced and rate-limit aware.
    ///
    /// `Ok(None)` is "asked and answered: there is nothing there" -- a 404 --
    /// which is a different thing from an error and must not be retried.
    fn get(&mut self, url: &str) -> Result<Option<Vec<u8>>, ureq::Error> {
        // A busy server is worth waiting out; being told off is not. One extra
        // attempt each, spaced like any other request.
        for attempt in 0..=BUSY_RETRIES {
            let (status, body) = self.send(url)?;
            match status {
                200..=299 => {
                    // A successful exchange means the service is happy again.
                    self.backoff = BACKOFF_START;
                    self.quiet_until = None;
                    return Ok(Some(body));
                }
                // Asked and answered: this release has no art.
                404 => return Ok(None),
                429 | 503 => {
                    if status == 503 && !is_rate_limit(&body) && attempt < BUSY_RETRIES {
                        // "The MusicBrainz web server is currently busy."
                        // Their overload, not our pace, and it clears in
                        // seconds. Poisoning the whole fetcher for half a
                        // minute over it would strand every album behind one
                        // bad moment.
                        let pause = (BUSY_PAUSE * (1 << attempt)).min(BUSY_PAUSE_MAX);
                        tracing::debug!("musicbrainz is busy; retrying in {pause:?}");
                        std::thread::sleep(pause);
                        continue;
                    }
                    self.quiet_until = Some(Instant::now() + self.backoff);
                    tracing::warn!("backing off {:?} after {status}", self.backoff);
                    self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                    return Err(ureq::Error::StatusCode(status));
                }
                other => return Err(ureq::Error::StatusCode(other)),
            }
        }
        Err(ureq::Error::StatusCode(503))
    }

    /// One request on the wire, spaced from the last.
    fn send(&mut self, url: &str) -> Result<(u16, Vec<u8>), ureq::Error> {
        if let Some(last) = self.last_request {
            let since = last.elapsed();
            if since < SPACING {
                std::thread::sleep(SPACING - since);
            }
        }
        self.last_request = Some(Instant::now());

        let response = self.agent.get(url).call()?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(8 * 1024 * 1024)
            .read_to_end(&mut body)?;
        Ok((status, body))
    }

    fn miss_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.miss"))
    }

    /// Has this album already been looked up and found to have nothing?
    fn recently_missed(&self, key: &str) -> bool {
        let path = self.miss_path(key);
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        let Ok(modified) = meta.modified() else {
            return true;
        };
        match SystemTime::now().duration_since(modified) {
            Ok(age) if age > MISS_TTL => {
                // Expired. Removing it now means one lookup, not one per track.
                let _ = std::fs::remove_file(&path);
                false
            }
            // A file from the future -- a clock that moved -- is not a reason
            // to hammer the service.
            _ => true,
        }
    }

    /// Record that this album has no cover of its own, and what was near.
    ///
    /// The near misses are written into the same file, so opening the chooser
    /// later costs nothing and a coverless album is not re-queried on every
    /// track change.
    fn remember_miss(&self, key: &str, candidates: &[Release]) {
        let body: String = candidates
            .iter()
            .filter(|r| r.similarity >= OFFER)
            .map(format_offer)
            .collect();
        let _ = std::fs::write(self.miss_path(key), body);
    }
}

use std::io::Read as _;

/// The HTTP client.
///
/// Split out so a test can prove the User-Agent actually goes out on the wire.
/// It is not decoration: MusicBrainz answers 503 to a client that does not name
/// itself, and a header that silently failed to apply would look exactly like
/// the service being down.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        // Statuses are read rather than raised, because the body is what
        // distinguishes a busy server from a rate limit, and an error that has
        // already thrown the body away cannot tell them apart.
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into()
}

/// A stable name for an album, independent of where its files are.
///
/// Keyed on the album's identity rather than its path so that a rescan, a
/// remount at a different letter, or a reorganised library all keep the cover
/// that was already fetched.
pub fn album_key(artist: &str, album: &str, year: Option<i64>) -> String {
    let identity = format!(
        "{}\u{1f}{}\u{1f}{}",
        artist.trim().to_lowercase(),
        album.trim().to_lowercase(),
        year.unwrap_or(0)
    );
    blake3::hash(identity.as_bytes()).to_hex()[..32].to_string()
}

/// Wrap a term in quotes for Lucene, escaping what would end the quote.
///
/// Without this an artist of more than one word matches nothing at all: only
/// the first word stays attached to the field and the rest becomes free text.
fn quote_term(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// One release the archive might hold a cover for.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub mbid: String,
    /// Its release group, which often has art when the specific pressing does
    /// not. Worth two of twelve on its own in the reference library.
    pub group: Option<String>,
    pub title: String,
    /// Year and country, so two pressings of one record can be told apart.
    pub date: Option<String>,
    pub country: Option<String>,
    /// How alike this title and the tagged one are, 0 to 1.
    pub similarity: f32,
}

impl Release {
    /// What the chooser shows on one line.
    pub fn describe(&self) -> String {
        let mut out = self.title.clone();
        let year = self.date.as_deref().and_then(|d| d.get(..4));
        match (year, self.country.as_deref()) {
            (Some(y), Some(c)) => out.push_str(&format!("  {y} {c}")),
            (Some(y), None) => out.push_str(&format!("  {y}")),
            (None, Some(c)) => out.push_str(&format!("  {c}")),
            (None, None) => {}
        }
        out
    }
}

/// An album title reduced to the words that identify the record.
///
/// Rip tags carry a great deal that is not part of the title: catalogue
/// numbers, `(Japanese Edition)`, `(24 BIT Remastered)`, `CD1`, `[FLAC]`. Of
/// the 747 albums here with no local artwork, 174 have a bracketed suffix of
/// some kind. None of it helps identify the record and all of it drags a
/// comparison down, so it comes off before the titles are compared.
fn normalise_title(title: &str) -> Vec<String> {
    const NOISE: &[&str] = &[
        "japan",
        "japanese",
        "deluxe",
        "limited",
        "remaster",
        "remastered",
        "bonus",
        "expanded",
        "ultimate",
        "special",
        "anniversary",
        "reissue",
        "digipak",
        "edition",
        "version",
        "ed",
        "disc",
        "disk",
        "cd",
        "vinyl",
        "flac",
        "mp3",
        "bit",
        "the",
        "a",
        "an",
    ];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut word = String::new();
    // Bracketed groups go entirely, at any depth: `(2017 - CD1)` is noise
    // whole, and its pieces are noise separately too.
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => word.push(c.to_ascii_lowercase()),
            _ => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
        }
        if !c.is_alphanumeric() && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out.retain(|w| !NOISE.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()));
    out
}

/// How alike two album titles are, 0 to 1.
///
/// The shared words over the longer title's length. Word overlap rather than
/// edit distance because the differences that matter here are whole words --
/// an extra `Anthology`, a missing `Live` -- while the differences that do not
/// matter are punctuation, which normalising has already removed.
pub fn similarity(tagged: &str, candidate: &str) -> f32 {
    let a = normalise_title(tagged);
    let b = normalise_title(candidate);
    if a.is_empty() || b.is_empty() {
        // Nothing left to compare on. Equal emptiness is not a match.
        return 0.0;
    }
    let shared = a.iter().filter(|w| b.contains(w)).count();
    shared as f32 / a.len().max(b.len()) as f32
}

/// An album title reduced to plain words for a term search.
///
/// Everything Lucene treats as syntax has to go, not just for tidiness: an
/// unescaped `:` inside a group -- and album titles are full of them, as in
/// `Chapter 3: The Way of the Light` -- reads as a field separator and changes
/// what the query means.
fn loose_terms(album: &str) -> String {
    const SYNTAX: &[char] = &[
        '+', '-', '&', '|', '!', '(', ')', '{', '}', '[', ']', '^', '"', '~', '*', '?', ':', '\\',
        '/',
    ];
    album
        .split(|c: char| c.is_whitespace() || SYNTAX.contains(&c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Is this 503 about our pace, or about their afternoon?
///
/// MusicBrainz returns 503 for both, and they call for opposite responses: one
/// means stop sending, the other means try again shortly. Only the body says
/// which, so this reads it rather than guessing.
fn is_rate_limit(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body).to_lowercase();
    text.contains("rate limit") || text.contains("exceeding")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The releases in a MusicBrainz search response.
///
/// Parsed rather than scanned. One field out of one object did not justify a
/// JSON dependency and was picked out by hand; five fields across a list of
/// nested objects is a different question, and the hand-written version was
/// one response-format change away from building a URL out of whatever
/// happened to be there.
fn parse_releases(body: &[u8]) -> Vec<Release> {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(list) = root.get("releases").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    list.iter()
        .filter_map(|r| {
            let mbid = r.get("id")?.as_str()?;
            // A MusicBrainz identifier is a UUID. Checking the shape means a
            // change in the response reads as no cover rather than as a
            // request for a nonsense URL.
            if !is_uuid(mbid) {
                return None;
            }
            let group = r
                .get("release-group")
                .and_then(|g| g.get("id"))
                .and_then(|g| g.as_str())
                .filter(|g| is_uuid(g))
                .map(str::to_string);
            Some(Release {
                mbid: mbid.to_string(),
                group,
                title: r.get("title")?.as_str()?.to_string(),
                date: r.get("date").and_then(|d| d.as_str()).map(str::to_string),
                country: r
                    .get("country")
                    .and_then(|c| c.as_str())
                    .map(str::to_string),
                // Filled in by the caller, which knows what was searched for.
                similarity: 0.0,
            })
        })
        .collect()
}

/// Throw away every remembered "this album has no cover".
///
/// For after a batch of tags has been fixed, or after the archive has had a
/// bad day: one at a time through the player would be tedious, and waiting out
/// the week is worse.
pub fn forget_all(cache_dir: &Path) -> std::io::Result<usize> {
    let dir = cache_dir.join("art");
    let mut n = 0;
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        // No cache is not a failure; there is simply nothing to forget.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "miss") {
            std::fs::remove_file(&path)?;
            n += 1;
        }
    }
    Ok(n)
}

/// One offered release, as a line in the miss file.
///
/// Tab separated and hand written: this is staramp's own file, read only by
/// staramp, and a line of text survives being looked at with `cat`.
fn format_offer(r: &Release) -> String {
    format!(
        "{}\t{}\t{:.3}\t{}\t{}\t{}\n",
        r.mbid,
        r.group.as_deref().unwrap_or(""),
        r.similarity,
        r.date.as_deref().unwrap_or(""),
        r.country.as_deref().unwrap_or(""),
        r.title.replace(['\t', '\n'], " "),
    )
}

fn parse_offer(line: &str) -> Option<Release> {
    let mut f = line.split('\t');
    let mbid = f.next()?;
    if !is_uuid(mbid) {
        return None;
    }
    let group = f.next()?;
    let similarity = f.next()?.parse().ok()?;
    let date = f.next()?;
    let country = f.next()?;
    let title = f.next()?.trim_end();
    Some(Release {
        mbid: mbid.to_string(),
        group: (!group.is_empty()).then(|| group.to_string()),
        title: title.to_string(),
        date: (!date.is_empty()).then(|| date.to_string()),
        country: (!country.is_empty()).then(|| country.to_string()),
        similarity,
    })
}

/// The studio albums a recording search says a song appeared on, earliest
/// first.
///
/// Two filters do the work. The release group has to be an `Album` -- not a
/// single or an EP -- and it must carry no secondary type, which is what
/// excludes the compilations, live records and soundtracks a well-known song
/// accumulates. Searching Blondie's "A Shark in Jets Clothing" returns
/// thirteen recordings, almost all of them live bootlegs; those two conditions
/// leave the debut album and one 2022 collection.
///
/// Earliest first, because the original release is the one being asked for and
/// a reissue's date is still closer to it than a later collection's.
fn parse_recording_albums(body: &[u8]) -> Vec<Release> {
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(recordings) = root.get("recordings").and_then(|r| r.as_array()) else {
        return Vec::new();
    };

    let mut out: Vec<(String, Release)> = Vec::new();
    for rec in recordings {
        if rec.get("score").and_then(|s| s.as_u64()).unwrap_or(0) < RECORDING_SCORE {
            continue;
        }
        let Some(releases) = rec.get("releases").and_then(|r| r.as_array()) else {
            continue;
        };
        for rel in releases {
            let group = rel.get("release-group");
            let primary = group
                .and_then(|g| g.get("primary-type"))
                .and_then(|t| t.as_str());
            if primary != Some("Album") {
                continue;
            }
            // Any secondary type at all disqualifies it: Compilation, Live,
            // Soundtrack, Remix, DJ-mix. None of them is the original record.
            let secondary = group
                .and_then(|g| g.get("secondary-types"))
                .and_then(|t| t.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
            if secondary {
                continue;
            }
            // And it has to be an official release. Bootlegs are typed as
            // albums and are often dated before the record they are taken
            // from: two of them sit ahead of `Extreme II: Pornograffitti` and
            // would win on date alone.
            if rel.get("status").and_then(|s| s.as_str()) != Some("Official") {
                continue;
            }
            let Some(mbid) = rel
                .get("id")
                .and_then(|i| i.as_str())
                .filter(|i| is_uuid(i))
            else {
                continue;
            };
            if out.iter().any(|(_, r)| r.mbid == mbid) {
                continue;
            }
            let date = rel
                .get("date")
                .and_then(|d| d.as_str())
                .or_else(|| {
                    group
                        .and_then(|g| g.get("first-release-date"))
                        .and_then(|d| d.as_str())
                })
                .unwrap_or("")
                .to_string();
            out.push((
                // An undated release sorts last rather than first: no date is
                // not evidence of being early.
                if date.is_empty() {
                    "9999".into()
                } else {
                    date.clone()
                },
                Release {
                    mbid: mbid.to_string(),
                    group: group
                        .and_then(|g| g.get("id"))
                        .and_then(|g| g.as_str())
                        .filter(|g| is_uuid(g))
                        .map(str::to_string),
                    title: rel
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    date: (!date.is_empty()).then_some(date),
                    country: rel
                        .get("country")
                        .and_then(|c| c.as_str())
                        .map(str::to_string),
                    // Not comparable to an album title, and not used: the song
                    // matched, which is the whole basis for this route.
                    similarity: 1.0,
                },
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, r)| r).collect()
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first 700 bytes of a real response, captured from the live service.
    /// The point of keeping the real bytes is the sibling keys: `status-id`,
    /// `packaging-id` and `artist-credit-id` all appear before the release id
    /// in the artist block, and a looser scan picks one of them.
    const RESPONSE: &str = r#"{"created":"2026-08-30T04:58:35.317Z","count":4,"offset":0,"releases":[{"id":"7e2cd751-d89e-4449-a55f-a257fb5ab522","score":100,"status-id":"4e304316-386d-3409-af2e-78857eec5cfe","packaging-id":"ec27701a-4a22-37f4-bfac-6616e0f9750a","artist-credit-id":"bfca69b1-9531-3aaf-867b-e03dd96048b1","count":1,"title":"Dragonchaser","status":"Official","packaging":"Jewel Case","text-representation":{"language":"eng","script":"Latn"},"artist-credit":[{"name":"At Vance","artist":{"id":"17828264-0f4a-40b3-bfc5-8544f30debed","name":"At Vance"}}]}]}"#;

    #[test]
    fn a_miss_is_remembered_and_then_forgotten() {
        // The whole reason this exists: 43% of the reference library has no
        // local art, and without a remembered miss each of those albums
        // repeats two network requests on every single track change.
        let dir = std::env::temp_dir().join(format!("staramp-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = Fetcher::new(&dir).expect("cache dir");
        let key = album_key("Nobody", "Nothing", None);

        assert!(!f.recently_missed(&key), "nothing is known yet");
        f.remember_miss(&key, &[]);
        assert!(f.recently_missed(&key), "the miss must stand");

        // Backdate it past the time to live. An album that had no art a year
        // ago may well have some now.
        let path = f.miss_path(&key);
        let old = SystemTime::now() - MISS_TTL - Duration::from_secs(60);
        filetime(&path, old);
        assert!(!f.recently_missed(&key), "an expired miss must not stand");
        assert!(
            !path.exists(),
            "and it should be cleared, so this costs one lookup rather than one per track"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Set a file's modification time, so a cache entry can be aged without
    /// waiting a week for it.
    fn filetime(path: &Path, when: SystemTime) {
        // `FileTimes::set_modified` is stable and portable; the unix extension trait
        // is only needed for access times, which do not matter here.
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_times(std::fs::FileTimes::new().set_modified(when))
            .unwrap();
    }

    #[test]
    fn a_cached_cover_is_returned_without_asking_anyone() {
        let dir = std::env::temp_dir().join(format!("staramp-hit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut f = Fetcher::new(&dir).expect("cache dir");
        let detail = AlbumDetail {
            album: Some("Master Of Light".into()),
            artist: Some("Freedom Call".into()),
            year: Some(2016),
            codec: Some("flac".into()),
            track_count: 12,
            total_ms: 0,
            dir_id: 1,
            file_rel: "a/b.flac".into(),
            track_title: None,
            track_artist: None,
        };
        let cached = dir.join("art").join(format!(
            "{}.jpg",
            album_key("Freedom Call", "Master Of Light", Some(2016))
        ));
        std::fs::write(&cached, b"not really a jpeg").unwrap();

        // No network is reachable from a unit test, so this returning the path
        // at all is the proof that nothing was sent.
        assert_eq!(f.cover(&detail), Ok(Some(cached)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_album_with_no_artist_or_title_is_never_looked_up() {
        // Sending an empty query would be one request to learn nothing.
        let dir = std::env::temp_dir().join(format!("staramp-blank-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut f = Fetcher::new(&dir).expect("cache dir");
        let blank = AlbumDetail {
            album: Some(String::new()),
            artist: None,
            year: None,
            codec: None,
            track_count: 0,
            total_ms: 0,
            dir_id: 1,
            file_rel: "a/b.flac".into(),
            track_title: None,
            track_artist: None,
        };
        assert_eq!(
            f.cover(&blank),
            Ok(None),
            "a settled answer, not a failure to ask"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn titles_are_compared_on_the_words_that_identify_the_record() {
        // Every pair here came out of a real search against the library.
        // The ones that must be taken automatically:
        for (tagged, found) in [
            ("Powerplant (VICP-61808)", "Powerplant"),
            ("The Book Of Souls (WPCR-16857)", "The Book of Souls"),
            (
                "Magical Mystery Tour (24 BIT Remastered)",
                "Magical Mystery Tour",
            ),
            (
                "The Wicked Symphony (Deluxe Edition)",
                "The Wicked Symphony",
            ),
            ("The Gates of Oblivion CD", "The Gates of Oblivion"),
            (
                "Just What I Needed - The Cars Anthology (CD1)",
                "Just What I Needed: The Cars Anthology",
            ),
            (
                "Legend Of The Shadowking (Japanese Edition)",
                "Legend of the Shadowking",
            ),
            ("Doom Of Destiny", "Doom of Destiny"),
        ] {
            let s = similarity(tagged, found);
            assert!(s >= AUTO_ACCEPT, "{tagged:?} vs {found:?} scored {s}");
        }

        // And the ones that must not: MusicBrainz rates these highly relevant
        // and they are the wrong record.
        for (tagged, found) in [
            ("Monster Ballads", "Best Ballads"),
            ("Bach: Bandenburg Concertos", "Bach / Vivaldi"),
        ] {
            let s = similarity(tagged, found);
            assert!(s < AUTO_ACCEPT, "{tagged:?} vs {found:?} scored {s}");
        }
    }

    #[test]
    fn a_title_that_is_all_noise_matches_nothing() {
        // Refusing on no evidence beats matching on no evidence.
        assert_eq!(similarity("(Japanese Edition)", "Powerplant"), 0.0);
        assert_eq!(similarity("", ""), 0.0);
        assert_eq!(similarity("CD1", "CD2"), 0.0);
    }

    #[test]
    fn a_release_reads_as_one_line() {
        let r = Release {
            mbid: "x".into(),
            group: None,
            title: "Powerplant".into(),
            date: Some("1999-04-12".into()),
            country: Some("JP".into()),
            similarity: 1.0,
        };
        assert_eq!(r.describe(), "Powerplant  1999 JP");
    }

    #[test]
    fn the_two_kinds_of_503_are_told_apart() {
        // Both arrive as 503 and they call for opposite responses: one means
        // stop sending, the other means try again shortly. These are the real
        // bodies, copied from the live service.
        assert!(is_rate_limit(
            br#"{"error": "Your requests are exceeding the allowable rate limit."}"#
        ));
        assert!(!is_rate_limit(
            br#"{"error": "The MusicBrainz web server is currently busy. Please try again later."}"#
        ));
        assert!(!is_rate_limit(b""));
    }

    #[test]
    fn the_user_agent_names_the_application_and_a_contact() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        // MusicBrainz refuses a generic agent with a 503, which is
        // indistinguishable from the service being unwell. Prove the header
        // reaches the socket.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            (&stream)
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            headers
        });

        let _ = agent().get(format!("http://127.0.0.1:{port}/")).call();
        let headers = handle.join().unwrap().to_lowercase();
        assert!(
            headers.contains("user-agent: staramp/"),
            "no user-agent went out: {headers}"
        );
        assert!(
            headers.contains("github.com/bstar/staramp"),
            "the agent must carry a contact: {headers}"
        );
    }

    #[test]
    fn a_search_response_yields_its_releases() {
        let out = parse_releases(RESPONSE.as_bytes());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mbid, "7e2cd751-d89e-4449-a55f-a257fb5ab522");
        assert_eq!(out[0].title, "Dragonchaser");
        // The artist object carries an id too, and picking that one up would
        // send us asking the archive about a person.
        assert_ne!(out[0].mbid, "17828264-0f4a-40b3-bfc5-8544f30debed");
    }

    #[test]
    fn the_release_group_is_kept_because_it_often_has_the_art() {
        let body = br#"{"releases":[{"id":"7e2cd751-d89e-4449-a55f-a257fb5ab522",
            "title":"x","release-group":{"id":"bd9e29e2-93e1-3bef-9c3a-bd63af3a93f4"}}]}"#;
        assert_eq!(
            parse_releases(body)[0].group.as_deref(),
            Some("bd9e29e2-93e1-3bef-9c3a-bd63af3a93f4")
        );
    }

    /// A recording search, cut down to the shape that matters. This is what
    /// Blondie's "A Shark in Jets Clothing" actually returns: the studio album
    /// buried among live records and a later collection.
    const RECORDINGS: &str = r#"{"recordings":[
      {"score":100,"releases":[
        {"id":"11111111-1111-1111-1111-111111111111","title":"Blondie... Live","date":"1978","status":"Official",
         "release-group":{"id":"aaaaaaaa-1111-1111-1111-111111111111","primary-type":"Album",
                          "secondary-types":["Live"]}},
        {"id":"22222222-2222-2222-2222-222222222222","title":"Blondie","date":"2003","status":"Official",
         "release-group":{"id":"bbbbbbbb-2222-2222-2222-222222222222","primary-type":"Album"}},
        {"id":"33333333-3333-3333-3333-333333333333","title":"The Broadcast Collection","date":"2022","status":"Official",
         "release-group":{"id":"cccccccc-3333-3333-3333-333333333333","primary-type":"Album"}},
        {"id":"44444444-4444-4444-4444-444444444444","title":"Greatest Hits","date":"1981","status":"Official",
         "release-group":{"primary-type":"Album","secondary-types":["Compilation"]}},
        {"id":"55555555-5555-5555-5555-555555555555","title":"Denis","date":"1978","status":"Official",
         "release-group":{"primary-type":"Single"}}]},
      {"score":40,"releases":[
        {"id":"66666666-6666-6666-6666-666666666666","title":"Too Weak A Match","date":"1970","status":"Official",
         "release-group":{"primary-type":"Album"}}]}]}"#;

    #[test]
    fn a_song_resolves_to_its_original_studio_album() {
        let out = parse_recording_albums(RECORDINGS.as_bytes());
        let titles: Vec<&str> = out.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Blondie", "The Broadcast Collection"],
            "live records, compilations and singles are not the original"
        );
        assert_eq!(
            out[0].title, "Blondie",
            "the earliest studio album is what was asked for"
        );
        assert_eq!(
            out[0].group.as_deref(),
            Some("bbbbbbbb-2222-2222-2222-222222222222"),
            "the group is kept, since the archive often has art there and not on the release"
        );
    }

    #[test]
    fn a_bootleg_does_not_beat_the_record_it_was_taken_from() {
        // Two bootlegs are dated ahead of `Extreme II: Pornograffitti` and
        // typed as plain albums, so date and type alone are not enough.
        let body = br#"{"recordings":[{"score":100,"releases":[
            {"id":"11111111-1111-1111-1111-111111111111","title":"More Extreme Words","date":"1992",
             "status":"Bootleg","release-group":{"primary-type":"Album"}},
            {"id":"22222222-2222-2222-2222-222222222222","title":"Extreme II: Pornograffitti",
             "date":"1993-03-01","status":"Official","release-group":{"primary-type":"Album"}}]}]}"#;
        let out = parse_recording_albums(body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Extreme II: Pornograffitti");
    }

    #[test]
    fn a_poorly_matched_recording_is_not_consulted() {
        // Its release is older than everything else and would sort first, so
        // the score filter has to come before the date sort.
        let out = parse_recording_albums(RECORDINGS.as_bytes());
        assert!(
            !out.iter().any(|r| r.title == "Too Weak A Match"),
            "a recording scoring 40 is not this song"
        );
    }

    #[test]
    fn an_undated_release_does_not_sort_as_the_earliest() {
        // No date is not evidence of being early, and treating it that way
        // puts an unknown pressing ahead of the actual original.
        let body = br#"{"recordings":[{"score":100,"releases":[
            {"id":"11111111-1111-1111-1111-111111111111","title":"Undated","status":"Official",
             "release-group":{"primary-type":"Album"}},
            {"id":"22222222-2222-2222-2222-222222222222","title":"Nineteen Ninety","date":"1990","status":"Official",
             "release-group":{"primary-type":"Album"}}]}]}"#;
        let out = parse_recording_albums(body);
        assert_eq!(out[0].title, "Nineteen Ninety");
    }

    #[test]
    fn a_malformed_response_yields_nothing_rather_than_nonsense() {
        assert!(parse_releases(b"not json").is_empty());
        assert!(parse_releases(br#"{"releases":[]}"#).is_empty());
        // An id that is not a UUID must never reach a URL.
        assert!(parse_releases(br#"{"releases":[{"id":"../../etc","title":"x"}]}"#).is_empty());
        assert!(parse_releases(br#"{"releases":[{"title":"no id"}]}"#).is_empty());
    }

    #[test]
    fn offered_releases_survive_a_round_trip_through_the_cache() {
        // The chooser reads these back on a later run, so the format has to
        // hold everything it shows.
        let r = Release {
            mbid: "7e2cd751-d89e-4449-a55f-a257fb5ab522".into(),
            group: Some("bd9e29e2-93e1-3bef-9c3a-bd63af3a93f4".into()),
            title: "Monster\tBallads".into(),
            date: Some("1999-04-12".into()),
            country: Some("JP".into()),
            similarity: 0.75,
        };
        let back = parse_offer(&format_offer(&r)).expect("parses");
        assert_eq!(back.mbid, r.mbid);
        assert_eq!(back.group, r.group);
        assert_eq!(back.date, r.date);
        assert_eq!(back.country, r.country);
        assert!((back.similarity - 0.75).abs() < 0.001);
        assert_eq!(back.title, "Monster Ballads", "a tab would split the line");
    }

    #[test]
    fn a_release_with_nothing_optional_still_round_trips() {
        let r = Release {
            mbid: "7e2cd751-d89e-4449-a55f-a257fb5ab522".into(),
            group: None,
            title: "x".into(),
            date: None,
            country: None,
            similarity: 1.0,
        };
        let back = parse_offer(&format_offer(&r)).expect("parses");
        assert_eq!(back, r);
    }

    #[test]
    fn terms_are_quoted_because_unquoted_ones_match_nothing() {
        assert_eq!(quote_term("Blind Guardian"), "\"Blind Guardian\"");
        assert_eq!(quote_term("Say \"Yes\""), "\"Say \\\"Yes\\\"\"");
    }

    #[test]
    fn the_query_is_encoded_for_a_url() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("\"x\""), "%22x%22");
        assert_eq!(urlencode("Motorhead-2.0_a~z"), "Motorhead-2.0_a~z");
        // Non-ASCII goes through as UTF-8 bytes, which is what the service
        // expects and what half this library's artist names need.
        assert_eq!(urlencode("Björk"), "Bj%C3%B6rk");
    }

    #[test]
    fn the_cache_key_follows_the_album_not_the_path() {
        // Same record, different capitalisation and spacing: one key.
        let a = album_key("At Vance", "Dragonchaser", Some(2001));
        let b = album_key(" at vance ", "DRAGONCHASER", Some(2001));
        assert_eq!(a, b);
        // A different year is a different release, and often different art.
        assert_ne!(a, album_key("At Vance", "Dragonchaser", Some(2011)));
        assert_ne!(a, album_key("At Vance", "Only Human", Some(2001)));
        assert_eq!(a.len(), 32, "short enough to be a sane filename");
    }
}
