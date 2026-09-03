//! Permanent local listening history and optional network scrobbling.
//!
//! Activity is deliberately separate from the library index. An index is a
//! rebuildable description of files (and a remote index is replaced by the
//! next download); listening history is user data and must survive both.

use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::audio::player::PlayState;
use crate::playlist::queue::QueueItem;

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
CREATE TABLE IF NOT EXISTS track_stat (
  uri              TEXT PRIMARY KEY,
  play_count       INTEGER NOT NULL DEFAULT 0,
  skip_count       INTEGER NOT NULL DEFAULT 0,
  last_played_at   INTEGER,
  first_played_at  INTEGER,
  total_played_ms  INTEGER NOT NULL DEFAULT 0,
  completion_ratio REAL,
  rating           INTEGER,
  loved            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS stat_play_idx ON track_stat(play_count DESC);
CREATE INDEX IF NOT EXISTS stat_last_idx ON track_stat(last_played_at DESC);
CREATE TABLE IF NOT EXISTS play_history (
  id           INTEGER PRIMARY KEY,
  uri          TEXT NOT NULL,
  artist       TEXT,
  title        TEXT,
  album        TEXT,
  year         INTEGER,
  album_artist TEXT,
  track_no     INTEGER,
  started_at   INTEGER NOT NULL,
  ended_at     INTEGER,
  duration_ms  INTEGER,
  listened_ms  INTEGER NOT NULL DEFAULT 0,
  outcome      TEXT NOT NULL DEFAULT 'playing',
  local_play   INTEGER NOT NULL DEFAULT 0,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS history_started_idx ON play_history(started_at DESC);
CREATE TABLE IF NOT EXISTS scrobble_delivery (
  history_id   INTEGER NOT NULL REFERENCES play_history(id) ON DELETE CASCADE,
  provider     TEXT NOT NULL,
  state        TEXT NOT NULL DEFAULT 'pending',
  attempts     INTEGER NOT NULL DEFAULT 0,
  next_retry_at INTEGER NOT NULL DEFAULT 0,
  submitted_at INTEGER,
  last_error   TEXT,
  PRIMARY KEY (history_id, provider)
);
CREATE INDEX IF NOT EXISTS delivery_retry_idx
  ON scrobble_delivery(provider, state, next_retry_at);
PRAGMA user_version = 2;
"#;

const ATTACHED_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS activity.track_stat (
  uri TEXT PRIMARY KEY, play_count INTEGER NOT NULL DEFAULT 0,
  skip_count INTEGER NOT NULL DEFAULT 0, last_played_at INTEGER,
  first_played_at INTEGER, total_played_ms INTEGER NOT NULL DEFAULT 0,
  completion_ratio REAL, rating INTEGER, loved INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS activity.stat_play_idx ON track_stat(play_count DESC);
CREATE INDEX IF NOT EXISTS activity.stat_last_idx ON track_stat(last_played_at DESC);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum Provider {
    Lastfm,
    Listenbrainz,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Lastfm, Provider::Listenbrainz];

    pub fn key(self) -> &'static str {
        match self {
            Provider::Lastfm => "lastfm",
            Provider::Listenbrainz => "listenbrainz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Lastfm => "Last.fm",
            Provider::Listenbrainz => "ListenBrainz",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Credentials {
    lastfm: LastfmCredentials,
    listenbrainz: ListenbrainzCredentials,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct LastfmCredentials {
    api_key: String,
    api_secret: String,
    session_key: String,
    username: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct ListenbrainzCredentials {
    token: String,
    username: String,
}

impl Credentials {
    fn load() -> Self {
        let Ok(path) = crate::paths::credentials_file() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn configured(&self, p: Provider) -> bool {
        match p {
            Provider::Lastfm => {
                !self.lastfm.api_key.is_empty()
                    && !self.lastfm.api_secret.is_empty()
                    && !self.lastfm.session_key.is_empty()
            }
            Provider::Listenbrainz => !self.listenbrainz.token.is_empty(),
        }
    }

    fn username(&self, p: Provider) -> &str {
        match p {
            Provider::Lastfm => &self.lastfm.username,
            Provider::Listenbrainz => &self.listenbrainz.username,
        }
    }

    fn save(&self) -> Result<()> {
        let path = crate::paths::credentials_file()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("toml.{}", std::process::id()));
        let text = toml::to_string_pretty(self)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

/// Create or migrate the permanent activity store before a read-only library
/// connection tries to attach it.
pub fn ensure_store() -> Result<()> {
    let path = crate::paths::activity_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating activity directory at {}", parent.display()))?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("creating activity database at {}", path.display()))?;
    conn.execute_batch(SCHEMA)
        .with_context(|| format!("initializing activity database at {}", path.display()))?;
    migrate_schema(&conn)?;
    Ok(())
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    let has_year: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('play_history') WHERE name='year')",
        [],
        |r| r.get(0),
    )?;
    if !has_year {
        conn.execute("ALTER TABLE play_history ADD COLUMN year INTEGER", [])?;
    }
    Ok(())
}

/// Attach the permanent activity statistics to a library connection.
pub fn attach(conn: &Connection, memory: bool, initialize: bool) -> Result<()> {
    if memory {
        conn.execute("ATTACH DATABASE ':memory:' AS activity", [])?;
    } else {
        let path = crate::paths::activity_file()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        conn.execute(
            "ATTACH DATABASE ?1 AS activity",
            [path.to_string_lossy().as_ref()],
        )?;
    }
    if initialize {
        conn.execute_batch(ATTACHED_SCHEMA)?;
    }
    // Preserve hand-entered stats from builds predating the activity store.
    let has_legacy: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='table' AND name='track_stat')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if has_legacy && initialize {
        conn.execute_batch(
            "INSERT OR IGNORE INTO activity.track_stat
             SELECT * FROM main.track_stat;",
        )?;
    }
    Ok(())
}

struct Store {
    conn: Connection,
}

#[derive(Debug, Clone)]
struct Submission {
    id: i64,
    provider: Provider,
    artist: String,
    title: String,
    album: Option<String>,
    album_artist: Option<String>,
    track_no: Option<u32>,
    duration_secs: Option<i64>,
    started_at: i64,
    attempts: i64,
}

impl Store {
    fn open() -> Result<Self> {
        let path = crate::paths::activity_file()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening activity at {}", path.display()))?;
        conn.execute_batch(SCHEMA)?;
        migrate_schema(&conn)?;
        // Reap only stale attempts. Another Staramp window may have opened
        // this database while the playback owner is still checkpointing.
        conn.execute(
            "UPDATE play_history SET outcome='interrupted', ended_at=updated_at
             WHERE ended_at IS NULL AND updated_at < ?1",
            [crate::library::db::now_secs() - 15],
        )?;
        Ok(Self { conn })
    }

    fn start(&self, item: &QueueItem, duration_ms: Option<i64>, now: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO play_history
             (uri, artist, title, album, year, album_artist, track_no, started_at,
              duration_ms, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?8)",
            params![
                item.uri.to_string(),
                item.artist,
                item.title,
                item.album,
                item.year,
                item.album_artist,
                item.track_no,
                now,
                duration_ms
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn checkpoint(&self, id: i64, listened_ms: i64, duration_ms: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE play_history SET listened_ms=?2, duration_ms=COALESCE(?3,duration_ms),
             updated_at=?4 WHERE id=?1",
            params![id, listened_ms, duration_ms, crate::library::db::now_secs()],
        )?;
        Ok(())
    }

    fn mark_play(&self, id: i64, uri: &str, ratio: Option<f64>) -> Result<()> {
        let now = crate::library::db::now_secs();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE play_history SET local_play=1 WHERE id=?1", [id])?;
        tx.execute(
            "INSERT INTO track_stat
             (uri,play_count,last_played_at,first_played_at,completion_ratio)
             VALUES (?1,1,?2,?2,?3)
             ON CONFLICT(uri) DO UPDATE SET
               play_count=play_count+1, last_played_at=excluded.last_played_at,
               first_played_at=COALESCE(first_played_at,excluded.first_played_at),
               completion_ratio=excluded.completion_ratio",
            params![uri, now, ratio],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn finish(
        &self,
        id: i64,
        uri: &str,
        listened_ms: i64,
        duration_ms: Option<i64>,
        outcome: &str,
        skip: bool,
    ) -> Result<()> {
        let now = crate::library::db::now_secs();
        let ratio = duration_ms
            .filter(|d| *d > 0)
            .map(|d| (listened_ms as f64 / d as f64).clamp(0.0, 1.0));
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE play_history SET ended_at=?2, listened_ms=?3,
             duration_ms=COALESCE(?4,duration_ms), outcome=?5, updated_at=?2 WHERE id=?1",
            params![id, now, listened_ms, duration_ms, outcome],
        )?;
        tx.execute(
            "INSERT INTO track_stat (uri,total_played_ms,completion_ratio)
             VALUES (?1,?2,?3) ON CONFLICT(uri) DO UPDATE SET
             total_played_ms=total_played_ms+excluded.total_played_ms,
             completion_ratio=excluded.completion_ratio",
            params![uri, listened_ms, ratio],
        )?;
        if skip {
            tx.execute(
                "UPDATE track_stat SET skip_count=skip_count+1 WHERE uri=?1",
                [uri],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn queue(&self, id: i64, provider: Provider) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO scrobble_delivery(history_id,provider,state)
             VALUES (?1,?2,'pending')",
            params![id, provider.key()],
        )?;
        Ok(())
    }

    fn pending(&self, enabled: [bool; 2]) -> Result<Vec<Submission>> {
        let now = crate::library::db::now_secs();
        let mut stmt = self.conn.prepare(
            "SELECT h.id,d.provider,h.artist,h.title,h.album,h.album_artist,h.track_no,
                    h.duration_ms,h.started_at,d.attempts
             FROM scrobble_delivery d JOIN play_history h ON h.id=d.history_id
             WHERE d.state IN ('pending','retry') AND d.next_retry_at<=?1
             ORDER BY h.started_at LIMIT 50",
        )?;
        let rows = stmt.query_map([now], |r| {
            let key: String = r.get(1)?;
            let provider = if key == "lastfm" {
                Provider::Lastfm
            } else {
                Provider::Listenbrainz
            };
            Ok(Submission {
                id: r.get(0)?,
                provider,
                artist: r.get(2)?,
                title: r.get(3)?,
                album: r.get(4)?,
                album_artist: r.get(5)?,
                track_no: r.get::<_, Option<u32>>(6)?,
                duration_secs: r.get::<_, Option<i64>>(7)?.map(|v| v / 1000),
                started_at: r.get(8)?,
                attempts: r.get(9)?,
            })
        })?;
        Ok(rows
            .flatten()
            .filter(|s| enabled[provider_index(s.provider)])
            .collect())
    }

    fn delivered(&self, s: &Submission) -> Result<()> {
        self.conn.execute(
            "UPDATE scrobble_delivery SET state='sent',submitted_at=?3,last_error=NULL
             WHERE history_id=?1 AND provider=?2",
            params![s.id, s.provider.key(), crate::library::db::now_secs()],
        )?;
        Ok(())
    }

    fn failed(&self, s: &Submission, error: &str, permanent: bool) -> Result<()> {
        let attempts = s.attempts + 1;
        let delay = (30_i64.saturating_mul(1_i64 << (attempts - 1).min(7))).min(3600);
        self.conn.execute(
            "UPDATE scrobble_delivery SET state=?3,attempts=?4,next_retry_at=?5,last_error=?6
             WHERE history_id=?1 AND provider=?2",
            params![
                s.id,
                s.provider.key(),
                if permanent { "blocked" } else { "retry" },
                attempts,
                crate::library::db::now_secs() + delay,
                truncate_error(error)
            ],
        )?;
        Ok(())
    }

    fn retry_all(&self) -> Result<()> {
        self.conn.execute(
            "UPDATE scrobble_delivery SET state='pending',next_retry_at=0,last_error=NULL
             WHERE state IN ('retry','blocked')",
            [],
        )?;
        Ok(())
    }

    fn recent(&self, n: usize) -> Result<Vec<Recent>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.artist,h.title,h.album,h.year,h.uri,h.outcome,h.listened_ms,
                    SUM(d.state='sent'),COUNT(d.history_id),SUM(d.state IN ('retry','blocked'))
             FROM play_history h LEFT JOIN scrobble_delivery d ON d.history_id=h.id
             GROUP BY h.id ORDER BY h.started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([n as i64], |r| {
            Ok(Recent {
                artist: r.get(0)?,
                title: r.get(1)?,
                album: r.get(2)?,
                year: r.get(3)?,
                uri: r.get(4)?,
                outcome: r.get(5)?,
                listened_ms: r.get(6)?,
                sent: r.get::<_, i64>(7).unwrap_or(0),
                deliveries: r.get(8)?,
                errors: r.get::<_, i64>(9).unwrap_or(0),
            })
        })?;
        Ok(rows.flatten().collect())
    }

    fn pending_count(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM scrobble_delivery WHERE state!='sent'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }
}

fn truncate_error(s: &str) -> String {
    s.chars().take(240).collect()
}

#[derive(Debug, Clone, Default)]
pub struct Recent {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub year: Option<i64>,
    pub uri: String,
    pub outcome: String,
    pub listened_ms: i64,
    pub sent: i64,
    pub deliveries: i64,
    pub errors: i64,
}

impl Recent {
    pub fn name(&self) -> String {
        let mut name = match (&self.artist, &self.title) {
            (Some(a), Some(t)) if !a.trim().is_empty() && !t.trim().is_empty() => {
                format!("{a} — {t}")
            }
            (_, Some(t)) if !t.trim().is_empty() => t.clone(),
            _ => self.uri.clone(),
        };
        if let Some(album) = self.album.as_deref().filter(|v| !v.trim().is_empty()) {
            name.push_str(&format!(" ({album})"));
        }
        if let Some(year) = self.year.filter(|v| *v > 0) {
            name.push_str(&format!(" · {year}"));
        }
        name
    }

    pub fn state(&self) -> String {
        if self.outcome == "playing" {
            return "playing".into();
        }
        if self.errors > 0 {
            return "error".into();
        }
        if self.deliveries > 0 && self.sent == self.deliveries {
            return "sent".into();
        }
        if self.deliveries > 0 {
            return if self.sent > 0 { "partial" } else { "queued" }.into();
        }
        self.outcome.clone()
    }
}

#[derive(Debug, Clone)]
pub struct ProviderStatus {
    pub provider: Provider,
    pub enabled: bool,
    pub configured: bool,
    pub username: String,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub providers: Vec<ProviderStatus>,
    pub recent: Vec<Recent>,
    pub pending: i64,
}

enum Message {
    Observe {
        owner: bool,
        revision: u64,
        state: PlayState,
        item: Option<Box<QueueItem>>,
        position: f64,
        duration: f64,
        at: Instant,
    },
    ManualEnd,
    Enabled(Provider, bool),
    Retry,
    Shutdown,
}

#[derive(Clone)]
pub struct Control {
    tx: Sender<Message>,
}

impl Control {
    pub fn manual_end(&self) {
        let _ = self.tx.send(Message::ManualEnd);
    }

    pub fn set_enabled(&self, provider: Provider, enabled: bool) {
        let _ = self.tx.send(Message::Enabled(provider, enabled));
    }
}

pub struct Handle {
    tx: Sender<Message>,
    snapshot: Arc<Mutex<Snapshot>>,
    worker: Option<JoinHandle<()>>,
    last_sent: Instant,
    last_revision: u64,
    last_state: PlayState,
}

impl Handle {
    pub fn spawn(lastfm: bool, listenbrainz: bool) -> Result<Self> {
        // Open synchronously so startup reports an unusable activity store.
        let store = Store::open()?;
        let (tx, rx) = unbounded();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let published = Arc::clone(&snapshot);
        let worker = thread::Builder::new()
            .name("staramp-scrobbler".into())
            .spawn(move || run(rx, store, [lastfm, listenbrainz], published))?;
        Ok(Self {
            tx,
            snapshot,
            worker: Some(worker),
            last_sent: Instant::now(),
            last_revision: u64::MAX,
            last_state: PlayState::Stopped,
        })
    }

    pub fn observe(
        &mut self,
        owner: bool,
        revision: u64,
        state: PlayState,
        item: Option<QueueItem>,
        position: f64,
        duration: f64,
    ) {
        let now = Instant::now();
        if revision == self.last_revision
            && state == self.last_state
            && now.duration_since(self.last_sent) < Duration::from_millis(250)
        {
            return;
        }
        self.last_sent = now;
        self.last_revision = revision;
        self.last_state = state;
        let _ = self.tx.send(Message::Observe {
            owner,
            revision,
            state,
            item: item.map(Box::new),
            position,
            duration,
            at: now,
        });
    }

    pub fn manual_end(&self) {
        self.control().manual_end();
    }

    pub fn set_enabled(&self, provider: Provider, enabled: bool) {
        self.control().set_enabled(provider, enabled);
    }

    pub fn retry(&self) {
        let _ = self.tx.send(Message::Retry);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn control(&self) -> Control {
        Control {
            tx: self.tx.clone(),
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Active {
    id: i64,
    revision: u64,
    item: QueueItem,
    listened_ms: i64,
    duration_ms: Option<i64>,
    last: Instant,
    state: PlayState,
    local_play: bool,
    queued: [bool; 2],
    now_playing: [bool; 2],
    last_checkpoint: Instant,
}

fn run(rx: Receiver<Message>, store: Store, mut enabled: [bool; 2], out: Arc<Mutex<Snapshot>>) {
    let mut active: Option<Active> = None;
    let mut stop = false;
    while !stop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Message::Observe {
                owner,
                revision,
                state,
                item,
                position,
                duration,
                at,
            }) => {
                accrue(active.as_mut(), at);
                if !owner {
                    finish_active(&store, &mut active, "interrupted", false, enabled);
                } else {
                    let changed = active.as_ref().is_some_and(|a| a.revision != revision);
                    if changed {
                        finish_active(&store, &mut active, "played", true, enabled);
                    }
                    if active.is_none() && state != PlayState::Stopped {
                        if let Some(item) = item {
                            let item = *item;
                            let duration_ms =
                                (duration > 0.0).then_some((duration * 1000.0) as i64);
                            match store.start(&item, duration_ms, crate::library::db::now_secs()) {
                                Ok(id) => {
                                    active = Some(Active {
                                        id,
                                        revision,
                                        item,
                                        listened_ms: 0,
                                        duration_ms,
                                        last: at,
                                        state,
                                        local_play: false,
                                        queued: [false; 2],
                                        now_playing: [false; 2],
                                        last_checkpoint: at,
                                    })
                                }
                                Err(e) => tracing::error!("cannot record play: {e}"),
                            }
                        }
                    }
                    if let Some(a) = active.as_mut() {
                        a.state = state;
                        if duration > 0.0 {
                            a.duration_ms = Some((duration * 1000.0) as i64);
                        }
                        if state == PlayState::Stopped {
                            let natural = duration > 0.0 && position >= duration - 0.75;
                            finish_active(
                                &store,
                                &mut active,
                                if natural { "played" } else { "interrupted" },
                                natural,
                                enabled,
                            );
                        }
                    }
                    qualify(&store, active.as_mut(), enabled);
                }
            }
            Ok(Message::ManualEnd) => {
                accrue(active.as_mut(), Instant::now());
                finish_active(&store, &mut active, "skipped", false, enabled);
            }
            Ok(Message::Enabled(p, on)) => {
                enabled[provider_index(p)] = on;
                qualify(&store, active.as_mut(), enabled);
            }
            Ok(Message::Retry) => {
                if let Err(e) = store.retry_all() {
                    tracing::warn!("cannot retry scrobbles: {e}");
                }
            }
            Ok(Message::Shutdown) => {
                accrue(active.as_mut(), Instant::now());
                finish_active(&store, &mut active, "interrupted", false, enabled);
                stop = true;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                accrue(active.as_mut(), Instant::now());
                qualify(&store, active.as_mut(), enabled);
            }
        }

        if let Ok(pending) = store.pending(enabled) {
            let creds = Credentials::load();
            for provider in Provider::ALL {
                let batch: Vec<&Submission> =
                    pending.iter().filter(|s| s.provider == provider).collect();
                if batch.is_empty() {
                    continue;
                }
                if !creds.configured(provider) {
                    for s in batch {
                        let _ = store.failed(s, "authentication required", true);
                    }
                    continue;
                }
                match submit_batch(provider, &batch, &creds) {
                    Ok(()) => {
                        for s in batch {
                            let _ = store.delivered(s);
                        }
                    }
                    Err(e) => {
                        for s in batch {
                            let _ = store.failed(s, &e.message, e.permanent);
                        }
                    }
                }
            }
        }
        publish(&store, enabled, &out);
    }
}

fn accrue(active: Option<&mut Active>, now: Instant) {
    let Some(a) = active else { return };
    if a.state == PlayState::Playing {
        a.listened_ms += now.duration_since(a.last).as_millis().min(i64::MAX as u128) as i64;
    }
    a.last = now;
}

fn local_threshold(duration_ms: Option<i64>) -> i64 {
    duration_ms.map(|d| (d / 2).min(240_000)).unwrap_or(240_000)
}

fn qualify(store: &Store, active: Option<&mut Active>, enabled: [bool; 2]) {
    let Some(a) = active else { return };
    if !a.local_play && a.listened_ms >= local_threshold(a.duration_ms) {
        let ratio = a
            .duration_ms
            .filter(|d| *d > 0)
            .map(|d| (a.listened_ms as f64 / d as f64).clamp(0.0, 1.0));
        if store
            .mark_play(a.id, &a.item.uri.to_string(), ratio)
            .is_ok()
        {
            a.local_play = true;
        }
    }
    announce_now_playing(a, enabled);
    let network_ok = a.duration_ms.is_some_and(|d| d > 30_000)
        && a.listened_ms >= local_threshold(a.duration_ms)
        && a.item
            .artist
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
        && a.item
            .title
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty());
    if network_ok {
        for p in Provider::ALL {
            let i = provider_index(p);
            if enabled[i] && !a.queued[i] && store.queue(a.id, p).is_ok() {
                a.queued[i] = true;
            }
        }
    }
    if a.last_checkpoint.elapsed() >= Duration::from_secs(5) {
        let _ = store.checkpoint(a.id, a.listened_ms, a.duration_ms);
        a.last_checkpoint = Instant::now();
    }
}

fn announce_now_playing(active: &mut Active, enabled: [bool; 2]) {
    let tagged = active
        .item
        .artist
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
        && active
            .item
            .title
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty());
    if !tagged || active.state != PlayState::Playing {
        return;
    }
    let creds = Credentials::load();
    for provider in Provider::ALL {
        let i = provider_index(provider);
        if enabled[i] && !active.now_playing[i] && creds.configured(provider) {
            active.now_playing[i] = true;
            if let Err(e) = submit_now_playing(&active.item, active.duration_ms, provider, &creds) {
                tracing::debug!(provider = %provider, "now-playing update failed: {}", e.message);
            }
        }
    }
}

fn finish_active(
    store: &Store,
    active: &mut Option<Active>,
    outcome: &str,
    natural: bool,
    enabled: [bool; 2],
) {
    let Some(mut a) = active.take() else { return };
    if natural && !a.local_play {
        let ratio = a
            .duration_ms
            .filter(|d| *d > 0)
            .map(|d| (a.listened_ms as f64 / d as f64).clamp(0.0, 1.0));
        if store
            .mark_play(a.id, &a.item.uri.to_string(), ratio)
            .is_ok()
        {
            a.local_play = true;
        }
    }
    qualify(store, Some(&mut a), enabled);
    let skip = outcome == "skipped" && !a.local_play;
    let final_outcome = if a.local_play { "played" } else { outcome };
    let _ = store.finish(
        a.id,
        &a.item.uri.to_string(),
        a.listened_ms,
        a.duration_ms,
        final_outcome,
        skip,
    );
}

fn publish(store: &Store, enabled: [bool; 2], out: &Arc<Mutex<Snapshot>>) {
    let creds = Credentials::load();
    let providers = Provider::ALL
        .into_iter()
        .map(|p| ProviderStatus {
            provider: p,
            enabled: enabled[provider_index(p)],
            configured: creds.configured(p),
            username: creds.username(p).to_string(),
        })
        .collect();
    let snap = Snapshot {
        providers,
        recent: store.recent(5).unwrap_or_default(),
        pending: store.pending_count(),
    };
    if let Ok(mut held) = out.lock() {
        *held = snap;
    }
}

fn provider_index(p: Provider) -> usize {
    match p {
        Provider::Lastfm => 0,
        Provider::Listenbrainz => 1,
    }
}

struct SubmitError {
    message: String,
    permanent: bool,
}

fn submit_batch(
    provider: Provider,
    batch: &[&Submission],
    creds: &Credentials,
) -> std::result::Result<(), SubmitError> {
    match provider {
        Provider::Lastfm => lastfm_scrobble_batch(batch, &creds.lastfm),
        Provider::Listenbrainz => listenbrainz_scrobble_batch(batch, &creds.listenbrainz),
    }
}

fn submit_now_playing(
    item: &QueueItem,
    duration_ms: Option<i64>,
    provider: Provider,
    creds: &Credentials,
) -> std::result::Result<(), SubmitError> {
    let artist = item.artist.as_deref().unwrap_or_default();
    let title = item.title.as_deref().unwrap_or_default();
    match provider {
        Provider::Lastfm => {
            let c = &creds.lastfm;
            let mut fields = vec![
                ("api_key".into(), c.api_key.clone()),
                ("artist".into(), artist.into()),
                ("method".into(), "track.updateNowPlaying".into()),
                ("sk".into(), c.session_key.clone()),
                ("track".into(), title.into()),
            ];
            if let Some(v) = &item.album {
                fields.push(("album".into(), v.clone()));
            }
            if let Some(v) = &item.album_artist {
                fields.push(("albumArtist".into(), v.clone()));
            }
            if let Some(v) = item.track_no {
                fields.push(("trackNumber".into(), v.to_string()));
            }
            if let Some(v) = duration_ms {
                fields.push(("duration".into(), (v / 1000).to_string()));
            }
            lastfm_call(fields, &c.api_secret).map(|_| ())
        }
        Provider::Listenbrainz => {
            let payload = serde_json::json!({
                "listen_type": "playing_now",
                "payload": [{
                    "track_metadata": {
                        "artist_name": artist,
                        "track_name": title,
                        "release_name": item.album,
                        "additional_info": {
                            "duration_ms": duration_ms,
                            "tracknumber": item.track_no,
                            "release_artist_name": item.album_artist,
                            "media_player": "staramp",
                            "submission_client": "staramp",
                            "submission_client_version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }]
            });
            listenbrainz_call(&payload, &creds.listenbrainz)
        }
    }
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(concat!("staramp/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into()
}

fn lastfm_signature(fields: &[(String, String)], secret: &str) -> String {
    let mut sorted = fields.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut text = String::new();
    for (k, v) in sorted {
        text.push_str(&k);
        text.push_str(&v);
    }
    text.push_str(secret);
    format!("{:x}", md5::compute(text))
}

fn lastfm_call(
    mut fields: Vec<(String, String)>,
    secret: &str,
) -> std::result::Result<serde_json::Value, SubmitError> {
    let sig = lastfm_signature(&fields, secret);
    fields.push(("api_sig".into(), sig));
    fields.push(("format".into(), "json".into()));
    let response = http_agent()
        .post("https://ws.audioscrobbler.com/2.0/")
        .send_form(fields.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_err(|e| SubmitError {
            message: e.to_string(),
            permanent: false,
        })?;
    let status = response.status().as_u16();
    let body: serde_json::Value = response.into_body().read_json().map_err(|e| SubmitError {
        message: format!("invalid Last.fm response: {e}"),
        permanent: false,
    })?;
    if status >= 400 || body.get("error").is_some() {
        let code = body
            .get("error")
            .and_then(|v| v.as_i64())
            .unwrap_or(status as i64);
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Last.fm rejected request");
        return Err(SubmitError {
            message: format!("Last.fm {code}: {msg}"),
            permanent: matches!(code, 4 | 6 | 9 | 10 | 13 | 26),
        });
    }
    Ok(body)
}

fn lastfm_scrobble_batch(
    batch: &[&Submission],
    c: &LastfmCredentials,
) -> std::result::Result<(), SubmitError> {
    let mut f = vec![
        ("api_key".into(), c.api_key.clone()),
        ("method".into(), "track.scrobble".into()),
        ("sk".into(), c.session_key.clone()),
    ];
    for (i, s) in batch.iter().enumerate() {
        f.push((format!("artist[{i}]"), s.artist.clone()));
        f.push((format!("timestamp[{i}]"), s.started_at.to_string()));
        f.push((format!("track[{i}]"), s.title.clone()));
        if let Some(v) = &s.album {
            f.push((format!("album[{i}]"), v.clone()));
        }
        if let Some(v) = &s.album_artist {
            f.push((format!("albumArtist[{i}]"), v.clone()));
        }
        if let Some(v) = s.track_no {
            f.push((format!("trackNumber[{i}]"), v.to_string()));
        }
        if let Some(v) = s.duration_secs {
            f.push((format!("duration[{i}]"), v.to_string()));
        }
    }
    let body = lastfm_call(f, &c.api_secret)?;
    let ignored = body
        .pointer("/scrobbles/@attr/ignored")
        .and_then(|v| v.as_str())
        .unwrap_or("0");
    if ignored != "0" {
        return Err(SubmitError {
            message: "Last.fm ignored the scrobble".into(),
            permanent: true,
        });
    }
    Ok(())
}

fn listenbrainz_scrobble_batch(
    batch: &[&Submission],
    c: &ListenbrainzCredentials,
) -> std::result::Result<(), SubmitError> {
    let listens: Vec<_> = batch
        .iter()
        .map(|s| {
            serde_json::json!({
                "listened_at": s.started_at,
                "track_metadata": {
                    "artist_name": s.artist,
                    "track_name": s.title,
                    "release_name": s.album,
                    "additional_info": {
                        "duration_ms": s.duration_secs.map(|v| v * 1000),
                        "tracknumber": s.track_no,
                        "release_artist_name": s.album_artist,
                        "media_player": "staramp",
                        "submission_client": "staramp",
                        "submission_client_version": env!("CARGO_PKG_VERSION")
                    }
                }
            })
        })
        .collect();
    let payload = serde_json::json!({
        "listen_type": if batch.len() == 1 { "single" } else { "import" },
        "payload": listens
    });
    listenbrainz_call(&payload, c)
}

fn listenbrainz_call(
    payload: &serde_json::Value,
    c: &ListenbrainzCredentials,
) -> std::result::Result<(), SubmitError> {
    let response = http_agent()
        .post("https://api.listenbrainz.org/1/submit-listens")
        .header("Authorization", format!("Token {}", c.token))
        .send_json(payload)
        .map_err(|e| SubmitError {
            message: e.to_string(),
            permanent: false,
        })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(SubmitError {
            message: format!("ListenBrainz HTTP {status}"),
            permanent: matches!(status, 400 | 401 | 403),
        });
    }
    Ok(())
}

fn prompt(label: &str, hidden: bool) -> Result<String> {
    if hidden {
        return rpassword::prompt_password(format!("{label}: ")).context("reading secret");
    }
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_string())
}

#[derive(Clone)]
pub struct LastfmPending {
    api_key: String,
    api_secret: String,
    token: String,
    pub url: String,
}

pub fn begin_lastfm(api_key: String, api_secret: String) -> Result<LastfmPending> {
    anyhow::ensure!(
        !api_key.trim().is_empty() && !api_secret.trim().is_empty(),
        "API key and shared secret are required"
    );
    let api_key = api_key.trim().to_string();
    let api_secret = api_secret.trim().to_string();
    let token = lastfm_call(
        vec![
            ("api_key".into(), api_key.clone()),
            ("method".into(), "auth.getToken".into()),
        ],
        &api_secret,
    )
    .map_err(|e| anyhow::anyhow!(e.message))?
    .get("token")
    .and_then(|v| v.as_str())
    .context("Last.fm returned no token")?
    .to_string();
    let url = format!("https://www.last.fm/api/auth/?api_key={api_key}&token={token}");
    let _ = webbrowser::open(&url);
    Ok(LastfmPending {
        api_key,
        api_secret,
        token,
        url,
    })
}

pub fn complete_lastfm(pending: &LastfmPending) -> Result<String> {
    let session = lastfm_call(
        vec![
            ("api_key".into(), pending.api_key.clone()),
            ("method".into(), "auth.getSession".into()),
            ("token".into(), pending.token.clone()),
        ],
        &pending.api_secret,
    )
    .map_err(|e| anyhow::anyhow!(e.message))?;
    let session = session
        .get("session")
        .context("Last.fm returned no session")?;
    let username = session
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut creds = Credentials::load();
    creds.lastfm = LastfmCredentials {
        api_key: pending.api_key.clone(),
        api_secret: pending.api_secret.clone(),
        session_key: session
            .get("key")
            .and_then(|v| v.as_str())
            .context("Last.fm returned no session key")?
            .into(),
        username: username.clone(),
    };
    creds.save()?;
    Ok(username)
}

pub fn authenticate_listenbrainz(token: String) -> Result<String> {
    let token = token.trim().to_string();
    anyhow::ensure!(!token.is_empty(), "token is required");
    let response = http_agent()
        .get("https://api.listenbrainz.org/1/validate-token")
        .header("Authorization", format!("Token {token}"))
        .call()?;
    let status = response.status().as_u16();
    let body: serde_json::Value = response.into_body().read_json()?;
    anyhow::ensure!(
        status == 200 && body.get("valid").and_then(|v| v.as_bool()) == Some(true),
        "ListenBrainz rejected that token"
    );
    let username = body
        .get("user_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut creds = Credentials::load();
    creds.listenbrainz = ListenbrainzCredentials {
        token,
        username: username.clone(),
    };
    creds.save()?;
    Ok(username)
}

pub fn auth(provider: Provider) -> Result<()> {
    match provider {
        Provider::Lastfm => {
            let api_key = prompt("Last.fm API key", false)?;
            let api_secret = prompt("Last.fm shared secret", true)?;
            let pending = begin_lastfm(api_key, api_secret)?;
            println!("Authorize Staramp here:\n{}", pending.url);
            let _ = prompt("Press Enter after authorizing", false)?;
            complete_lastfm(&pending)?;
        }
        Provider::Listenbrainz => {
            let token = prompt("ListenBrainz user token", true)?;
            authenticate_listenbrainz(token)?;
        }
    }
    println!("{} authenticated", provider.label());
    Ok(())
}

pub fn logout(provider: Provider) -> Result<()> {
    let mut creds = Credentials::load();
    match provider {
        Provider::Lastfm => creds.lastfm = LastfmCredentials::default(),
        Provider::Listenbrainz => creds.listenbrainz = ListenbrainzCredentials::default(),
    }
    creds.save()?;
    let path = crate::paths::config_file()?;
    crate::config::edit::set(
        &path,
        "scrobble",
        provider.key(),
        &crate::config::edit::Value::Bool(false),
    )?;
    println!("{} credentials removed", provider.label());
    Ok(())
}

pub fn print_status() -> Result<()> {
    let cfg = crate::config::Config::load()?;
    let creds = Credentials::load();
    let store = Store::open()?;
    for p in Provider::ALL {
        let enabled = match p {
            Provider::Lastfm => cfg.scrobble.lastfm,
            Provider::Listenbrainz => cfg.scrobble.listenbrainz,
        };
        let who = creds.username(p);
        println!(
            "{:<12} {:<8} {}{}",
            p.label(),
            if enabled { "enabled" } else { "disabled" },
            if creds.configured(p) {
                "authenticated"
            } else {
                "not authenticated"
            },
            if who.is_empty() {
                String::new()
            } else {
                format!(" as {who}")
            }
        );
    }
    println!("pending      {}", store.pending_count());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        Store { conn }
    }

    #[test]
    fn local_threshold_is_half_or_four_minutes() {
        assert_eq!(local_threshold(Some(60_000)), 30_000);
        assert_eq!(local_threshold(Some(600_000)), 240_000);
        assert_eq!(local_threshold(None), 240_000);
    }

    #[test]
    fn lastfm_signature_sorts_fields() {
        let f = vec![
            ("track".into(), "Song".into()),
            ("artist".into(), "Artist".into()),
        ];
        assert_eq!(
            lastfm_signature(&f, "secret"),
            format!("{:x}", md5::compute("artistArtisttrackSongsecret"))
        );
    }

    #[test]
    fn recent_names_fall_back_to_uri() {
        let r = Recent {
            uri: "A/B.flac".into(),
            ..Recent::default()
        };
        assert_eq!(r.name(), "A/B.flac");
    }

    #[test]
    fn recent_names_include_the_album_and_year() {
        let r = Recent {
            artist: Some("Artist".into()),
            title: Some("Track".into()),
            album: Some("Record".into()),
            year: Some(1999),
            uri: "record/track.flac".into(),
            ..Recent::default()
        };
        assert_eq!(r.name(), "Artist — Track (Record) · 1999");
    }

    #[test]
    fn a_qualified_play_credits_the_full_listen_exactly_once() {
        let store = memory_store();
        let item = QueueItem::new(crate::playlist::uri::TrackUri::parse("Album/song.flac"));
        let id = store.start(&item, Some(600_000), 100).unwrap();
        store.mark_play(id, "Album/song.flac", Some(0.5)).unwrap();
        store
            .finish(
                id,
                "Album/song.flac",
                420_000,
                Some(600_000),
                "played",
                false,
            )
            .unwrap();
        let stat: (i64, i64, i64) = store
            .conn
            .query_row(
                "SELECT play_count,skip_count,total_played_ms FROM track_stat WHERE uri=?1",
                ["Album/song.flac"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stat, (1, 0, 420_000));
    }

    #[test]
    fn an_early_manual_end_is_a_skip_not_a_play() {
        let store = memory_store();
        let item = QueueItem::new(crate::playlist::uri::TrackUri::parse("Album/song.flac"));
        let id = store.start(&item, Some(600_000), 100).unwrap();
        store
            .finish(
                id,
                "Album/song.flac",
                10_000,
                Some(600_000),
                "skipped",
                true,
            )
            .unwrap();
        let stat: (i64, i64, i64) = store
            .conn
            .query_row(
                "SELECT play_count,skip_count,total_played_ms FROM track_stat WHERE uri=?1",
                ["Album/song.flac"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stat, (0, 1, 10_000));
    }
}
