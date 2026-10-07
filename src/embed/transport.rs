//! Render-only transport protocol. No player, output device or startup writes.
use std::io::{BufRead, Read, Write};

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

use super::{native, render::HitTarget, Palette};

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    cells: Option<[u16; 2]>,
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
struct CellTransport {
    columns: u16,
    rows: u16,
    cells: Vec<super::Cell>,
}
#[derive(Serialize)]
struct Response {
    surface: Option<starkit::native_surface::Surface>,
    cells: Option<CellTransport>,
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
        cells: None,
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
        if let Some([columns, rows]) = request.cells {
            response.cells = Some(cell_transport(&surface, columns, rows, &request.theme)?);
        }
        response.surface = Some(surface);
    }
    Ok(response)
}

/// Cell artwork uses the same transport surface and hit regions as native buttons.
fn cell_transport(
    surface: &starkit::native_surface::Surface,
    columns: u16,
    rows: u16,
    palette: &Palette,
) -> Result<CellTransport> {
    use starkit::{
        native_surface::Primitive,
        ratatui::{
            buffer::Buffer,
            layout::Rect,
            style::{Color, Modifier, Style},
        },
    };
    ensure!(
        columns > 0 && rows > 0 && usize::from(columns) * usize::from(rows) <= 512,
        "Transport cell grid exceeds limits"
    );
    let area = Rect::new(0, 0, columns, rows);
    let mut buffer = Buffer::empty(area);
    buffer.set_style(
        area,
        Style::default()
            .fg(Color::Rgb(palette.fg[0], palette.fg[1], palette.fg[2]))
            .bg(Color::Rgb(palette.bg[0], palette.bg[1], palette.bg[2])),
    );
    let color = |s: &str| {
        let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
    };
    for node in &surface.nodes {
        let (rect, text, foreground, bold) = match node {
            Primitive::Text {
                rect,
                text,
                color,
                bold,
                ..
            } => (*rect, text.as_str(), color.as_str(), *bold),
            Primitive::Icon { rect, name, color } => (
                *rect,
                match name.as_str() {
                    "media-rewind" | "previous" => "<<",
                    "media-forward" | "next" => ">>",
                    "media-play" => ">",
                    "media-pause" => "||",
                    "media-stop" => "[]",
                    "speaker" => "vol",
                    "fullscreen" => "full",
                    "audio_tracks" => "audio",
                    "subtitle_tracks" => "subs",
                    _ => "",
                },
                color.as_str(),
                false,
            ),
            _ => continue,
        };
        let x = (u32::from(rect.x) * u32::from(columns) / u32::from(surface.width)) as u16;
        let y = ((u32::from(rect.y) + u32::from(rect.height) / 2) * u32::from(rows)
            / u32::from(surface.height)) as u16;
        if x < columns && y < rows {
            buffer.set_stringn(
                x,
                y,
                text,
                usize::from(columns - x),
                Style::default()
                    .fg(color(foreground))
                    .add_modifier(if bold {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            );
        }
    }
    Ok(CellTransport {
        columns,
        rows,
        cells: buffer
            .content
            .iter()
            .map(|c| {
                let rgb = |c: Color, default| {
                    if let Color::Rgb(r, g, b) = c {
                        [r, g, b]
                    } else {
                        default
                    }
                };
                super::Cell {
                    symbol: c.symbol().into(),
                    fg: rgb(c.fg, palette.fg),
                    bg: rgb(c.bg, palette.bg),
                    modifiers: c.modifier.bits(),
                }
            })
            .collect(),
    })
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
            cells: None,
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
    fn cell_controls_reuse_the_native_buttons_and_pointer_actions() {
        let mut r = request();
        r.movie = true;
        r.cells = Some([80, 2]);
        let response = respond(r).unwrap();
        let cells = response.cells.unwrap();
        assert_eq!(cells.cells.len(), 160);
        assert!(cells.cells.iter().any(|c| c.symbol == "|"));
        let surface = response.surface.unwrap();
        for hit in &surface.hits {
            if hit.action == "volume" {
                continue;
            }
            let mut r = request();
            r.movie = true;
            r.cells = Some([80, 2]);
            r.pointer = Some([
                hit.rect.x + hit.rect.width / 2,
                hit.rect.y + hit.rect.height / 2,
            ]);
            assert_eq!(
                respond(r).unwrap().action.as_deref(),
                Some(hit.action.as_str())
            );
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
    #[test]
    fn movie_transport_has_one_play_pause_toggle_and_seek_controls() {
        for paused in [false, true] {
            let mut r = request();
            r.movie = true;
            r.width = 1200;
            r.paused = paused;
            let surface = respond(r).unwrap().surface.unwrap();
            let actions: Vec<_> = surface.hits.iter().map(|h| h.action.as_str()).collect();
            assert!(actions.contains(&if paused { "play" } else { "pause" }));
            assert!(!actions.contains(&if paused { "pause" } else { "play" }));
            assert!(
                actions.contains(&"previous")
                    && actions.contains(&"next")
                    && actions.contains(&"stop")
            );
        }
    }
    #[test]
    fn volume_slider_has_space_before_audio_picker() {
        let mut r = request();
        r.movie = true;
        r.width = 1200;
        let surface = respond(r).unwrap().surface.unwrap();
        let volume = surface
            .hits
            .iter()
            .find(|h| h.action == "volume")
            .unwrap()
            .rect;
        let audio = surface
            .hits
            .iter()
            .find(|h| h.action == "audio_tracks")
            .unwrap()
            .rect;
        assert!(audio.x >= volume.x + volume.width + 24);
    }
    proptest::proptest! {
        #[test]
        fn bounded_geometry_never_panics(width in 0u16..9000, height in 0u16..160, volume in 0u8..=100) {
            let mut r = request(); r.width = width; r.height = height; r.volume = f32::from(volume) / 100.0;
            let result = respond(r);
            proptest::prop_assert_eq!(result.is_ok(), (1..=8192).contains(&width) && (1..=8192).contains(&height) && u64::from(width)*u64::from(height)<=32_000_000);
        }
    }
    proptest::proptest! {
        #[test]
        fn movie_controls_stay_inside_compact_and_fullscreen_surfaces(width in 1u16..8193, height in 1u16..160) {
            let mut r = request(); r.width = width; r.height = height; r.movie = true;
            proptest::prop_assert!(respond(r).is_ok());
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
