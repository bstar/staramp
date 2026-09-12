//! Rebuildable local audio features for Gravity Queue.
//!
//! The spectral signature here is the transparent fallback and the input
//! contract for the bundled embedding model. It makes journeys useful while a
//! model is absent or being upgraded, and its version is deliberately distinct
//! so model-produced rows can replace it without ambiguity.

use std::path::Path;

use anyhow::{Context, Result};
use realfft::RealFftPlanner;
use rusqlite::{params, Connection};

use crate::audio::decode::Decoder;
use crate::library::db::Db;
use crate::playlist::uri::TrackUri;
use crate::vfs::Vfs;

pub const ANALYZER_VERSION: i64 = 2;
pub const MODEL_VERSION: &str = "staramp-spectral-fallback-v2";
const WINDOW_SECS: usize = 8;
const EMBEDDING_DIMS: usize = 32;
const FFT_SIZE: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coverage {
    pub analyzed: usize,
    pub total: usize,
    pub pending: usize,
}

#[derive(Debug, Clone)]
struct Pending {
    uri: String,
    size: i64,
    modified: i64,
}

#[derive(Debug, Clone)]
struct Window {
    energy: f32,
    brightness: f32,
    dynamics: f32,
    distortion: f32,
    tempo: f32,
    embedding: Vec<f32>,
}

pub fn coverage(conn: &Connection) -> Result<Coverage> {
    let total = conn.query_row("SELECT COUNT(*) FROM track WHERE hidden=0", [], |r| {
        r.get::<_, i64>(0)
    })? as usize;
    let analyzed = conn.query_row(
        "SELECT COUNT(*) FROM sonic_feature s
          JOIN track t ON t.uri=s.uri
         WHERE t.hidden=0 AND s.analyzer_version=?1",
        [ANALYZER_VERSION],
        |r| r.get::<_, i64>(0),
    )? as usize;
    Ok(Coverage {
        analyzed,
        total,
        pending: total.saturating_sub(analyzed),
    })
}

pub fn analyze(db: &mut Db, root: &Path, limit: Option<usize>) -> Result<(usize, usize)> {
    analyze_inner(db, root, limit, true)
}

fn analyze_inner(
    db: &mut Db,
    root: &Path,
    limit: Option<usize>,
    announce: bool,
) -> Result<(usize, usize)> {
    analyze_rows(db, root, pending(&db.conn, limit)?, announce)
}

fn analyze_rows(
    db: &mut Db,
    root: &Path,
    pending: Vec<Pending>,
    announce: bool,
) -> Result<(usize, usize)> {
    let attempted = pending.len();
    let vfs = Vfs::local(root);
    let mut done = 0;
    for row in pending {
        match analyze_one(&vfs, db, &TrackUri::parse(&row.uri)) {
            Ok(windows) if !windows.is_empty() => {
                store(&db.conn, &row, &windows)?;
                done += 1;
                if announce {
                    eprintln!("analyzed  {}", row.uri);
                }
            }
            Ok(_) if announce => eprintln!("skipped   {} (no audio)", row.uri),
            Err(error) if announce => eprintln!("skipped   {} ({error})", row.uri),
            _ => {}
        }
    }
    Ok((done, attempted))
}

/// Low-priority analyzer controlled by the UI. Its channel has capacity one,
/// so repeated idle ticks collapse instead of building hours of queued work.
pub struct Worker {
    priority: crossbeam_channel::Sender<Vec<String>>,
    idle: crossbeam_channel::Sender<usize>,
}

impl Worker {
    pub fn spawn(root: std::path::PathBuf, index: std::path::PathBuf) -> Self {
        let (priority, priority_rx) = crossbeam_channel::bounded::<Vec<String>>(1);
        let (idle, idle_rx) = crossbeam_channel::bounded::<usize>(1);
        std::thread::Builder::new()
            .name("sonic-analysis".into())
            .spawn(move || loop {
                let requested = crossbeam_channel::select_biased! {
                    recv(priority_rx) -> message => match message {
                        Ok(uris) => Some(uris),
                        Err(_) => break,
                    },
                    recv(idle_rx) -> message => match message {
                        Ok(limit) => {
                            let result = Db::open(&index).and_then(|mut db| {
                                analyze_inner(&mut db, &root, Some(limit.max(1)), false)
                                    .map(|_| ())
                            });
                            if let Err(error) = result {
                                tracing::debug!("background sonic analysis: {error:#}");
                            }
                            None
                        }
                        Err(_) => break,
                    },
                };
                let Some(uris) = requested else { continue };
                let result = Db::open(&index).and_then(|mut db| {
                    let mut rows: std::collections::HashMap<String, Pending> =
                        pending(&db.conn, None)?
                            .into_iter()
                            .map(|row| (row.uri.clone(), row))
                            .collect();
                    let selected = uris
                        .iter()
                        .filter_map(|uri| rows.remove(uri))
                        .take(25)
                        .collect();
                    analyze_rows(&mut db, &root, selected, false).map(|_| ())
                });
                if let Err(error) = result {
                    tracing::debug!("background sonic analysis: {error:#}");
                }
            })
            .expect("spawn sonic analysis worker");
        Self { priority, idle }
    }

    pub fn request_idle(&self, limit: usize) {
        let _ = self.idle.try_send(limit.max(1));
    }

    pub fn request_tracks(&self, uris: Vec<String>) {
        let _ = self.priority.try_send(uris);
    }
}

fn pending(conn: &Connection, limit: Option<usize>) -> Result<Vec<Pending>> {
    let sql = "SELECT t.uri,f.size,f.mtime_ns
                 FROM track t JOIN file f ON f.id=t.file_id
                 LEFT JOIN sonic_feature s ON s.uri=t.uri
                WHERE t.hidden=0 AND
                      (s.uri IS NULL OR s.file_size<>f.size OR s.modified_at<>f.mtime_ns OR
                       s.analyzer_version<>?1 OR s.model_version<>?2)
                ORDER BY CASE WHEN t.cue_ordinal IS NULL THEN 0 ELSE 1 END, t.id
                LIMIT ?3";
    let cap = limit.unwrap_or(usize::MAX).min(i64::MAX as usize) as i64;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![ANALYZER_VERSION, MODEL_VERSION, cap], |r| {
        Ok(Pending {
            uri: r.get(0)?,
            size: r.get(1)?,
            modified: r.get(2)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

fn analyze_one(vfs: &Vfs, db: &Db, uri: &TrackUri) -> Result<Vec<Window>> {
    let mut opened = crate::audio::source::open(vfs, Some(db), uri)?;
    let rate = opened.decoder.spec().sample_rate.max(1) as usize;
    let total = opened.decoder.total_frames().unwrap_or((rate * 30) as u64);
    let window = (rate * WINDOW_SECS) as u64;
    let positions = [
        0,
        total.saturating_sub(window) / 2,
        total.saturating_sub(window),
    ];
    let mut out = Vec::new();
    for position in positions {
        opened.decoder.seek(position)?;
        let mono = read_mono(opened.decoder.as_mut(), window as usize)?;
        if !mono.is_empty() {
            out.push(describe(&mono, rate));
        }
    }
    Ok(out)
}

fn read_mono(decoder: &mut dyn Decoder, frames: usize) -> Result<Vec<f32>> {
    let channels = decoder.spec().channels.max(1) as usize;
    let mut interleaved = vec![0.0; 4096 * channels];
    let mut mono = Vec::with_capacity(frames);
    while mono.len() < frames {
        let want = (frames - mono.len()).min(4096);
        let got = decoder.read(&mut interleaved[..want * channels])?;
        if got == 0 {
            break;
        }
        for frame in interleaved[..got * channels].chunks_exact(channels) {
            mono.push(frame.iter().copied().sum::<f32>() / channels as f32);
        }
    }
    Ok(mono)
}

fn describe(samples: &[f32], rate: usize) -> Window {
    let mean_square = samples.iter().map(|v| v * v).sum::<f32>() / samples.len().max(1) as f32;
    let rms = mean_square.sqrt().max(1e-9);
    let db = 20.0 * rms.log10();
    let energy = ((db + 60.0) / 60.0).clamp(0.0, 1.0);

    let chunk_rms: Vec<f32> = samples
        .chunks((rate / 10).max(1))
        .map(|chunk| (chunk.iter().map(|v| v * v).sum::<f32>() / chunk.len().max(1) as f32).sqrt())
        .collect();
    let low = percentile(&chunk_rms, 0.1);
    let high = percentile(&chunk_rms, 0.9);
    let dynamics = ((high - low) / (high + 1e-6)).clamp(0.0, 1.0);
    let tempo = estimate_tempo(&chunk_rms);
    let peak = samples.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
    // A sine's RMS/peak ratio is ~0.707, so only crest factors compressed
    // beyond an ordinary clean waveform should increase this signal.
    let compression = ((rms / peak.max(1e-9) - 0.68) / 0.30).clamp(0.0, 1.0);
    let sustain = (low / high.max(1e-9)).clamp(0.0, 1.0);
    let (embedding, brightness, texture) = spectral_embedding(samples, rate);
    // Distorted guitar is sustained, compressed, and harmonically dense in
    // the midrange. Texture is deliberately capped by a high-frequency noise
    // penalty so cymbals and tape hiss do not look like a wall of guitars.
    let distortion = if energy < 0.08 {
        0.0
    } else {
        (compression * 0.38 + sustain * 0.27 + texture * 0.35).clamp(0.0, 1.0)
    };
    Window {
        energy,
        brightness,
        dynamics,
        distortion,
        tempo,
        embedding,
    }
}

fn percentile(values: &[f32], at: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() - 1) as f32 * at).round() as usize]
}

fn estimate_tempo(envelope: &[f32]) -> f32 {
    if envelope.len() < 8 {
        return 0.0;
    }
    // Envelope is 10 Hz. Search 60..180 BPM and prefer the strongest
    // autocorrelation, including a small center prior to avoid octave jumps.
    let mean = envelope.iter().sum::<f32>() / envelope.len() as f32;
    let centered: Vec<f32> = envelope.iter().map(|v| v - mean).collect();
    let mut best = (120.0, f32::NEG_INFINITY);
    for bpm in 60..=180 {
        let lag = (600.0 / bpm as f32).round() as usize;
        if lag == 0 || lag >= centered.len() {
            continue;
        }
        let corr = centered[..centered.len() - lag]
            .iter()
            .zip(&centered[lag..])
            .map(|(a, b)| a * b)
            .sum::<f32>()
            - (bpm as f32 - 120.0).abs() * 1e-7;
        if corr > best.1 {
            best = (bpm as f32, corr);
        }
    }
    best.0
}

fn spectral_embedding(samples: &[f32], rate: usize) -> (Vec<f32>, f32, f32) {
    let n = samples
        .len()
        .min(FFT_SIZE)
        .next_power_of_two()
        .clamp(2, FFT_SIZE);
    let mut input = vec![0.0f32; n];
    for (i, value) in samples.iter().take(n).enumerate() {
        let phase = i as f32 / (n - 1) as f32;
        input[i] = *value * (0.5 - 0.5 * (std::f32::consts::TAU * phase).cos());
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut spectrum = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    if fft
        .process_with_scratch(&mut input, &mut spectrum, &mut scratch)
        .is_err()
    {
        return (vec![0.0; EMBEDDING_DIMS], 0.5, 0.0);
    }
    let max_bin = ((20_000.0 * n as f32 / rate.max(1) as f32) as usize)
        .min(spectrum.len().saturating_sub(1))
        .max(1);
    let mut embedding = vec![0.0f32; EMBEDDING_DIMS];
    let mut weighted = 0.0;
    let mut total = 0.0;
    let mut mid_total = 0.0;
    let mut high_total = 0.0;
    let mut mid_bins = Vec::new();
    for (bin, value) in spectrum.iter().enumerate().take(max_bin + 1).skip(1) {
        let magnitude = value.norm();
        let hz = bin as f32 * rate.max(1) as f32 / n as f32;
        let position = bin as f32 / max_bin as f32;
        let band = ((position.sqrt() * EMBEDDING_DIMS as f32) as usize).min(EMBEDDING_DIMS - 1);
        embedding[band] += magnitude;
        weighted += magnitude * position;
        total += magnitude;
        if (180.0..=6_500.0).contains(&hz) {
            mid_total += magnitude;
            mid_bins.push(magnitude);
        } else if hz > 8_000.0 {
            high_total += magnitude;
        }
    }
    let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut embedding {
            *value /= norm;
        }
    }
    let mid_peak = mid_bins.iter().copied().fold(0.0f32, f32::max);
    let dense = if mid_peak <= f32::EPSILON || mid_bins.is_empty() {
        0.0
    } else {
        mid_bins.iter().filter(|&&v| v >= mid_peak * 0.015).count() as f32 / mid_bins.len() as f32
    };
    let density = (dense / 0.18).clamp(0.0, 1.0);
    let mid_share = (mid_total / total.max(1e-9)).clamp(0.0, 1.0);
    let noise = (high_total / total.max(1e-9)).clamp(0.0, 1.0);
    let texture = (density * 0.65 + mid_share * 0.35 - noise * 0.75).clamp(0.0, 1.0);
    (
        embedding,
        (weighted / total.max(1e-9)).clamp(0.0, 1.0),
        texture,
    )
}

fn store(conn: &Connection, row: &Pending, windows: &[Window]) -> Result<()> {
    let middle = &windows[windows.len() / 2];
    let avg = |take: fn(&Window) -> f32| {
        windows.iter().map(take).sum::<f32>() / windows.len().max(1) as f32
    };
    conn.execute(
        "INSERT INTO sonic_feature
         (uri,file_size,modified_at,analyzer_version,model_version,energy,brightness,dynamics,
          distortion,tempo,musical_key,embedding,intro_embedding,outro_embedding,analyzed_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,?11,?12,?13,?14)
         ON CONFLICT(uri) DO UPDATE SET
          file_size=excluded.file_size,modified_at=excluded.modified_at,
          analyzer_version=excluded.analyzer_version,model_version=excluded.model_version,
          energy=excluded.energy,brightness=excluded.brightness,dynamics=excluded.dynamics,
          distortion=excluded.distortion,tempo=excluded.tempo,embedding=excluded.embedding,
          intro_embedding=excluded.intro_embedding,outro_embedding=excluded.outro_embedding,
          analyzed_at=excluded.analyzed_at",
        params![
            row.uri,
            row.size,
            row.modified,
            ANALYZER_VERSION,
            MODEL_VERSION,
            avg(|w| w.energy),
            avg(|w| w.brightness),
            avg(|w| w.dynamics),
            avg(|w| w.distortion),
            avg(|w| w.tempo),
            crate::journey::encode_vector(&middle.embedding),
            crate::journey::encode_vector(&windows[0].embedding),
            crate::journey::encode_vector(&windows[windows.len() - 1].embedding),
            crate::library::db::now_secs(),
        ],
    )?;
    Ok(())
}

pub fn run(root: &Path, db: &mut Db, status_only: bool, all: bool, limit: usize) -> Result<()> {
    let before = coverage(&db.conn)?;
    println!(
        "sonic index  {}/{} analyzed · {} pending",
        before.analyzed, before.total, before.pending
    );
    if status_only {
        return Ok(());
    }
    let cap = (!all).then_some(limit.max(1));
    let (done, attempted) = analyze(db, root, cap).context("analyzing library audio")?;
    let after = coverage(&db.conn)?;
    println!(
        "completed    {done}/{attempted} · {}/{} analyzed",
        after.analyzed, after.total
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_finite_features() {
        let feature = describe(&vec![0.0; 44_100], 44_100);
        assert!(feature.energy.is_finite());
        assert!(feature.brightness.is_finite());
        assert!(feature.distortion.is_finite());
        assert!(feature.embedding.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn high_frequency_audio_is_brighter() {
        let wave = |hz: f32| {
            (0..16_384)
                .map(|i| (std::f32::consts::TAU * hz * i as f32 / 44_100.0).sin())
                .collect::<Vec<_>>()
        };
        assert!(
            describe(&wave(8_000.0), 44_100).brightness > describe(&wave(200.0), 44_100).brightness
        );
    }

    #[test]
    fn clipped_harmonic_wave_is_more_distorted_than_a_clean_tone() {
        let wave = |clipped: bool| {
            (0..44_100)
                .map(|i| {
                    let phase = std::f32::consts::TAU * 110.0 * i as f32 / 44_100.0;
                    let clean = phase.sin();
                    if clipped {
                        (clean * 5.0).tanh()
                    } else {
                        clean * 0.7
                    }
                })
                .collect::<Vec<_>>()
        };
        let clean = describe(&wave(false), 44_100).distortion;
        let clipped = describe(&wave(true), 44_100).distortion;
        assert!(clipped > clean + 0.15, "clean={clean} clipped={clipped}");
    }
}
