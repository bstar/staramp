//! Exercise the installed-process boundary without claiming an audio device.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
    isolated: Arc<TempDir>,
}

impl Session {
    fn start() -> Self {
        Self::start_with(Arc::new(TempDir::new().unwrap()))
    }

    fn start_with(isolated: Arc<TempDir>) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_staramp"))
            .args(["embed", "--stdio"])
            .env("STARAMP_DIR", isolated.path().join("data"))
            .env("STARAMP_CONFIG_DIR", isolated.path().join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    break;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            messages,
            isolated,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn until(&self, kind: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self
                .messages
                .recv_timeout(remaining)
                .unwrap_or_else(|e| panic!("waiting for {kind}: {e}"));
            if message["type"] == kind {
                return message;
            }
        }
    }

    fn finish(mut self) {
        self.finish_impl(true);
    }

    fn finish_profile(mut self) {
        self.finish_impl(false);
    }

    fn finish_impl(&mut self, assert_empty: bool) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exit) = self.child.try_wait().unwrap() {
                assert!(exit.success(), "embedded child exited with {exit}");
                break;
            }
            assert!(Instant::now() < deadline, "embedded child did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        if assert_empty {
            assert_eq!(
                std::fs::read_dir(self.isolated.path()).unwrap().count(),
                0,
                "embed created STAR/AMP config, history, cache, or log files"
            );
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn theme() -> Value {
    json!({
        "bg": [10, 12, 14], "fg": [220, 220, 220], "muted": [120, 120, 120],
        "accent": [80, 170, 230], "selected": [40, 60, 80],
        "border": [100, 100, 100], "error": [240, 80, 80]
    })
}

#[test]
fn handshake_frame_shutdown_and_no_user_files() {
    let mut child = Session::start();
    let hello = child.until("hello");
    assert_eq!(hello["protocol"], 1);
    assert!(hello["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("transport_images")));
    assert!(hello["extensions"]
        .as_array()
        .unwrap()
        .contains(&json!("flac")));
    assert!(!hello["extensions"]
        .as_array()
        .unwrap()
        .contains(&json!("mkv")));

    child.send(json!({
        "type": "configure", "generation": 7, "width": 60, "height": 10,
        "focused": true, "theme": theme()
    }));
    let frame = child.until("frame");
    assert_eq!(frame["generation"], 7);
    assert_eq!(frame["width"], 60);
    assert_eq!(frame["height"], 10);
    assert_eq!(frame["cells"].as_array().unwrap().len(), 600);
    assert!(frame["images"].as_array().unwrap().is_empty());
    child.send(json!({"type": "shutdown"}));
    child.finish();
}

#[test]
fn graphics_config_adds_bounded_rgba_controls_without_changing_cells() {
    let mut child = Session::start();
    child.until("hello");
    child.send(json!({
        "type": "configure", "generation": 11, "width": 64, "height": 5,
        "focused": true, "theme": theme(),
        "graphics": {"cell_width": 8, "cell_height": 16}
    }));
    let frame = child.until("frame");
    assert_eq!(frame["cells"].as_array().unwrap().len(), 64 * 5);
    let images = frame["images"].as_array().unwrap();
    assert_eq!(images.len(), 5);
    let mut total_bytes = 0usize;
    for image in images {
        let x = image["x"].as_u64().unwrap();
        let y = image["y"].as_u64().unwrap();
        let width = image["width"].as_u64().unwrap();
        let height = image["height"].as_u64().unwrap();
        let pw = image["pixel_width"].as_u64().unwrap();
        let ph = image["pixel_height"].as_u64().unwrap();
        assert!(x + width <= 64 && y + height <= 5);
        assert_eq!(pw, width * 8);
        assert_eq!(ph, height * 16);
        assert_eq!(height, 1);
        assert_eq!(y, 4);
        assert!(pw * ph <= 65_536);
        let rgba = image["rgba"].as_array().unwrap();
        assert_eq!(rgba.len(), (pw * ph * 4) as usize);
        total_bytes += rgba.len();
    }
    assert!(total_bytes <= 262_144);
    child.send(json!({"type": "shutdown"}));
    child.finish();
}

#[test]
fn eof_exits_cleanly_without_writes() {
    let mut child = Session::start();
    child.until("hello");
    child.stdin.take();
    child.finish();
}

#[test]
fn missing_file_reports_error_then_accepts_stop_without_audio_device() {
    let mut child = Session::start();
    child.until("hello");
    child.send(json!({
        "type": "configure", "generation": 1, "width": 60, "height": 10,
        "focused": false, "theme": theme()
    }));
    let missing = child.isolated.path().join("missing.flac");
    child.send(json!({
        "type": "play", "generation": 1,
        "paths": [missing.to_str().unwrap()], "index": 0
    }));
    let error = child.until("error");
    assert!(error["message"].as_str().unwrap().contains("cannot open"));
    child.send(json!({"type": "control", "action": "stop"}));
    child.until("stopped");
    child.send(json!({"type": "shutdown"}));
    child.finish();
}

#[test]
fn malformed_audio_reports_decoder_error_without_writing_user_files() {
    let media = TempDir::new().unwrap();
    let broken = media.path().join("broken.flac");
    std::fs::write(&broken, b"not really FLAC").unwrap();

    let mut child = Session::start();
    child.until("hello");
    child.send(json!({
        "type": "configure", "generation": 2, "width": 60, "height": 10,
        "focused": false, "theme": theme()
    }));
    child.send(json!({
        "type": "play", "generation": 2,
        "paths": [broken.to_str().unwrap()], "index": 0
    }));
    let error = child.until("error");
    assert!(error["message"].as_str().unwrap().contains("cannot open"));
    child.send(json!({"type": "shutdown"}));
    child.finish();
}

#[cfg(unix)]
#[test]
fn fifo_is_rejected_before_decoder_open() {
    let media = TempDir::new().unwrap();
    let fifo = media.path().join("blocked.flac");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());

    let mut child = Session::start();
    child.until("hello");
    child.send(json!({
        "type": "configure", "generation": 3, "width": 60, "height": 10,
        "focused": false, "theme": theme()
    }));
    child.send(json!({
        "type": "play", "generation": 3,
        "paths": [fifo.to_str().unwrap()], "index": 0
    }));
    let error = child.until("error");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("not a regular file"));
    child.send(json!({"type": "shutdown"}));
    child.finish();
}

#[test]
fn profile_styles_persist_across_embed_processes_without_rewriting_standalone_config() {
    let isolated = Arc::new(TempDir::new().unwrap());
    let config_dir = isolated.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let standalone = config_dir.join("config.toml");
    // Config::load accepts partial files; the embed profile must remain separate.
    let original = b"[vis]\nmode = 'bars'\n[ui]\nseek_style = 'thin'\n";
    std::fs::write(&standalone, original).unwrap();
    let mut child = Session::start_with(Arc::clone(&isolated));
    assert!(child.until("hello")["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("player_styles")));
    child.send(json!({"type":"configure","generation":1,"width":64,"height":10,"focused":true,"theme":theme(),"profile":"starfold"}));
    child.until("frame");
    assert!(!config_dir.join("embed/starfold.toml").exists());
    child.send(json!({"type":"control","action":"next_visualizer"}));
    child.send(json!({"type":"control","action":"next_seek_style"}));
    let profile = config_dir.join("embed/starfold.toml");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::fs::read_to_string(&profile).is_ok_and(|text| text.contains("seek_style = \"bar\""))
    {
        assert!(Instant::now() < deadline, "profile save did not complete");
        thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(&profile).unwrap();
    assert!(text.contains("visualizer = \"peaks\""));
    assert_eq!(std::fs::read(&standalone).unwrap(), original);
    child.send(json!({"type":"shutdown"}));
    child.finish_profile();

    let mut reopened = Session::start_with(Arc::clone(&isolated));
    reopened.until("hello");
    reopened.send(json!({"type":"configure","generation":2,"width":64,"height":10,"focused":true,"theme":theme(),"profile":"starfold"}));
    reopened.until("frame");
    reopened.send(json!({"type":"control","action":"next_visualizer"}));
    reopened.send(json!({"type":"control","action":"next_seek_style"}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::fs::read_to_string(&profile)
        .is_ok_and(|text| text.contains("seek_style = \"blocks\""))
    {
        assert!(
            Instant::now() < deadline,
            "reopened profile save did not complete"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(&profile).unwrap();
    assert!(text.contains("visualizer = \"dots\""));
    assert_eq!(std::fs::read(&standalone).unwrap(), original);
    reopened.send(json!({"type":"shutdown"}));
    reopened.finish_profile();
}

#[test]
fn malformed_profile_reports_notice_and_never_gets_overwritten() {
    let mut child = Session::start();
    let profile = child.isolated.path().join("config/embed/starfold.toml");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    let original = b"version = 99\nvisualizer = 'bars'\nseek_style = 'thin'\n";
    std::fs::write(&profile, original).unwrap();
    child.until("hello");
    child.send(json!({"type":"configure","generation":1,"width":64,"height":10,"focused":true,"theme":theme(),"profile":"starfold"}));
    assert!(child.until("notice")["message"]
        .as_str()
        .unwrap()
        .contains("will not be overwritten"));
    child.until("frame");
    child.send(json!({"type":"control","action":"next_visualizer"}));
    assert!(child.until("notice")["message"]
        .as_str()
        .unwrap()
        .contains("not saved"));
    child.until("frame");
    assert_eq!(std::fs::read(&profile).unwrap(), original);
    child.send(json!({"type":"shutdown"}));
    child.finish_profile();
}

#[test]
fn pointer_style_targets_match_visible_native_rows() {
    let mut child = Session::start();
    child.until("hello");
    child.send(json!({"type":"configure","generation":1,"width":64,"height":5,"focused":true,"theme":theme(),"profile":"starfold"}));
    child.until("frame");
    // Compact row zero is a one-line clock, not the analyzer.
    child.send(json!({"type":"pointer","x":30,"y":0,"button":"left"}));
    child.send(json!({"type":"configure","generation":2,"width":64,"height":10,"focused":true,"theme":theme(),"profile":"starfold"}));
    child.until("frame");
    let profile = child.isolated.path().join("config/embed/starfold.toml");
    assert!(!profile.exists());
    child.send(json!({"type":"pointer","x":30,"y":0,"button":"left"}));
    child.send(json!({"type":"pointer","x":30,"y":0,"button":"right"}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::fs::read_to_string(&profile)
        .is_ok_and(|text| text.contains("seek_style = \"thin\""))
    {
        assert!(
            Instant::now() < deadline,
            "pointer style changes did not save"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let text = std::fs::read_to_string(profile).unwrap();
    assert!(text.contains("visualizer = \"peaks\""));
    child.send(json!({"type":"shutdown"}));
    child.finish_profile();
}
