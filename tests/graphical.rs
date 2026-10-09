#![cfg(feature = "terminal-graphics")]
//! Exercise the real native compositor without a terminal, sound device, or
//! any writes to the user's player configuration and listening history.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
#[test]
fn native_rack_renders_all_themes_and_keeps_transport_fixed_while_scrolling() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("renders");
    let mut child = Command::new(env!("CARGO_BIN_EXE_staramp"))
        .arg("render-rack")
        .arg(&out)
        .env("STARAMP_DIR", dir.path().join("data"))
        .env("STARAMP_CONFIG_DIR", dir.path().join("config"))
        .env("XDG_RUNTIME_DIR", dir.path().join("runtime"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/staramp-test-bus",
        )
        .env("STAR_GRAPHICS_SYSTEM_FONTS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("native renderer did not finish; possible state lock deadlock");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    let renders = std::fs::read_dir(out).unwrap().count();
    assert!(
        renders >= 32,
        "missing theme/viewport references: {renders}"
    );
}
