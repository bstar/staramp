//! The `ssh` connection, and keeping it alive.
//!
//! star/amp does not speak SSH. It runs OpenSSH, which already knows about the
//! user's `~/.ssh/config`, their keys, their agent, their `known_hosts` and
//! their jump hosts -- none of which a music player has any business
//! reimplementing. What we own is the *lifetime*: one multiplexed master that
//! stays up for as long as music is playing, and a cheap channel over it for
//! every file we read.
//!
//! # Why `ControlPersist=no`
//!
//! This is the load-bearing flag and it reads backwards, so it is worth
//! stating plainly. `ControlPersist` combined with `-N` sets
//! `fork_after_authentication` unconditionally inside `ssh_session2()`, and
//! the detach that would normally keep the process in the foreground is only
//! armed when there is a session channel or a stdio forward -- which `-N`, by
//! definition, does not open. So the child we spawn daemonises and exits zero
//! a moment after authenticating, leaving a master we do not own and cannot
//! signal, while our supervisor sees a clean exit and concludes the link died.
//! It would then reconnect for ever against a connection that was working.
//!
//! With `ControlPersist=no` the master runs in the foreground as our direct
//! child, and `set_control_persist_exit_time()` short-circuits, so there is no
//! idle timeout at all. That is a stronger guarantee than any keepalive
//! tuning: the connection cannot expire while we hold it, because nothing is
//! counting.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// How long to wait for a master to authenticate before giving up on it.
const CONNECT_TIMEOUT_SECS: u32 = 10;

/// How often the far end is asked whether it is still there, and how many
/// unanswered probes it takes to call the link dead. Fifteen seconds times
/// three is about forty-five to notice a peer that vanished without a FIN --
/// a laptop lid, a dropped VPN -- which the read-ahead window covers.
const ALIVE_INTERVAL: u32 = 15;
const ALIVE_COUNT: u32 = 3;

/// Backoff between reconnection attempts.
///
/// A different curve from the Cover Art Archive's in `library/remote.rs`, and
/// deliberately: that one is placating a shared public service and should back
/// off hard, where this is a private link between two machines the user owns
/// and should come back the moment it can.
const BACKOFF_START_MS: u64 = 250;
const BACKOFF_MAX_MS: u64 = 30_000;

/// How long a connection must last before it counts as healthy.
///
/// Resetting the backoff on connect rather than on *staying* connected means a
/// link that flaps -- authenticating and dropping every second -- resets its
/// curve every cycle and hammers the far end for ever.
const HEALTHY_AFTER: Duration = Duration::from_secs(60);

/// Whether a failure is worth retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Nothing we do will help: a wrong key, a refused password, a host key
    /// that does not match. Retrying is not persistence, it is a lockout.
    Fatal,
    /// The network, most likely. Try again.
    Retryable,
}

/// What `ssh` said on its way out.
///
/// `ssh` exits 255 for everything it considers its own error, so the exit code
/// alone cannot tell a dropped connection from a revoked key. The message can,
/// and it is the only thing that can.
pub fn classify(stderr: &str) -> Failure {
    let s = stderr.to_ascii_lowercase();
    const FATAL: &[&str] = &[
        "permission denied",
        "no matching host key",
        "no matching key exchange",
        "too many authentication failures",
        "host key verification failed",
        "remote host identification has changed",
        "not a valid identity",
        "bad configuration option",
        "no such identity",
    ];
    if FATAL.iter().any(|m| s.contains(m)) {
        return Failure::Fatal;
    }
    Failure::Retryable
}

/// Full-jitter exponential backoff.
///
/// Jittered rather than plain doubling because several windows of the same
/// library reconnect together otherwise, and arrive together, and are refused
/// together.
pub fn backoff(attempt: u32, roll: u64) -> Duration {
    Duration::from_millis(roll % (ceiling_ms(attempt) + 1))
}

/// The longest [`backoff`] may return for a given attempt.
///
/// Separate because it is the part with a single right answer: the delay
/// itself is deliberately random within it, so only this can be asserted.
fn ceiling_ms(attempt: u32) -> u64 {
    BACKOFF_START_MS
        // `min(20)` bounds the shift; the multiply cannot overflow after it,
        // and `min` below caps the result regardless.
        .saturating_mul(1u64 << attempt.min(20))
        .min(BACKOFF_MAX_MS)
}

/// The argv for the master: one connection, no session, ours.
///
/// Everything here that is not obvious is neutralising something the user's
/// own `~/.ssh/config` may say. A `Host *` block setting `ControlPersist`, or
/// `RemoteCommand`, or a `LocalForward`, is entirely reasonable for their
/// shell and would quietly break this.
pub fn master_argv(host: &str, ctl: &Path) -> Vec<String> {
    let mut v: Vec<String> = ["-M", "-S"].iter().map(|s| s.to_string()).collect();
    v.push(ctl.display().to_string());
    for a in [
        // No session, no TTY, no stdin.
        "-N", "-T", "-n",
    ] {
        v.push(a.into());
    }
    for o in [
        // See the module comment. This one is not optional.
        "ControlPersist=no".to_string(),
        "ControlMaster=yes".to_string(),
        format!("ServerAliveInterval={ALIVE_INTERVAL}"),
        format!("ServerAliveCountMax={ALIVE_COUNT}"),
        // Pinned, so a user's `TCPKeepAlive no` cannot hide a dead peer from
        // us for as long as the kernel's own timeout.
        "TCPKeepAlive=yes".to_string(),
        format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"),
        // We own the retry loop. ssh's own would hide state from the
        // supervisor and confuse the backoff.
        "ConnectionAttempts=1".to_string(),
        // Never prompt: there is a full-screen UI on this terminal, and a
        // password prompt drawn under it is invisible and unanswerable.
        "BatchMode=yes".to_string(),
        // FLAC, WavPack and DSD are already compressed. zlib would spend CPU
        // on both machines to make the stream very slightly larger.
        "Compression=no".to_string(),
        "ClearAllForwardings=yes".to_string(),
        "ForwardAgent=no".to_string(),
        "ForwardX11=no".to_string(),
        "ForwardX11Trusted=no".to_string(),
        "PermitLocalCommand=no".to_string(),
        // A `RemoteCommand` in the user's config is fatal in combination with
        // -N; refusing it here turns a confusing failure into none.
        "RemoteCommand=none".to_string(),
        "RequestTTY=no".to_string(),
        "SessionType=none".to_string(),
    ] {
        v.push("-o".into());
        v.push(o);
    }
    v.push("--".into());
    v.push(host.into());
    v
}

/// The argv for one SFTP channel over an existing master.
///
/// Cheap: no key exchange, no authentication, no new TCP connection -- it
/// opens a channel on the master and speaks the SFTP subsystem down it.
///
/// Five of these options are what OpenSSH's own `sftp(1)` passes. Two are not,
/// and both guard the same thing: this pipe carries binary audio, and a
/// setting that injects text into it corrupts every file. A PTY would
/// translate `\n` into `\r\n`; a `LocalCommand`'s output would land in the
/// stream.
pub fn slave_argv(host: &str, ctl: &Path) -> Vec<String> {
    let mut v = vec!["-S".to_string(), ctl.display().to_string()];
    for o in [
        // Never become a master, whatever `ControlMaster auto` in the user's
        // config would otherwise do.
        "ControlMaster=no",
        "ControlPersist=no",
        "BatchMode=yes",
        "RequestTTY=no",
        "PermitLocalCommand=no",
        "RemoteCommand=none",
        "ClearAllForwardings=yes",
        "ForwardX11=no",
        "ForwardAgent=no",
        "ConnectTimeout=5",
    ] {
        v.push("-o".into());
        v.push(o.into());
    }
    v.push("-s".into());
    v.push("--".into());
    v.push(host.into());
    v.push("sftp".into());
    v
}

/// Ask an existing master whether it is alive.
///
/// `-M` fails *soft*: if the control socket already exists, `muxserver_listen`
/// prints "ControlSocket ... already exists, disabling multiplexing" and
/// carries on with multiplexing switched off, exiting non-zero for nothing. So
/// the socket existing is not evidence of anything, and this is the only way
/// to know.
pub fn master_alive(host: &str, ctl: &Path) -> bool {
    Command::new("ssh")
        .args(["-O", "check", "-S"])
        .arg(ctl)
        .args(["--", host])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ask an existing master to shut down.
fn master_exit(host: &str, ctl: &Path) {
    let _ = Command::new("ssh")
        .args(["-O", "exit", "-S"])
        .arg(ctl)
        .args(["--", host])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The state of the link, as the UI reads it.
///
/// Atomics read once a frame and never blocking, mirroring how `PlayerState`
/// is published. Drawing must never wait on the network to find out whether
/// the network is up.
#[derive(Debug, Default)]
pub struct LinkState {
    /// 0 down, 1 connecting, 2 up, 3 given up.
    pub state: AtomicU64,
    pub reconnects: AtomicU64,
    pub retry_in_ms: AtomicU64,
    /// Bumped every time the connection is replaced. A file handle opened
    /// under an older epoch has to be reopened before it is used again --
    /// which is cheap, because SFTP reads carry their own offset.
    pub epoch: AtomicU64,
    pub last_error: Mutex<Option<String>>,
}

pub const LINK_DOWN: u64 = 0;
pub const LINK_CONNECTING: u64 = 1;
pub const LINK_UP: u64 = 2;
pub const LINK_FATAL: u64 = 3;

impl LinkState {
    pub fn is_up(&self) -> bool {
        self.state.load(Ordering::Relaxed) == LINK_UP
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    fn fail(&self, msg: String, fatal: bool) {
        tracing::warn!("ssh: {msg}");
        *self.last_error.lock().unwrap() = Some(msg);
        self.state.store(
            if fatal { LINK_FATAL } else { LINK_DOWN },
            Ordering::Relaxed,
        );
    }
}

/// A supervised `ssh` master.
pub struct Master {
    host: String,
    ctl: PathBuf,
    pub link: Arc<LinkState>,
    child: Mutex<Option<Child>>,
}

impl Master {
    /// Start a master for `host`, waiting for it to authenticate.
    pub fn connect(host: &str) -> Result<Self> {
        let ctl = crate::paths::control_socket(host)?;
        let m = Self {
            host: host.to_string(),
            ctl,
            link: Arc::new(LinkState::default()),
            child: Mutex::new(None),
        };
        m.spawn()?;
        Ok(m)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn control_path(&self) -> &Path {
        &self.ctl
    }

    /// Bring a master up and wait until it is reachable.
    fn spawn(&self) -> Result<()> {
        self.link.state.store(LINK_CONNECTING, Ordering::Relaxed);

        // A live master from an earlier run of ours is worth keeping.
        if master_alive(&self.host, &self.ctl) {
            self.link.state.store(LINK_UP, Ordering::Relaxed);
            return Ok(());
        }
        // Not alive, so anything at that path is a corpse. `muxclient` only
        // unlinks it itself on ECONNREFUSED, which is not the case we are in.
        let _ = std::fs::remove_file(&self.ctl);

        let mut child = Command::new("ssh")
            .args(master_argv(&self.host, &self.ctl))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("running ssh")?;

        // stderr is the only diagnosis channel there is: `ssh` exits 255 for
        // everything. Drained on its own thread so a chatty connection cannot
        // fill the pipe and block the master.
        let notes = Arc::new(Mutex::new(String::new()));
        if let Some(err) = child.stderr.take() {
            let notes = Arc::clone(&notes);
            std::thread::Builder::new()
                .name("staramp-ssh-log".into())
                .spawn(move || {
                    for line in BufReader::new(err).lines().map_while(Result::ok) {
                        tracing::debug!("ssh: {line}");
                        let mut n = notes.lock().unwrap();
                        n.push_str(&line);
                        n.push('\n');
                    }
                })
                .ok();
        }

        // Wait for the socket to appear, for the child to give up, or for the
        // connect timeout to pass. Polled rather than watched: two seconds of
        // 25 ms ticks is nothing next to a key exchange.
        let deadline = Instant::now() + Duration::from_secs(CONNECT_TIMEOUT_SECS as u64 + 2);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let said = notes.lock().unwrap().clone();
                let fatal = classify(&said) == Failure::Fatal;
                let msg =
                    first_useful_line(&said).unwrap_or_else(|| format!("ssh exited with {status}"));
                self.link.fail(format!("{}: {msg}", self.host), fatal);
                anyhow::bail!("connecting to {}: {msg}", self.host);
            }
            if self.ctl.exists() && master_alive(&self.host, &self.ctl) {
                *self.child.lock().unwrap() = Some(child);
                self.link.state.store(LINK_UP, Ordering::Relaxed);
                self.link.epoch.fetch_add(1, Ordering::Release);
                return Ok(());
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let said = notes.lock().unwrap().clone();
                let msg = first_useful_line(&said)
                    .unwrap_or_else(|| "timed out waiting for ssh".to_string());
                self.link.fail(format!("{}: {msg}", self.host), false);
                anyhow::bail!("connecting to {}: {msg}", self.host);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// True while the master we spawned is still running.
    pub fn running(&self) -> bool {
        let mut guard = self.child.lock().unwrap();
        match guard.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            // No child of ours: a master someone else started, so ask it.
            None => master_alive(&self.host, &self.ctl),
        }
    }

    /// Bring the link back, with backoff, until it works or is hopeless.
    ///
    /// Every open file handle becomes stale when this succeeds -- the epoch
    /// moves -- but nothing above needs telling: an SFTP read carries its own
    /// offset, so a reopened handle resumes exactly where the old one was.
    pub fn reconnect(&self, attempt: u32) -> Result<()> {
        let wait = backoff(attempt, roll());
        self.link
            .retry_in_ms
            .store(wait.as_millis() as u64, Ordering::Relaxed);
        std::thread::sleep(wait);
        self.link.retry_in_ms.store(0, Ordering::Relaxed);
        *self.child.lock().unwrap() = None;
        let r = self.spawn();
        if r.is_ok() {
            self.link.reconnects.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
}

impl Drop for Master {
    fn drop(&mut self) {
        master_exit(&self.host, &self.ctl);
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.ctl);
    }
}

/// The first line of `ssh` output worth showing a person.
///
/// Skips the banner noise and, for the two failures a user can actually do
/// something about, says what to do rather than what happened.
fn first_useful_line(stderr: &str) -> Option<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))?;
    let lower = line.to_ascii_lowercase();
    if lower.contains("host key verification failed") {
        return Some(format!(
            "{line} — run `ssh <host>` once by hand to accept its host key"
        ));
    }
    if lower.contains("permission denied") {
        return Some(format!(
            "{line} — staramp cannot prompt for a password; use a key or an agent"
        ));
    }
    Some(line.to_string())
}

/// A cheap random number for jitter. Nothing here is security-sensitive.
fn roll() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    n.wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
}

/// True once a connection has been up long enough to trust.
pub fn healthy_since(up: Instant) -> bool {
    up.elapsed() >= HEALTHY_AFTER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(argv: &[String]) -> Vec<String> {
        argv.windows(2)
            .filter(|w| w[0] == "-o")
            .map(|w| w[1].clone())
            .collect()
    }

    /// The flag the whole design rests on. With `ControlPersist` set to
    /// anything else, `-N` makes ssh daemonise and exit, and the supervisor
    /// reconnects for ever against a connection that is working fine.
    #[test]
    fn the_master_never_persists() {
        let argv = master_argv("music", Path::new("/run/x.sock"));
        assert!(opts(&argv).contains(&"ControlPersist=no".to_string()));
    }

    /// Each of these neutralises something a perfectly reasonable `Host *`
    /// block would otherwise do to us.
    #[test]
    fn the_master_overrides_the_users_config_where_it_must() {
        let o = opts(&master_argv("music", Path::new("/run/x.sock")));
        for want in [
            "BatchMode=yes",
            "ControlMaster=yes",
            "TCPKeepAlive=yes",
            "ConnectionAttempts=1",
            "ClearAllForwardings=yes",
            "RemoteCommand=none",
            "PermitLocalCommand=no",
            "SessionType=none",
        ] {
            assert!(o.contains(&want.to_string()), "missing {want}");
        }
    }

    /// We request no forwardings, so `ExitOnForwardFailure` guards nothing.
    /// Carrying it would imply it does.
    #[test]
    fn the_master_asks_for_no_forwardings_and_so_needs_no_forward_guard() {
        let o = opts(&master_argv("music", Path::new("/run/x.sock")));
        assert!(!o.iter().any(|s| s.starts_with("ExitOnForwardFailure")));
        assert!(!o.iter().any(|s| s.starts_with("LocalForward")));
    }

    /// A `Ciphers` line would override the user's own crypto policy, which is
    /// not a music player's decision to make.
    #[test]
    fn the_master_does_not_touch_the_crypto_policy() {
        let o = opts(&master_argv("music", Path::new("/run/x.sock")));
        for opt in &o {
            assert!(!opt.starts_with("Ciphers"), "{opt}");
            assert!(!opt.starts_with("MACs"), "{opt}");
            assert!(!opt.starts_with("KexAlgorithms"), "{opt}");
        }
    }

    #[test]
    fn the_master_opens_no_session_and_reads_no_stdin() {
        let argv = master_argv("music", Path::new("/run/x.sock"));
        for flag in ["-N", "-T", "-n", "-M"] {
            assert!(argv.contains(&flag.to_string()), "missing {flag}");
        }
    }

    /// The two that keep binary audio intact. A PTY would rewrite every `\n`
    /// in the stream, and a LocalCommand would print into it.
    #[test]
    fn a_channel_can_never_corrupt_the_byte_stream() {
        let o = opts(&slave_argv("music", Path::new("/run/x.sock")));
        assert!(o.contains(&"RequestTTY=no".to_string()));
        assert!(o.contains(&"PermitLocalCommand=no".to_string()));
        assert!(o.contains(&"RemoteCommand=none".to_string()));
    }

    /// A channel must never promote itself to master, whatever the user's
    /// `ControlMaster auto` would do.
    #[test]
    fn a_channel_never_becomes_a_master() {
        let o = opts(&slave_argv("music", Path::new("/run/x.sock")));
        assert!(o.contains(&"ControlMaster=no".to_string()));
    }

    #[test]
    fn a_channel_asks_for_the_sftp_subsystem() {
        let argv = slave_argv("music", Path::new("/run/x.sock"));
        assert!(argv.contains(&"-s".to_string()));
        assert_eq!(argv.last().unwrap(), "sftp");
        // `--` before the host, so an alias beginning with a dash is a host
        // and not a flag.
        let dashdash = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv[dashdash + 1], "music");
    }

    /// Both forms must put the control path immediately after `-S`, or ssh
    /// dials a fresh connection and silently re-authenticates per file.
    #[test]
    fn both_forms_name_the_control_socket() {
        for argv in [
            master_argv("h", Path::new("/run/ctl.sock")),
            slave_argv("h", Path::new("/run/ctl.sock")),
        ] {
            let at = argv.iter().position(|a| a == "-S").expect("-S");
            assert_eq!(argv[at + 1], "/run/ctl.sock");
        }
    }

    // -- failure classification -------------------------------------------

    #[test]
    fn a_refused_key_is_not_retried() {
        for said in [
            "user@host: Permission denied (publickey).",
            "Too many authentication failures",
            "Host key verification failed.",
            "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@",
            "Unable to negotiate: no matching host key type found",
        ] {
            assert_eq!(classify(said), Failure::Fatal, "{said}");
        }
    }

    #[test]
    fn a_network_failure_is_retried() {
        for said in [
            "ssh: connect to host music port 22: Connection timed out",
            "ssh: connect to host music port 22: Connection refused",
            "client_loop: send disconnect: Broken pipe",
            "Timeout, server music not responding.",
            "",
        ] {
            assert_eq!(classify(said), Failure::Retryable, "{said}");
        }
    }

    /// The message a user can act on has to survive into the error, or the
    /// only symptom is a player that will not connect.
    #[test]
    fn an_unknown_host_key_says_what_to_do_about_it() {
        let msg = first_useful_line("Host key verification failed.\n").unwrap();
        assert!(msg.contains("ssh <host>"), "{msg}");
    }

    #[test]
    fn the_added_host_key_banner_is_not_mistaken_for_an_error() {
        let said = "Warning: Permanently added 'music' (ED25519) to the list of known hosts.\n\
                    ssh: connect to host music port 22: Connection refused\n";
        assert!(first_useful_line(said)
            .unwrap()
            .contains("Connection refused"));
    }

    // -- backoff ------------------------------------------------------------

    #[test]
    fn the_backoff_ceiling_doubles_and_then_stops() {
        assert_eq!(ceiling_ms(0), BACKOFF_START_MS);
        assert_eq!(ceiling_ms(1), BACKOFF_START_MS * 2);
        assert_eq!(ceiling_ms(2), BACKOFF_START_MS * 4);
        assert_eq!(ceiling_ms(20), BACKOFF_MAX_MS, "capped");
        assert_eq!(ceiling_ms(60), BACKOFF_MAX_MS, "and does not overflow");
        assert_eq!(ceiling_ms(u32::MAX), BACKOFF_MAX_MS);
    }

    /// Full jitter: the delay is anywhere in `[0, ceiling]`, so what can be
    /// asserted of any single roll is that it stays inside it.
    #[test]
    fn every_backoff_lands_inside_its_ceiling() {
        for attempt in 0..40u32 {
            for roll in [0u64, 1, 249, 250, 999, u64::MAX / 3, u64::MAX] {
                let ms = backoff(attempt, roll).as_millis() as u64;
                assert!(
                    ms <= ceiling_ms(attempt),
                    "attempt {attempt} roll {roll} gave {ms}ms"
                );
            }
        }
    }

    #[test]
    fn backoff_is_jittered_rather_than_fixed() {
        let a = backoff(6, 7);
        let b = backoff(6, 999_983);
        assert_ne!(a, b, "two callers must not wake together");
    }

    #[test]
    fn the_first_retry_is_quick_because_the_link_is_ours() {
        assert!(backoff(0, u64::MAX) <= Duration::from_millis(BACKOFF_START_MS));
    }
}
