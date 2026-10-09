//! Validated EQ profile imports. Never approximate a different DSP algorithm.
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::apo::{BiquadKind, ChannelMask, Filter, Profile, Stage, Width};

pub struct Import {
    pub profile: Profile,
    /// Other processors in a whole EasyEffects preset require acknowledgement.
    pub excluded: Vec<String>,
}

pub fn load(path: &Path) -> Result<Import> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        return Ok(Import {
            profile: Profile::parse_file(path)?,
            excluded: vec![],
        });
    }
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(1_048_577)
        .read_to_string(&mut text)?;
    if text.len() > 1_048_576 {
        bail!("EQ preset exceeds 1 MiB");
    }
    let json: Value = serde_json::from_str(&text).context("invalid EasyEffects JSON")?;
    easyeffects(
        &json,
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported EQ"),
    )
}

fn number(v: &Value, key: &str, default: f64) -> Result<f64> {
    match v.get(key) {
        None => Ok(default),
        Some(n) => n
            .as_f64()
            .filter(|n| n.is_finite())
            .with_context(|| format!("invalid {key}")),
    }
}
fn flag(v: &Value, key: &str) -> Result<bool> {
    match v.get(key) {
        None => Ok(false),
        Some(b) => b.as_bool().with_context(|| format!("invalid {key}")),
    }
}
fn label<'a>(v: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    match v.get(key) {
        None => Ok(default),
        Some(s) => s.as_str().with_context(|| format!("invalid {key}")),
    }
}

pub fn easyeffects(json: &Value, name: &str) -> Result<Import> {
    let section = json
        .get("output")
        .or_else(|| json.get("input"))
        .unwrap_or(json);
    let object = section
        .as_object()
        .context("EasyEffects preset must be an object")?;
    let eqs: Vec<_> = object
        .iter()
        .filter(|(k, _)| *k == "equalizer" || k.starts_with("equalizer#"))
        .collect();
    if eqs.len() != 1 {
        bail!(
            "select a preset containing exactly one equalizer (found {})",
            eqs.len()
        );
    }
    let (eq_id, eq) = eqs[0];
    // Follow the active chain; disabled processors elsewhere in the JSON are irrelevant.
    let order = section
        .get("plugins_order")
        .or_else(|| section.get("plugins-order"));
    if let Some(order) = order {
        let order = order.as_array().context("invalid plugins_order")?;
        if order.iter().any(|v| v.as_str().is_none()) {
            bail!("plugins_order must contain processor names");
        }
        if !order.iter().any(|v| v.as_str() == Some(eq_id)) {
            bail!("equalizer is not in the preset's active chain");
        }
    }
    let excluded = if let Some(order) = order {
        order
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|p| *p != eq_id)
            .map(str::to_owned)
            .collect()
    } else {
        object
            .iter()
            .filter(|(k, v)| *k != eq_id && v.is_object())
            .map(|(k, _)| k.clone())
            .collect()
    };
    if label(eq, "mode", "IIR")? != "IIR" {
        bail!("only EasyEffects IIR EQ can be imported; FIR/FFT processing is unsupported");
    }
    if flag(eq, "decramp")? {
        bail!("EasyEffects decramping is unsupported; disable it or export an APO profile");
    }
    for key in ["balance", "pitch-left", "pitch-right"] {
        if number(eq, key, 0.)? != 0. {
            bail!("EasyEffects {key} is unsupported; no profile was changed");
        }
    }
    let count = eq
        .get("num-bands")
        .and_then(Value::as_u64)
        .context("invalid or missing num-bands")?;
    if !(1..=32).contains(&count) {
        bail!("EasyEffects EQ needs 1–32 bands");
    }
    let bypass = flag(eq, "bypass")?;
    let mut stages = vec![Stage {
        enabled: !bypass,
        channels: ChannelMask::ALL,
        filter: Filter::Preamp {
            gain_db: number(eq, "input-gain", 0.)?,
        },
    }];
    let split = flag(eq, "split-channels")?;
    let channels: &[(&str, ChannelMask)] = if split {
        &[("left", ChannelMask(1)), ("right", ChannelMask(2))]
    } else {
        &[("left", ChannelMask::ALL)]
    };
    for &(channel, mask) in channels {
        let bands = eq
            .get(channel)
            .with_context(|| format!("missing {channel} bands"))?;
        let solo = (0..count).try_fold(false, |any, n| -> Result<_> {
            Ok(any | flag(&bands[format!("band{n}")], "solo")?)
        })?;
        for n in 0..count {
            let band = bands
                .get(format!("band{n}"))
                .with_context(|| format!("missing {channel} band{n}"))?;
            let ty = label(band, "type", "Off")?;
            if ty == "Off" {
                continue;
            }
            // APO (DR) is the same RBJ biquad family as AMP's APO backend.
            // RLC/BWC/LRX and their slope variants must not be relabeled as APO.
            if label(band, "mode", "")? != "APO (DR)" {
                bail!("{channel} band{} uses unsupported mode {}; export this EQ as APO text or use APO (DR)",n+1,label(band,"mode","unspecified")?);
            }
            if label(band, "slope", "x1")? != "x1" {
                bail!("{channel} band{} has an unsupported filter slope", n + 1);
            }
            let kind = match ty {
                "Bell" => BiquadKind::Peaking,
                "Low Pass" => BiquadKind::LowPass,
                "High Pass" => BiquadKind::HighPass,
                "Low Shelf" => BiquadKind::LowShelf,
                "High Shelf" => BiquadKind::HighShelf,
                "Notch" => BiquadKind::Notch,
                "All Pass" => BiquadKind::AllPass,
                _ => bail!("{channel} band{} has unsupported filter type {ty}", n + 1),
            };
            stages.push(Stage {
                enabled: !bypass && !flag(band, "mute")? && (!solo || flag(band, "solo")?),
                channels: mask,
                filter: Filter::Biquad {
                    kind,
                    frequency: number(band, "frequency", f64::NAN)?,
                    gain_db: number(band, "gain", 0.)?,
                    width: Width::Q(number(band, "q", f64::NAN)?),
                    corner_frequency: false,
                },
            });
        }
    }
    stages.push(Stage {
        enabled: !bypass,
        channels: ChannelMask::ALL,
        filter: Filter::Preamp {
            gain_db: number(eq, "output-gain", 0.)?,
        },
    });
    let profile = Profile {
        name: name.to_owned(),
        stages,
    };
    profile.validate()?;
    Ok(Import { profile, excluded })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn preset() -> Value {
        json!({"output":{"plugins_order":["equalizer#0"],"equalizer#0":{"mode":"IIR","num-bands":1,"input-gain":-4.,"left":{"band0":{"type":"Bell","mode":"APO (DR)","slope":"x1","frequency":560.,"gain":-3.8,"q":1.4}}}}})
    }
    #[test]
    fn preserves_parameters_and_preamp() {
        let p = easyeffects(&preset(), "Reference").unwrap().profile;
        assert_eq!(p.stages.len(), 3);
        assert_eq!(p.stages[0].filter, Filter::Preamp { gain_db: -4. });
        assert_eq!(
            p.stages[1].filter,
            Filter::Biquad {
                kind: BiquadKind::Peaking,
                frequency: 560.,
                gain_db: -3.8,
                width: Width::Q(1.4),
                corner_frequency: false
            }
        );
    }
    #[test]
    fn refuses_approximation_and_bad_ranges() {
        for (key, value) in [
            ("mode", json!("RLC (BT)")),
            ("q", json!(0.)),
            ("gain", json!(400.)),
            ("slope", json!("x4")),
        ] {
            let mut p = preset();
            p["output"]["equalizer#0"]["left"]["band0"][key] = value;
            assert!(easyeffects(&p, "bad").is_err(), "accepted {key}");
        }
    }
    #[test]
    fn refuses_global_processing_that_cannot_be_preserved() {
        for (key, value) in [
            ("mode", json!("FIR")),
            ("decramp", json!(true)),
            ("balance", json!(0.5)),
            ("pitch-left", json!(1.)),
        ] {
            let mut p = preset();
            p["output"]["equalizer#0"][key] = value;
            assert!(easyeffects(&p, "Unsupported").is_err(), "accepted {key}");
        }
    }
    #[test]
    fn reports_other_processors_before_application() {
        let mut p = preset();
        p["output"]["plugins_order"] = json!(["compressor#0", "equalizer#0"]);
        assert_eq!(
            easyeffects(&p, "EQ only").unwrap().excluded,
            vec!["compressor#0"]
        );
    }
    #[test]
    fn split_and_bypass_are_preserved() {
        let mut p = preset();
        let e = &mut p["output"]["equalizer#0"];
        e["split-channels"] = json!(true);
        e["bypass"] = json!(true);
        e["right"] = e["left"].clone();
        let p = easyeffects(&p, "split").unwrap().profile;
        assert_eq!(p.stages[1].channels, ChannelMask(1));
        assert_eq!(p.stages[2].channels, ChannelMask(2));
        assert!(p.stages.iter().all(|s| !s.enabled));
    }
}
