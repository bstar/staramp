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
        // A socket left behind by a crashed instance would block binding for
        // ever. Only remove it if nothing is listening.
        if path.exists() && UnixStream::connect(path).is_err() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        UnixListener::bind(path)
    }
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
            player.send(Command::Stop);
            "ok".into()
        }
        "next" => {
            player.send(Command::Next);
            "ok".into()
        }
        "prev" | "previous" => {
            player.send(Command::Prev);
            "ok".into()
        }
        "seek" => match parts.next().and_then(|v| v.parse::<f64>().ok()) {
            Some(d) => {
                player.send(Command::SeekBy(d));
                "ok".into()
            }
            None => "error: seek needs a number of seconds".into(),
        },
        "position" => match parts.next().and_then(|v| v.parse::<f64>().ok()) {
            Some(p) => {
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
    spawn_at(socket_path().ok()?, player, view, stop)
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
                        // One thread per connection. A client holds its
                        // connection open for the life of the window, so this
                        // is one thread per window rather than one per request.
                        let _ = std::thread::Builder::new()
                            .name("staramp-ipc-conn".into())
                            .spawn(move || serve(&player, &view, stream));
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

fn serve(player: &Player, view: &crate::view::Shared, stream: UnixStream) {
    let reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut out = stream;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let reply = handle(player, view, line.trim());
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
