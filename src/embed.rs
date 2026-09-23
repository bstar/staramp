//! A deliberately small, independent player for file-manager embedding.
//!
//! Stdio is a versioned JSON-lines protocol. This path never constructs the
//! normal UI App, starts a control socket, records activity, or saves a session.

mod metadata;
mod render;
mod styles;

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, RecvTimeoutError};
use serde::{Deserialize, Serialize};

use crate::audio::player::{Command, PlayState, Player};
use crate::config::Config;
use crate::library::tags;
use crate::playlist::queue::QueueItem;
use crate::playlist::uri::TrackUri;
use crate::vfs::Vfs;
use crate::vis::meter::Meters;
use crate::vis::spectrum::{Motion, Spectrum};
use metadata::{MetadataResult, MetadataWorker};
use render::{HitTarget, PlayerRenderState};
use styles::Styles;

const PROTOCOL: u32 = 1;
const FRAME_TIME: Duration = Duration::from_millis(33);
const MAX_INPUT: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
const MAX_PATHS: usize = 2048;
const MAX_PATH_LEN: usize = 8192;
const MAX_WIDTH: u16 = 240;
const MAX_HEIGHT: u16 = 20;
const CAPABILITIES: &[&str] = &["transport_images", "player_styles"];

// These are candidates the decoder can attempt, not a claim that every file
// with this suffix is valid audio. Keep the host's picker broad enough for
// mounted collections with less common audio codecs.
const EXTENSIONS: &[&str] = &[
    "flac", "mp3", "ogg", "oga", "opus", "m4a", "m4b", "aac", "alac", "wav", "wave", "aif", "aiff",
    "aifc", "ape", "wv", "mpc", "mp+", "dsf", "dff", "wma", "tta", "tak", "shn", "mka", "caf",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Palette {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
    pub selected: [u8; 3],
    pub border: [u8; 3],
    pub error: [u8; 3],
}

/// Terminal cell pixels, supplied only when the host can draw image overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct GraphicsConfig {
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransportImage {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cell {
    pub symbol: String,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub modifiers: u16,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Configure {
        generation: u64,
        width: u16,
        height: u16,
        focused: bool,
        theme: Palette,
        #[serde(default)]
        graphics: Option<GraphicsConfig>,
        #[serde(default)]
        profile: Option<String>,
    },
    Play {
        generation: u64,
        paths: Vec<String>,
        index: usize,
    },
    Control {
        action: String,
        #[serde(default)]
        value: Option<f64>,
    },
    Pointer {
        x: u16,
        y: u16,
        button: String,
    },
    Shutdown,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Response<'a> {
    Hello {
        protocol: u32,
        extensions: &'a [&'a str],
        capabilities: &'a [&'a str],
    },
    Frame {
        generation: u64,
        width: u16,
        height: u16,
        cells: Vec<Cell>,
        images: Vec<TransportImage>,
    },
    Status {
        playing: bool,
        paused: bool,
        title: String,
        path: Option<String>,
    },
    Error {
        message: String,
    },
    Notice {
        message: String,
    },
    Stopped,
}

enum Input {
    Request(Request),
    Invalid(String),
    Eof,
}

struct View {
    generation: u64,
    width: u16,
    height: u16,
    focused: bool,
    palette: Palette,
    graphics: Option<GraphicsConfig>,
}

struct RenderLook<'a> {
    styles: &'a Styles,
    wave: &'a [f32],
    seek_phase: f32,
    cfg: &'a Config,
}

pub fn run_stdio() -> Result<()> {
    let (tx, rx) = bounded::<Input>(32);
    thread::Builder::new()
        .name("staramp-embed-input".into())
        .spawn(move || {
            let mut input = BufReader::new(io::stdin().lock());
            loop {
                match read_line_bounded(&mut input, MAX_INPUT) {
                    Ok(Some(line)) => {
                        let parsed = serde_json::from_slice::<Request>(&line)
                            .map(Input::Request)
                            .unwrap_or_else(|e| Input::Invalid(format!("invalid request: {e}")));
                        if tx.send(parsed).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        let _ = tx.send(Input::Eof);
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                        if tx.send(Input::Invalid(e.to_string())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Input::Invalid(format!("input failed: {e}")));
                        let _ = tx.send(Input::Eof);
                        break;
                    }
                }
            }
        })?;

    let mut output = BufWriter::new(io::stdout().lock());
    write_response(
        &mut output,
        &Response::Hello {
            protocol: PROTOCOL,
            extensions: EXTENSIONS,
            capabilities: CAPABILITIES,
        },
    )?;

    let (cfg, config_error) = match Config::load() {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(format!("STAR/AMP preferences: {e}")),
        ),
    };
    if let Some(message) = config_error {
        send_error(&mut output, message)?;
    }
    let player = Player::new_embedded(
        Arc::new(Vfs::local_files()),
        cfg.output.fixed_rate(),
        cfg.disk.warm_on_pause.then(|| cfg.disk.warm_max_bytes()),
    )?;
    player.set_volume(cfg.volume);
    player.set_crossfade(cfg.playlist.crossfade);
    player.set_replaygain(cfg.rg.mode(), cfg.rg.preamp, cfg.rg.prevent_clipping);
    player
        .queue
        .lock()
        .unwrap()
        .set_repeat(crate::playlist::queue::RepeatMode::parse(
            &cfg.playlist.repeat,
        ));
    let mut view: Option<View> = None;
    let mut spectrum = Spectrum::new(2048, 20, 44_100.0);
    spectrum.set_gain_db(cfg.vis.gain_db);
    let mut fluid = Spectrum::with_motion(2048, 64, 44_100.0, Motion::Fluid);
    fluid.set_gain_db(cfg.vis.gain_db);
    fluid.set_smoothing(cfg.vis.smoothing as f32);
    let mut styles = Styles::new(&cfg);
    let mut meters = Meters::new();
    let mut tap_buf = vec![0.0f32; 4096];
    let mut wave = vec![0.0f32; 1024];
    let mut seek_phase = 0.0f32;
    let mut last_sample_rate = None;
    let mut last_tick = Instant::now();
    let mut last_status: Option<(bool, bool, String, Option<String>)> = None;
    let mut last_error: Option<String> = None;
    let metadata = MetadataWorker::new();
    let mut metadata_key: Option<(String, u64)> = None;
    let mut current_metadata: Option<MetadataResult> = None;
    let mut image_cache = render::TransportImageCache::default();

    loop {
        match rx.recv_timeout(FRAME_TIME) {
            Ok(Input::Request(Request::Shutdown) | Input::Eof) => break,
            Ok(Input::Request(request)) => {
                if let Err(e) =
                    handle_request(request, &player, &mut view, &mut styles, &mut output)
                {
                    send_error(&mut output, e.to_string())?;
                }
                while let Ok(input) = rx.try_recv() {
                    match input {
                        Input::Request(Request::Shutdown) | Input::Eof => return Ok(()),
                        Input::Request(request) => {
                            if let Err(e) = handle_request(
                                request,
                                &player,
                                &mut view,
                                &mut styles,
                                &mut output,
                            ) {
                                send_error(&mut output, e.to_string())?;
                            }
                        }
                        Input::Invalid(message) => send_error(&mut output, message)?,
                    }
                }
            }
            Ok(Input::Invalid(message)) => send_error(&mut output, message)?,
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32();
        last_tick = now;
        let tapped = player.tap.read(&mut tap_buf);
        let samples = if tapped { tap_buf.as_slice() } else { &[] };
        let sample_rate = player.state.sample_rate.load(Relaxed).max(8000);
        let rate = sample_rate as f32;
        if last_sample_rate != Some(sample_rate) {
            spectrum.set_rate(rate);
            fluid.set_rate(rate);
            last_sample_rate = Some(sample_rate);
        }
        if styles.vis.uses_fluid() {
            // Native PlayerView gives the first 24 body columns to the clock.
            let width = view.as_ref().map_or(64, |v| v.width).saturating_sub(24);
            let count = crate::ui::panels::visualizer::fluid_bar_count(width).max(1);
            fluid.set_bands(count, rate);
            fluid.analyze(samples, dt);
            meters.update(fluid.bands(), dt);
        } else {
            spectrum.analyze(samples, dt);
            meters.update(spectrum.bands(), dt);
        }
        if styles.vis.needs_waveform() {
            wave.fill(0.0);
            let count = samples.len().min(wave.len());
            wave[..count].copy_from_slice(&samples[samples.len() - count..]);
        }
        if cfg.fx.active() && player.state.state() == PlayState::Playing {
            seek_phase = (seek_phase + dt / 3.5).fract();
        } else {
            seek_phase = 0.0;
        }

        let current_error = player.state.last_error.load_full().map(|v| (*v).clone());
        if current_error != last_error {
            if let Some(message) = current_error.as_ref() {
                send_error(&mut output, message.clone())?;
            }
            last_error = current_error;
        }

        let playing = player.state.playing.load(Relaxed);
        let paused = player.state.paused.load(Relaxed);
        let current = player.current_item();
        let revision = player.state.track_revision.load(Relaxed);
        let key = current
            .as_ref()
            .map(|item| (item.uri.to_string(), revision));
        if key != metadata_key {
            current_metadata = None;
            metadata_key = key.clone();
            if playing {
                if let Some((path, revision)) = key.as_ref() {
                    metadata.request(path.clone(), *revision);
                }
            }
        }
        if let Some(result) = metadata.try_latest() {
            if metadata_key.as_ref() == Some(&(result.path.clone(), result.revision)) {
                current_metadata = Some(result);
            }
        }
        let title = current_metadata
            .as_ref()
            .and_then(|tags| tags.title.clone())
            .or_else(|| current.as_ref().and_then(|item| item.title.clone()))
            .or_else(|| current.as_ref().map(|item| item.uri.to_string()))
            .unwrap_or_else(|| "nothing playing".into());
        let path = current.as_ref().map(|item| item.uri.to_string());
        let status = (playing, paused, title, path);
        if last_status.as_ref() != Some(&status) {
            write_response(
                &mut output,
                &Response::Status {
                    playing: status.0,
                    paused: status.1,
                    title: status.2.clone(),
                    path: status.3.clone(),
                },
            )?;
            last_status = Some(status);
        }

        if let Some(v) = &view {
            let state = render_state(
                &player,
                v.focused,
                &meters,
                current_metadata.as_ref(),
                RenderLook {
                    styles: &styles,
                    wave: &wave,
                    seek_phase,
                    cfg: &cfg,
                },
            );
            let cells = render::render_frame(v.width, v.height, &v.palette, &state);
            if cells.len() != usize::from(v.width) * usize::from(v.height) {
                send_error(&mut output, "renderer returned the wrong cell count".into())?;
                continue;
            }
            let images = v.graphics.map_or_else(Vec::new, |graphics| {
                image_cache
                    .images(v.width, v.height, &v.palette, &state, graphics)
                    .to_vec()
            });
            write_response(
                &mut output,
                &Response::Frame {
                    generation: v.generation,
                    width: v.width,
                    height: v.height,
                    cells,
                    images,
                },
            )?;
        }
    }

    // Dropping Player sends Quit and joins only the standalone audio worker.
    drop(player);
    Ok(())
}

fn handle_request(
    request: Request,
    player: &Player,
    view: &mut Option<View>,
    styles: &mut Styles,
    output: &mut impl Write,
) -> Result<()> {
    match request {
        Request::Configure {
            generation,
            width,
            height,
            focused,
            theme,
            graphics,
            profile,
        } => {
            if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
                bail!("invalid panel size {width}x{height}");
            }
            if let Some(graphics) = graphics {
                if graphics.cell_width == 0
                    || graphics.cell_height == 0
                    || graphics.cell_width > 64
                    || graphics.cell_height > 128
                {
                    bail!("invalid graphics cell size");
                }
            }
            if let Some(message) = styles.configure(profile)? {
                send_notice(output, message)?;
            }
            *view = Some(View {
                generation,
                width,
                height,
                focused,
                palette: theme,
                graphics,
            });
        }
        Request::Play {
            generation,
            paths,
            index,
        } => {
            let Some(v) = view.as_ref() else {
                bail!("configure the panel before playing");
            };
            if generation != v.generation {
                bail!("stale play request");
            }
            let queue = queue_items(paths, index)?;
            player.send_checked(Command::ReplaceQueueAndPlay {
                tracks: queue,
                index,
            })?;
        }
        Request::Control { action, value } => {
            let notice = match action.as_str() {
                "next_visualizer" => Some(styles.next_vis()),
                "prev_visualizer" => Some(styles.prev_vis()),
                "next_seek_style" => Some(styles.next_seek()),
                _ => None,
            };
            if let Some(notice) = notice {
                if let Some(message) = notice {
                    send_notice(output, message)?;
                }
                return Ok(());
            }
            control(player, &action, value)?;
            if action == "stop" {
                write_response(output, &Response::Stopped)?;
            }
        }
        Request::Pointer { x, y, button } => {
            match button.as_str() {
                "scroll_up" | "scroll_down" | "left" | "right" | "drag" => {}
                _ => return Ok(()),
            }
            let Some(v) = view.as_ref() else {
                return Ok(());
            };
            if x >= v.width || y >= v.height {
                return Ok(());
            }
            let default_cfg = Config::default();
            let state = render_state(
                player,
                v.focused,
                &Meters::new(),
                None,
                RenderLook {
                    styles,
                    wave: &[],
                    seek_phase: 0.0,
                    cfg: &default_cfg,
                },
            );
            if let Some(target) = render::hit_test(v.width, v.height, x, y, &state) {
                if matches!(button.as_str(), "scroll_up" | "scroll_down") {
                    if target == HitTarget::Visualizer {
                        let notice = if button == "scroll_up" {
                            styles.next_vis()
                        } else {
                            styles.prev_vis()
                        };
                        if let Some(message) = notice {
                            send_notice(output, message)?;
                        }
                        return Ok(());
                    }
                    return control(
                        player,
                        "volume_by",
                        Some(if button == "scroll_up" { 0.05 } else { -0.05 }),
                    );
                }
                if button == "right" {
                    if matches!(
                        target,
                        HitTarget::Visualizer | HitTarget::Seek(_) | HitTarget::SeekRow
                    ) {
                        if let Some(message) = styles.next_seek() {
                            send_notice(output, message)?;
                        }
                    }
                    return Ok(());
                }
                if target == HitTarget::Visualizer {
                    if button == "left" {
                        if let Some(message) = styles.next_vis() {
                            send_notice(output, message)?;
                        }
                    }
                    return Ok(());
                }
                if target == HitTarget::SeekRow {
                    return Ok(());
                }
                if button == "drag" && !matches!(target, HitTarget::Seek(_) | HitTarget::Volume(_))
                {
                    return Ok(());
                }
                let stopped = target == HitTarget::Stop;
                pointer(player, target)?;
                if stopped {
                    write_response(output, &Response::Stopped)?;
                }
            }
            if matches!(button.as_str(), "scroll_up" | "scroll_down") {
                return control(
                    player,
                    "volume_by",
                    Some(if button == "scroll_up" { 0.05 } else { -0.05 }),
                );
            }
        }
        Request::Shutdown => {}
    }
    Ok(())
}

fn queue_items(paths: Vec<String>, index: usize) -> Result<Vec<QueueItem>> {
    if paths.is_empty() || paths.len() > MAX_PATHS || index >= paths.len() {
        bail!("invalid playback queue");
    }
    paths
        .into_iter()
        .map(|path| {
            if path.len() > MAX_PATH_LEN || !Path::new(&path).is_absolute() {
                bail!("playback paths must be absolute UTF-8 paths within the size limit");
            }
            let extension = Path::new(&path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !EXTENSIONS.contains(&extension.as_str()) {
                bail!("unsupported audio filename: {path}");
            }
            let mut item = QueueItem::new(TrackUri::File {
                rel_path: path.clone(),
            });
            let (artist, title) = tags::from_filename(Path::new(&path));
            item.artist = artist;
            item.title = title;
            Ok(item)
        })
        .collect()
}

fn control(player: &Player, action: &str, value: Option<f64>) -> Result<()> {
    let state = player.state.state();
    let cmd = match action {
        "toggle" | "toggle_pause" => match state {
            PlayState::Stopped => Command::PlayIndex(current_index(player)?),
            PlayState::Playing => Command::Pause,
            PlayState::Paused => Command::Resume,
        },
        "play" | "resume" => match state {
            PlayState::Stopped => Command::PlayIndex(current_index(player)?),
            _ => Command::Resume,
        },
        "pause" => Command::Pause,
        "stop" => Command::Stop,
        "next" => Command::Next,
        "previous" | "prev" => Command::Prev,
        "seek" => {
            let seconds = value.context("seek requires a value")?;
            if !seconds.is_finite() || seconds < 0.0 {
                bail!("seek value must be a nonnegative number");
            }
            Command::SeekTo(seconds)
        }
        "seek_by" => {
            let seconds = value.context("seek_by requires a value")?;
            if !seconds.is_finite() || seconds.abs() > 3600.0 {
                bail!("seek_by value is out of range");
            }
            Command::SeekBy(seconds)
        }
        "volume" => {
            let volume = value.context("volume requires a value")?;
            if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
                bail!("volume value must be between 0 and 1");
            }
            player.set_volume(volume as f32);
            return Ok(());
        }
        "repeat" => {
            player.cycle_repeat();
            return Ok(());
        }
        "volume_by" => {
            let delta = value.context("volume_by requires a value")?;
            if !delta.is_finite() || delta.abs() > 1.0 {
                bail!("volume_by value is out of range");
            }
            player.set_volume((f64::from(player.volume()) + delta).clamp(0.0, 1.0) as f32);
            return Ok(());
        }
        _ => bail!("unknown control action: {action}"),
    };
    player.send_checked(cmd)
}

fn current_index(player: &Player) -> Result<usize> {
    let queue = player.queue.lock().unwrap();
    queue.current_index().context("playback queue is empty")
}

fn pointer(player: &Player, target: HitTarget) -> Result<()> {
    match target {
        HitTarget::Previous => control(player, "previous", None),
        HitTarget::Play => control(player, "play", None),
        HitTarget::Pause => control(player, "pause", None),
        HitTarget::Stop => control(player, "stop", None),
        HitTarget::Next => control(player, "next", None),
        HitTarget::Seek(value) => control(player, "seek", Some(value)),
        HitTarget::Volume(value) => control(player, "volume", Some(f64::from(value))),
        HitTarget::Visualizer | HitTarget::SeekRow => Ok(()),
    }
}

fn render_state(
    player: &Player,
    focused: bool,
    meters: &Meters,
    metadata: Option<&MetadataResult>,
    look: RenderLook<'_>,
) -> PlayerRenderState {
    let st = &player.state;
    let item = player.current_item();
    let (title, subtitle) = match item {
        Some(item) => {
            let title = metadata
                .and_then(|tags| tags.title.clone())
                .or(item.title)
                .unwrap_or_else(|| item.uri.to_string());
            let artist = metadata
                .and_then(|tags| tags.artist.clone())
                .or(item.artist);
            let album = metadata.and_then(|tags| tags.album.clone()).or(item.album);
            let subtitle = [artist, album]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            (title, subtitle)
        }
        None => ("nothing playing".into(), String::new()),
    };
    let rate = st.sample_rate.load(Relaxed);
    let codec = st.codec.load_full();
    let depth = st.bit_depth.load(Relaxed);
    let channels = st.channels.load(Relaxed);
    let tech = if rate == 0 {
        String::new()
    } else {
        format!(
            "{} · {:.1} kHz · {} · {}",
            codec,
            rate as f64 / 1000.0,
            if depth == 0 {
                "float".into()
            } else {
                format!("{depth}-bit")
            },
            if channels == 1 { "mono" } else { "stereo" }
        )
    };
    PlayerRenderState {
        title,
        subtitle,
        tech,
        state: st.state(),
        position: st.position_secs(),
        duration: st.duration_secs(),
        volume: player.volume(),
        repeat: player.queue.lock().unwrap().repeat(),
        bit_perfect: st.bit_perfect.load(Relaxed),
        focused,
        bands: meters.bars().to_vec(),
        peaks: meters.peaks().to_vec(),
        underruns: st.underruns.load(Relaxed),
        wave: look.wave.to_vec(),
        vis_mode: look.styles.vis,
        seek_style: look.styles.seek,
        bars: crate::ui::panels::visualizer::BarLayout::sanitised(
            look.cfg.vis.bar_width,
            look.cfg.vis.bar_gap,
        ),
        seek_phase: look.seek_phase,
    }
}

fn send_error(output: &mut impl Write, message: String) -> Result<()> {
    write_response(
        output,
        &Response::Error {
            message: message.chars().take(4096).collect(),
        },
    )
}

fn send_notice(output: &mut impl Write, message: String) -> Result<()> {
    write_response(
        output,
        &Response::Notice {
            message: message.chars().take(4096).collect(),
        },
    )
}

fn write_response(output: &mut impl Write, response: &Response<'_>) -> Result<()> {
    let bytes = serde_json::to_vec(response)?;
    if bytes.len() > MAX_OUTPUT {
        bail!("embed response exceeds size limit");
    }
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn read_line_bounded(reader: &mut impl BufRead, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if oversized {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request too large",
                ));
            }
            return Ok((!line.is_empty()).then_some(line));
        }
        let end = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |i| i + 1);
        let finished = chunk.get(end - 1) == Some(&b'\n');
        if line.len().saturating_add(end) <= max && !oversized {
            line.extend_from_slice(&chunk[..end]);
        } else {
            oversized = true;
        }
        reader.consume(end);
        if finished {
            return if oversized {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request too large",
                ))
            } else {
                Ok(Some(line))
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_and_unsupported_paths() {
        assert!(queue_items(vec!["song.flac".into()], 0).is_err());
        assert!(queue_items(vec!["/tmp/song.txt".into()], 0).is_err());
        assert!(queue_items(vec!["/tmp/song.flac".into()], 1).is_err());
    }

    #[test]
    fn bounded_reader_drains_bad_line_and_recovers() {
        let mut input: &[u8] = b"abcdef\n{}\n";
        assert_eq!(
            read_line_bounded(&mut input, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read_line_bounded(&mut input, 4).unwrap(),
            Some(b"{}\n".to_vec())
        );
    }

    #[test]
    fn stopped_event_has_an_explicit_wire_type() {
        assert_eq!(
            serde_json::to_string(&Response::Stopped).unwrap(),
            r#"{"type":"stopped"}"#
        );
    }

    #[test]
    fn queue_preserves_host_order_and_selected_index() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("b.flac");
        let a = dir.path().join("a.flac");
        std::fs::write(&b, b"").unwrap();
        std::fs::write(&a, b"").unwrap();
        let queue = queue_items(
            vec![b.to_str().unwrap().into(), a.to_str().unwrap().into()],
            1,
        )
        .unwrap();
        assert_eq!(queue[0].uri.to_string(), b.to_str().unwrap());
        assert_eq!(queue[1].uri.to_string(), a.to_str().unwrap());
        assert_eq!(queue[1].title.as_deref(), Some("a"));
    }
}
