//! Mirroring another running instance.
//!
//! The first instance binds the control socket and owns the audio device; any
//! later one finds that socket and becomes a mirror. A mirror renders the
//! leader's state and forwards every key to it, so several terminals show and
//! control one playing session rather than fighting over the sound card.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::playlist::queue::QueueItem;

/// One frame of the leader's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MirrorState {
    pub state: String,
    pub artist: String,
    pub title: String,
    /// The leader's current track URI, for looking the album up in the index.
    /// Empty when the leader is stopped, or is an older build.
    pub uri: String,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub shuffle: bool,
    /// Whether the leader has the queue in album order, so a mirror knows to
    /// draw the dividers. It cannot work them out for itself: the leader sends
    /// tags, and none of them is a year.
    pub group: bool,
    /// The shared view's revision, so a follower knows when to ask for it.
    pub view_revision: u64,
    pub repeat: String,
    /// The leader's position in the *order*, not a track index -- the queue
    /// arrives in that order too. -1 when nothing is playing.
    pub index: i64,
    pub total: usize,
    /// Bumped by the leader whenever the queue changes.
    pub revision: u64,
    pub rate: u64,
    pub depth: u64,
    pub channels: u64,
    /// Codec short name as the leader's decoder reported it.
    pub codec: String,
    /// Average bitrate, kbps. `0` when the leader does not know it.
    pub bitrate_kbps: u64,
    pub bit_perfect: bool,
    pub bands: Vec<f32>,
}

pub struct Mirror {
    path: std::path::PathBuf,
    /// The connection, held open for the life of the window.
    ///
    /// It used to be one connect per request, which at a request per frame
    /// meant a connect, an accept and a thread spawn thirty times a second --
    /// on the UI thread, in front of the draw. The protocol always allowed
    /// several requests down one connection; nothing used it.
    ///
    /// `None` after a failure, so the next request reconnects: an instance
    /// that restarts should be picked back up rather than needing this window
    /// restarted too.
    conn: RefCell<Option<Conn>>,
}

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Mirror {
    /// Connect to a running instance, if there is one.
    ///
    /// A socket left behind by a killed instance refuses connections, so
    /// failing to connect is the same as there being no leader.
    pub fn connect() -> Option<Self> {
        let path = crate::ipc::socket_path().ok()?;
        crate::ipc::connect(&path).ok()?;
        Some(Self {
            path,
            conn: RefCell::new(None),
        })
    }

    fn dial(&self) -> Result<Conn> {
        let stream = crate::ipc::connect(&self.path)
            .with_context(|| format!("connecting to {}", self.path.display()))?;
        // Never let a wedged leader freeze this window's own UI.
        stream.set_read_timeout(Some(Duration::from_millis(500)))?;
        stream.set_write_timeout(Some(Duration::from_millis(500)))?;
        Ok(Conn {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    fn request(&self, req: &str) -> Result<String> {
        // Two attempts: the held connection, and if that fails a fresh one.
        // The leader may have restarted since the last request, and one dead
        // socket should not cost the reply.
        for attempt in 0..2 {
            let mut held = self.conn.borrow_mut();
            if held.is_none() {
                *held = Some(self.dial()?);
            }
            let Some(conn) = held.as_mut() else {
                unreachable!()
            };

            let sent = writeln!(conn.writer, "{req}").and_then(|()| conn.writer.flush());
            let mut reply = String::new();
            let got = sent.and_then(|()| conn.reader.read_line(&mut reply));
            match got {
                // A clean zero-length read is the far end having gone away.
                Ok(0) => *held = None,
                Ok(_) => return Ok(reply.trim_end().to_string()),
                Err(e) => {
                    *held = None;
                    if attempt == 1 {
                        return Err(e.into());
                    }
                }
            }
        }
        anyhow::bail!("no reply from {}", self.path.display())
    }

    /// Send a command. Failure is not fatal: the leader may have exited.
    pub fn send(&self, req: &str) {
        let _ = self.request(req);
    }

    /// Send, and hand back what the session said.
    ///
    /// For the requests whose answer changes what the user is told. A leader
    /// older than this window will not know a verb it never had, and reporting
    /// success for a request that was refused is worse than reporting nothing.
    pub fn ask(&self, req: &str) -> Option<String> {
        self.request(req).ok()
    }

    pub fn poll(&self) -> Option<MirrorState> {
        parse_state(&self.request("mirror").ok()?)
    }

    /// The shared view, as the instance that owns the session holds it.
    pub fn view(&self) -> Option<crate::view::View> {
        serde_json::from_str(&self.request("view").ok()?).ok()
    }

    pub fn queue(&self) -> Option<Vec<QueueItem>> {
        Some(parse_queue(&self.request("queue").ok()?))
    }

    /// Is the leader still there?
    pub fn alive(&self) -> bool {
        crate::ipc::connect(&self.path).is_ok()
    }
}

/// Pull a value out of the compact JSON the leader sends.
///
/// A hand-written scan rather than a JSON dependency: the shape is fixed and
/// this runs several times a second.
fn field<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let at = src.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &src[at..];
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = find_unescaped_quote(stripped)?;
        Some(&stripped[..end])
    } else {
        let end = rest.find([',', '}'])?;
        Some(rest[..end].trim())
    }
}

fn find_unescaped_quote(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_state(src: &str) -> Option<MirrorState> {
    if !src.starts_with('{') {
        return None;
    }
    let num = |k: &str| field(src, k).and_then(|v| v.parse::<f64>().ok());
    Some(MirrorState {
        state: field(src, "state").unwrap_or("stopped").to_string(),
        artist: field(src, "artist").map(unescape).unwrap_or_default(),
        title: field(src, "title").map(unescape).unwrap_or_default(),
        uri: field(src, "uri").map(unescape).unwrap_or_default(),
        position: num("position").unwrap_or(0.0),
        duration: num("duration").unwrap_or(0.0),
        volume: num("volume").unwrap_or(1.0) as f32,
        shuffle: field(src, "shuffle") == Some("true"),
        group: field(src, "group") == Some("true"),
        view_revision: num("view").unwrap_or(0.0) as u64,
        repeat: field(src, "repeat").unwrap_or("Off").to_string(),
        index: num("index").unwrap_or(-1.0) as i64,
        total: num("total").unwrap_or(0.0) as usize,
        revision: num("revision").unwrap_or(0.0) as u64,
        rate: num("rate").unwrap_or(0.0) as u64,
        depth: num("depth").unwrap_or(0.0) as u64,
        channels: num("channels").unwrap_or(2.0) as u64,
        codec: field(src, "codec").map(unescape).unwrap_or_default(),
        bitrate_kbps: num("bitrate").unwrap_or(0.0) as u64,
        bit_perfect: field(src, "bit_perfect") == Some("true"),
        bands: field(src, "bands").map(parse_bands).unwrap_or_default(),
    })
}

/// Bands arrive as two-digit integers, 00 to 99.
fn parse_bands(s: &str) -> Vec<f32> {
    s.as_bytes()
        .chunks_exact(2)
        .filter_map(|c| std::str::from_utf8(c).ok())
        .filter_map(|c| c.parse::<u32>().ok())
        .map(|v| v as f32 / 99.0)
        .collect()
}

/// The queue as the other instance sees it, in play order.
///
/// Whole `QueueItem`s. The five-tag form this replaced could not carry a URI,
/// which is the one field needed to ask for a track to be played -- so a second
/// window could render the playlist and do nothing with it.
pub fn parse_queue(src: &str) -> Vec<QueueItem> {
    serde_json::from_str(src).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"state":"playing","artist":"Angra","title":"Nova Era","position":134.50,"duration":300.00,"volume":0.800,"shuffle":true,"repeat":"All","index":41,"total":103,"revision":7,"rate":44100,"depth":16,"channels":2,"codec":"flac","bitrate":1006,"bit_perfect":true,"uri":"Angra/1996 - Holy Land/03.flac","bands":"009950"}"#;

    #[test]
    fn parses_a_full_state_line() {
        let s = parse_state(SAMPLE).expect("should parse");
        assert_eq!(s.state, "playing");
        assert_eq!(s.artist, "Angra");
        assert_eq!(s.uri, "Angra/1996 - Holy Land/03.flac");
        assert_eq!(s.title, "Nova Era");
        assert!((s.position - 134.5).abs() < 1e-6);
        assert!((s.volume - 0.8).abs() < 1e-6);
        assert!(s.shuffle);
        assert_eq!(s.repeat, "All");
        assert_eq!(s.index, 41);
        assert_eq!(s.total, 103);
        assert_eq!(s.revision, 7);
        assert_eq!(s.rate, 44100);
        assert_eq!(s.codec, "flac");
        assert_eq!(s.bitrate_kbps, 1006);
        assert!(s.bit_perfect);
    }

    #[test]
    fn bands_decode_to_the_zero_to_one_range() {
        let s = parse_state(SAMPLE).unwrap();
        assert_eq!(s.bands.len(), 3);
        assert!((s.bands[0] - 0.0).abs() < 1e-6);
        assert!((s.bands[1] - 1.0).abs() < 1e-6);
        assert!((s.bands[2] - 50.0 / 99.0).abs() < 1e-6);
    }

    #[test]
    fn a_title_containing_quotes_survives_the_round_trip() {
        // Real titles contain quotes; a naive scan would stop at the first one.
        let src = r#"{"state":"playing","artist":"X","title":"Ira Sancti (When the \"Saints\" are going Wild)","position":1.00,"duration":2.00,"volume":1.000,"shuffle":false,"repeat":"Off","index":0,"total":1,"revision":1,"rate":44100,"depth":16,"channels":2,"codec":"flac","bitrate":0,"bit_perfect":false,"bands":""}"#;
        let s = parse_state(src).unwrap();
        assert!(s.title.contains("\"Saints\""), "got {:?}", s.title);
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing() {
        let s = parse_state(r#"{"state":"paused"}"#).unwrap();
        assert_eq!(s.state, "paused");
        assert_eq!(s.total, 0);
        assert_eq!(s.index, -1);
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(parse_state("error: unknown command").is_none());
        assert!(parse_state("").is_none());
    }

    #[test]
    fn a_queue_round_trips_with_everything_needed_to_act_on_it() {
        use crate::playlist::uri::TrackUri;
        let mut one = QueueItem::new(TrackUri::File {
            rel_path: "Angra/Rebirth/02.flac".into(),
        });
        one.artist = Some("Angra".into());
        one.title = Some("Nova Era".into());
        one.album = Some("Rebirth".into());
        one.year = Some(2001);
        one.duration_secs = Some(298);
        let mut two = QueueItem::new(TrackUri::CueTrack {
            cue_rel_path: "Gone/x.cue".into(),
            ordinal: 3,
        });
        two.unplayable = true;

        let wire = serde_json::to_string(&[&one, &two]).unwrap();
        let q = parse_queue(&wire);
        assert_eq!(q.len(), 2);
        // The URI is the point: without it a second window can render the
        // playlist and has no way to ask for a row to be played.
        assert_eq!(q[0].uri, one.uri);
        assert_eq!(q[1].uri, two.uri);
        assert_eq!(q[0].artist.as_deref(), Some("Angra"));
        assert_eq!(q[0].year, Some(2001), "album order needs this");
        assert_eq!(q[1].album, None, "an untagged album stays absent");
        assert!(!q[0].unplayable);
        assert!(q[1].unplayable, "missing tracks stay marked");
    }

    #[test]
    fn an_empty_queue_parses_to_nothing() {
        assert!(parse_queue("").is_empty());
        assert!(parse_queue("[]").is_empty());
        assert!(
            parse_queue("not json at all").is_empty(),
            "and rubbish is not a queue"
        );
    }
}
