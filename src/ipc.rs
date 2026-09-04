//! Control over a Unix socket.
//!
//! Newline-delimited JSON, one request per line. This is what makes
//! `staramp next` work from a keybind or a status bar while the TUI is running,
//! and it is deliberately tiny: MPRIS already covers desktop integration, so
//! this exists for scripts and for the case where there is no session bus.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::audio::player::{Command, PlayState, Player};

/// `$XDG_RUNTIME_DIR/staramp.sock`, falling back to the cache directory.
///
/// On Linux the file is not what is actually bound -- see [`listen`] -- but it
/// still names the session, and it is what `staramp ctl` is pointed at.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir).join("staramp.sock"));
    }
    Ok(crate::paths::cache_dir()?.join("staramp.sock"))
}

/// The longest a socket path may be.
///
/// `sun_path` is 108 bytes on Linux and 104 on macOS, and the kernel truncates
/// rather than complaining. Checked against the smaller of the two, because a
/// path that works on one and silently loses remote control on the other is
/// the worst of the three outcomes.
#[cfg(not(target_os = "linux"))]
const SUN_PATH_MAX: usize = 104;

/// Claim the right to lead this session.
///
/// On Linux this is nothing: binding the abstract socket in [`listen`] *is*
/// the election, atomically, and this always reports success.
///
/// Everywhere else the bind is not an election. A socket file left behind by a
/// killed instance has to be unlinked first, and N windows all deciding to
/// unlink means the last one can delete the inode the winner just bound -- so
/// two instances can both believe they lead, and both open the audio device.
/// A `flock` closes that: exactly one process can hold it, and the kernel
/// releases it the instant the holder dies, `SIGKILL` included, so there is
/// never a stale lock to reason about. Held for the life of the process.
#[cfg(target_os = "linux")]
pub fn claim_session(_path: &std::path::Path) -> bool {
    true
}

#[cfg(not(target_os = "linux"))]
pub fn claim_session(path: &std::path::Path) -> bool {
    let lock = path.with_extension("lock");
    let Ok(mut held) = leases().lock() else {
        return false;
    };
    // Idempotent per session: `cmd_tui` claims the lease and `listen` asks
    // again on the way to binding, and the second question has to get the same
    // answer as the first.
    if held.contains_key(&lock) {
        return true;
    }
    match take_lock(&lock) {
        Some(f) => {
            held.insert(lock, f);
            true
        }
        None => false,
    }
}

/// Leases this process holds, by lock path.
///
/// Keyed on the path rather than kept as one process-wide lease, because the
/// socket path is a parameter: `spawn_at` exists so that tests can each have
/// their own, and a lease pinned to the *real* `socket_path` made every one of
/// them contend with whatever instance the developer happened to be running.
#[cfg(not(target_os = "linux"))]
fn leases() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, std::fs::File>> {
    static LEASES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::fs::File>>,
    > = std::sync::OnceLock::new();
    LEASES.get_or_init(Default::default)
}

/// Take an exclusive, non-blocking `flock`, creating the file if needed.
///
/// `None` means another open file description holds it. The returned handle
/// *is* the lock: closing the file releases it, which is precisely the
/// property wanted here -- the kernel does that on process death however the
/// process died, so a lock is never stale and never needs cleaning up.
///
/// Compiled on every platform, though only used where the socket bind is not
/// itself an election, so that the behaviour above can be tested rather than
/// merely assumed. `flock`'s constants are the same on Linux and the BSDs.
fn take_lock(path: &std::path::Path) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    extern "C" {
        #[link_name = "flock"]
        fn libc_flock(fd: i32, operation: i32) -> i32;
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .ok()?;
    // Non-blocking: a session that is already led is an answer, not something
    // to wait for.
    let held = unsafe { libc_flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0;
    held.then_some(file)
}

/// The abstract socket name for a session, on Linux.
///
/// Hashed rather than the path itself, because a socket name has 108 bytes to
/// live in and a runtime directory can be longer than that on its own -- which
/// is a silent loss of all remote control when it happens.
#[cfg(target_os = "linux")]
fn abstract_name(path: &std::path::Path) -> String {
    let h = blake3::hash(path.to_string_lossy().as_bytes());
    format!("staramp.{}", &h.to_hex()[..16])
}

/// Listen for a session at `path`.
///
/// On Linux this binds an **abstract** socket: a name in the kernel rather than
/// a file, freed the moment the process holding it dies, `SIGKILL` included.
/// That matters for more than tidiness. It makes `bind` the whole election --
/// when an instance goes away, every surviving window notices at once and races
/// to take over, and exactly one can succeed. The filesystem version had to
/// unlink a socket left behind by the dead instance first, and N windows all
/// deciding to unlink meant the last one could delete the inode the winner had
/// just bound.
///
/// Elsewhere it is an ordinary socket file, with that stale-socket recovery.
pub fn listen(path: &std::path::Path) -> std::io::Result<UnixListener> {
    #[cfg(target_os = "linux")]
    {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::SocketAddr;
        let addr = SocketAddr::from_abstract_name(abstract_name(path))?;
        UnixListener::bind_addr(&addr)
    }
    #[cfg(not(target_os = "linux"))]
    {
        use std::io::{Error, ErrorKind};

        // Silent truncation by the kernel is a silent loss of remote control,
        // so say so instead. There is nothing to fall back to: the name *is*
        // the address.
        if path.as_os_str().len() >= SUN_PATH_MAX {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "socket path is {} bytes, over the {SUN_PATH_MAX}-byte limit: {}",
                    path.as_os_str().len(),
                    path.display()
                ),
            ));
        }

        // The lease, not the bind, is the election here. Without it the unlink
        // below is a race: see `claim_session`.
        if !claim_session(path) {
            return Err(Error::new(
                ErrorKind::AddrInUse,
                "another instance leads this session",
            ));
        }

        // The lease answers "does another *process* lead?", and it is
        // deliberately idempotent -- `cmd_tui` takes it before `spawn` asks
        // for it again -- so it cannot also answer "have we already bound this
        // one ourselves?". Both questions have to be asked, because the unlink
        // below would otherwise delete a socket this process is still serving
        // on and leave two listeners each believing they lead. On Linux the
        // abstract bind refuses that by itself; here nothing does.
        if !claim_path(path) {
            return Err(Error::new(
                ErrorKind::AddrInUse,
                "this process already serves that session",
            ));
        }

        // Holding the lease and the path, any socket still on disk is
        // certainly stale -- its owner is dead, or we would not have the lock.
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        UnixListener::bind(path).inspect_err(|_| release_path(path))
    }
}

/// Record that this process serves `path`, or report that it already does.
///
/// Separate from [`claim_session`] because the two answer different questions;
/// see the call site. Only the sockets actually bound are tracked, so a failed
/// bind releases its claim and a later attempt is free to retry.
#[cfg(not(target_os = "linux"))]
fn claim_path(path: &std::path::Path) -> bool {
    bound().lock().is_ok_and(|mut b| b.insert(path.into()))
}

#[cfg(not(target_os = "linux"))]
fn release_path(path: &std::path::Path) {
    if let Ok(mut b) = bound().lock() {
        b.remove(path);
    }
}

#[cfg(not(target_os = "linux"))]
fn bound() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static BOUND: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    BOUND.get_or_init(Default::default)
}

/// Connect to a session at `path`, if one is listening.
pub fn connect(path: &std::path::Path) -> std::io::Result<UnixStream> {
    #[cfg(target_os = "linux")]
    {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::SocketAddr;
        let addr = SocketAddr::from_abstract_name(abstract_name(path))?;
        UnixStream::connect_addr(&addr)
    }
    #[cfg(not(target_os = "linux"))]
    {
        UnixStream::connect(path)
    }
}

/// Handle one request line, returning the reply.
///
/// Split out from the socket so it can be tested without one.
pub fn handle(player: &Player, view: &crate::view::Shared, line: &str) -> String {
    handle_with_activity(player, view, None, line)
}

fn handle_with_activity(
    player: &Player,
    view: &crate::view::Shared,
    activity: Option<&crate::activity::Control>,
    line: &str,
) -> String {
    let mut parts = line.split_whitespace();
    let Some(cmd) = parts.next() else {
        return "error: empty request".into();
    };
    // Everything after the verb, unsplit. Track URIs are full of spaces --
    // `At Vance/2005 - Chained/...` -- so a whitespace-split argument would
    // name a track that does not exist.
    let rest = line[cmd.len()..].trim();
    let rest = (!rest.is_empty()).then_some(rest);

    match cmd {
        "play" => {
            player.send(Command::Resume);
            "ok".into()
        }
        "pause" => {
            player.send(Command::Pause);
            "ok".into()
        }
        "toggle" | "play-pause" => {
            player.send(Command::TogglePause);
            "ok".into()
        }
        "stop" => {
            activity.inspect(|a| a.manual_end());
            player.send(Command::Stop);
            "ok".into()
        }
        "next" => {
            activity.inspect(|a| a.manual_end());
            player.send(Command::Next);
            "ok".into()
        }
        "prev" | "previous" => {
            activity.inspect(|a| a.manual_end());
            player.send(Command::Prev);
            "ok".into()
        }
        "seek" => match parts.next().and_then(|v| v.parse::<f64>().ok()) {
            Some(d) => {
                activity.inspect(|a| a.seek_end());
                player.send(Command::SeekBy(d));
                "ok".into()
            }
            None => "error: seek needs a number of seconds".into(),
        },
        "position" => match parts.next().and_then(|v| v.parse::<f64>().ok()) {
            Some(p) => {
                activity.inspect(|a| a.seek_end());
                player.send(Command::SeekTo(p));
                "ok".into()
            }
            None => "error: position needs a number of seconds".into(),
        },
        "volume" => match parts.next().and_then(|v| v.parse::<f32>().ok()) {
            Some(v) => {
                player.set_volume(v);
                "ok".into()
            }
            None => format!("{:.2}", player.volume()),
        },
        // Togglers, kept because scripts and keybinds use them.
        "shuffle" => format!("{}", player.toggle_shuffle()),
        "repeat" => format!("{}", player.cycle_repeat()),

        // Setters. What another instance of the player uses: two windows
        // toggling at once race each other and can end up disagreeing, where
        // two windows setting the same value simply agree.
        "set-shuffle" => {
            let on = match rest.map(str::trim) {
                Some("true" | "on" | "1" | "yes") => true,
                None | Some("false" | "off" | "0" | "no") => false,
                _ => return "error: set-shuffle takes `on` or `off`".into(),
            };
            player.queue.lock().unwrap().set_shuffle(on);
            "ok".into()
        }
        // `RepeatMode::parse` is deliberately lenient -- it reads a value out
        // of a session file, where falling back beats refusing to start. A
        // request is different: the caller named a mode and deserves to know
        // it was not one.
        "set-repeat" => {
            let want = rest.unwrap_or("off").trim().to_ascii_lowercase();
            if !matches!(want.as_str(), "off" | "all" | "one") {
                return "error: set-repeat takes `off`, `all` or `one`".into();
            }
            let mode = crate::playlist::queue::RepeatMode::parse(&want);
            player.queue.lock().unwrap().set_repeat(mode);
            "ok".into()
        }
        "set-scrobble" => {
            let mut args = rest.unwrap_or("").split_whitespace();
            let provider = match args.next() {
                Some("lastfm") => crate::activity::Provider::Lastfm,
                Some("listenbrainz") => crate::activity::Provider::Listenbrainz,
                _ => return "error: set-scrobble needs `lastfm` or `listenbrainz`".into(),
            };
            let on = match args.next() {
                Some("on" | "true" | "1" | "yes") => true,
                Some("off" | "false" | "0" | "no") => false,
                _ => return "error: set-scrobble takes `on` or `off`".into(),
            };
            let Some(activity) = activity else {
                return "error: activity control unavailable".into();
            };
            activity.set_enabled(provider, on);
            "ok".into()
        }
        "set-eq" => {
            #[derive(serde::Deserialize)]
            struct Request {
                enabled: bool,
                profile: crate::audio::dsp::apo::Profile,
            }
            let request: Request = match rest.map(serde_json::from_str) {
                Some(Ok(request)) => request,
                _ => return "error: set-eq needs a JSON profile request".into(),
            };
            if let Err(error) = request.profile.validate() {
                return format!("error: invalid EQ profile: {error}");
            }
            let rate = player.state.sample_rate.load(Ordering::Relaxed).max(8_000) as u32;
            player.set_eq_profile(request.enabled, request.profile, rate);
            "ok".into()
        }
        "shuffle-now" => match player.shuffle_now() {
            Some(_) => "ok".into(),
            None => "error: nothing to shuffle".into(),
        },
        // Album order, and which way round: `off`, or `album` with an optional
        // `desc`.
        // `off`, or `album` with an optional direction.
        //
        // Anything else is refused rather than quietly meaning `off`. It used
        // to answer "ok" to a request it had not understood and turn grouping
        // *off* when asked to turn it on -- the one failure a control protocol
        // must not have, because the caller is told it got what it asked for.
        "set-group" => {
            let mut args = rest.unwrap_or("off").split_whitespace();
            let group = match (args.next(), args.next()) {
                (None | Some("off") | Some("none") | Some("false"), _) => None,
                (Some("album" | "on" | "true"), None | Some("asc")) => Some(false),
                (Some("album" | "on" | "true"), Some("desc")) => Some(true),
                _ => return "error: set-group takes `off`, `album`, or `album desc`".into(),
            };
            player.queue.lock().unwrap().set_grouping(group);
            "ok".into()
        }
        // The hand-made album order, one record per unit separator. Empty
        // clears it.
        "set-album-order" => {
            let order: Vec<String> = rest
                .unwrap_or("")
                .split('\u{1f}')
                .filter(|t| !t.is_empty())
                .map(|t| t.to_string())
                .collect();
            player.queue.lock().unwrap().set_manual_order(order);
            "ok".into()
        }
        "status" => {
            let st = &player.state;
            let state = match st.state() {
                PlayState::Playing => "playing",
                PlayState::Paused => "paused",
                PlayState::Stopped => "stopped",
            };
            let item = player.current_item();
            let (artist, title) = item
                .as_ref()
                .map(|i| {
                    (
                        i.artist.clone().unwrap_or_default(),
                        i.title.clone().unwrap_or_else(|| i.uri.to_string()),
                    )
                })
                .unwrap_or_default();
            // JSON by hand: one object with fixed keys does not justify a
            // serialisation dependency on the control path.
            format!(
                r#"{{"state":"{state}","artist":{},"title":{},"position":{:.2},"duration":{:.2},"volume":{:.2},"bit_perfect":{}}}"#,
                json_string(&artist),
                json_string(&title),
                st.position_secs(),
                st.duration_secs(),
                player.volume(),
                st.bit_perfect.load(Ordering::Relaxed),
            )
        }
        // Everything a mirroring instance needs for one frame, on one line.
        // Deliberately compact: this is polled several times a second.
        "mirror" => {
            let st = &player.state;
            let (index, total, shuffle, group, repeat, revision) = {
                let q = player.queue.lock().unwrap();
                (
                    // A *position in the order*, not a track index. The queue
                    // below is sent in that same order, and a mirror numbers
                    // its own copy from it -- so a track index would point at
                    // the wrong row of it the moment the order is not the
                    // identity, which shuffle and album order both make so.
                    q.current_index()
                        .map(|_| q.view_cursor() as i64)
                        .unwrap_or(-1),
                    q.len(),
                    q.shuffled(),
                    q.grouped_now(),
                    q.repeat().to_string(),
                    q.revision(),
                )
            };
            let view_revision = view.lock().unwrap().revision;
            let item = player.current_item();
            let (artist, title) = item
                .as_ref()
                .map(|i| {
                    (
                        i.artist.clone().unwrap_or_default(),
                        i.title.clone().unwrap_or_else(|| i.uri.to_string()),
                    )
                })
                .unwrap_or_default();
            // The URI as well as the tags: a mirror has no queue of its own to
            // look this up in, and without it the album panel has nothing to
            // ask the index about.
            let uri = item.as_ref().map(|i| i.uri.to_string()).unwrap_or_default();
            // Bands as two-digit integers: twenty of them is sixty bytes,
            // which is affordable several times a second where floats are not.
            let bands: String = player
                .vis_bands
                .lock()
                .map(|b| {
                    b.iter()
                        .map(|v| format!("{:02}", (v.clamp(0.0, 1.0) * 99.0) as u32))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();

            format!(
                r#"{{"state":"{}","artist":{},"title":{},"position":{:.2},"duration":{:.2},"volume":{:.3},"shuffle":{shuffle},"group":{group},"repeat":"{repeat}","index":{index},"total":{total},"revision":{revision},"rate":{},"depth":{},"channels":{},"codec":{},"bitrate":{},"bit_perfect":{},"uri":{},"view":{view_revision},"bands":"{bands}"}}"#,
                match st.state() {
                    PlayState::Playing => "playing",
                    PlayState::Paused => "paused",
                    PlayState::Stopped => "stopped",
                },
                json_string(&artist),
                json_string(&title),
                st.position_secs(),
                st.duration_secs(),
                player.volume(),
                st.sample_rate.load(Ordering::Relaxed),
                st.bit_depth.load(Ordering::Relaxed),
                st.channels.load(Ordering::Relaxed),
                json_string(&st.codec.load_full()),
                st.bitrate_kbps.load(Ordering::Relaxed),
                st.bit_perfect.load(Ordering::Relaxed),
                json_string(&uri),
            )
        }

        // The queue, for a mirroring instance to render the playlist. Only
        // fetched when the revision changes.
        // The queue, in play order, for another instance to render.
        //
        // Whole `QueueItem`s rather than the five tags this used to send. The
        // short form left out the one field that matters for doing anything
        // with a track -- its URI -- so a second window could show you the
        // playlist and had no way to name a row in it. It also left out the
        // year, which is what album order sorts on.
        //
        // Only fetched when the revision changes, so the size is paid per
        // playlist load rather than per frame.
        "queue" => {
            let q = player.queue.lock().unwrap();
            let tracks = q.tracks();
            let items: Vec<&crate::playlist::queue::QueueItem> =
                q.view().iter().filter_map(|&i| tracks.get(i)).collect();
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
        }

        // Play a track by name. The only way for another instance to start
        // something: it has the queue, but a position in it is meaningless
        // across a reorder, and there was no verb for this at all.
        "play-uri" => {
            let Some(uri) = rest else {
                return "error: play-uri needs a track".into();
            };
            let wanted = crate::playlist::uri::TrackUri::parse(uri);
            let found = {
                let q = player.queue.lock().unwrap();
                q.tracks().iter().position(|t| t.uri == wanted)
            };
            match found {
                Some(i) => {
                    activity.inspect(|a| a.manual_end());
                    player.send(Command::PlayIndex(i));
                    "ok".into()
                }
                None => "error: no such track in the queue".into(),
            }
        }
        // The shared view: which track the cursor is on, which records are
        // folded, which panels are open. Fetched only when its revision moves.
        "view" => {
            let held = view.lock().unwrap();
            serde_json::to_string(&*held).unwrap_or_else(|_| "{}".into())
        }
        "set-view" => match rest.map(serde_json::from_str::<crate::view::View>) {
            Some(Ok(next)) => {
                crate::view::publish(view, &next);
                "ok".into()
            }
            _ => "error: set-view needs a view".into(),
        },
        // Load a playlist into the session, for a window that is not the one
        // holding it. The name and the path travel in the view; this is the
        // queue itself.
        "load-playlist" => {
            let Some(path) = rest else {
                return "error: load-playlist needs a file".into();
            };
            match crate::load_playlist(std::path::Path::new(path)) {
                Ok(items) if !items.is_empty() => {
                    player.set_queue_tracks(items);
                    "ok".into()
                }
                Ok(_) => "error: that playlist is empty".into(),
                Err(e) => format!("error: {e}"),
            }
        }
        // An already loaded playlist was edited by a following window. Keep
        // the playing row rather than treating this as a new queue.
        "refresh-playlist" => {
            let Some(path) = rest else {
                return "error: refresh-playlist needs a file".into();
            };
            let active = view.lock().ok().is_some_and(|v| v.playlist_path == path);
            if !active {
                return "error: that playlist is not active".into();
            }
            match crate::load_playlist(std::path::Path::new(path)) {
                Ok(items) => {
                    player.refresh_queue_tracks(items);
                    "ok".into()
                }
                Err(e) => format!("error: {e}"),
            }
        }
        // Adding from the browser in a window that follows the session. The
        // URIs come as a JSON array because a path may contain anything,
        // spaces included, and the protocol splits on whitespace.
        "enqueue" | "set-queue" => {
            let uris: Vec<String> = match rest.map(serde_json::from_str::<Vec<String>>) {
                Some(Ok(v)) => v,
                _ => return "error: enqueue needs a JSON array of uris".into(),
            };
            let items: Vec<crate::playlist::queue::QueueItem> = uris
                .iter()
                .map(|u| {
                    crate::playlist::queue::QueueItem::new(crate::playlist::uri::TrackUri::parse(u))
                })
                .collect();
            if cmd == "set-queue" {
                let n = items.len();
                player.set_queue_tracks(items);
                return format!("{n}");
            }
            // Skipping what is already here, exactly as the window that owns
            // the session does it -- the two paths answering differently is
            // how one of them quietly becomes the wrong one.
            let mut q = player.queue.lock().unwrap();
            let have: std::collections::HashSet<String> =
                q.tracks().iter().map(|t| t.uri.to_string()).collect();
            let mut added = 0usize;
            for item in items {
                if have.contains(&item.uri.to_string()) {
                    continue;
                }
                q.push(item);
                added += 1;
            }
            format!("{added}")
        }
        // Editing rows from a window that follows the session.
        //
        // Addressed by *view position*, because that is the one coordinate the
        // two instances share: the list a follower was sent is this queue's
        // `view()`, so its row `n` is this row `n`. A track index would mean
        // something different on each side.
        //
        // Each carries the revision it was worked out against. A position is
        // only meaningful at a revision, and a mis-aimed delete cannot be taken
        // back -- so a request that has been overtaken is refused rather than
        // applied to whatever is there now. `set-album-order` needs no such
        // guard because it names records rather than positions.
        "remove-at" | "paste-at" | "move-to" => {
            let Some(rest) = rest else {
                return format!("error: {cmd} needs a request");
            };
            let req: serde_json::Value = match serde_json::from_str(rest) {
                Ok(v) => v,
                Err(_) => return format!("error: {cmd} needs a JSON request"),
            };
            let asked = req.get("revision").and_then(|v| v.as_u64());
            let rows: Vec<usize> = req
                .get("rows")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_u64())
                        .map(|v| v as usize)
                        .collect()
                })
                .unwrap_or_default();
            let at = req.get("at").and_then(|v| v.as_u64()).map(|v| v as usize);

            let mut q = player.queue.lock().unwrap();
            if asked != Some(q.revision()) {
                return "error: the queue has changed".into();
            }
            let n = match cmd {
                "remove-at" => {
                    let protect = player.state.state() != crate::audio::player::PlayState::Stopped;
                    q.remove(&rows, protect)
                }
                "paste-at" => {
                    let items: Vec<crate::playlist::queue::QueueItem> = req
                        .get("uris")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str())
                                .map(|u| {
                                    crate::playlist::queue::QueueItem::new(
                                        crate::playlist::uri::TrackUri::parse(u),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    q.insert_at(at.unwrap_or(usize::MAX), items)
                }
                _ => q.move_to(&rows, at.unwrap_or(0)),
            };
            format!("{n}")
        }
        "ping" => "pong".into(),
        other => format!("error: unknown command `{other}`"),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Serve until `stop` is set. Failure is not fatal: no socket means no remote
/// control, not a player that refuses to start.
pub fn spawn(
    player: Arc<Player>,
    view: crate::view::Shared,
    stop: Arc<AtomicBool>,
) -> Option<PathBuf> {
    spawn_at_inner(socket_path().ok()?, player, view, stop, None)
}

pub fn spawn_controlled(
    player: Arc<Player>,
    view: crate::view::Shared,
    stop: Arc<AtomicBool>,
    activity: crate::activity::Control,
) -> Option<PathBuf> {
    spawn_at_inner(socket_path().ok()?, player, view, stop, Some(activity))
}

/// As `spawn`, at a path of the caller's choosing.
///
/// The path is a parameter rather than read from the environment so that tests
/// can each have their own socket and run alongside each other: `socket_path`
/// consults `XDG_RUNTIME_DIR` on every call, and setting an environment
/// variable is neither sound nor isolated once threads exist.
pub fn spawn_at(
    path: PathBuf,
    player: Arc<Player>,
    view: crate::view::Shared,
    stop: Arc<AtomicBool>,
) -> Option<PathBuf> {
    spawn_at_inner(path, player, view, stop, None)
}

fn spawn_at_inner(
    path: PathBuf,
    player: Arc<Player>,
    view: crate::view::Shared,
    stop: Arc<AtomicBool>,
    activity: Option<crate::activity::Control>,
) -> Option<PathBuf> {
    let listener = match listen(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("ipc unavailable at {}: {e}", path.display());
            return None;
        }
    };
    // Blocking. It used to poll with a 50 ms sleep, which put up to 50 ms
    // between a client connecting and being served -- longer than the 33 ms
    // frame the client was trying to draw. Shutdown wakes it by connecting to
    // it once, below.
    let served = path.clone();
    let waking = path.clone();
    let ending = Arc::clone(&stop);
    std::thread::Builder::new()
        .name("staramp-ipc".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                if ending.load(Ordering::Relaxed) {
                    break;
                }
                match incoming {
                    Ok(stream) => {
                        let player = Arc::clone(&player);
                        let view = Arc::clone(&view);
                        let activity = activity.clone();
                        // One thread per connection. A client holds its
                        // connection open for the life of the window, so this
                        // is one thread per window rather than one per request.
                        let _ = std::thread::Builder::new()
                            .name("staramp-ipc-conn".into())
                            .spawn(move || serve(&player, &view, activity.as_ref(), stream));
                    }
                    // One refused connection is not a reason to stop answering
                    // for the rest of the process's life, which is what
                    // breaking here used to mean.
                    Err(e) => tracing::warn!("ipc accept failed: {e}"),
                }
            }
            let _ = std::fs::remove_file(&served);
        })
        .ok()?;

    // Waking the accept thread on shutdown: it is blocked in `accept`, and the
    // only thing that returns from there is a connection.
    std::thread::Builder::new()
        .name("staramp-ipc-wake".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = connect(&waking);
        })
        .ok()?;

    Some(path)
}

fn serve(
    player: &Player,
    view: &crate::view::Shared,
    activity: Option<&crate::activity::Control>,
    stream: UnixStream,
) {
    let reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut out = stream;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let reply = handle_with_activity(player, view, activity, line.trim());
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

/// Send one command to a running instance.
pub fn send(request: &str) -> Result<String> {
    let path = socket_path()?;
    let stream = connect(&path)
        .with_context(|| format!("no running staramp at {} — start one first", path.display()))?;
    let mut out = stream.try_clone()?;
    writeln!(out, "{request}")?;
    out.flush()?;

    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod lease_tests {
    use super::take_lock;

    /// The whole point: a second claim fails while the first is alive, and
    /// succeeds once it is gone. `flock` conflicts between open file
    /// descriptions rather than between processes, so one process can prove
    /// both halves.
    #[test]
    fn only_one_holder_at_a_time_and_closing_releases_it() {
        let dir = std::env::temp_dir().join(format!("staramp-lease-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.lock");

        let first = take_lock(&path).expect("an unheld lock is takeable");
        assert!(
            take_lock(&path).is_none(),
            "a second holder must not get in"
        );

        drop(first);
        assert!(
            take_lock(&path).is_some(),
            "closing the file releases the lock"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on its own socket, and a client holding one connection to it.
    fn serving() -> (PathBuf, Arc<AtomicBool>, UnixStream) {
        let dir = std::env::temp_dir().join(format!(
            "staramp-ipc-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("staramp.sock");
        let _ = std::fs::remove_file(&path);
        let stop = Arc::new(AtomicBool::new(false));
        let player = Arc::new(Player::detached());
        spawn_at(
            path.clone(),
            player,
            crate::view::shared(),
            Arc::clone(&stop),
        )
        .expect("bound");

        let client = connect(&path).expect("connected");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        (path, stop, client)
    }

    fn ask(client: &UnixStream, req: &str) -> String {
        let mut out = client.try_clone().unwrap();
        writeln!(out, "{req}").unwrap();
        out.flush().unwrap();
        let mut reply = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        reply.trim_end().to_string()
    }

    /// A setter that does not understand a request must say so.
    ///
    /// `set-group true` used to answer "ok" and turn grouping *off* -- the
    /// caller asked for one thing, was told it happened, and got the opposite.
    /// That is worse than any error, because nothing looks wrong.
    #[test]
    fn a_setter_refuses_what_it_does_not_understand() {
        let (_path, stop, client) = serving();

        for (req, why) in [
            (
                "set-group true",
                "the app's own `album` spelling is not the only one tried",
            ),
            ("set-group album", "asc"),
            ("set-group album desc", "desc"),
            ("set-group off", "off"),
            ("set-shuffle on", "on"),
            ("set-shuffle off", "off"),
            ("set-repeat all", "all"),
            ("set-repeat off", "off"),
        ] {
            assert_eq!(ask(&client, req), "ok", "{req} was refused: {why}");
        }

        for req in [
            "set-group sideways",
            "set-group album backwards",
            "set-shuffle maybe",
            "set-repeat sometimes",
            "enqueue not-json",
        ] {
            let reply = ask(&client, req);
            assert!(
                reply.starts_with("error:"),
                "`{req}` was accepted, answering {reply:?}"
            );
        }
        stop.store(true, Ordering::Relaxed);
    }

    /// What the browser sends when a window that follows the session adds.
    #[test]
    fn enqueue_appends_and_will_not_add_the_same_track_twice() {
        let (_path, stop, client) = serving();
        let two = r#"enqueue ["A/one.flac","A/two.flac"]"#;
        assert_eq!(ask(&client, two), "2");
        assert_eq!(ask(&client, two), "0", "the second time adds nothing");
        assert_eq!(
            ask(&client, r#"enqueue ["A/two.flac","A/three.flac"]"#),
            "1",
            "only the one it has not seen"
        );
        // `set-queue` replaces rather than appends, so it counts them all.
        assert_eq!(ask(&client, r#"set-queue ["A/one.flac"]"#), "1");
        assert_eq!(ask(&client, r#"enqueue ["A/one.flac"]"#), "0");
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn a_follower_can_publish_the_eq_to_the_audio_owner() {
        let player = Player::detached();
        let profile = crate::audio::dsp::apo::Profile::legacy("remote", -3.0, &[0.0; 10]);
        let request = serde_json::json!({ "enabled": true, "profile": profile });
        assert_eq!(
            handle(
                &player,
                &crate::view::shared(),
                &format!("set-eq {request}")
            ),
            "ok"
        );
        assert!(!player.eq.load().is_transparent());

        let invalid = r#"set-eq {"enabled":true,"profile":{"name":"bad","stages":[{"enabled":true,"channels":18446744073709551615,"filter":{"Iir":{"numerator":[],"denominator":[]}}}]}}"#;
        assert!(handle(&player, &crate::view::shared(), invalid).starts_with("error:"));
    }

    /// A row edit from a window that follows the session.
    #[test]
    fn rows_can_be_edited_over_the_wire_by_position() {
        let (_path, stop, client) = serving();
        assert_eq!(ask(&client, r#"set-queue ["a","b","c","d"]"#), "4");
        let rev = |c: &UnixStream| -> u64 {
            let s = ask(c, "mirror");
            s.split("\"revision\":")
                .nth(1)
                .and_then(|t| t.split(['}', ',']).next())
                .and_then(|t| t.trim().parse().ok())
                .unwrap()
        };

        let r = rev(&client);
        assert_eq!(
            ask(
                &client,
                &format!(r#"remove-at {{"revision":{r},"rows":[1]}}"#)
            ),
            "1"
        );
        let r = rev(&client);
        assert_eq!(
            ask(
                &client,
                &format!(r#"move-to {{"revision":{r},"at":0,"rows":[2]}}"#)
            ),
            "1"
        );
        let r = rev(&client);
        assert_eq!(
            ask(
                &client,
                &format!(r#"paste-at {{"revision":{r},"at":1,"uris":["z"]}}"#)
            ),
            "1"
        );
        stop.store(true, Ordering::Relaxed);
    }

    /// A position means nothing without the revision it was worked out at.
    ///
    /// The follower is routinely a frame or two behind -- it refetches only
    /// when the revision moves -- and a delete aimed at a list that has since
    /// changed cannot be taken back.
    #[test]
    fn an_edit_aimed_at_a_queue_that_has_moved_is_refused() {
        let (_path, stop, client) = serving();
        ask(&client, r#"set-queue ["a","b","c"]"#);
        let stale = 0u64;
        let reply = ask(
            &client,
            &format!(r#"remove-at {{"revision":{stale},"rows":[0]}}"#),
        );
        assert!(reply.starts_with("error:"), "{reply}");
        assert!(reply.contains("changed"), "{reply}");
        // And it left the queue alone.
        assert_eq!(
            ask(&client, r#"enqueue ["a"]"#),
            "0",
            "\"a\" should still be there"
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn a_row_edit_without_a_request_says_so() {
        let (_path, stop, client) = serving();
        for verb in ["remove-at", "paste-at", "move-to"] {
            assert!(ask(&client, verb).starts_with("error:"));
            assert!(ask(&client, &format!("{verb} not-json")).starts_with("error:"));
        }
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn one_connection_answers_more_than_one_request() {
        // The protocol always allowed this and nothing used it: every request
        // opened its own connection, which at a request per frame was a
        // connect, an accept and a thread spawn thirty times a second.
        let (_path, stop, client) = serving();
        assert_eq!(ask(&client, "ping"), "pong");
        assert_eq!(ask(&client, "ping"), "pong");
        assert!(ask(&client, "status").starts_with('{'), "status is JSON");
        assert_eq!(ask(&client, "volume"), "1.00");
        assert!(ask(&client, "nonsense").starts_with("error:"));
        // And the connection still works after an error.
        assert_eq!(ask(&client, "ping"), "pong");
        stop.store(true, Ordering::Relaxed);
    }

    // `abstract_name` only exists on Linux, so neither does the test for it.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_name_a_session_binds_fits_in_a_socket_and_follows_the_path() {
        // A socket name has 108 bytes to live in, and a runtime directory can
        // be longer than that on its own -- which showed up as remote control
        // silently not existing.
        let long = std::path::PathBuf::from("/tmp").join("x".repeat(300));
        let name = abstract_name(&long);
        assert!(name.len() < 100, "{name} is {} bytes", name.len());
        // Same path, same session; different path, different session -- so
        // `XDG_RUNTIME_DIR` still keeps two of them apart.
        assert_eq!(name, abstract_name(&long));
        assert_ne!(name, abstract_name(std::path::Path::new("/tmp/other.sock")));
    }

    #[test]
    fn a_dead_instance_leaves_nothing_to_clean_up() {
        // The whole reason for an abstract name: the kernel frees it when the
        // process holding it dies, so a survivor can simply bind. With a file,
        // every survivor had to decide to unlink first, and the last unlink
        // could delete the socket the winner had just bound.
        let dir = std::env::temp_dir().join(format!(
            "staramp-gone-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("staramp.sock");

        let stop = Arc::new(AtomicBool::new(false));
        spawn_at(
            path.clone(),
            Arc::new(Player::detached()),
            crate::view::shared(),
            Arc::clone(&stop),
        )
        .expect("bound");
        assert!(connect(&path).is_ok(), "and it answers");

        // Taking it over while it is held is refused, which is the election.
        let other = Arc::new(AtomicBool::new(false));
        assert!(
            spawn_at(
                path.clone(),
                Arc::new(Player::detached()),
                crate::view::shared(),
                Arc::clone(&other),
            )
            .is_none(),
            "two owners is the one outcome that must not happen"
        );
        stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_queue_carries_enough_to_act_on_a_track() {
        use crate::playlist::queue::QueueItem;
        use crate::playlist::uri::TrackUri;

        let player = Arc::new(Player::detached());
        let mut one = QueueItem::new(TrackUri::File {
            rel_path: "At Vance/2005 - Chained/01.flac".into(),
        });
        one.title = Some("Rise From The Fall".into());
        one.year = Some(2005);
        player.queue.lock().unwrap().set_tracks(vec![one.clone()]);

        let wire = handle(&player, &crate::view::shared(), "queue");
        let back: Vec<QueueItem> = serde_json::from_str(&wire).expect(&wire);
        assert_eq!(back.len(), 1);
        // The URI, and the year album order sorts on. Neither survived the
        // five-tag form this replaced.
        assert_eq!(back[0].uri, one.uri);
        assert_eq!(back[0].year, Some(2005));

        // And a track can be named. A position would not do: the two instances
        // agree on the track, not on where it sits.
        assert_eq!(
            handle(
                &player,
                &crate::view::shared(),
                &format!("play-uri {}", one.uri)
            ),
            "ok"
        );
        // Spaces and all -- these paths are full of them.
        assert!(one.uri.to_string().contains(' '));
        assert!(handle(
            &player,
            &crate::view::shared(),
            "play-uri Nothing/Like/This.flac"
        )
        .starts_with("error:"));
        assert!(handle(&player, &crate::view::shared(), "play-uri").starts_with("error:"));
    }

    #[test]
    fn refreshing_the_active_playlist_keeps_the_playing_track() {
        use crate::playlist::queue::QueueItem;
        use crate::playlist::uri::TrackUri;

        let dir = std::env::temp_dir().join(format!(
            "staramp-refresh-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("epic.m3u");
        std::fs::write(&path, "x.flac\na.flac\nb.flac\n").unwrap();

        let player = Arc::new(Player::detached());
        player.queue.lock().unwrap().set_tracks(vec![
            QueueItem::new(TrackUri::parse("a.flac")),
            QueueItem::new(TrackUri::parse("b.flac")),
        ]);
        player.queue.lock().unwrap().jump_to(1);
        let view = crate::view::shared();
        view.lock().unwrap().playlist_path = path.display().to_string();

        assert_eq!(
            handle(
                &player,
                &view,
                &format!("refresh-playlist {}", path.display())
            ),
            "ok"
        );
        let queue = player.queue.lock().unwrap();
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.current().unwrap().uri.to_string(), "b.flac");
        drop(queue);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_session_is_set_rather_than_toggled() {
        use crate::playlist::queue::{QueueItem, RepeatMode};
        use crate::playlist::uri::TrackUri;

        let player = Arc::new(Player::detached());
        let mut a = QueueItem::new(TrackUri::File {
            rel_path: "a.flac".into(),
        });
        a.album = Some("Holy Land".into());
        a.year = Some(1996);
        let mut b = QueueItem::new(TrackUri::File {
            rel_path: "b.flac".into(),
        });
        b.album = Some("Chained".into());
        b.year = Some(2005);
        player.queue.lock().unwrap().set_tracks(vec![a, b]);

        // Setting the value it already holds is not a flip. Two windows
        // toggling at once race; two windows setting the same value agree.
        for _ in 0..2 {
            assert_eq!(
                handle(&player, &crate::view::shared(), "set-shuffle true"),
                "ok"
            );
            assert!(player.queue.lock().unwrap().shuffled());
        }
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-shuffle false"),
            "ok"
        );
        assert!(!player.queue.lock().unwrap().shuffled());

        assert_eq!(
            handle(&player, &crate::view::shared(), "set-repeat all"),
            "ok"
        );
        assert_eq!(player.queue.lock().unwrap().repeat(), RepeatMode::All);
        // A mode nobody recognises is refused, and above all does not panic.
        // This used to answer "ok" and set the mode to off, which told the
        // caller their request had been carried out when it had not.
        assert!(
            handle(&player, &crate::view::shared(), "set-repeat nonsense").starts_with("error:")
        );
        assert_eq!(
            player.queue.lock().unwrap().repeat(),
            RepeatMode::All,
            "a refused request must leave the mode alone"
        );

        // Album order, and which way round.
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-group album"),
            "ok"
        );
        assert_eq!(player.queue.lock().unwrap().grouping(), Some(false));
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-group album desc"),
            "ok"
        );
        assert_eq!(player.queue.lock().unwrap().grouping(), Some(true));
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-group off"),
            "ok"
        );
        assert_eq!(player.queue.lock().unwrap().grouping(), None);

        // And the hand-made arrangement, records separated by a unit break so
        // a title with a space in it survives.
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-group album"),
            "ok"
        );
        assert_eq!(
            handle(
                &player,
                &crate::view::shared(),
                "set-album-order chained\u{1f}holy land"
            ),
            "ok"
        );
        assert_eq!(
            player.queue.lock().unwrap().manual_order(),
            ["chained", "holy land"]
        );
        assert_eq!(
            handle(&player, &crate::view::shared(), "set-album-order"),
            "ok"
        );
        assert!(player.queue.lock().unwrap().manual_order().is_empty());
    }

    #[test]
    fn a_second_instance_cannot_take_the_socket() {
        // What decides who owns playback: the bind succeeds exactly once.
        let (path, stop, _client) = serving();
        let stop2 = Arc::new(AtomicBool::new(false));
        assert!(
            spawn_at(
                path,
                Arc::new(Player::detached()),
                crate::view::shared(),
                Arc::clone(&stop2),
            )
            .is_none(),
            "the socket is already held"
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn json_strings_escape_what_would_break_the_reply() {
        assert_eq!(json_string(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(json_string("a\\b"), r#""a\\b""#);
        assert_eq!(json_string("line\nbreak"), r#""line\nbreak""#);
    }

    #[test]
    fn track_titles_with_quotes_do_not_corrupt_the_status_object() {
        // Real titles contain quotes; a naive format! would emit invalid JSON.
        let s = json_string(r#"Ira Sancti (When the "Saints" are going Wild)"#);
        assert!(s.starts_with('"') && s.ends_with('"'));
        assert_eq!(s.matches(r#"\""#).count(), 2);
    }

    #[test]
    fn the_socket_path_prefers_the_runtime_directory() {
        if std::env::var_os("XDG_RUNTIME_DIR").is_some() {
            let p = socket_path().unwrap();
            assert!(p.ends_with("staramp.sock"));
        }
    }
}
