use std::process::{Command, Stdio};

#[test]
fn fresh_noninteractive_launch_explains_setup_without_sqlite_error() {
    let isolated = tempfile::tempdir().unwrap();
    let data = isolated.path().join("data");
    let result = Command::new(env!("CARGO_BIN_EXE_staramp"))
        .env_remove("SSH_CONNECTION")
        .env("STARAMP_DIR", &data)
        .env("STARAMP_CONFIG_DIR", isolated.path().join("config"))
        .env("XDG_RUNTIME_DIR", isolated.path().join("runtime"))
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(result.status.success(), "{result:?}");
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("Welcome to STAR/AMP"), "{stderr}");
    assert!(
        stderr.contains("staramp scan <music-directory>"),
        "{stderr}"
    );
    assert!(!stderr.contains("unable to open database file"), "{stderr}");
    assert!(!data.join("index.sqlite").exists());
}
