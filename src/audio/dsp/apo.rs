//! Equalizer APO's portable equalizer configuration format.
//!
//! This deliberately stops at equalization.  Device routing, mixing, delays,
//! convolution files and the expression language describe a Windows audio
//! graph rather than an equalizer preset and are rejected instead of being
//! silently ignored.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub enabled: bool,
    pub channels: ChannelMask,
    pub filter: Filter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMask(pub u64);

impl ChannelMask {
    pub const ALL: Self = Self(u64::MAX);

    pub fn contains(self, channel: usize) -> bool {
        channel < 64 && self.0 & (1u64 << channel) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BiquadKind {
    Peaking,
    LowPass,
    HighPass,
    BandPass,
    LowShelf,
    HighShelf,
    Notch,
    AllPass,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Width {
    Q(f64),
    Bandwidth(f64),
    Slope(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    Preamp {
        gain_db: f64,
    },
    Biquad {
        kind: BiquadKind,
        frequency: f64,
        gain_db: f64,
        width: Width,
        /// `LS`/`HS` use a corner frequency; `LSC`/`HSC` use the centre.
        corner_frequency: bool,
    },
    Iir {
        numerator: Vec<f64>,
        denominator: Vec<f64>,
    },
    GraphicEq {
        points: Vec<(f64, f64)>,
    },
}

impl Profile {
    /// Validate profiles assembled by the editor or received over IPC. File
    /// imports perform the same checks while parsing, but serde deliberately
    /// does not make untrusted JSON a way around them.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("profile name cannot be empty");
        }
        for (index, stage) in self.stages.iter().enumerate() {
            let valid = match &stage.filter {
                Filter::Preamp { gain_db } => gain_db.is_finite(),
                Filter::Biquad {
                    kind,
                    frequency,
                    gain_db,
                    width,
                    ..
                } => {
                    let shelf = matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf);
                    let compatible = match width {
                        Width::Q(_) => true,
                        Width::Bandwidth(_) => !shelf,
                        Width::Slope(_) => shelf,
                    };
                    frequency.is_finite()
                        && *frequency > 0.0
                        && gain_db.is_finite()
                        && width_value(*width).is_finite()
                        && width_value(*width) > 0.0
                        && compatible
                }
                Filter::Iir {
                    numerator,
                    denominator,
                } => {
                    !numerator.is_empty()
                        && numerator.len() == denominator.len()
                        && numerator.len() <= 65
                        && denominator[0] != 0.0
                        && numerator.iter().chain(denominator).all(|v| v.is_finite())
                }
                Filter::GraphicEq { points } => {
                    points.len() >= 2
                        && points
                            .iter()
                            .all(|(f, g)| f.is_finite() && *f > 0.0 && g.is_finite())
                        && points.windows(2).all(|pair| pair[0].0 < pair[1].0)
                }
            };
            if !valid {
                bail!("stage {} has invalid parameters", index + 1);
            }
        }
        Ok(())
    }

    pub fn legacy(name: impl Into<String>, preamp: f32, gains: &[f32; 10]) -> Self {
        let mut stages = vec![Stage {
            enabled: true,
            channels: ChannelMask::ALL,
            filter: Filter::Preamp {
                gain_db: preamp as f64,
            },
        }];
        stages.extend(
            super::eq::BANDS
                .iter()
                .zip(gains)
                .map(|(&frequency, &gain)| Stage {
                    enabled: true,
                    channels: ChannelMask::ALL,
                    filter: Filter::Biquad {
                        kind: BiquadKind::Peaking,
                        frequency,
                        gain_db: gain as f64,
                        width: Width::Q(super::eq::Q),
                        corner_frequency: false,
                    },
                }),
        );
        Self {
            name: name.into(),
            stages,
        }
    }

    pub fn parse_file(path: &Path) -> Result<Self> {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported".into());
        let mut stages = Vec::new();
        let mut stack = Vec::new();
        parse_one(path, &mut stages, &mut stack, ChannelMask::ALL)?;
        if stages.is_empty() {
            bail!("{} contains no supported EQ commands", path.display());
        }
        let profile = Self { name, stages };
        profile.validate()?;
        Ok(profile)
    }

    pub fn to_apo(&self) -> String {
        let mut out = format!("# Exported by staramp: {}\n", self.name);
        let mut channels = ChannelMask::ALL;
        for (i, stage) in self.stages.iter().enumerate() {
            if stage.channels != channels {
                channels = stage.channels;
                let _ = writeln!(out, "Channel: {}", format_channels(channels));
            }
            let state = if stage.enabled { "ON" } else { "OFF" };
            match &stage.filter {
                Filter::Preamp { gain_db } => {
                    if stage.enabled {
                        let _ = writeln!(out, "Preamp: {} dB", number(*gain_db));
                    } else {
                        let _ = writeln!(out, "# staramp-disabled Preamp: {} dB", number(*gain_db));
                    }
                }
                Filter::Biquad {
                    kind,
                    frequency,
                    gain_db,
                    width,
                    corner_frequency,
                } => {
                    let ty = filter_name(*kind, *corner_frequency, *width);
                    let _ = write!(out, "Filter {}: {state} {ty}", i + 1);
                    if let Width::Slope(slope) = width {
                        let _ = write!(out, " {}dB", number(*slope));
                    }
                    let _ = write!(out, " Fc {} Hz", number(*frequency));
                    if matches!(
                        kind,
                        BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf
                    ) {
                        let _ = write!(out, " Gain {} dB", number(*gain_db));
                    }
                    match width {
                        Width::Q(q) => {
                            let _ = write!(out, " Q {}", number(*q));
                        }
                        Width::Bandwidth(bw) => {
                            let _ = write!(out, " BW Oct {}", number(*bw));
                        }
                        Width::Slope(_) => {}
                    }
                    out.push('\n');
                }
                Filter::Iir {
                    numerator,
                    denominator,
                } => {
                    let _ = write!(out, "Filter {}: {state} IIR", i + 1);
                    let _ = write!(out, " Order {} Coefficients", numerator.len() - 1);
                    for value in numerator.iter().chain(denominator) {
                        let _ = write!(out, " {}", number(*value));
                    }
                    out.push('\n');
                }
                Filter::GraphicEq { points } => {
                    if !stage.enabled {
                        out.push_str("# staramp-disabled GraphicEQ:");
                    } else {
                        out.push_str("GraphicEQ:");
                    }
                    for (n, (frequency, gain)) in points.iter().enumerate() {
                        if n > 0 {
                            out.push(';');
                        }
                        let _ = write!(out, " {} {}", number(*frequency), number(*gain));
                    }
                    out.push('\n');
                }
            }
        }
        out
    }

    pub fn managed_path(&self) -> Result<PathBuf> {
        let name = safe_name(&self.name)?;
        Ok(crate::paths::equalizer_dir()?.join(format!("{name}.txt")))
    }

    pub fn save_managed(&self) -> Result<PathBuf> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);

        let path = self.managed_path()?;
        let parent = path.parent().expect("managed profile has a parent");
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("txt.tmp-{}-{serial}", std::process::id()));
        std::fs::write(&tmp, self.to_apo())
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("saving {}", path.display()))?;
        Ok(path)
    }
}

pub fn managed_profiles() -> Vec<Profile> {
    let Ok(dir) = crate::paths::equalizer_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut profiles = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|v| v.to_str()),
                Some("txt" | "apo")
            )
        })
        .filter_map(|path| Profile::parse_file(&path).ok())
        .collect::<Vec<_>>();
    profiles.sort_by_key(|profile| profile.name.to_lowercase());
    profiles
}

fn safe_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        bail!("profile name must be a plain file name");
    }
    Ok(name.to_string())
}

fn parse_one(
    path: &Path,
    stages: &mut Vec<Stage>,
    stack: &mut Vec<PathBuf>,
    inherited_channels: ChannelMask,
) -> Result<ChannelMask> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("opening APO preset {}", path.display()))?;
    if stack.contains(&canonical) {
        bail!("APO include cycle through {}", canonical.display());
    }
    if stack.len() >= 32 {
        bail!("APO includes are nested more than 32 files deep");
    }
    stack.push(canonical.clone());
    let text = std::fs::read_to_string(&canonical)
        .with_context(|| format!("reading APO preset {}", canonical.display()))?;
    if text.len() > 4 * 1024 * 1024 {
        bail!(
            "{} is larger than the 4 MiB preset limit",
            canonical.display()
        );
    }
    let mut channels = inherited_channels;
    for (line_index, raw) in text.lines().enumerate() {
        let line_no = line_index + 1;
        let line = raw.trim();
        if let Some(disabled) = line.strip_prefix("# staramp-disabled ") {
            let (command, parameters) = disabled
                .split_once(':')
                .context("invalid staramp disabled EQ stage")?;
            let filter = if command.eq_ignore_ascii_case("Preamp") {
                Filter::Preamp {
                    gain_db: first_number(parameters.trim())?,
                }
            } else if command.eq_ignore_ascii_case("GraphicEQ") {
                Filter::GraphicEq {
                    points: parse_graphic(parameters.trim())?,
                }
            } else {
                bail!("unsupported disabled EQ stage {command:?}")
            };
            stages.push(Stage {
                enabled: false,
                channels,
                filter,
            });
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((command, parameters)) = line.split_once(':') else {
            continue; // Equalizer APO also treats free text as a comment.
        };
        let command = command.trim();
        let parameters = parameters.trim();
        let result = if command.eq_ignore_ascii_case("Preamp") {
            let gain_db = first_number(parameters)?;
            stages.push(Stage {
                enabled: true,
                channels,
                filter: Filter::Preamp { gain_db },
            });
            Ok(())
        } else if command.eq_ignore_ascii_case("GraphicEQ") {
            let points = parse_graphic(parameters)?;
            stages.push(Stage {
                enabled: true,
                channels,
                filter: Filter::GraphicEq { points },
            });
            Ok(())
        } else if command.eq_ignore_ascii_case("Channel") {
            channels = parse_channels(parameters)?;
            Ok(())
        } else if command.eq_ignore_ascii_case("Include") {
            let include = Path::new(parameters.trim_matches('"'));
            let include = if include.is_absolute() {
                include.to_path_buf()
            } else {
                canonical.parent().unwrap_or(Path::new(".")).join(include)
            };
            channels = parse_one(&include, stages, stack, channels)?;
            Ok(())
        } else if command.to_ascii_lowercase().starts_with("filter") {
            stages.push(parse_filter(parameters, channels)?);
            Ok(())
        } else {
            Err(anyhow!("unsupported APO directive {command:?}"))
        };
        result.with_context(|| format!("{}:{line_no}", canonical.display()))?;
    }
    stack.pop();
    Ok(channels)
}

fn parse_filter(text: &str, channels: ChannelMask) -> Result<Stage> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some(on) = words.first() else {
        bail!("empty Filter command")
    };
    let enabled = if on.eq_ignore_ascii_case("on") {
        true
    } else if on.eq_ignore_ascii_case("off") {
        false
    } else {
        bail!("Filter must be ON or OFF")
    };
    let ty = words
        .get(1)
        .context("Filter has no type")?
        .to_ascii_uppercase();
    if ty == "IIR" {
        let order = value_after(&words, "Order")? as usize;
        if order == 0 || order > 64 {
            bail!("IIR order must be between 1 and 64")
        }
        let at = position(&words, "Coefficients")? + 1;
        let values = words[at..]
            .iter()
            .map(|v| parse_number(v))
            .collect::<Result<Vec<_>>>()?;
        if values.len() != 2 * (order + 1) {
            bail!(
                "IIR order {order} requires {} coefficients, found {}",
                2 * (order + 1),
                values.len()
            );
        }
        if values[order + 1] == 0.0 {
            bail!("IIR a0 cannot be zero")
        }
        return Ok(Stage {
            enabled,
            channels,
            filter: Filter::Iir {
                numerator: values[..=order].to_vec(),
                denominator: values[order + 1..].to_vec(),
            },
        });
    }
    let (kind, corner_frequency) = match ty.as_str() {
        "PK" | "PEQ" | "MODAL" => (BiquadKind::Peaking, false),
        "LP" | "LPQ" => (BiquadKind::LowPass, false),
        "HP" | "HPQ" => (BiquadKind::HighPass, false),
        "BP" => (BiquadKind::BandPass, false),
        "LS" => (BiquadKind::LowShelf, true),
        "LSC" => (BiquadKind::LowShelf, false),
        "HS" => (BiquadKind::HighShelf, true),
        "HSC" => (BiquadKind::HighShelf, false),
        "NO" => (BiquadKind::Notch, false),
        "AP" => (BiquadKind::AllPass, false),
        _ => bail!("unsupported APO filter type {ty}"),
    };
    let frequency = value_after(&words, "Fc")?;
    if !frequency.is_finite() || frequency <= 0.0 {
        bail!("filter frequency must be positive")
    }
    let gain_db = optional_value_after(&words, "Gain")?.unwrap_or(0.0);
    let width = if let Some(q) = optional_value_after(&words, "Q")? {
        Width::Q(q)
    } else if let Some(bw) = optional_value_after(&words, "Oct")? {
        Width::Bandwidth(bw)
    } else if matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf)
        && words
            .get(2)
            .is_some_and(|v| v.to_ascii_lowercase().ends_with("db"))
    {
        Width::Slope(parse_number(words[2].trim_end_matches(|c: char| {
            c.eq_ignore_ascii_case(&'d') || c.eq_ignore_ascii_case(&'b')
        }))?)
    } else if matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf)
        && words.get(3).is_some_and(|v| v.eq_ignore_ascii_case("dB"))
    {
        Width::Slope(parse_number(words[2])?)
    } else {
        Width::Q(match kind {
            BiquadKind::Peaking | BiquadKind::AllPass => bail!("{ty} requires Q or BW Oct"),
            BiquadKind::Notch => 30.0,
            BiquadKind::LowShelf | BiquadKind::HighShelf => {
                return Ok(Stage {
                    enabled,
                    channels,
                    filter: Filter::Biquad {
                        kind,
                        frequency,
                        gain_db,
                        width: Width::Slope(10.8),
                        corner_frequency,
                    },
                })
            }
            _ => std::f64::consts::FRAC_1_SQRT_2,
        })
    };
    if width_value(width) <= 0.0 || !width_value(width).is_finite() {
        bail!("Q, bandwidth, or slope must be positive")
    }
    Ok(Stage {
        enabled,
        channels,
        filter: Filter::Biquad {
            kind,
            frequency,
            gain_db,
            width,
            corner_frequency,
        },
    })
}

fn parse_graphic(text: &str) -> Result<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for pair in text.split(';') {
        let values = pair
            .split_whitespace()
            .map(parse_number)
            .collect::<Result<Vec<_>>>()?;
        if values.is_empty() {
            continue;
        }
        if values.len() != 2 || values[0] <= 0.0 {
            bail!("GraphicEQ points must be positive-frequency/gain pairs")
        }
        points.push((values[0], values[1]));
    }
    if points.len() < 2 {
        bail!("GraphicEQ requires at least two points")
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    if points.windows(2).any(|p| p[0].0 == p[1].0) {
        bail!("GraphicEQ frequencies must be unique")
    }
    Ok(points)
}

fn parse_channels(text: &str) -> Result<ChannelMask> {
    if text.trim().eq_ignore_ascii_case("all") {
        return Ok(ChannelMask::ALL);
    }
    let mut bits = 0u64;
    for name in text.split_whitespace() {
        let index = match name.to_ascii_uppercase().as_str() {
            "L" => 0,
            "R" => 1,
            "C" => 2,
            "LFE" | "SUB" => 3,
            "RL" => 4,
            "RR" => 5,
            "RC" => 6,
            "SL" => 6,
            "SR" => 7,
            _ => name
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .context("unknown channel identifier")?,
        };
        if index >= 64 {
            bail!("channel indices above 64 are unsupported")
        }
        bits |= 1u64 << index;
    }
    if bits == 0 {
        bail!("Channel selects no channels")
    }
    Ok(ChannelMask(bits))
}

fn format_channels(mask: ChannelMask) -> String {
    if mask == ChannelMask::ALL {
        return "all".into();
    }
    (0..64)
        .filter(|&i| mask.contains(i))
        .map(|i| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn position(words: &[&str], key: &str) -> Result<usize> {
    words
        .iter()
        .position(|v| v.eq_ignore_ascii_case(key))
        .with_context(|| format!("missing {key}"))
}
fn value_after(words: &[&str], key: &str) -> Result<f64> {
    optional_value_after(words, key)?.with_context(|| format!("missing {key} value"))
}
fn optional_value_after(words: &[&str], key: &str) -> Result<Option<f64>> {
    let Some(at) = words.iter().position(|v| v.eq_ignore_ascii_case(key)) else {
        return Ok(None);
    };
    Ok(Some(parse_number(
        words.get(at + 1).context("missing numeric value")?,
    )?))
}
fn first_number(text: &str) -> Result<f64> {
    parse_number(text.split_whitespace().next().context("missing number")?)
}
fn parse_number(text: &str) -> Result<f64> {
    let value: f64 = text
        // Equalizer APO accepts the locale decimal comma by normalising it
        // to a dot before parsing; it does not use thousands separators in
        // configuration numbers.
        .replace(',', ".")
        .parse()
        .with_context(|| format!("invalid number {text:?}"))?;
    if !value.is_finite() {
        bail!("number must be finite")
    }
    Ok(value)
}
fn width_value(width: Width) -> f64 {
    match width {
        Width::Q(v) | Width::Bandwidth(v) | Width::Slope(v) => v,
    }
}
fn number(value: f64) -> String {
    value.to_string()
}
fn filter_name(kind: BiquadKind, corner: bool, _width: Width) -> &'static str {
    match (kind, corner) {
        (BiquadKind::Peaking, _) => "PK",
        (BiquadKind::LowPass, _) => "LPQ",
        (BiquadKind::HighPass, _) => "HPQ",
        (BiquadKind::BandPass, _) => "BP",
        (BiquadKind::LowShelf, true) => "LS",
        (BiquadKind::LowShelf, false) => "LSC",
        (BiquadKind::HighShelf, true) => "HS",
        (BiquadKind::HighShelf, false) => "HSC",
        (BiquadKind::Notch, _) => "NO",
        (BiquadKind::AllPass, _) => "AP",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_exports_the_portable_apo_forms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("phones.txt");
        std::fs::write(&path, "Preamp: -6.5 dB\nFilter 1: ON PK Fc 50 Hz Gain -3 dB Q 2.5\nGraphicEQ: 20 0; 1000 -2; 20000 1\n").unwrap();
        let p = Profile::parse_file(&path).unwrap();
        assert_eq!(p.stages.len(), 3);
        assert!(p
            .to_apo()
            .contains("Filter 2: ON PK Fc 50 Hz Gain -3 dB Q 2.5"));
    }

    #[test]
    fn unsupported_processing_is_not_silently_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        std::fs::write(&path, "Convolution: room.wav\n").unwrap();
        let e = Profile::parse_file(&path).unwrap_err().to_string();
        assert!(e.contains(":1"), "{e}");
    }

    #[test]
    fn includes_channels_and_disabled_stages_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("part.txt"),
            "Channel: L\nFilter 1: OFF HPQ Fc 80 Hz Q 0.7071067811865476\n",
        )
        .unwrap();
        let root = dir.path().join("phones.txt");
        std::fs::write(&root, "Include: part.txt\n").unwrap();
        let profile = Profile::parse_file(&root).unwrap();
        assert_eq!(profile.stages[0].channels, ChannelMask(1));
        assert!(!profile.stages[0].enabled);

        let exported = dir.path().join("roundtrip.txt");
        std::fs::write(&exported, profile.to_apo()).unwrap();
        let reparsed = Profile::parse_file(&exported).unwrap();
        assert_eq!(reparsed.stages, profile.stages);
    }

    #[test]
    fn locale_decimal_commas_match_equalizer_apo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("comma.txt");
        std::fs::write(
            &path,
            "Preamp: -6,5 dB\nFilter 1: ON PK Fc 1000 Hz Gain 2,25 dB Q 1,4\n",
        )
        .unwrap();
        let profile = Profile::parse_file(&path).unwrap();
        assert_eq!(profile.stages[0].filter, Filter::Preamp { gain_db: -6.5 });
        let Filter::Biquad { gain_db, width, .. } = profile.stages[1].filter else {
            unreachable!()
        };
        assert_eq!(gain_db, 2.25);
        assert_eq!(width, Width::Q(1.4));
    }
}
