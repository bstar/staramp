//! Deterministic, explainable ordering of the unplayed queue.
//!
//! A query supplies the source pool; a journey derives a seed-first list of
//! related tracks and decides how they unfold. CUE tracks remain independently
//! addressable, so a journey can shape them like ordinary files.

use std::collections::{HashMap, VecDeque};

use rusqlite::Connection;

use crate::playlist::queue::QueueItem;
use crate::util::rng::Lcg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    #[default]
    Off,
    Rediscover,
    Climb,
    Wander,
    Bridge,
}

impl Preset {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Rediscover,
        Self::Climb,
        Self::Wander,
        Self::Bridge,
    ];

    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rediscover => "rediscover",
            Self::Climb => "climb",
            Self::Wander => "wander",
            Self::Bridge => "bridge to selected",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "rediscover" => Self::Rediscover,
            "climb" => Self::Climb,
            "wander" => Self::Wander,
            "bridge" | "bridge to selected" => Self::Bridge,
            _ => Self::Off,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub preset: Preset,
    /// How strongly the journey wins over smooth transitions, 0 to 1.
    pub intensity: f32,
    pub artist_gap: usize,
    /// How strongly to prefer artists used less often in this journey, 0 to 1.
    pub artist_variety: f32,
    /// Lowest seed-relative evidence admitted to a generated list, 0 to 1.
    pub min_match: f32,
    pub seed: u64,
    /// A storage index, not a view row. Used by Bridge and as a steering
    /// magnet by the other presets.
    pub anchor: Option<usize>,
    pub anchor_pull: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Preset::Off,
            intensity: 0.75,
            artist_gap: 4,
            artist_variety: 0.75,
            min_match: 0.70,
            seed: 1,
            anchor: None,
            anchor_pull: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Feature {
    pub energy: Option<f32>,
    pub brightness: Option<f32>,
    pub dynamics: Option<f32>,
    pub distortion: Option<f32>,
    pub tempo: Option<f32>,
    pub genre_family: Option<&'static str>,
    pub embedding: Vec<f32>,
    pub intro: Vec<f32>,
    pub outro: Vec<f32>,
    pub plays: u64,
    pub skips: u64,
    pub completion: Option<f32>,
    pub last_played: Option<i64>,
    /// Sum of explicit local more-like / less-like steering signals.
    pub affinity: f32,
}

impl Feature {
    fn familiarity(&self) -> f32 {
        let plays = (self.plays as f32 + 1.0).ln() / 4.0;
        let completion = self.completion.unwrap_or(0.0);
        let penalty = (self.skips as f32 + 1.0).ln() / 5.0;
        (plays * 0.55 + completion * 0.55 - penalty * 0.35).clamp(0.0, 1.0)
    }

    fn novelty(&self) -> f32 {
        if self.plays == 0 {
            1.0
        } else {
            (1.0 - self.familiarity()) * 0.65
        }
    }

    fn rediscovery(&self, now: i64) -> f32 {
        if self.plays < 2 {
            return 0.0;
        }
        let days = self
            .last_played
            .map(|at| (now - at).max(0) as f32 / 86_400.0)
            .unwrap_or(3650.0);
        self.familiarity() * ((days + 1.0).ln() / 8.0).clamp(0.0, 1.0)
    }
}

/// Load rebuildable sonic features and permanent activity without making the
/// planner know that they live in two attached databases.
pub fn load_features(conn: &Connection, items: &[QueueItem]) -> HashMap<String, Feature> {
    let mut out: HashMap<String, Feature> = items
        .iter()
        .map(|item| (item.uri.to_string(), Feature::default()))
        .collect();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT uri,energy,brightness,dynamics,distortion,tempo,embedding,intro_embedding,outro_embedding
           FROM sonic_feature",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<f32>>(1)?,
                r.get::<_, Option<f32>>(2)?,
                r.get::<_, Option<f32>>(3)?,
                r.get::<_, Option<f32>>(4)?,
                r.get::<_, Option<f32>>(5)?,
                r.get::<_, Option<Vec<u8>>>(6)?,
                r.get::<_, Option<Vec<u8>>>(7)?,
                r.get::<_, Option<Vec<u8>>>(8)?,
            ))
        }) {
            for row in rows.flatten() {
                if let Some(f) = out.get_mut(&row.0) {
                    f.energy = row.1;
                    f.brightness = row.2;
                    f.dynamics = row.3;
                    f.distortion = row.4;
                    f.tempo = row.5;
                    f.embedding = decode_vector(row.6.as_deref().unwrap_or_default());
                    f.intro = decode_vector(row.7.as_deref().unwrap_or_default());
                    f.outro = decode_vector(row.8.as_deref().unwrap_or_default());
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare("SELECT uri,genre FROM track WHERE hidden=0") {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        }) {
            for row in rows.flatten() {
                if let Some(f) = out.get_mut(&row.0) {
                    f.genre_family = primary_genre(row.1.as_deref());
                }
            }
        }
    }

    if let Ok(mut stmt) = conn
        .prepare("SELECT uri,COALESCE(SUM(signal),0) FROM activity.journey_feedback GROUP BY uri")
    {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f32>(1)?)))
        {
            for row in rows.flatten() {
                if let Some(f) = out.get_mut(&row.0) {
                    f.affinity = (row.1 / 5.0).clamp(-1.0, 1.0);
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT uri,play_count,skip_count,completion_ratio,last_played_at
           FROM activity.track_stat",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<f32>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        }) {
            for row in rows.flatten() {
                if let Some(f) = out.get_mut(&row.0) {
                    f.plays = row.1.max(0) as u64;
                    f.skips = row.2.max(0) as u64;
                    f.completion = row.3;
                    f.last_played = row.4;
                }
            }
        }
    }
    out
}

pub fn encode_vector(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    let (chunks, _) = bytes.as_chunks::<4>();
    chunks.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// Storage indices, suitable for the queue's order vector.
    pub order: Vec<usize>,
    pub reasons: HashMap<usize, String>,
    /// Seed-relative quality for admitted matches. The seed and an explicit
    /// Bridge destination are not matches and therefore do not appear here.
    pub match_quality: HashMap<usize, f32>,
    pub analyzed: usize,
}

#[derive(Debug)]
struct Block {
    tracks: Vec<usize>,
}

#[derive(Debug, Default)]
struct PlaylistContext {
    genre: Option<&'static str>,
    genre_confidence: f32,
    /// True when the genre describes the seed itself or a trustworthy cohort.
    genre_seeded: bool,
    vinyl_genre: Option<&'static str>,
    distortion: Option<f32>,
    distortion_confidence: f32,
}

struct Scoring<'a> {
    items: &'a [QueueItem],
    keys: &'a [String],
    features: &'a HashMap<String, Feature>,
    settings: &'a Settings,
    context: &'a PlaylistContext,
    now: i64,
}

#[derive(Debug, Clone, Copy)]
struct Score {
    total: f32,
    transition: f32,
    energy: f32,
    style: f32,
    variety: f32,
}

const CANDIDATE_BUDGET: usize = 256;

/// Derive a related list beginning with the playing or explicitly chosen seed.
pub fn plan(
    items: &[QueueItem],
    current_order: &[usize],
    current_pos: usize,
    features: &HashMap<String, Feature>,
    settings: &Settings,
    now: i64,
) -> Plan {
    if settings.preset == Preset::Off || current_order.len() < 2 {
        return Plan {
            order: current_order.to_vec(),
            reasons: HashMap::new(),
            match_quality: HashMap::new(),
            analyzed: features
                .values()
                .filter(|f| !f.embedding.is_empty())
                .count(),
        };
    }

    let current_pos = current_pos.min(current_order.len().saturating_sub(1));
    let playing = current_order[current_pos];
    let seed = settings
        .anchor
        .filter(|_| settings.anchor_pull > 0.0 && settings.preset != Preset::Bridge)
        .filter(|anchor| current_order.contains(anchor))
        .unwrap_or(playing);
    let seed_pos = current_order
        .iter()
        .position(|&track| track == seed)
        .unwrap_or(current_pos);
    let mut order = vec![seed];
    let keys: Vec<String> = items.iter().map(|item| item.uri.to_string()).collect();
    let context = playlist_context(items, current_order, seed_pos, features, &keys);
    let mut match_quality = HashMap::new();
    let related: Vec<usize> = current_order
        .iter()
        .copied()
        .filter(|&track| track != seed)
        .filter(|&track| {
            if settings.preset == Preset::Bridge && settings.anchor == Some(track) {
                return true;
            }
            let Some(quality) =
                match_quality_to_seed(track, seed, items, features, &keys, &context)
            else {
                return false;
            };
            if quality < settings.min_match.clamp(0.0, 1.0) {
                return false;
            }
            match_quality.insert(track, quality);
            true
        })
        .collect();
    let mut blocks = track_blocks(&related);
    let destination = settings
        .anchor
        .filter(|_| settings.preset == Preset::Bridge)
        .and_then(|anchor| blocks.iter().position(|b| b.tracks.contains(&anchor)))
        .map(|at| blocks.remove(at));
    let total = blocks.len() + usize::from(destination.is_some());
    let mut reasons = HashMap::new();
    reasons.insert(seed, "gravity seed".into());
    let mut previous = order.last().copied();
    let mut rng = Lcg::new(settings.seed);
    let artist_keys: Vec<String> = items.iter().map(artist_key).collect();
    let mut recent_artists: VecDeque<&str> = VecDeque::new();
    let mut artist_counts: HashMap<&str, usize> = HashMap::new();
    if let Some(&current) = order.last() {
        let artist = artist_keys[current].as_str();
        if !artist.is_empty() {
            if settings.artist_gap > 0 {
                recent_artists.push_back(artist);
            }
            artist_counts.insert(artist, 1);
        }
    }
    let scoring = Scoring {
        items,
        keys: &keys,
        features,
        settings,
        context: &context,
        now,
    };

    for slot in 0..blocks.len() {
        let progress = if total <= 1 {
            1.0
        } else {
            slot as f32 / (total - 1) as f32
        };
        let candidates = blocks.len().min(CANDIDATE_BUDGET);
        let start = rng.below(blocks.len());
        let mut best: Option<(usize, f32, Score)> = None;
        for sample in 0..candidates {
            // An evenly spaced, seed-rotated sample sees the whole remaining
            // queue without an O(n²) exhaustive scan.
            let at = (start + sample * blocks.len() / candidates) % blocks.len();
            let block = &blocks[at];
            let track = block.tracks[0];
            let artist = artist_keys[track].as_str();
            if !artist.is_empty() && recent_artists.contains(&artist) {
                continue;
            }
            let artist_uses =
                (!artist.is_empty()).then(|| artist_counts.get(artist).copied().unwrap_or(0));
            let score = score(track, previous, progress, artist_uses, &scoring);
            let ranked = score.total + rng.next_f32() * 0.0001;
            if best.as_ref().is_none_or(|(_, rank, _)| ranked > *rank) {
                best = Some((at, ranked, score));
            }
        }
        // Artist spacing is a preference, never a reason to strand the queue.
        // Even its fallback keeps a real score so every accepted row retains
        // the match-quality indicator promised by the UI.
        let (at, chosen_score) = match best {
            Some((at, _, score)) => (at, score),
            None => {
                let track = blocks[0].tracks[0];
                let artist = artist_keys[track].as_str();
                let uses =
                    (!artist.is_empty()).then(|| artist_counts.get(artist).copied().unwrap_or(0));
                (0, score(track, previous, progress, uses, &scoring))
            }
        };
        let block = blocks.swap_remove(at);
        let reason = reason(
            block.tracks[0],
            progress,
            &scoring,
            chosen_score,
            match_quality[&block.tracks[0]],
        );
        let artist = artist_keys[block.tracks[0]].as_str();
        if !artist.is_empty() {
            recent_artists.push_back(artist);
            *artist_counts.entry(artist).or_default() += 1;
            while recent_artists.len() > settings.artist_gap {
                recent_artists.pop_front();
            }
        }
        for track in block.tracks {
            reasons.insert(track, reason.clone());
            order.push(track);
            previous = Some(track);
        }
    }
    if let Some(block) = destination {
        for track in block.tracks {
            reasons.insert(track, "pinned destination".into());
            order.push(track);
        }
    }

    Plan {
        order,
        reasons,
        match_quality,
        analyzed: features
            .values()
            .filter(|f| !f.embedding.is_empty())
            .count(),
    }
}

fn track_blocks(order: &[usize]) -> Vec<Block> {
    order
        .iter()
        .map(|&track| Block {
            tracks: vec![track],
        })
        .collect()
}

fn score(
    track: usize,
    previous: Option<usize>,
    progress: f32,
    artist_uses: Option<usize>,
    scoring: &Scoring<'_>,
) -> Score {
    let Scoring {
        items,
        keys,
        features,
        settings,
        context,
        now,
    } = scoring;
    let empty = Feature::default();
    let feature = features.get(&keys[track]).unwrap_or(&empty);
    let transition = previous
        .and_then(|p| {
            let from = features.get(&keys[p])?;
            let a = if from.outro.is_empty() {
                &from.embedding
            } else {
                &from.outro
            };
            let b = if feature.intro.is_empty() {
                &feature.embedding
            } else {
                &feature.intro
            };
            Some(cosine(a, b))
        })
        .unwrap_or(0.5);
    let energy = feature
        .energy
        .unwrap_or_else(|| year_energy(items[track].year));
    let (style, style_confidence) = context.fit(feature);
    let variety = artist_uses
        .map(|uses| 1.0 / (uses as f32 + 1.0))
        .unwrap_or(0.5);
    let journey = match settings.preset {
        Preset::Rediscover => feature.rediscovery(*now),
        Preset::Climb => 1.0 - (energy - (0.15 + progress * 0.8)).abs(),
        Preset::Wander => {
            feature.novelty() * (0.45 + progress * 0.55)
                + feature.affinity * 0.15
                + (style - 0.5) * 1.4 * style_confidence
        }
        Preset::Bridge => {
            let Some(anchor) = settings.anchor.filter(|&a| a < keys.len()) else {
                return Score {
                    total: transition,
                    transition,
                    energy,
                    style,
                    variety,
                };
            };
            let Some(target) = features.get(&keys[anchor]) else {
                return Score {
                    total: transition,
                    transition,
                    energy,
                    style,
                    variety,
                };
            };
            cosine(&feature.embedding, &target.embedding) * progress + transition * (1.0 - progress)
        }
        Preset::Off => 0.0,
    };
    let anchor = settings
        .anchor
        .filter(|&a| a < keys.len())
        .and_then(|a| features.get(&keys[a]))
        .map(|a| cosine(&feature.embedding, &a.embedding))
        .unwrap_or(0.0)
        * settings.anchor_pull;
    let total = journey * settings.intensity
        + transition * (1.0 - settings.intensity) * 0.8
        + anchor * 0.35
        + variety * settings.artist_variety * 0.45;
    Score {
        total,
        transition,
        energy,
        style,
        variety,
    }
}

fn reason(
    track: usize,
    progress: f32,
    scoring: &Scoring<'_>,
    score: Score,
    quality: f32,
) -> String {
    let empty = Feature::default();
    let feature = scoring.features.get(&scoring.keys[track]).unwrap_or(&empty);
    match scoring.settings.preset {
        Preset::Rediscover => format!(
            "match {:.0}% · rediscovery {:.0}% · smooth {:.0}% · artist variety {:.0}%",
            quality * 100.0,
            feature.rediscovery(scoring.now) * 100.0,
            score.transition * 100.0,
            score.variety * 100.0
        ),
        Preset::Climb => format!(
            "match {:.0}% · energy {:.0}% · rising · artist variety {:.0}%",
            quality * 100.0,
            score.energy * 100.0,
            score.variety * 100.0
        ),
        Preset::Wander => format!(
            "match {:.0}% · novelty {:.0}% · connection {:.0}% · {} · artist variety {:.0}% · preference {:+.0}%",
            quality * 100.0,
            feature.novelty() * 100.0,
            score.transition * 100.0,
            scoring.context.describe_fit(feature, score.style),
            score.variety * 100.0,
            feature.affinity * 100.0
        ),
        Preset::Bridge => format!(
            "match {:.0}% · bridge {:.0}% · position {:.0}% · artist variety {:.0}%",
            quality * 100.0,
            score.total * 100.0,
            progress * 100.0,
            score.variety * 100.0
        ),
        Preset::Off => String::new(),
    }
}

impl PlaylistContext {
    fn fit(&self, feature: &Feature) -> (f32, f32) {
        let genre = self.genre.map(|dominant| {
            feature
                .genre_family
                .map(|candidate| if candidate == dominant { 1.0 } else { 0.0 })
                .unwrap_or(0.5)
        });
        let distortion = self.distortion.map(|target| {
            feature
                .distortion
                .map(|value| (1.0 - (value - target).abs() * 1.7).clamp(0.0, 1.0))
                .unwrap_or(0.5)
        });
        let mut weighted = 0.0;
        let mut weight = 0.0;
        if let Some(value) = genre {
            weighted += value * 0.72;
            weight += 0.72;
        }
        if let Some(value) = distortion {
            weighted += value * 0.28;
            weight += 0.28;
        }
        if weight == 0.0 {
            return (0.5, 0.0);
        }
        let fit = weighted / weight;
        let confidence =
            (self.genre_confidence * 0.72 + self.distortion_confidence * 0.28) / weight;
        (fit, confidence.clamp(0.0, 1.0))
    }

    fn describe_fit(&self, feature: &Feature, fit: f32) -> String {
        let detail = match (self.genre, feature.distortion) {
            (Some(family), Some(value)) => {
                format!(
                    "{family} gravity {:.0}% · guitar {:.0}%",
                    fit * 100.0,
                    value * 100.0
                )
            }
            (Some(family), None) => format!("{family} gravity {:.0}%", fit * 100.0),
            (None, Some(value)) => format!(
                "guitar fit {:.0}% · signal {:.0}%",
                fit * 100.0,
                value * 100.0
            ),
            (None, None) => "style unknown".into(),
        };
        detail
    }
}

fn playlist_context(
    items: &[QueueItem],
    order: &[usize],
    current_pos: usize,
    features: &HashMap<String, Feature>,
    keys: &[String],
) -> PlaylistContext {
    let current = order
        .get(current_pos)
        .and_then(|&track| items.get(track).map(|item| (track, item)));
    let current_feature = current.and_then(|(track, _)| features.get(&keys[track]));
    let mut peer_genres: HashMap<&'static str, usize> = HashMap::new();
    let mut peer_distortion = Vec::new();
    let current_is_vinyl = current.is_some_and(|(_, item)| is_vinyl_transfer(item));
    let mut vinyl_genres: HashMap<&'static str, usize> = HashMap::new();
    for &track in order {
        let Some(item) = items.get(track) else {
            continue;
        };
        let Some(feature) = features.get(&keys[track]) else {
            continue;
        };
        if current.is_some_and(|(_, seed)| same_recording(seed, item)) {
            if let Some(family) = feature.genre_family {
                *peer_genres.entry(family).or_default() += 1;
            }
            if let Some(value) = feature.distortion.filter(|v| v.is_finite()) {
                peer_distortion.push(value.clamp(0.0, 1.0));
            }
        }
        if current_is_vinyl && is_vinyl_transfer(item) {
            if let Some(family) = feature.genre_family {
                *vinyl_genres.entry(family).or_default() += 1;
            }
        }
    }
    let peer_tagged = peer_genres.values().sum::<usize>();
    let peer_genre = peer_genres
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(family, count)| (Some(family), 0.9 * count as f32 / peer_tagged as f32))
        .unwrap_or((None, 0.0));
    let vinyl_tagged = vinyl_genres.values().sum::<usize>();
    let vinyl_genre = vinyl_genres
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| vinyl_tagged >= 3 && *count * 2 >= vinyl_tagged)
        // A learned source-cohort prior is useful evidence, but it must remain
        // weaker than this recording's tags or those of a matching edition.
        .map(|(family, count)| (Some(family), 0.75 * count as f32 / vinyl_tagged as f32))
        .unwrap_or((None, 0.0));
    let direct_genre = current_feature.and_then(|feature| feature.genre_family);
    let (genre, genre_confidence, genre_seeded) = if let Some(family) = direct_genre {
        (Some(family), 1.0, true)
    } else if peer_genre.0.is_some() {
        (peer_genre.0, peer_genre.1, true)
    } else if vinyl_genre.0.is_some() {
        (vinyl_genre.0, vinyl_genre.1, true)
    } else {
        (None, 0.0, false)
    };
    peer_distortion.sort_by(f32::total_cmp);
    let current_distortion = current_feature.and_then(|feature| feature.distortion);
    let (distortion, distortion_confidence) = current_distortion
        .map(|value| (Some(value.clamp(0.0, 1.0)), 1.0))
        .or_else(|| {
            (!peer_distortion.is_empty()).then(|| {
                (
                    Some(peer_distortion[peer_distortion.len() / 2]),
                    (peer_distortion.len() as f32 / 3.0).clamp(0.5, 0.9),
                )
            })
        })
        .unwrap_or((None, 0.0));
    PlaylistContext {
        genre,
        genre_confidence,
        genre_seeded,
        vinyl_genre: vinyl_genre.0,
        distortion,
        distortion_confidence,
    }
}

/// Calibrated evidence that a track belongs in a list generated from the seed.
///
/// Genre conflict is a hard rejection. Everything else must have analysis:
/// cosine values cluster tightly for music, so the raw 0.82..1.0 range is
/// expanded to 0..1 before it is combined with genre and guitar texture. This
/// prevents a merely average 0.88 cosine from looking like an excellent 88%.
fn match_quality_to_seed(
    track: usize,
    seed: usize,
    items: &[QueueItem],
    features: &HashMap<String, Feature>,
    keys: &[String],
    context: &PlaylistContext,
) -> Option<f32> {
    if same_recording(&items[seed], &items[track]) {
        // Alternate encodings are useful for repairing sparse metadata on the
        // seed, but putting them beside it would make the generated list start
        // with two copies of the same song.
        return None;
    }
    let empty = Feature::default();
    let candidate = features.get(&keys[track]).unwrap_or(&empty);
    let seed_feature = features.get(&keys[seed]).unwrap_or(&empty);
    let candidate_genre = candidate.genre_family.or_else(|| {
        is_vinyl_transfer(&items[track])
            .then_some(context.vinyl_genre)
            .flatten()
    });
    if context.genre_seeded && candidate_genre != context.genre {
        return None;
    }
    if let (Some(expected), Some(actual)) = (context.genre, candidate_genre) {
        if expected != actual {
            return None;
        }
    }

    // A quality number without comparable analysis would be false precision.
    // Newly added tracks become eligible as soon as the incremental analyzer
    // has processed them.
    if seed_feature.embedding.is_empty() || candidate.embedding.is_empty() {
        return None;
    }
    let sonic =
        ((cosine(&seed_feature.embedding, &candidate.embedding) - 0.82) / 0.18).clamp(0.0, 1.0);
    let mut weighted = sonic * 0.65;
    let mut weight = 0.65;
    if let Some(expected) = context.genre {
        weighted += if candidate_genre == Some(expected) {
            0.25
        } else {
            0.0
        };
        weight += 0.25;
    }
    if let (Some(a), Some(b)) = (seed_feature.distortion, candidate.distortion) {
        let guitar = (1.0 - (a - b).abs() / 0.35).clamp(0.0, 1.0);
        weighted += guitar * 0.10;
        weight += 0.10;
    }
    Some((weighted / weight).clamp(0.0, 1.0))
}

/// Alternate rips and formats of the same recording can repair sparse tags on
/// the playing copy. A title alone is too broad ("Intro", for example), so
/// require durations close enough to describe the same performance as well.
fn same_recording(seed: &QueueItem, candidate: &QueueItem) -> bool {
    let same_title = seed
        .title
        .as_deref()
        .zip(candidate.title.as_deref())
        .is_some_and(|(a, b)| a.trim().eq_ignore_ascii_case(b.trim()));
    let same_duration = seed
        .duration_secs
        .zip(candidate.duration_secs)
        .is_some_and(|(a, b)| a.abs_diff(b) <= 8);
    same_title && same_duration
}

fn is_vinyl_transfer(item: &QueueItem) -> bool {
    item.uri
        .backing_path()
        .split(['/', '\\'])
        .any(|part| part.eq_ignore_ascii_case("vinyl"))
}

fn primary_genre(genre: Option<&str>) -> Option<&'static str> {
    let value = genre?.to_ascii_lowercase();
    let families: &[(&str, &[&str])] = &[
        ("metal", &["metal", "nwobhm"]),
        (
            "latin",
            &[
                "latin",
                "calypso",
                "salsa",
                "cumbia",
                "bachata",
                "merengue",
                "reggaeton",
                "samba",
                "bossa nova",
            ],
        ),
        ("punk", &["punk", "hardcore"]),
        ("hip-hop", &["hip hop", "hip-hop", "rap"]),
        (
            "electronic",
            &[
                "electronic",
                "electronica",
                "techno",
                "house",
                "trance",
                "synthwave",
                "industrial",
                "ambient",
            ],
        ),
        ("jazz", &["jazz", "bebop", "swing"]),
        (
            "classical",
            &["classical", "orchestral", "baroque", "romantic"],
        ),
        ("reggae", &["reggae", "ska", "dub"]),
        ("country", &["country", "bluegrass", "americana"]),
        ("folk", &["folk", "singer-songwriter"]),
        ("blues", &["blues"]),
        ("rock", &["rock", "grunge", "shoegaze"]),
        ("pop", &["pop"]),
    ];
    families
        .iter()
        .find(|(_, aliases)| aliases.iter().any(|alias| value.contains(alias)))
        .map(|(family, _)| *family)
}

fn artist_key(item: &QueueItem) -> String {
    item.artist
        .as_deref()
        .or(item.album_artist.as_deref())
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

fn year_energy(year: Option<i64>) -> f32 {
    // A neutral, deterministic fallback. The slight era slope prevents every
    // unanalyzed track tying without pretending metadata can hear the music.
    year.map(|y| (0.45 + (y - 1980) as f32 / 400.0).clamp(0.35, 0.65))
        .unwrap_or(0.5)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.5;
    }
    let (mut dot, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (&x, &y) in a.iter().zip(b) {
        dot += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa <= f32::EPSILON || bb <= f32::EPSILON {
        0.5
    } else {
        ((dot / (aa.sqrt() * bb.sqrt())) + 1.0) * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::uri::TrackUri;

    fn item(uri: &str, artist: &str) -> QueueItem {
        let mut item = QueueItem::new(TrackUri::parse(uri));
        item.artist = Some(artist.into());
        item
    }

    fn matching_features(items: &[QueueItem]) -> HashMap<String, Feature> {
        items
            .iter()
            .map(|item| {
                (
                    item.uri.to_string(),
                    Feature {
                        embedding: vec![1.0, 0.0],
                        intro: vec![1.0, 0.0],
                        outro: vec![1.0, 0.0],
                        ..Feature::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_plan_is_reproducible_and_starts_from_the_seed() {
        let items = vec![
            item("a.flac", "A"),
            item("b.flac", "B"),
            item("c.flac", "C"),
            item("d.flac", "D"),
        ];
        let order = vec![2, 0, 3, 1];
        let settings = Settings {
            preset: Preset::Wander,
            seed: 42,
            ..Settings::default()
        };
        let features = matching_features(&items);
        let a = plan(&items, &order, 1, &features, &settings, 100);
        let b = plan(&items, &order, 1, &features, &settings, 100);
        assert_eq!(a.order, b.order);
        assert_eq!(a.order[0], 0);
        let mut sorted = a.order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
    }

    #[test]
    fn bridge_finishes_at_the_destination() {
        let items = vec![
            item("a.flac", "A"),
            item("b.flac", "B"),
            item("c.flac", "C"),
        ];
        let settings = Settings {
            preset: Preset::Bridge,
            anchor: Some(1),
            ..Settings::default()
        };
        let plan = plan(&items, &[0, 1, 2], 0, &HashMap::new(), &settings, 100);
        assert_eq!(plan.order.last(), Some(&1));
    }

    #[test]
    fn cue_tracks_can_be_split_by_the_journey() {
        let items = vec![
            item("start.flac", "A"),
            item("album.cue/track0001", "B"),
            item("album.cue/track0002", "B"),
            item("other.flac", "C"),
        ];
        let mut features = matching_features(&items);
        for (uri, energy) in [
            ("album.cue/track0001", 0.9),
            ("album.cue/track0002", 0.1),
            ("other.flac", 0.5),
        ] {
            features
                .get_mut(&TrackUri::parse(uri).to_string())
                .unwrap()
                .energy = Some(energy);
        }
        let settings = Settings {
            preset: Preset::Climb,
            ..Settings::default()
        };
        let plan = plan(&items, &[0, 1, 2, 3], 0, &features, &settings, 100);
        let first = plan.order.iter().position(|i| *i == 1).unwrap();
        let second = plan.order.iter().position(|i| *i == 2).unwrap();
        assert_ne!(second.abs_diff(first), 1);
    }

    #[test]
    fn artist_variety_uses_the_breadth_of_the_queue() {
        let items = vec![
            item("playing.flac", "A"),
            item("a-2.flac", "A"),
            item("a-3.flac", "A"),
            item("b-1.flac", "B"),
            item("b-2.flac", "B"),
            item("c-1.flac", "C"),
        ];
        let settings = Settings {
            preset: Preset::Wander,
            artist_gap: 0,
            artist_variety: 1.0,
            seed: 42,
            ..Settings::default()
        };
        let result = plan(
            &items,
            &[0, 1, 2, 3, 4, 5],
            0,
            &matching_features(&items),
            &settings,
            100,
        );
        let first_three: std::collections::HashSet<_> = result.order[..3]
            .iter()
            .map(|&track| items[track].artist.as_deref().unwrap())
            .collect();
        assert_eq!(first_three.len(), 3, "the journey should visit A, B and C");
    }

    #[test]
    fn vectors_round_trip() {
        let values = [-1.0, 0.25, 9.5];
        assert_eq!(decode_vector(&encode_vector(&values)), values);
    }

    #[test]
    fn weak_matches_are_excluded_and_quality_is_explained() {
        let items = vec![
            item("seed.flac", "A"),
            item("strong.flac", "B"),
            item("weak.flac", "C"),
        ];
        let mut features = matching_features(&items);
        features.get_mut("weak.flac").unwrap().embedding = vec![0.0, 1.0];
        let settings = Settings {
            preset: Preset::Wander,
            min_match: 0.70,
            ..Settings::default()
        };
        let result = plan(&items, &[0, 1, 2], 0, &features, &settings, 100);
        assert_eq!(result.order, [0, 1]);
        assert!(result.match_quality[&1] >= 0.70);
        assert!(!result.match_quality.contains_key(&2));
        assert!(result.reasons[&1].starts_with("match 100%"));
    }

    #[test]
    fn wander_resists_a_novel_genre_outlier_in_a_metal_playlist() {
        let items = vec![
            item("playing.flac", "A"),
            item("metal-1.flac", "B"),
            item("calypso.flac", "C"),
            item("metal-2.flac", "D"),
            item("metal-3.flac", "E"),
        ];
        let mut features = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            let is_outlier = index == 2;
            features.insert(
                item.uri.to_string(),
                Feature {
                    genre_family: Some(if is_outlier { "latin" } else { "metal" }),
                    embedding: vec![1.0, 0.0],
                    intro: vec![1.0, 0.0],
                    outro: vec![1.0, 0.0],
                    plays: if is_outlier { 0 } else { 8 },
                    ..Feature::default()
                },
            );
        }
        let settings = Settings {
            preset: Preset::Wander,
            ..Settings::default()
        };
        let result = plan(&items, &[0, 1, 2, 3, 4], 0, &features, &settings, 100);
        assert_ne!(
            result.order[1], 2,
            "novel calypso must not beat metal context"
        );
        assert!(result.reasons[&result.order[1]].contains("metal gravity"));
    }

    #[test]
    fn an_untagged_playing_copy_borrows_genre_from_the_same_recording() {
        let mut playing = item("vinyl.flac", "");
        playing.title = Some("More Than A Feeling".into());
        playing.duration_secs = Some(283);
        let mut tagged_copy = item("remaster.flac", "Boston");
        tagged_copy.title = Some("More Than a Feeling".into());
        tagged_copy.duration_secs = Some(285);
        let items = vec![
            playing,
            tagged_copy,
            item("power-1.flac", "Metal A"),
            item("rock.flac", "Rock B"),
            item("power-2.flac", "Metal C"),
            item("power-3.flac", "Metal D"),
        ];
        let mut features = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            features.insert(
                item.uri.to_string(),
                Feature {
                    genre_family: match index {
                        1 | 3 => Some("rock"),
                        2 | 4 | 5 => Some("metal"),
                        _ => None,
                    },
                    embedding: vec![1.0, 0.0],
                    intro: vec![1.0, 0.0],
                    outro: vec![1.0, 0.0],
                    ..Feature::default()
                },
            );
        }
        let settings = Settings {
            preset: Preset::Wander,
            artist_gap: 0,
            artist_variety: 0.0,
            ..Settings::default()
        };
        let result = plan(&items, &[0, 1, 2, 3, 4, 5], 0, &features, &settings, 100);
        assert_eq!(result.order[0], 0, "the base track starts the new list");
        assert_eq!(result.order[1], 3, "the alternate rip is not duplicated");
        assert!(result
            .order
            .iter()
            .all(|track| !matches!(*track, 1 | 2 | 4 | 5)));
        assert!(result.reasons[&result.order[1]].contains("rock gravity"));
    }

    #[test]
    fn an_untagged_vinyl_transfer_learns_its_librarys_vinyl_context() {
        let items = vec![
            item("Boston/vinyl/seed.flac", ""),
            item("A/vinyl/rock-1.flac", "Rock A"),
            item("B/vinyl/rock-2.flac", "Rock B"),
            item("C/vinyl/rock-3.flac", "Rock C"),
            item("Metal/metal-1.flac", "Metal A"),
            item("Metal/metal-2.flac", "Metal B"),
            item("Metal/metal-3.flac", "Metal C"),
            item("Metal/metal-4.flac", "Metal D"),
        ];
        let mut features = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            features.insert(
                item.uri.to_string(),
                Feature {
                    genre_family: match index {
                        1..=3 => Some("rock"),
                        4..=7 => Some("metal"),
                        _ => None,
                    },
                    embedding: vec![1.0, 0.0],
                    intro: vec![1.0, 0.0],
                    outro: vec![1.0, 0.0],
                    ..Feature::default()
                },
            );
        }
        let settings = Settings {
            preset: Preset::Wander,
            artist_gap: 0,
            artist_variety: 0.0,
            ..Settings::default()
        };
        let result = plan(
            &items,
            &[0, 1, 2, 3, 4, 5, 6, 7],
            0,
            &features,
            &settings,
            100,
        );
        assert!(matches!(result.order[1], 1..=3));
        assert!(result.reasons[&result.order[1]].contains("rock gravity"));
    }

    #[test]
    fn genre_families_recognize_metal_and_latin_subgenres() {
        assert_eq!(primary_genre(Some("Symphonic Power Metal")), Some("metal"));
        assert_eq!(primary_genre(Some("Caribbean / Calypso")), Some("latin"));
    }
}
