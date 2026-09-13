//! Opt-in Discord Rich Presence for the playback-owning window.
//!
//! This talks only to the local Discord desktop client's IPC socket. There is
//! no bot token, server membership, or Discord account credential to store.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{unbounded, RecvTimeoutError, Sender};
use discord_rich_presence::activity::{Activity, Assets, Button, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use rusqlite::{Connection, OpenFlags};

use crate::audio::player::PlayState;
use crate::playlist::queue::QueueItem;

const HEARTBEAT: Duration = Duration::from_secs(15);
/// Legcord/arRPC can acknowledge writes while silently losing the activity it
/// associates with a long-lived client. Periodically replacing that client
/// makes the presence self-healing instead of reporting a misleading
/// `connected` forever.
const RECONNECT_AFTER: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub enabled: bool,
    pub configured: bool,
    pub connected: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Presence {
    revision: u64,
    state: PlayState,
    title: String,
    artist: String,
    album: String,
    uri: String,
    art_url: Option<String>,
    year: Option<i64>,
    start: i64,
    end: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Signature {
    revision: u64,
    state: PlayState,
    start_second: i64,
}

impl Signature {
    /// The wall clock and playback clock are sampled separately, so their
    /// inferred start can wobble by a second as either clock crosses a whole
    /// second. That is not a seek and must not consume Discord's small update
    /// allowance. A real seek moves the inferred start by much more.
    fn same_activity(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.state == other.state
            && self.start_second.abs_diff(other.start_second) <= 2
    }
}

enum Message {
    Presence(Option<Presence>),
    Enabled(bool),
    Shutdown,
}

struct Observed {
    signature: Option<Signature>,
    sent_at: Instant,
}

pub struct Handle {
    tx: Sender<Message>,
    status: Arc<Mutex<Snapshot>>,
    observed: Mutex<Observed>,
}

impl Handle {
    pub fn spawn(config: &crate::config::DiscordPresence, index_path: Option<PathBuf>) -> Self {
        let configured = valid_client_id(&config.client_id);
        let status = Arc::new(Mutex::new(Snapshot {
            enabled: config.enabled,
            configured,
            connected: false,
            error: (!config.client_id.is_empty() && !configured)
                .then(|| "client_id must contain only digits".into()),
        }));
        let (tx, rx) = unbounded();
        let worker_status = Arc::clone(&status);
        let client_id = config.client_id.clone();
        let lastfm_username = config.lastfm_username.clone();
        let initial_enabled = config.enabled;
        std::thread::Builder::new()
            .name("discord-presence".into())
            .spawn(move || {
                let mut enabled = initial_enabled;
                let mut client: Option<DiscordIpcClient> = None;
                let mut connected_at = Instant::now();
                let mut last_presence: Option<Presence> = None;
                let index = index_path.and_then(open_index);
                let mut art_cache: HashMap<String, Option<String>> = HashMap::new();
                let http = discord_http_agent();
                loop {
                    // Own the refresh clock here rather than relying on the UI
                    // to keep sending observations. Besides making reconnects
                    // independent of rendering, this lets a newly restarted
                    // Discord client recover the last known track immediately.
                    let message = match rx.recv_timeout(HEARTBEAT) {
                        Ok(Message::Presence(presence)) => {
                            last_presence = presence.clone();
                            Message::Presence(presence)
                        }
                        Ok(message) => message,
                        Err(RecvTimeoutError::Timeout) => Message::Presence(last_presence.clone()),
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    match message {
                        Message::Enabled(value) => {
                            enabled = value;
                            let mut status = worker_status.lock().unwrap();
                            status.enabled = value;
                            if !value {
                                last_presence = None;
                                clear(&mut client);
                                status.connected = false;
                                status.error = None;
                            }
                        }
                        Message::Presence(presence) => {
                            if !enabled || !configured {
                                continue;
                            }
                            let result = match presence {
                                Some(mut presence) => {
                                    if let Some(cached) = art_cache.get(&presence.uri) {
                                        presence.art_url = cached.clone();
                                        publish(
                                            &mut client,
                                            &mut connected_at,
                                            &client_id,
                                            &lastfm_username,
                                            &presence,
                                        )
                                    } else {
                                        presence.art_url =
                                            cover_art_url(index.as_ref(), &presence.uri);
                                        if presence.art_url.is_none() {
                                            // Presence is the primary feature. Never hold it
                                            // behind a DNS request or a slow metadata service;
                                            // publish immediately, then enrich it with art.
                                            match publish(
                                                &mut client,
                                                &mut connected_at,
                                                &client_id,
                                                &lastfm_username,
                                                &presence,
                                            ) {
                                                Err(error) => Err(error),
                                                Ok(()) => {
                                                    presence.art_url = musicbrainz_cover_url(
                                                        &http,
                                                        &presence.artist,
                                                        &presence.album,
                                                    );
                                                    art_cache.insert(
                                                        presence.uri.clone(),
                                                        presence.art_url.clone(),
                                                    );
                                                    if presence.art_url.is_some() {
                                                        publish(
                                                            &mut client,
                                                            &mut connected_at,
                                                            &client_id,
                                                            &lastfm_username,
                                                            &presence,
                                                        )
                                                    } else {
                                                        Ok(())
                                                    }
                                                }
                                            }
                                        } else {
                                            art_cache.insert(
                                                presence.uri.clone(),
                                                presence.art_url.clone(),
                                            );
                                            publish(
                                                &mut client,
                                                &mut connected_at,
                                                &client_id,
                                                &lastfm_username,
                                                &presence,
                                            )
                                        }
                                    }
                                }
                                None => {
                                    clear(&mut client);
                                    Ok(())
                                }
                            };
                            let mut status = worker_status.lock().unwrap();
                            match result {
                                Ok(()) => {
                                    status.connected = client.is_some();
                                    status.error = None;
                                }
                                Err(error) => {
                                    tracing::warn!("Discord presence unavailable: {error}");
                                    clear(&mut client);
                                    status.connected = false;
                                    status.error = Some(error);
                                }
                            }
                        }
                        Message::Shutdown => {
                            clear(&mut client);
                            break;
                        }
                    }
                }
            })
            .expect("spawn Discord presence worker");
        Self {
            tx,
            status,
            observed: Mutex::new(Observed {
                signature: None,
                sent_at: Instant::now() - HEARTBEAT,
            }),
        }
    }

    pub fn observe(
        &self,
        owner: bool,
        revision: u64,
        state: PlayState,
        item: Option<QueueItem>,
        position: f64,
        duration: f64,
    ) {
        if !owner {
            return;
        }
        let now = Instant::now();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i64::MAX as u64) as i64;
        let start = epoch.saturating_sub(position.max(0.0) as i64);
        let presence = item
            .filter(|_| state != PlayState::Stopped)
            .map(|item| Presence {
                revision,
                state,
                title: item.title.unwrap_or_else(|| item.uri.to_string()),
                artist: item.artist.unwrap_or_default(),
                album: item.album.unwrap_or_default(),
                uri: item.uri.to_string(),
                art_url: None,
                year: item.year,
                start,
                end: start.saturating_add(duration.max(0.0) as i64),
            });
        let signature = presence.as_ref().map(|p| Signature {
            revision: p.revision,
            state: p.state,
            start_second: p.start,
        });
        let mut observed = self.observed.lock().unwrap();
        let unchanged = observed
            .signature
            .as_ref()
            .zip(signature.as_ref())
            .is_some_and(|(old, new)| old.same_activity(new))
            || observed.signature.is_none() && signature.is_none();
        if unchanged && now.duration_since(observed.sent_at) < HEARTBEAT {
            return;
        }
        observed.signature = signature;
        observed.sent_at = now;
        let _ = self.tx.send(Message::Presence(presence));
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.status.lock().unwrap().enabled = enabled;
        let mut observed = self.observed.lock().unwrap();
        observed.signature = None;
        observed.sent_at = Instant::now() - HEARTBEAT;
        let _ = self.tx.send(Message::Enabled(enabled));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.status.lock().unwrap().clone()
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Message::Shutdown);
    }
}

fn publish(
    client: &mut Option<DiscordIpcClient>,
    connected_at: &mut Instant,
    client_id: &str,
    lastfm_username: &str,
    presence: &Presence,
) -> Result<(), String> {
    if client.is_some() && connected_at.elapsed() >= RECONNECT_AFTER {
        // Do not send CLEAR_ACTIVITY: the replacement publish follows
        // immediately. Dropping the stale socket is enough to make arRPC
        // discard its broken client state.
        *client = None;
    }
    if client.is_none() {
        safe_socket_dir()?;
        let mut connected = DiscordIpcClient::new(client_id);
        connected
            .connect()
            .map_err(|error| format!("Discord desktop is unavailable: {error}"))?;
        *client = Some(connected);
        *connected_at = Instant::now();
    }

    let activity = activity_for(presence, lastfm_username);
    let connected = client.as_mut().unwrap();
    connected
        .set_activity(activity)
        .map_err(|error| format!("could not send Discord presence: {error}"))?;
    // arRPC implementations do not consistently answer SET_ACTIVITY. Reading
    // here can block the only publishing worker forever. The socket is
    // replaced regularly, so at most a handful of unread optional replies can
    // accumulate before they are discarded with the old connection.
    Ok(())
}

/// Refuse to look for the Discord socket in a directory anyone can write to.
///
/// The client library searches `$XDG_RUNTIME_DIR` first and then falls back to
/// `$TMPDIR`, `$TMP` and `$TEMP`, testing each candidate with an `exists()`
/// that follows symlinks and no check of who owns what it finds. Where the
/// runtime directory is unset -- a cron job, a `sudo` shell, an ssh session
/// without a seat -- that fallback is `/tmp`, and any other local user can
/// leave a socket named `discord-ipc-0` there and be handed the presence
/// stream: what is playing, and the Last.fm name if one is configured.
///
/// No credential is at stake, which is why this refuses rather than tries
/// harder. Presence is a convenience, and a convenience is not worth a channel
/// to a stranger.
fn safe_socket_dir() -> Result<(), String> {
    if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        // `confstr(_CS_DARWIN_USER_TEMP_DIR)` gives launchd's per-user
        // directory, which is what TMPDIR holds in a desktop session.
        return Ok(());
    }
    Err("Discord presence needs XDG_RUNTIME_DIR set; \
         the fallback socket directory is shared with other users"
        .into())
}

fn activity_for(presence: &Presence, lastfm_username: &str) -> Activity<'static> {
    let mut state = if presence.artist.is_empty() {
        presence.album.clone()
    } else if presence.album.is_empty() {
        presence.artist.clone()
    } else {
        format!("{} · {}", presence.artist, presence.album)
    };
    if let Some(year) = presence.year.filter(|year| *year > 0) {
        if !state.is_empty() {
            state.push_str(" · ");
        }
        state.push_str(&year.to_string());
    }
    if presence.state == PlayState::Paused {
        if !state.is_empty() {
            state.push_str(" · ");
        }
        state.push_str("paused");
    }

    // Legcord's arRPC bridge clears an activity when its IPC socket closes and
    // only fills a missing name after an asynchronous application lookup. Send
    // the name explicitly, as Spotify's native presence does, so there is
    // never a transient nameless activity for Discord to discard.
    let mut activity = Activity::new()
        .name("STAR/AMP")
        .details(limit(&presence.title, 128))
        .state(limit(&state, 128));
    if let Some(url) = &presence.art_url {
        let label = if presence.album.is_empty() {
            presence.title.clone()
        } else {
            presence.album.clone()
        };
        activity = activity.assets(
            Assets::new()
                .large_image(url.clone())
                .large_text(limit(&label, 128)),
        );
    }
    if presence.state == PlayState::Playing && presence.end > presence.start {
        activity = activity.timestamps(Timestamps::new().start(presence.start).end(presence.end));
    }
    if valid_lastfm_username(lastfm_username) {
        activity = activity.buttons(vec![Button::new(
            "Last.fm profile",
            format!("https://www.last.fm/user/{lastfm_username}"),
        )]);
    }
    activity
}

fn open_index(path: PathBuf) -> Option<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

fn cover_art_url(index: Option<&Connection>, uri: &str) -> Option<String> {
    let release_id: Option<String> = index?
        .query_row(
            "SELECT mb_release_id FROM track WHERE uri = ?1 LIMIT 1",
            [uri],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let release_id = release_id?.trim().to_ascii_lowercase();
    valid_musicbrainz_id(&release_id)
        .then(|| format!("https://coverartarchive.org/release/{release_id}/front-500"))
}

fn discord_http_agent() -> ureq::Agent {
    starkit::net::agent(concat!("staramp/", env!("CARGO_PKG_VERSION")))
}

fn musicbrainz_cover_url(agent: &ureq::Agent, artist: &str, album: &str) -> Option<String> {
    if artist.is_empty() || album.is_empty() {
        return None;
    }
    let query = format!(
        "artist:{} AND release:{}",
        lucene_quote(artist),
        lucene_quote(album)
    );
    let response = agent
        .get("https://musicbrainz.org/ws/2/release")
        .query("query", query)
        .query("fmt", "json")
        .query("limit", "8")
        .call()
        .ok()?;
    let body: serde_json::Value = response.into_body().read_json().ok()?;
    let id = body
        .get("releases")?
        .as_array()?
        .iter()
        .find(|release| {
            let title = release
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            crate::library::remote::similarity(album, title) >= 0.9
        })?
        .get("id")?
        .as_str()?;
    valid_musicbrainz_id(id).then(|| format!("https://coverartarchive.org/release/{id}/front-500"))
}

fn lucene_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn valid_musicbrainz_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn clear(client: &mut Option<DiscordIpcClient>) {
    if let Some(mut connected) = client.take() {
        let _ = connected.clear_activity();
        let _ = connected.close();
    }
}

fn valid_client_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_lastfm_username(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn limit(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ids_are_numeric_and_not_optional_when_enabled() {
        assert!(valid_client_id("123456789012345678"));
        assert!(!valid_client_id(""));
        assert!(!valid_client_id("123-no"));
    }

    #[test]
    fn discord_field_limits_do_not_split_unicode() {
        assert_eq!(limit("ab💿cd", 3), "ab💿");
    }

    #[test]
    fn profile_names_cannot_turn_into_a_different_url() {
        assert!(valid_lastfm_username("my-name_2"));
        assert!(!valid_lastfm_username("../../elsewhere"));
    }

    #[test]
    fn activity_uses_the_rpc_compatible_shape() {
        let presence = Presence {
            revision: 7,
            state: PlayState::Playing,
            title: "Merlin".into(),
            artist: "Nightscape".into(),
            album: "Symphony of the Night".into(),
            uri: "Nightscape/Merlin.flac".into(),
            art_url: Some(
                "https://coverartarchive.org/release/12345678-1234-1234-1234-123456789abc/front-500"
                    .into(),
            ),
            year: Some(2005),
            start: 1_000,
            end: 1_180,
        };
        let json = serde_json::to_value(activity_for(&presence, "listener")).unwrap();
        assert_eq!(json["details"], "Merlin");
        assert_eq!(json["name"], "STAR/AMP");
        assert!(json.get("type").is_none());
        assert!(json.get("status_display_type").is_none());
        assert_eq!(json["timestamps"]["start"], 1_000);
        assert_eq!(json["buttons"][0]["label"], "Last.fm profile");
        assert_eq!(
            json["assets"]["large_image"],
            "https://coverartarchive.org/release/12345678-1234-1234-1234-123456789abc/front-500"
        );
    }

    #[test]
    fn only_musicbrainz_uuids_can_become_cover_urls() {
        assert!(valid_musicbrainz_id("12345678-1234-1234-1234-123456789abc"));
        assert!(!valid_musicbrainz_id("../../something-hostile"));
    }

    #[test]
    fn clock_jitter_is_not_a_new_activity_but_a_seek_is() {
        let original = Signature {
            revision: 7,
            state: PlayState::Playing,
            start_second: 1_000,
        };
        assert!(original.same_activity(&Signature {
            start_second: 1_001,
            ..original.clone()
        }));
        assert!(!original.same_activity(&Signature {
            start_second: 1_030,
            ..original
        }));
    }
}
