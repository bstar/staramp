//! staramp — a Winamp-feel terminal music player for local libraries.

mod audio;
mod config;
mod cue;
mod fx;
mod ipc;
mod library;
mod logging;
mod mirror;
// MPRIS is D-Bus, which exists on Linux and nowhere else this runs. The stub
// has the same shape, so nothing downstream needs to know which one it got.
#[cfg(target_os = "linux")]
mod mpris;
#[cfg(not(target_os = "linux"))]
#[path = "mpris_stub.rs"]
mod mpris;
mod paths;
mod playlist;
mod query;
mod remote;
mod session;
mod theme;
mod ui;
mod util;
mod vfs;
mod view;
mod vis;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use audio::decode::{self, Backend};
use audio::engine::Playback;
use audio::output::RateMode;
use audio::wav::WavWriter;

#[derive(Parser)]
#[command(name = "staramp", version, about, long_about = None)]
struct Cli {
    /// Log at debug level.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum ThemeCmd {
    /// List available themes.
    List,
    /// Show a theme's resolved colours.
    Show { id: String },
    /// Import a base16 scheme (the format Stylix uses) as a theme.
    ImportBase16 {
        scheme: PathBuf,
        /// Theme id. Defaults to the scheme filename.
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "dark")]
        variant: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Import a classic Winamp .wsz skin as a theme.
    Import {
        skin: PathBuf,
        /// Theme name. Defaults to the skin's filename.
        #[arg(long)]
        name: Option<String>,
        /// Print the generated theme instead of writing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ArtCmd {
    /// Forget every remembered "this album has no cover", so they are looked
    /// up again. For after fixing a batch of tags, or after the archive has
    /// had a bad day; a miss otherwise stands for a week.
    Retry,
}

#[derive(Subcommand)]
enum Command {
    /// Decode a file to WAV. Diagnostic: proves sample accuracy without
    /// involving any audio hardware.
    Decode {
        input: PathBuf,
        /// Output path. Defaults to the input's stem with a .wav extension.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Skip to this many seconds before decoding.
        #[arg(long)]
        start: Option<f64>,
        /// Decode at most this many seconds.
        #[arg(long)]
        duration: Option<f64>,
    },
    /// Report what staramp sees in a file: codec, rate, depth, channels, length.
    Probe { input: PathBuf },
    /// Parse every CUE sheet under a directory and classify the results.
    CueReport {
        /// Library root. Defaults to the configured library path.
        root: PathBuf,
        /// List every sheet that is not indexable, with the reason.
        #[arg(long)]
        verbose_list: bool,
    },
    /// Index a library directory.
    Scan {
        /// Library root. Defaults to `library_root` from the config, which is
        /// where the player itself gets it.
        root: Option<PathBuf>,
        /// Re-read tags for every file, ignoring change detection.
        #[arg(long)]
        force: bool,
    },
    /// Search the index.
    Search {
        query: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// Show index statistics.
    Stats,
    /// Print a shuffled order. Diagnostic for shuffle freshness.
    #[command(hide = true)]
    ShuffleProbe {
        #[arg(default_value_t = 12)]
        count: usize,
    },
    /// Control a running instance over its socket.
    #[command(name = "ctl")]
    Ctl {
        /// e.g. `next`, `toggle`, `status`, `seek 30`, `volume 0.5`
        request: Vec<String>,
    },
    /// Theme management.
    Theme {
        #[command(subcommand)]
        cmd: ThemeCmd,
    },
    /// Album art maintenance.
    Art {
        #[command(subcommand)]
        cmd: ArtCmd,
    },
    /// Run a smart-playlist query against the index.
    Query {
        /// e.g. `genre ~ "power metal" and year >= 2015 sort added desc limit 20`
        expr: Vec<String>,
        /// Only report how many tracks match.
        #[arg(long)]
        count: bool,
        /// Show the generated SQL.
        #[arg(long)]
        explain: bool,
    },
    /// Open the player UI on a playlist, directory, or the whole library.
    Ui { target: Option<PathBuf> },
    /// Play a library that lives on another machine, over SSH.
    ///
    /// Nothing is installed or left running there: one ssh connection is
    /// opened and the files are read through it. The far machine needs
    /// staramp installed and scanned, and `ssh <host>` must already work
    /// without a prompt.
    Remote {
        /// An ssh destination -- a host, a user@host, or an alias from
        /// ~/.ssh/config. Defaults to `[remote] host` in the config.
        host: Option<String>,
        /// The library root as the far machine sees it. Defaults to
        /// `[remote] root`, and then to the far machine's own config.
        #[arg(long)]
        root: Option<String>,
        /// Fetch the index again even if it looks current.
        #[arg(long)]
        refresh: bool,
    },
    /// Check playlists against the index: what resolves and what does not.
    Playlists {
        /// Directory of .m3u/.m3u8 files.
        dir: PathBuf,
        /// Show every unresolved entry.
        #[arg(long)]
        show_missing: bool,
        /// Verify each playlist rewrites byte-for-byte.
        #[arg(long)]
        check_roundtrip: bool,
    },
    /// Play a file through the default output device.
    Play {
        input: PathBuf,
        /// Start this many seconds in.
        #[arg(long)]
        start: Option<f64>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Move an older XDG-style install into the single base directory. No-op
    // once done, and it never overwrites anything already there.
    for moved in paths::migrate_legacy() {
        eprintln!("moved {moved}");
    }

    // A commented config to read and edit, written once. Best effort: a
    // read-only home should not stop the music.
    if let Ok(true) = config::Config::write_template() {
        if let Ok(p) = paths::config_file() {
            eprintln!("wrote {p}", p = p.display());
        }
    }

    let _guard = logging::init(cli.verbose)?;

    match cli.command {
        Some(Command::Decode {
            input,
            output,
            start,
            duration,
        }) => cmd_decode(input, output, start, duration),
        Some(Command::Probe { input }) => cmd_probe(input),
        Some(Command::Play { input, start }) => cmd_play(input, start),
        Some(Command::CueReport { root, verbose_list }) => cmd_cue_report(root, verbose_list),
        Some(Command::Scan { root, force }) => cmd_scan(root, force),
        Some(Command::Search { query, limit }) => cmd_search(query, limit),
        Some(Command::Stats) => cmd_stats(),
        Some(Command::ShuffleProbe { count }) => cmd_shuffle_probe(count),
        Some(Command::Ctl { request }) => {
            if request.is_empty() {
                anyhow::bail!("nothing to send — try `staramp ctl status`");
            }
            println!("{}", ipc::send(&request.join(" "))?);
            Ok(())
        }
        Some(Command::Theme { cmd }) => cmd_theme(cmd),
        Some(Command::Art { cmd }) => cmd_art(cmd),
        Some(Command::Query {
            expr,
            count,
            explain,
        }) => cmd_query(expr.join(" "), count, explain),
        Some(Command::Ui { target }) => cmd_tui(target),
        Some(Command::Remote {
            host,
            root,
            refresh,
        }) => cmd_remote(host, root, refresh),
        Some(Command::Playlists {
            dir,
            show_missing,
            check_roundtrip,
        }) => cmd_playlists(dir, show_missing, check_roundtrip),
        None => cmd_tui(None),
    }
}

fn cmd_probe(input: PathBuf) -> Result<()> {
    let uri = playlist::uri::TrackUri::parse(&input.to_string_lossy());
    // The CLI names a file directly, so the URI is already absolute.
    let opened = audio::source::open(&vfs::Vfs::local(""), None, &uri)?;
    let backend = decode::backend_for_path(&opened.backing_path);
    let spec = opened.decoder.spec();
    let frames = opened.decoder.total_frames();

    println!("path        {}", input.display());
    if let Some(vt) = &opened.virtual_track {
        println!(
            "cue track   ordinal {} (TRACK {:02})",
            vt.ordinal, vt.number
        );
        if let Some(t) = &vt.title {
            println!("title       {t}");
        }
        println!("backing     {}", opened.backing_path.display());
        println!(
            "window      frame {} .. {}",
            vt.start_frame,
            match vt.end_frame {
                Some(e) => e.to_string(),
                None => "EOF".into(),
            }
        );
    }
    println!(
        "backend     {}",
        match backend {
            Backend::Symphonia => "symphonia",
            Backend::Libav => "libav",
        }
    );
    println!("codec       {}", opened.decoder.codec());
    match opened.decoder.bitrate_kbps() {
        Some(b) => println!("bitrate     {b} kbps (average)"),
        None => println!("bitrate     (unknown)"),
    }
    println!("sample rate {} Hz", spec.sample_rate);
    println!("channels    {}", spec.channels);
    match spec.bit_depth {
        Some(b) => println!("bit depth   {b}"),
        None => println!("bit depth   (not reported)"),
    }
    match frames {
        Some(f) => {
            let secs = f as f64 / spec.sample_rate as f64;
            println!("frames      {f}");
            println!(
                "duration    {:02}:{:05.2}",
                (secs / 60.0).floor() as u64,
                secs % 60.0
            );
        }
        None => println!("duration    (not reported)"),
    }
    Ok(())
}

fn cmd_decode(
    input: PathBuf,
    output: Option<PathBuf>,
    start: Option<f64>,
    duration: Option<f64>,
) -> Result<()> {
    let output = output.unwrap_or_else(|| input.with_extension("wav"));

    let mut dec = decode::open_path(&input)?;
    let spec = dec.spec();
    let ch = spec.channels as usize;

    if let Some(s) = start {
        let frame = (s * spec.sample_rate as f64) as u64;
        let landed = dec.seek(frame)?;
        tracing::info!("seek to {frame} landed at {landed}");
    }

    let limit_frames = duration.map(|d| (d * spec.sample_rate as f64) as u64);

    let mut wav = WavWriter::create(&output, spec.sample_rate, spec.channels)?;
    // 8192 frames is a decode-side working size, unrelated to the audio ring.
    let mut buf = vec![0f32; 8192 * ch];

    let began = Instant::now();
    let mut written: u64 = 0;
    loop {
        if let Some(limit) = limit_frames {
            if written >= limit {
                break;
            }
        }
        let frames = dec.read(&mut buf).context("decoding")?;
        if frames == 0 {
            break;
        }
        let mut frames = frames as u64;
        if let Some(limit) = limit_frames {
            frames = frames.min(limit - written);
        }
        wav.write(&buf[..frames as usize * ch])?;
        written += frames;
    }
    wav.finish()?;

    let secs = written as f64 / spec.sample_rate as f64;
    let elapsed = began.elapsed().as_secs_f64();
    println!(
        "{} -> {}  ({} frames, {:.2}s audio, {:.2}s wall, {:.0}x realtime)",
        input.display(),
        output.display(),
        written,
        secs,
        elapsed,
        if elapsed > 0.0 { secs / elapsed } else { 0.0 },
    );
    Ok(())
}

fn cmd_play(input: PathBuf, start: Option<f64>) -> Result<()> {
    let fixed_rate = config::Config::load()
        .unwrap_or_default()
        .output
        .fixed_rate();
    let playback = Playback::start_at(&input, start.unwrap_or(0.0), fixed_rate)?;

    let spec = playback.spec;
    let src = playback.src_spec;
    println!("{}", input.display());
    println!(
        "  {} Hz · {} ch · {} · device \"{}\"",
        src.sample_rate,
        src.channels,
        match src.bit_depth {
            Some(b) => format!("{b}-bit"),
            None => "float".into(),
        },
        playback.device_name(),
    );
    // Both halves of the shape, because either can cost bit-perfect playback
    // and saying "bit-perfect" while the channels were remixed is exactly the
    // lie this indicator exists not to tell.
    let mut refused = Vec::new();
    if let RateMode::Resampled { from, .. } = playback.rate_mode() {
        refused.push(format!("{from} Hz"));
    }
    if let Some(from) = playback.remixed_from() {
        refused.push(format!("{from}-channel output"));
    }
    if refused.is_empty() {
        println!(
            "  output {} Hz · {} ch · bit-perfect (device took the file's own shape)",
            playback.output_rate(),
            spec.channels,
        );
    } else {
        println!(
            "  output {} Hz · {} ch · NOT bit-perfect (device refused {}, converting)",
            playback.output_rate(),
            spec.channels,
            refused.join(" and "),
        );
    }

    let total = playback
        .total_frames
        .map(|f| f as f64 / spec.sample_rate as f64);

    // Ctrl-C should stop the device, not leave it running behind a dead process.
    install_sigint();

    while !playback.finished() {
        if INTERRUPTED.load(std::sync::atomic::Ordering::Relaxed) {
            println!();
            break;
        }
        let pos = playback.position_secs();
        match total {
            Some(t) => print!(
                "\r  {:02}:{:05.2} / {:02}:{:05.2}   underruns {}   ",
                (pos / 60.0) as u64,
                pos % 60.0,
                (t / 60.0) as u64,
                t % 60.0,
                playback.underruns()
            ),
            None => print!(
                "\r  {:02}:{:05.2}   underruns {}   ",
                (pos / 60.0) as u64,
                pos % 60.0,
                playback.underruns()
            ),
        }
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let underruns = playback.underruns();
    println!();
    if underruns > 0 {
        println!("  {underruns} underrun(s) — the decode thread could not keep up");
    } else {
        println!("  clean: no underruns");
    }
    playback.stop()
}

static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Minimal SIGINT hook: the handler only stores a flag, which is the one thing
/// that is safe to do from a signal handler. Teardown happens in normal control
/// flow, so the device is actually released.
fn install_sigint() {
    extern "C" fn on_signal(_: i32) {
        INTERRUPTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // SAFETY: the handler is async-signal-safe -- it only sets an atomic flag.
    unsafe {
        libc_signal(2 /* SIGINT */, on_signal as *const () as usize);
    }
}

extern "C" {
    #[link_name = "signal"]
    fn libc_signal(sig: i32, handler: usize) -> usize;
}

fn cmd_cue_report(root: PathBuf, verbose_list: bool) -> Result<()> {
    use cue::resolve::{Disposition, MatchKind};
    use std::sync::Mutex;

    let began = Instant::now();

    // Collect sheets first so the parse itself can be parallel.
    let mut sheets = Vec::new();
    for entry in ignore::WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(false)
        .build()
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("cue"))
            .unwrap_or(false)
        {
            sheets.push(path.to_path_buf());
        }
    }
    let found = sheets.len();

    #[derive(Default)]
    struct Tally {
        index: usize,
        skip_per_track: usize,
        skip_archival: usize,
        orphaned: usize,
        unreadable: usize,
        multi_file: usize,
        non_utf8: usize,
        tracks: usize,
        with_warnings: usize,
        exact: usize,
        case_insensitive: usize,
        normalised: usize,
        ext_swapped: usize,
        problems: Vec<String>,
        encodings: std::collections::BTreeMap<String, usize>,
    }

    let tally = Mutex::new(Tally::default());

    use rayon::prelude::*;
    sheets.par_iter().for_each(|path| {
        let sheet = match cue::parser::parse_file(path) {
            Ok(s) => s,
            Err(e) => {
                let mut t = tally.lock().unwrap();
                t.unreadable += 1;
                t.problems
                    .push(format!("UNREADABLE  {}  ({e})", path.display()));
                return;
            }
        };
        let res = cue::resolve::resolve(&sheet, path);

        let mut t = tally.lock().unwrap();
        *t.encodings.entry(sheet.encoding.clone()).or_insert(0) += 1;
        if !sheet.encoding.starts_with("UTF-8") {
            t.non_utf8 += 1;
        }
        if sheet.is_multi_file() {
            t.multi_file += 1;
        }
        if !sheet.warnings.is_empty() {
            t.with_warnings += 1;
        }
        for f in res.files.iter().flatten() {
            match f.how {
                MatchKind::Exact => t.exact += 1,
                MatchKind::CaseInsensitive => t.case_insensitive += 1,
                MatchKind::UnicodeNormalised => t.normalised += 1,
                MatchKind::ExtensionSwapped => t.ext_swapped += 1,
            }
        }
        match res.disposition {
            Disposition::Index => {
                t.index += 1;
                t.tracks += sheet.track_count();
            }
            Disposition::SkipPerTrackCue => {
                t.skip_per_track += 1;
                t.problems.push(format!("PER-TRACK   {}", path.display()));
            }
            Disposition::SkipArchival => {
                t.skip_archival += 1;
                t.problems.push(format!("ARCHIVAL    {}", path.display()));
            }
            Disposition::Orphaned => {
                t.orphaned += 1;
                t.problems.push(format!("ORPHANED    {}", path.display()));
            }
        }
    });

    let t = tally.into_inner().unwrap();
    let elapsed = began.elapsed();

    println!("cue sheets found      {found}");
    println!();
    println!("  indexable           {}", t.index);
    println!("  skipped: per-track  {}", t.skip_per_track);
    println!("  skipped: archival   {}", t.skip_archival);
    println!("  orphaned            {}", t.orphaned);
    println!("  unreadable          {}", t.unreadable);
    println!();
    println!("  virtual tracks      {}", t.tracks);
    println!("  multi-FILE sheets   {}", t.multi_file);
    println!("  non-UTF-8 sheets    {}", t.non_utf8);
    println!("  sheets w/ warnings  {}", t.with_warnings);
    println!();
    println!("FILE resolution:");
    println!("  exact               {}", t.exact);
    println!("  case-insensitive    {}", t.case_insensitive);
    println!("  unicode-normalised  {}", t.normalised);
    println!("  extension-swapped   {}", t.ext_swapped);
    println!();
    println!("encodings:");
    for (enc, n) in &t.encodings {
        println!("  {enc:<18}  {n}");
    }
    println!();
    println!("parsed in {:.2}s", elapsed.as_secs_f64());

    if verbose_list && !t.problems.is_empty() {
        println!();
        let mut problems = t.problems;
        problems.sort();
        for p in problems {
            println!("{p}");
        }
    }
    Ok(())
}

/// Open a library on another machine and play it.
///
/// Two things cross the link and they are treated completely differently. The
/// index is copied once, because it is small and because every browse and
/// search afterwards then costs nothing. The audio is never copied: it is read
/// as it plays, so starting a track does not mean waiting for a file.
fn cmd_remote(host: Option<String>, root: Option<String>, refresh: bool) -> Result<()> {
    let cfg = config::Config::load()?;
    let host = host
        .or_else(|| cfg.remote.host.clone())
        .context("no host given -- pass one, or set [remote] host in the config")?;
    // `~` is expanded on the far machine, by it, because there is no shell
    // here and no way for this one to know what it means there.
    let root = root
        .or_else(|| cfg.remote.root.clone())
        .unwrap_or_else(|| "~/Music".to_string());

    eprintln!("connecting to {host}...");
    let lib = remote::Library::connect(&host, &root, cfg.remote.readahead_mb)?;
    eprintln!("connected to {}:{}", lib.host(), lib.root());

    if refresh {
        // Drop the stamp rather than the database: if the fetch then fails,
        // what is in hand is still a working index.
        if let Ok(local) = remote::index::local_copy(lib.host()) {
            let mut stamp = local.into_os_string();
            stamp.push(".stamp");
            let _ = std::fs::remove_file(PathBuf::from(stamp));
        }
    }
    let index = remote::index::sync(&lib)?;

    let vfs = std::sync::Arc::new(vfs::Vfs::Remote(std::sync::Arc::new(lib)));
    let (_root, items) = build_queue(&cfg, None, &index)?;
    if items.is_empty() {
        anyhow::bail!("{host} has an index but no playable tracks in it");
    }

    let graphics = ui::graphics::probe_if_tty(ui::graphics::Mode::parse(&cfg.ui.graphics));
    let mut app = ui::app::App::on(vfs, items, &cfg)?;
    app.set_graphics(graphics);
    app.run()
}

fn index_path() -> Result<PathBuf> {
    paths::index_file()
}

fn cmd_scan(root: Option<PathBuf>, force: bool) -> Result<()> {
    // Running the player takes no arguments, so neither should rescanning what
    // it plays. An explicit root still wins, for scanning one corner of a
    // library or a second one somewhere else.
    let cfg = crate::config::Config::load()?;
    let (root, remember) = match root {
        // Told where to look, and nothing has said before: remember it, or the
        // player still has no library after a scan that plainly worked. The
        // error this replaces recommended exactly this sequence and then left
        // the listener no better off.
        Some(r) => {
            let remember = cfg.library_root.is_none();
            (r, remember)
        }
        None => (cfg.require_library_root()?, false),
    };
    let path = index_path()?;
    let mut db = library::db::Db::open(&path)?;
    println!("index  {}", path.display());
    println!("root   {}", root.display());
    println!("scanning…");

    let stats = library::scan::scan(&mut db, &root, &library::scan::ScanOptions { force })?;

    if remember {
        let path = crate::paths::config_file()?;
        let _ = crate::config::Config::write_template();
        match crate::config::edit::set(
            &path,
            crate::config::edit::ROOT,
            "library_root",
            &crate::config::edit::Value::Str(root.display().to_string()),
        ) {
            Ok(()) => println!("remembered  library_root in {}", path.display()),
            Err(e) => eprintln!("could not remember library_root: {e}"),
        }
    }

    println!();
    println!("  files seen        {}", stats.files_seen);
    println!("    audio           {}", stats.audio_files);
    println!("    cue             {}", stats.cue_files);
    println!("    images          {}", stats.image_files);
    println!("  unchanged         {}", stats.unchanged);
    println!("  tags read         {}", stats.tagged);
    println!("  tag failures      {}", stats.tag_errors);
    println!();
    println!("  tracks written    {}", stats.tracks_inserted);
    println!("  cue virtual       {}", stats.cue_tracks);
    println!("  backing hidden    {}", stats.hidden_backing);
    println!("  removed (gone)    {}", stats.removed);
    println!();
    println!("  total in index    {}", db.track_count()?);
    println!("  scanned in        {:.1}s", stats.elapsed_secs);
    Ok(())
}

fn cmd_search(query: String, limit: usize) -> Result<()> {
    let db = library::db::Db::open_readonly(&index_path()?)?;
    let mut stmt = db.conn.prepare(
        "SELECT t.artist, t.album, t.title, t.duration_ms, t.codec, t.uri
         FROM track_fts f
         JOIN track t ON t.id = f.rowid
         WHERE track_fts MATCH ?1 AND t.hidden = 0
         ORDER BY rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![query, limit as i64], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;

    let mut n = 0;
    for row in rows.flatten() {
        let (artist, album, title, dur, codec, _uri) = row;
        let d = dur.unwrap_or(0) / 1000;
        println!(
            "{:<28} {:<34} {:<38} {:>3}:{:02}  {}",
            truncate(artist.as_deref().unwrap_or("-"), 28),
            truncate(album.as_deref().unwrap_or("-"), 34),
            truncate(title.as_deref().unwrap_or("-"), 38),
            d / 60,
            d % 60,
            codec,
        );
        n += 1;
    }
    if n == 0 {
        println!("no matches");
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn cmd_stats() -> Result<()> {
    let db = library::db::Db::open_readonly(&index_path()?)?;
    let q = |sql: &str| -> Result<i64> { Ok(db.conn.query_row(sql, [], |r| r.get(0))?) };
    println!("tracks            {}", q("SELECT COUNT(*) FROM track")?);
    println!(
        "  visible         {}",
        q("SELECT COUNT(*) FROM track WHERE hidden = 0")?
    );
    println!(
        "  cue virtual     {}",
        q("SELECT COUNT(*) FROM track WHERE cue_file_id IS NOT NULL")?
    );
    println!(
        "  hidden backing  {}",
        q("SELECT COUNT(*) FROM track WHERE hidden = 1")?
    );
    println!("albums            {}", q("SELECT COUNT(*) FROM album")?);
    println!("files             {}", q("SELECT COUNT(*) FROM file")?);
    println!(
        "with replaygain   {}",
        q("SELECT COUNT(*) FROM track WHERE rg_source = 1")?
    );
    println!();
    println!("by codec:");
    let mut stmt = db.conn.prepare(
        "SELECT codec, COUNT(*) FROM track WHERE hidden = 0
         GROUP BY codec ORDER BY COUNT(*) DESC LIMIT 15",
    )?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .flatten()
    {
        println!("  {:<8} {}", row.0, row.1);
    }
    Ok(())
}

fn cmd_playlists(dir: PathBuf, show_missing: bool, check_roundtrip: bool) -> Result<()> {
    use playlist::m3u;
    use std::collections::HashSet;

    let db = library::db::Db::open_readonly(&index_path()?)?;
    let known: HashSet<String> = {
        let mut stmt = db.conn.prepare("SELECT uri FROM track")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };
    println!("index holds {} track URIs", known.len());
    println!();

    let files = m3u::list_dir(&dir)?;
    let mut total = 0usize;
    let mut resolved = 0usize;
    let mut cue_total = 0usize;
    let mut cue_resolved = 0usize;
    let mut missing: Vec<String> = Vec::new();
    let mut roundtrip_failures = 0usize;

    println!(
        "{:<28} {:>7} {:>8} {:>7} {:>8}",
        "PLAYLIST", "ENTRIES", "RESOLVED", "CUE", "CUE OK"
    );

    for path in &files {
        let pl = m3u::read_file(path)?;
        let mut n = 0;
        let mut ok = 0;
        let mut cn = 0;
        let mut cok = 0;

        for item in &pl.items {
            n += 1;
            let uri = item.uri.to_string();
            let is_cue = item.uri.is_cue();
            if is_cue {
                cn += 1;
            }
            if known.contains(&uri) {
                ok += 1;
                if is_cue {
                    cok += 1;
                }
            } else {
                missing.push(format!("{}: {}", pl.name, uri));
            }
        }

        if check_roundtrip {
            let original = std::fs::read(path)?;
            let (text, _) = cue::parser::decode_bytes(&original);
            let rewritten = m3u::write_string(&pl, m3u::WriteStyle::Preserve);
            // Compare against the decoded text: a legacy-encoded playlist cannot
            // round-trip byte-for-byte through a UTF-8 string, and re-encoding it
            // would be a change we did not ask for.
            if rewritten.trim_end() != text.trim_end() {
                roundtrip_failures += 1;
                println!("  !! round-trip differs: {}", pl.name);
            }
        }

        total += n;
        resolved += ok;
        cue_total += cn;
        cue_resolved += cok;

        let pct = ok
            .checked_mul(100)
            .and_then(|v| v.checked_div(n))
            .unwrap_or(100);
        println!(
            "{:<28} {:>7} {:>7}{} {:>7} {:>8}",
            truncate(&pl.name, 28),
            n,
            ok,
            if pct == 100 { " " } else { "*" },
            cn,
            cok
        );
    }

    println!();
    println!("playlists            {}", files.len());
    println!(
        "entries              {total}  resolved {resolved} ({:.1}%)",
        if total > 0 {
            resolved as f64 * 100.0 / total as f64
        } else {
            0.0
        }
    );
    println!(
        "cue virtual tracks   {cue_total}  resolved {cue_resolved} ({:.1}%)",
        if cue_total > 0 {
            cue_resolved as f64 * 100.0 / cue_total as f64
        } else {
            0.0
        }
    );
    println!("unresolved           {}", total - resolved);
    if check_roundtrip {
        println!("round-trip failures  {roundtrip_failures}");
    }

    if show_missing {
        println!();
        missing.sort();
        for m in missing.iter().take(200) {
            println!("  {m}");
        }
        if missing.len() > 200 {
            println!("  … and {} more", missing.len() - 200);
        }
    }
    Ok(())
}

/// Build a queue from a playlist file, a directory, or the index.
fn build_queue(
    cfg: &config::Config,
    target: Option<&Path>,
    index: &Path,
) -> Result<(PathBuf, Vec<playlist::queue::QueueItem>)> {
    use playlist::queue::QueueItem;
    use playlist::uri::TrackUri;

    let root = cfg
        .library_root
        .clone()
        .or_else(|| target.and_then(|t| t.is_dir().then(|| t.to_path_buf())))
        .unwrap_or_else(|| PathBuf::from("/"));

    // A playlist file: take it verbatim, unresolved entries included.
    if let Some(t) = target {
        if t.is_file() {
            let pl = playlist::m3u::read_file(t)?;
            let mut items: Vec<QueueItem> = pl
                .items
                .into_iter()
                .map(|i| {
                    let mut q = QueueItem::new(i.uri);
                    q.title = i.ext_title;
                    q.duration_secs = i.ext_duration_secs;
                    q
                })
                .collect();
            // Bare MPD playlists carry no metadata at all, so without this the
            // list is a wall of file paths. Anything still unknown afterwards
            // is genuinely not in the library, and is marked unplayable rather
            // than silently dropped.
            enrich_from_index(&mut items);
            return Ok((root, items));
        }
        if t.is_dir() {
            let mut items = Vec::new();
            for e in ignore::WalkBuilder::new(t).hidden(false).build().flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                if matches!(
                    p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.to_ascii_lowercase())
                        .as_deref(),
                    Some(
                        "flac"
                            | "mp3"
                            | "ogg"
                            | "m4a"
                            | "ape"
                            | "wv"
                            | "mpc"
                            | "dsf"
                            | "aif"
                            | "aiff"
                            | "wav"
                            | "opus"
                    )
                ) {
                    items.push(QueueItem::new(TrackUri::File {
                        rel_path: p.to_string_lossy().into_owned(),
                    }));
                }
            }
            items.sort_by_key(|a| a.uri.to_string());
            return Ok((PathBuf::from("/"), items));
        }
    }

    // Otherwise: everything the index knows, in album order. The whole
    // library, not a slice of it: a cap of five thousand quietly dropped the
    // records past Q on a large collection, and nothing said so.
    let db = library::db::Db::open_readonly(index)?;
    let mut stmt = db.conn.prepare(&format!(
        "{META_SELECT}
         WHERE t.hidden = 0
         ORDER BY t.album_artist, t.album, t.disc_no, t.track_no"
    ))?;
    let rows = stmt.query_map([], read_meta)?;
    let items = rows
        .flatten()
        .map(|(uri, meta)| {
            let mut q = QueueItem::new(TrackUri::parse(&uri));
            meta.fill(&mut q);
            q
        })
        .collect();
    Ok((root, items))
}

/// What the index knows about a track, beyond its URI.
///
/// A struct rather than a tuple because there are eight of them and they are
/// mostly `Option<String>`: a tuple of that shape lets two fields swap places
/// silently, and the compiler would have nothing to say about it.
#[derive(Debug, Default, Clone)]
struct TrackMeta {
    artist: Option<String>,
    album_artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
    year: Option<i64>,
    disc_no: Option<u32>,
    track_no: Option<u32>,
    duration_ms: Option<i64>,
    rg: crate::audio::dsp::gain::ReplayGain,
}

/// The columns [`read_meta`] expects, in the order it expects them, and the
/// join that makes one of them answerable.
///
/// A cue virtual track carries no year of its own -- not one of the 10,840 in
/// the reference library does -- so it is taken from the audio file underneath
/// it, which is the same file and the same record. `album_for_uri` in
/// `library::db` does the same thing for the same reason. For an ordinary
/// track the join finds the row itself and the coalesce changes nothing.
///
/// Callers append their own `WHERE` and `ORDER BY`, qualified with `t.`.
/// Shared, so the two queries that build a queue cannot drift apart.
const META_SELECT: &str = "SELECT t.uri, t.artist, t.album_artist, t.title, t.album,
                                  COALESCE(t.year, b.year), t.disc_no, t.track_no, t.duration_ms,
                                  COALESCE(t.rg_track_gain, b.rg_track_gain),
                                  COALESCE(t.rg_track_peak, b.rg_track_peak),
                                  COALESCE(t.rg_album_gain, b.rg_album_gain),
                                  COALESCE(t.rg_album_peak, b.rg_album_peak)
                             FROM track t
                             LEFT JOIN track b
                                    ON b.file_id = t.file_id AND b.cue_ordinal IS NULL";

fn read_meta(r: &rusqlite::Row) -> rusqlite::Result<(String, TrackMeta)> {
    // Small counts stored as signed integers. Anything that does not fit is
    // not a disc number, so it is dropped rather than wrapped.
    let count = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
    Ok((
        r.get(0)?,
        TrackMeta {
            artist: r.get(1)?,
            album_artist: r.get(2)?,
            title: r.get(3)?,
            album: r.get(4)?,
            year: r.get(5)?,
            disc_no: count(r.get(6)?),
            track_no: count(r.get(7)?),
            duration_ms: r.get(8)?,
            // Through the same backing-file join the year uses: a cue virtual
            // track carries no tags of its own, so its gain is the file's.
            rg: crate::audio::dsp::gain::ReplayGain {
                track_gain_db: r.get::<_, Option<f64>>(9)?.map(|v| v as f32),
                track_peak: r.get::<_, Option<f64>>(10)?.map(|v| v as f32),
                album_gain_db: r.get::<_, Option<f64>>(11)?.map(|v| v as f32),
                album_peak: r.get::<_, Option<f64>>(12)?.map(|v| v as f32),
            },
        },
    ))
}

impl TrackMeta {
    /// Fill in whatever the item is not already carrying.
    ///
    /// The playlist's own `#EXTINF` wins where it has an answer: it is what the
    /// person curating the file wrote down.
    fn fill(&self, item: &mut playlist::queue::QueueItem) {
        // Not fill-if-absent like the rest: a playlist cannot carry ReplayGain,
        // so the index is the only source and always the better answer.
        item.rg = self.rg;
        if item.title.is_none() {
            item.title = self.title.clone();
        }
        if item.artist.is_none() {
            item.artist = self.artist.clone();
        }
        if item.album.is_none() {
            item.album = self.album.clone();
        }
        if item.album_artist.is_none() {
            item.album_artist = self.album_artist.clone();
        }
        if item.year.is_none() {
            item.year = self.year;
        }
        if item.disc_no.is_none() {
            item.disc_no = self.disc_no;
        }
        if item.track_no.is_none() {
            item.track_no = self.track_no;
        }
        if item.duration_secs.is_none() {
            item.duration_secs = self.duration_ms.map(|d| d / 1000);
        }
    }
}

/// Fill in what the index knows, and flag what it has never heard of.
fn enrich_from_index(items: &mut [playlist::queue::QueueItem]) {
    use std::collections::HashMap;

    let Ok(path) = index_path() else { return };
    let Ok(db) = library::db::Db::open_readonly(&path) else {
        return;
    };
    let Ok(mut stmt) = db.conn.prepare(META_SELECT) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], read_meta) else {
        return;
    };

    let meta: HashMap<String, TrackMeta> = rows.flatten().collect();

    for item in items {
        match meta.get(&item.uri.to_string()) {
            Some(m) => m.fill(item),
            None => item.unplayable = true,
        }
    }
}

/// Enumerate the configured playlist directory, counting what resolves.
///
/// Cheap enough to do at startup: one pass over the index URIs, then a read of
/// each playlist file.
pub fn scan_playlist_dir(dir: &Path) -> Vec<ui::panels::picker::PlaylistEntry> {
    use std::collections::HashSet;

    let known: HashSet<String> = index_path()
        .ok()
        .and_then(|p| library::db::Db::open_readonly(&p).ok())
        .and_then(|db| {
            let mut stmt = db.conn.prepare("SELECT uri FROM track").ok()?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).ok()?;
            Some(rows.flatten().collect())
        })
        .unwrap_or_default();

    let mut out = Vec::new();
    for path in playlist::m3u::list_dir(dir).unwrap_or_default() {
        let Ok(pl) = playlist::m3u::read_file(&path) else {
            continue;
        };
        // An empty playlist file is not worth offering.
        if pl.items.is_empty() {
            continue;
        }
        let missing = if known.is_empty() {
            0
        } else {
            pl.items
                .iter()
                .filter(|i| !known.contains(&i.uri.to_string()))
                .count()
        };
        out.push(ui::panels::picker::PlaylistEntry {
            name: pl.name.clone(),
            path,
            tracks: pl.items.len(),
            missing,
        });
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// Load one playlist file into queue items, enriched from the index.
pub fn load_playlist(path: &Path) -> Result<Vec<playlist::queue::QueueItem>> {
    use playlist::queue::QueueItem;
    let pl = playlist::m3u::read_file(path)?;
    let mut items: Vec<QueueItem> = pl
        .items
        .into_iter()
        .map(|i| {
            let mut q = QueueItem::new(i.uri);
            q.title = i.ext_title;
            q.duration_secs = i.ext_duration_secs;
            q
        })
        .collect();
    enrich_from_index(&mut items);
    Ok(items)
}

fn cmd_art(cmd: ArtCmd) -> Result<()> {
    match cmd {
        ArtCmd::Retry => {
            let n = library::remote::forget_all(&paths::cache_dir()?)?;
            match n {
                0 => println!("nothing was being remembered as coverless"),
                1 => println!("1 album will be looked up again"),
                n => println!("{n} albums will be looked up again"),
            }
            Ok(())
        }
    }
}

fn cmd_tui(target: Option<PathBuf>) -> Result<()> {
    let cfg = config::Config::load().unwrap_or_default();

    // Before anything touches the terminal. Detecting a graphics protocol
    // means writing a query and reading the answer off stdin, and once the app
    // has the keyboard that answer arrives as keystrokes.
    let graphics = ui::graphics::probe_if_tty(ui::graphics::Mode::parse(&cfg.ui.graphics));

    // Another instance already owns the audio device. Rather than fighting it
    // for the sound card, mirror it: render its state and forward every key.
    //
    // Two questions, in order: is somebody already leading, and if not, can we
    // take the lead ourselves? Asking the second is what closes the gap
    // between them -- two windows opened together can both find no leader, and
    // exactly one can then win the lease. The loser asks again, by which time
    // the winner is listening.
    let joined = mirror::Mirror::connect().or_else(|| {
        if ipc::socket_path().is_ok_and(|p| ipc::claim_session(&p)) {
            None
        } else {
            mirror::Mirror::connect()
        }
    });
    if let Some(m) = joined {
        let player_root = cfg
            .library_root
            .clone()
            .unwrap_or_else(|| PathBuf::from("/"));
        let mut app = ui::app::App::mirroring(player_root, &cfg)?;
        app.set_graphics(graphics);
        app.set_mirror(m);
        // A playlist was named and something is already playing. Neither
        // guess: the argument used to be dropped in silence here, and
        // demanding a flag would mean knowing there was a session to join
        // before typing the command that finds out.
        if let Some(t) = target.as_ref().filter(|t| t.is_file()) {
            app.ask_about(t.clone());
        }
        return app.run();
    }

    // With no argument, open on the configured playlist directory. A curated
    // playlist is a far better starting point than thirty thousand tracks in
    // album order, and it is what the playlists exist for.
    let playlists = match (&target, cfg.resolved_playlist_dir()) {
        (None, Some(dir)) => scan_playlist_dir(&dir),
        _ => Vec::new(),
    };

    let (root, items) = build_queue(&cfg, target.as_deref(), &index_path()?)?;
    if items.is_empty() && playlists.is_empty() {
        anyhow::bail!(
            "nothing to play — run `staramp scan <dir>` first, or pass a playlist or directory"
        );
    }

    let mut app = ui::app::App::new(root, items, &cfg)?;
    app.set_graphics(graphics);
    app.set_playlists(playlists);
    if let Some(t) = target.as_ref().filter(|t| t.is_file()) {
        app.set_source_playlist(Some(t.clone()));
    }
    // Only offer a resume when nothing specific was asked for -- naming a
    // playlist on the command line is a clear instruction to play that.
    if target.is_none() {
        if let Some(s) = session::Session::load() {
            app.offer_resume(s);
        }
    }
    app.run()
}

fn cmd_query(expr: String, count_only: bool, explain: bool) -> Result<()> {
    let parsed = match query::parser::parse(&expr) {
        Ok(q) => q,
        Err(e) => {
            // Point at the problem rather than just naming it.
            eprintln!("  {expr}");
            let (s, en) = e.span;
            let pad = expr[..s.min(expr.len())].chars().count();
            let width = expr[s.min(expr.len())..en.min(expr.len())]
                .chars()
                .count()
                .max(1);
            eprintln!("  {}{}", " ".repeat(pad), "^".repeat(width));
            eprintln!("  {e}");
            std::process::exit(1);
        }
    };

    let now = library::db::now_secs();
    let compiled = query::compile::compile(&parsed, now, count_only);

    if explain {
        println!("parsed   {parsed}");
        println!("sql      {}", compiled.sql);
        println!("params   {:?}", compiled.params);
        println!();
    }

    let db = library::db::Db::open_readonly(&index_path()?)?;
    let params: Vec<Box<dyn rusqlite::ToSql>> = compiled
        .params
        .iter()
        .map(|p| -> Box<dyn rusqlite::ToSql> {
            match p {
                query::compile::Param::Int(i) => Box::new(*i),
                query::compile::Param::Real(r) => Box::new(*r),
                query::compile::Param::Text(t) => Box::new(t.clone()),
            }
        })
        .collect();
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    if count_only {
        let n: i64 = db
            .conn
            .query_row(&compiled.sql, refs.as_slice(), |r| r.get(0))?;
        println!("{n}");
        return Ok(());
    }

    let mut stmt = db.conn.prepare(&compiled.sql)?;
    let ids: Vec<i64> = stmt
        .query_map(refs.as_slice(), |r| r.get::<_, i64>(0))?
        .flatten()
        .collect();

    if ids.is_empty() {
        println!("no matches");
        return Ok(());
    }

    let mut detail = db.conn.prepare(
        "SELECT artist, album, title, year, codec, duration_ms FROM track WHERE id = ?1",
    )?;
    for id in &ids {
        if let Ok((artist, album, title, year, codec, dur)) = detail.query_row([id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        }) {
            let d = dur.unwrap_or(0) / 1000;
            println!(
                "{:<26} {:<30} {:<34} {:>4}  {:>3}:{:02}  {}",
                truncate(artist.as_deref().unwrap_or("-"), 26),
                truncate(album.as_deref().unwrap_or("-"), 30),
                truncate(title.as_deref().unwrap_or("-"), 34),
                year.map(|y| y.to_string()).unwrap_or_else(|| "-".into()),
                d / 60,
                d % 60,
                codec
            );
        }
    }
    println!();
    println!("{} tracks", ids.len());
    Ok(())
}

fn cmd_theme(cmd: ThemeCmd) -> Result<()> {
    match cmd {
        ThemeCmd::List => {
            let (detected, source) = theme::system::detect();
            match detected {
                Some(s) => println!(
                    "system:  {} — from {source}",
                    s.name.as_deref().unwrap_or("base16 scheme")
                ),
                None => println!("system:  not detected"),
            }
            println!();
            println!("built in:");
            for id in theme::builtin::ids() {
                let t = theme::builtin::load(id).expect("builtin must load");
                println!("  {:<18} {}", id, t.name);
            }
            let dir = paths::config_dir()?.join("themes");
            if dir.is_dir() {
                println!();
                println!("user ({}):", dir.display());
                for e in std::fs::read_dir(&dir)?.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("toml") {
                        println!(
                            "  {}",
                            p.file_stem().and_then(|s| s.to_str()).unwrap_or("?")
                        );
                    }
                }
            }
            Ok(())
        }
        ThemeCmd::Show { id } => {
            let t = load_theme(&id)?;
            println!("{} ({})", t.name, t.id);
            println!();
            let sw = |label: &str, c: theme::color::Rgb| {
                // A visible swatch beats a hex code you have to imagine.
                println!(
                    "  \x1b[48;2;{};{};{}m      \x1b[0m  {:<22} {}",
                    c.r, c.g, c.b, label, c
                );
            };
            sw("background", t.bg);
            sw("border", t.border);
            sw("border (focused)", t.border_focused);
            sw("foreground", t.fg);
            sw("dim", t.dim);
            sw("accent", t.accent);
            sw("selected row bg", t.row_selected_bg);
            sw("playing row", t.row_playing_fg);
            sw("cue virtual track", t.row_virtual_fg);
            sw("seek bar", t.seek_filled_fg);
            println!();
            print!("  analyzer ramp   ");
            for c in &t.vis_ramp {
                print!("\x1b[48;2;{};{};{}m  \x1b[0m", c.r, c.g, c.b);
            }
            println!();
            print!("  peak / grid     ");
            for c in [t.vis_peak_fg, t.vis_grid_fg] {
                print!("\x1b[48;2;{};{};{}m  \x1b[0m", c.r, c.g, c.b);
            }
            println!();
            println!();
            println!(
                "  body text contrast   {:.2}:1 {}",
                t.bg.contrast(t.fg),
                if t.bg.contrast(t.fg) >= 4.5 {
                    "(AA)"
                } else {
                    "(below AA)"
                }
            );
            println!("  dim text contrast    {:.2}:1", t.bg.contrast(t.dim));
            // Chrome should be quiet; these are the numbers to check when a
            // border looks like it is competing with the content.
            println!(
                "  border               {:.2}:1 idle, {:.2}:1 focused, {:.2}:1 apart",
                t.bg.contrast(t.border),
                t.bg.contrast(t.border_focused),
                t.border.contrast(t.border_focused),
            );
            Ok(())
        }
        ThemeCmd::ImportBase16 {
            scheme,
            id,
            variant,
            dry_run,
        } => {
            let parsed = theme::base16::parse_file(&scheme)?;
            let id = id.unwrap_or_else(|| {
                scheme
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("base16")
                    .to_string()
            });
            let toml = theme::base16::to_theme_toml(&parsed, &id, &variant);
            if dry_run {
                println!("{toml}");
                return Ok(());
            }
            let dir = paths::themes_dir()?;
            std::fs::create_dir_all(&dir)?;
            let out = dir.join(format!("{id}.toml"));
            std::fs::write(&out, &toml)?;
            println!("wrote {}", out.display());
            println!("use it with:  staramp theme show {id}");
            Ok(())
        }
        ThemeCmd::Import {
            skin,
            name,
            dry_run,
        } => {
            let skin_colors = theme::wsz::read(&skin)?;
            let name = name.unwrap_or_else(|| {
                skin.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported")
                    .to_string()
            });
            let source = skin
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("skin.wsz");
            let toml = theme::wsz::to_theme_toml(&skin_colors, &name, source);

            for w in &skin_colors.warnings {
                eprintln!("  note: {w}");
            }

            if dry_run {
                println!("{toml}");
                return Ok(());
            }

            let dir = paths::config_dir()?.join("themes");
            std::fs::create_dir_all(&dir)?;
            let id = name.to_lowercase().replace(' ', "-");
            let out = dir.join(format!("{id}.toml"));
            std::fs::write(&out, &toml)?;
            println!("wrote {}", out.display());
            println!("use it with:  staramp theme show {id}");
            Ok(())
        }
    }
}

/// Load a theme by id: user themes override built-ins of the same name.
fn load_theme(id: &str) -> Result<theme::resolve::Theme> {
    let (t, why) = theme::builtin::resolve_named(id);
    if why.starts_with("no theme") {
        anyhow::bail!("{why} — try `staramp theme list`");
    }
    eprintln!("  ({why})");
    Ok(t)
}

/// Print the first few tracks of a shuffled queue. Diagnostic: a player that
/// shuffles the same way every launch is not shuffling.
fn cmd_shuffle_probe(n: usize) -> Result<()> {
    use playlist::queue::{Queue, QueueItem};
    use playlist::uri::TrackUri;

    let mut q = Queue::new();
    q.set_tracks(
        (0..50)
            .map(|i| {
                QueueItem::new(TrackUri::File {
                    rel_path: format!("t{i:02}.flac"),
                })
            })
            .collect(),
    );
    match std::env::var("STARAMP_PROBE_MODE").as_deref() {
        // What `s` does: toggle shuffle on.
        Ok("toggle") => {
            q.set_shuffle(true);
        }
        // What loading a playlist while shuffle is already on does.
        Ok("set_tracks") => {
            q.set_shuffle(true);
            q.set_tracks(
                (0..50)
                    .map(|i| {
                        QueueItem::new(TrackUri::File {
                            rel_path: format!("t{i:02}.flac"),
                        })
                    })
                    .collect(),
            );
        }
        // What `S` does.
        _ => {
            q.shuffle_now();
        }
    }
    let mut out = Vec::new();
    if let Some(i) = q.current_index() {
        out.push(i);
    }
    while out.len() < n {
        match q.next() {
            Some(i) => out.push(i),
            None => break,
        }
    }
    println!(
        "{}",
        out.iter()
            .map(|i| format!("{i:02}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}
