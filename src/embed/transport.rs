//! Render-only transport protocol. No player, output device or startup writes.
use std::io::{BufRead, Read, Write};

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

use super::{native, render::HitTarget, Palette};

#[derive(Deserialize)]
struct Request {
    width: u16,
    height: u16,
    theme: Palette,
    playing: bool,
    paused: bool,
    volume: f32,
    #[serde(default)]
    pointer: Option<[u16; 2]>,
    #[serde(default)]
    movie: bool,
    #[serde(default)]
    picker: bool,
    #[serde(default)]
    entries: Vec<String>,
    #[serde(default)]
    selected: usize,
    #[serde(default)]
    title: String,
    #[serde(default)]
    font: u16,
}

#[derive(Serialize)]
struct Response {
    surface: Option<starkit::native_surface::Surface>,
    action: Option<String>,
    value: Option<f32>,
}

fn respond(request: Request) -> Result<Response> {
    ensure!(
        (1..=8192).contains(&request.width)
            && (1..=8192).contains(&request.height)
            && u64::from(request.width) * u64::from(request.height) <= 32_000_000,
        "Invalid transport dimensions"
    );
    ensure!(
        request.volume.is_finite() && (0.0..=1.0).contains(&request.volume),
        "Invalid volume"
    );
    let surface = if request.picker {
        native::track_surface(
            request.width,
            request.height,
            &request.theme,
            &request.title,
            &request.entries,
            request.selected,
            request.font,
        )
    } else if request.movie {
        native::movie_transport_surface(
            request.width,
            request.height,
            &request.theme,
            request.playing,
            request.paused,
            request.volume,
            request.font,
        )
    } else {
        native::transport_surface(
            request.width,
            request.height,
            &request.theme,
            request.playing,
            request.paused,
            request.volume,
        )
    };
    surface.validate()?;
    let mut response = Response {
        surface: None,
        action: None,
        value: None,
    };
    if let Some([x, y]) = request.pointer {
        if let Some(hit) = surface.hit(x, y).filter(|h| {
            h.action.starts_with("track:")
                || matches!(
                    h.action.as_str(),
                    "audio_tracks" | "subtitle_tracks" | "fullscreen" | "picker_close"
                )
        }) {
            response.action = Some(hit.action.clone());
            return Ok(response);
        }
        response.action = match native::hit_test(&surface, x, y, 0.0) {
            Some(HitTarget::Previous) => Some("previous"),
            Some(HitTarget::Play) => Some("play"),
            Some(HitTarget::Pause) => Some("pause"),
            Some(HitTarget::Stop) => Some("stop"),
            Some(HitTarget::Next) => Some("next"),
            Some(HitTarget::Volume(value)) => {
                response.value = Some(value.clamp(0.0, 1.0));
                Some("volume")
            }
            _ => None,
        }
        .map(str::to_owned);
    } else {
        response.surface = Some(surface);
    }
    Ok(response)
}

pub fn run() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    loop {
        let mut line = Vec::new();
        if input.by_ref().take(65537).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        ensure!(
            line.len() <= 65536 && line.last() == Some(&b'\n'),
            "Transport request exceeds limit"
        );
        let response = respond(serde_json::from_slice(&line)?)?;
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            width: 600,
            height: 48,
            theme: Palette {
                bg: [24; 3],
                fg: [230; 3],
                muted: [100; 3],
                accent: [140; 3],
                selected: [40; 3],
                border: [60; 3],
                error: [200; 3],
            },
            playing: true,
            paused: false,
            volume: 0.8,
            pointer: None,
            movie: false,
            picker: false,
            entries: vec![],
            selected: 0,
            title: String::new(),
            font: 20,
        }
    }
    #[test]
    fn controls_use_player_artwork_and_hit_testing() {
        let surface = respond(request()).unwrap().surface.unwrap();
        for hit in surface.hits.iter().filter(|h| h.action != "volume") {
            let mut request = request();
            request.pointer = Some([
                hit.rect.x + hit.rect.width / 2,
                hit.rect.y + hit.rect.height / 2,
            ]);
            assert_eq!(
                respond(request).unwrap().action.as_deref(),
                Some(hit.action.as_str())
            );
        }
        assert_eq!(surface.hits.len(), 6);
    }
    #[test]
    fn movie_buttons_and_track_rows_accept_mouse_selection() {
        for picker in [false, true] {
            let mut r = request();
            r.movie = !picker;
            r.picker = picker;
            r.height = if picker { 300 } else { 48 };
            r.entries = vec!["Auto".into(), "Off".into(), "English".into()];
            let surface = respond(r).unwrap().surface.unwrap();
            for hit in &surface.hits {
                let mut r = request();
                r.movie = !picker;
                r.picker = picker;
                r.height = if picker { 300 } else { 48 };
                r.entries = vec!["Auto".into(), "Off".into(), "English".into()];
                r.pointer = Some([
                    hit.rect.x + hit.rect.width / 2,
                    hit.rect.y + hit.rect.height / 2,
                ]);
                if hit.action != "volume" {
                    assert_eq!(
                        respond(r).unwrap().action.as_deref(),
                        Some(hit.action.as_str())
                    );
                }
            }
        }
    }
    proptest::proptest! {
        #[test]
        fn bounded_geometry_never_panics(width in 0u16..9000, height in 0u16..160, volume in 0u8..=100) {
            let mut r = request(); r.width = width; r.height = height; r.volume = f32::from(volume) / 100.0;
            let result = respond(r);
            proptest::prop_assert_eq!(result.is_ok(), (1..=8192).contains(&width) && (1..=8192).contains(&height) && u64::from(width)*u64::from(height)<=32_000_000);
        }
    }
    #[test]
    fn rejects_invalid_dimensions_and_volume() {
        let mut r = request();
        r.width = 0;
        assert!(respond(r).is_err());
        let mut r = request();
        r.volume = f32::NAN;
        assert!(respond(r).is_err());
    }
}
