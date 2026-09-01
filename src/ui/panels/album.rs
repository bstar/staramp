//! The album window: the cover, and what the record is.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::library::art::{Album, Source};
use crate::theme::color::Rgb;
use crate::theme::resolve::Theme;
use crate::ui::digits;
use crate::ui::panels::player::truncate;

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Rows the panel occupies: border, header row, and six of body.
///
/// The six is what matters and is why this is nine rather than eight. A cover
/// is as wide as it is tall on screen, so its width follows the body's height
/// -- six rows come out twelve columns. Taking the header out of the body
/// instead would have narrowed the cover to ten, which is the wrong way to pay
/// for a row.
pub const PANEL_ROWS: u16 = super::header::ROWS + 8;

/// Blank columns between the cover and the text.
const GUTTER: u16 = 2;

/// The panel's interactive geometry.
///
/// The same contract as the other panels: one pure function the renderer
/// places things with and the mouse handler tests against.
pub struct Geometry {
    pub inner: Rect,
    /// Where the cover goes, already square on screen. Zero-width when covers
    /// are turned off, in which case the text has the panel to itself.
    pub art: Rect,
    /// The detail lines to its right.
    pub text: Rect,
}

/// The usual shape of a terminal cell, for when nothing has measured it.
pub const CELL_ASPECT: f32 = 2.0;

/// How wide the cover has to be, in columns, to be square on screen.
fn cover_cols(rows: u16, cell_aspect: f32) -> u16 {
    let aspect = if cell_aspect.is_finite() && cell_aspect > 0.5 {
        cell_aspect
    } else {
        CELL_ASPECT
    };
    ((rows as f32 * aspect).round() as u16).max(1)
}

pub fn geometry(area: Rect, with_cover: bool, cell_aspect: f32) -> Option<Geometry> {
    let inner = super::header::body(area);
    // Below this there is no room for a cover and a legible line beside it,
    // and half a panel is worse than a plain frame.
    if inner.height == 0 || inner.width < if with_cover { 16 } else { 8 } {
        return None;
    }
    // As many columns as it takes to be square on *this* terminal. Assuming
    // two leaves the picture letterboxed inside its own rect on any font
    // whose cells are not exactly half as wide as they are tall.
    let art_w = if with_cover {
        cover_cols(inner.height, cell_aspect).min(inner.width / 2)
    } else {
        0
    };
    let art = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: art_w,
        height: inner.height,
    };
    // With no cover the gutter goes too, so the text starts one column in
    // like every other panel's does.
    let text_x = art.x + art.width + if with_cover { GUTTER } else { 0 };
    Some(Geometry {
        inner,
        art,
        text: Rect {
            x: text_x,
            y: inner.y,
            width: (inner.x + inner.width).saturating_sub(text_x),
            height: inner.height,
        },
    })
}

/// What a line of the detail block is, so the renderer can style it and the
/// mouse handler can find the one that matters without either guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Title,
    Artist,
    Detail,
    /// Where the cover came from. The last line, and the only clickable one.
    Provenance,
}

/// The clickable word offering another look. Short, because it sits at the end
/// of a line that is already carrying a filename.
pub const RETRY: &str = "retry";

/// How the retry word is drawn.
///
/// A lookup can take seconds -- a search, then an image, over a service that
/// has its own opinions about how busy it is -- and a word that does nothing
/// visible when clicked reads as a word that did nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Retry {
    /// Offered, waiting to be clicked.
    Idle,
    /// A lookup is in flight. The sweep is at this phase, 0 to 1.
    Working(f32),
    /// In flight, but animations are switched off.
    Waiting,
}

/// Seconds for the highlight to cross the word once.
///
/// Faster than the seek bar's sheen, which is ambient. This one is saying that
/// something is happening now.
pub const SWEEP_PERIOD: f32 = 1.1;

/// How much of the word the highlight covers, as a fraction of it.
const SWEEP_WIDTH: f32 = 0.45;

/// How brightly the travelling highlight lands at `along`, 0 to 1.
///
/// The same shape as the seek bar's sheen: it enters and leaves cleanly rather
/// than appearing at one edge, and falls away squared so it reads as light
/// crossing the letters rather than a block sliding over them.
fn sweep(along: f32, phase: f32) -> f32 {
    let centre = phase * (1.0 + SWEEP_WIDTH * 2.0) - SWEEP_WIDTH;
    let d = (along - centre).abs() / SWEEP_WIDTH;
    if d >= 1.0 {
        return 0.0;
    }
    let f = 1.0 - d;
    f * f
}

/// The detail lines, in order, with the provenance always last.
///
/// Pure and shared: the renderer draws these and the mouse handler counts them
/// to work out where the retry word landed. Two separate computations of the
/// same layout would drift, and a click that misses what it is pointing at is
/// worse than no click at all.
pub fn lines(
    album: Option<&Album>,
    fallback_album: Option<&str>,
    fallback_artist: Option<&str>,
) -> Vec<(String, Kind)> {
    let detail = album.and_then(|a| a.detail.as_ref());
    let mut out = Vec::with_capacity(4);

    let title = detail
        .and_then(|d| d.album.as_deref())
        .or(fallback_album)
        .unwrap_or("unknown album");
    out.push((title.to_string(), Kind::Title));

    let artist = detail
        .and_then(|d| d.artist.as_deref())
        .or(fallback_artist)
        .unwrap_or("");
    let second = match detail.and_then(|d| d.year) {
        Some(y) if !artist.is_empty() => format!("{artist} \u{b7} {y}"),
        Some(y) => y.to_string(),
        None => artist.to_string(),
    };
    if !second.is_empty() {
        out.push((second, Kind::Artist));
    }

    if let Some(d) = detail {
        let mut parts = vec![format!(
            "{} track{}",
            d.track_count,
            if d.track_count == 1 { "" } else { "s" }
        )];
        if d.total_ms > 0 {
            parts.push(digits::clock(d.total_ms as f64 / 1000.0));
        }
        if let Some(c) = d.codec.as_deref() {
            parts.push(c.to_ascii_uppercase());
        }
        out.push((parts.join(" \u{b7} "), Kind::Detail));
    }

    out.push((provenance(album), Kind::Provenance));
    out
}

/// Where the cover came from, and which file it is.
///
/// A cover that is wrong is far easier to fix when the panel names the
/// offender than when it merely shows it.
fn provenance(album: Option<&Album>) -> String {
    let text = match album {
        Some(a) => match a.source {
            // A fetched cover is cached under a hash, and a hash tells nobody
            // anything; only a name the user could go and look at is worth it.
            Some(s @ (Source::Remote | Source::Original)) => s.name().to_string(),
            Some(s) => match a.art.as_deref().and_then(file_name) {
                Some(f) => format!("{} \u{b7} {f}", s.name()),
                None => s.name().to_string(),
            },
            None if a.detail.is_some() => "no cover found".into(),
            None => "not in the library index".into(),
        },
        None => "looking\u{2026}".into(),
    };
    // An alternative nobody knows about might as well not exist, so the count
    // is what makes the cover clickable rather than just decorative.
    match album.map(|a| a.choices).unwrap_or(0) {
        n if n > 1 => format!("{text}  {}/{n}", album.map(|a| a.choice + 1).unwrap_or(1)),
        _ => text,
    }
}

/// Is another look worth offering for this album?
///
/// Only when the album is known and has no cover: retrying something that
/// already worked, or that was never in the index to begin with, achieves
/// nothing.
pub fn can_retry(album: Option<&Album>) -> bool {
    album.is_some_and(|a| a.source.is_none() && a.detail.is_some())
}

/// Where the clickable retry word sits, if it is being offered.
///
/// Derived from the same layout the renderer uses, so the word and its hit box
/// cannot disagree.
pub fn retry_rect(
    area: Rect,
    show_cover: bool,
    cell_aspect: f32,
    album: Option<&Album>,
    fallback_album: Option<&str>,
    fallback_artist: Option<&str>,
) -> Option<Rect> {
    if !can_retry(album) {
        return None;
    }
    let g = geometry(area, show_cover, cell_aspect)?;
    let rows = lines(album, fallback_album, fallback_artist);
    let (x, y) = place(&g, &rows)?;
    Some(Rect {
        x,
        y,
        width: RETRY.chars().count() as u16,
        height: 1,
    })
}

/// The top-left of the retry word: after the provenance text, on its line.
fn place(g: &Geometry, rows: &[(String, Kind)]) -> Option<(u16, u16)> {
    let w = g.text.width as usize;
    if w == 0 {
        return None;
    }
    // Centred against the cover rather than pinned to the top: with four lines
    // beside six rows of art, hugging the top looks like a mistake.
    let top = g.text.y + (g.text.height.saturating_sub(rows.len() as u16)) / 2;
    let i = rows.iter().position(|(_, k)| *k == Kind::Provenance)?;
    let y = top + i as u16;
    if y >= g.inner.y + g.inner.height {
        return None;
    }
    let text = truncate(&rows[i].0, w);
    // A space, the separator, a space -- the same rhythm as the separators
    // inside the line itself.
    let used = text.chars().count() + 3;
    if used + RETRY.chars().count() > w {
        return None;
    }
    Some((g.text.x + used as u16, y))
}

pub struct AlbumView<'a> {
    pub theme: &'a Theme,
    /// `None` before the worker has answered, which is the normal state for
    /// the first frames after a track change.
    pub album: Option<&'a Album>,
    /// What is playing, so the panel says something useful even when the track
    /// is not in the index.
    pub fallback_album: Option<&'a str>,
    pub fallback_artist: Option<&'a str>,
    /// A real-pixel rendering of the cover, when the terminal can take one.
    /// `None` falls back to half blocks, which every terminal can.
    pub protocol: Option<&'a ratatui_image::protocol::Protocol>,
    /// False for `graphics = "off"`: the details, and no picture at all.
    pub show_cover: bool,
    /// Whether a lookup asked for by hand is in flight, and how far the
    /// highlight has travelled across the word.
    pub retry: Retry,
    /// How tall a cell is relative to its width, so the cover comes out square
    /// on this terminal rather than on an assumed one.
    pub cell_aspect: f32,
    pub focused: bool,
}

impl<'a> Widget for AlbumView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(rgb(if self.focused {
                t.border_focused
            } else {
                t.border
            })))
            .title(Span::styled(
                format!("{}ALBUM ", super::frame::TITLE_LEAD),
                Style::default().fg(rgb(t.header_fg)),
            ))
            .style(Style::default().bg(rgb(t.bg)));

        block.render(area, buf);
        super::frame::render_corners(area, buf, t);
        super::header::render(area, super::header::PLAIN, buf, t);

        let Some(g) = geometry(area, self.show_cover, self.cell_aspect) else {
            return;
        };
        // ---- the cover ----
        match (self.protocol, self.album.and_then(|a| a.image.as_ref())) {
            _ if !self.show_cover => {}
            // Real pixels, where the terminal has them. The widget writes
            // placeholder cells and an escape sequence; the terminal draws over
            // them.
            (Some(p), _) => {
                // Whole cells never divide exactly into a square, so a fitted
                // image can fall a row short. Centre what is left rather than
                // hanging it from the top with the slack all at the bottom.
                let s = p.size();
                let w = s.width.min(g.art.width);
                let h = s.height.min(g.art.height);
                let fitted = Rect {
                    x: g.art.x + (g.art.width - w) / 2,
                    y: g.art.y + (g.art.height - h) / 2,
                    width: w,
                    height: h,
                };
                ratatui_image::Image::new(p).render(fitted, buf)
            }
            (None, Some(img)) => draw_cover(g.art, buf, img, t),
            (None, None) => draw_placeholder(g.art, buf, t),
        }

        // ---- the details ----
        let w = g.text.width as usize;
        if w == 0 {
            return;
        }
        let rows = lines(self.album, self.fallback_album, self.fallback_artist);
        let top = g.text.y + (g.text.height.saturating_sub(rows.len() as u16)) / 2;

        for (i, (line, kind)) in rows.iter().enumerate() {
            let y = top + i as u16;
            if y >= g.inner.y + g.inner.height {
                break;
            }
            let style = match kind {
                Kind::Title => Style::default()
                    .fg(rgb(t.marquee_fg))
                    .add_modifier(Modifier::BOLD),
                Kind::Artist => Style::default().fg(rgb(t.row_fg)),
                Kind::Detail => Style::default().fg(rgb(t.row_meta_fg)),
                Kind::Provenance => Style::default().fg(rgb(t.dim)),
            };
            buf.set_string(g.text.x, y, truncate(line, w), style);
        }

        // The retry, offered where the problem is visible rather than only in
        // the help. Accented so it reads as something to press.
        if let Some(r) = retry_rect(
            area,
            self.show_cover,
            self.cell_aspect,
            self.album,
            self.fallback_album,
            self.fallback_artist,
        ) {
            buf.set_string(r.x - 2, r.y, "\u{b7}", Style::default().fg(rgb(t.dim)));
            match self.retry {
                Retry::Idle => {
                    buf.set_string(r.x, r.y, RETRY, Style::default().fg(rgb(t.accent)));
                }
                // Bold rather than still-and-identical: with motion off the
                // word still has to say it was heard.
                Retry::Waiting => buf.set_string(
                    r.x,
                    r.y,
                    RETRY,
                    Style::default()
                        .fg(rgb(t.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Retry::Working(phase) => {
                    let n = RETRY.chars().count() as f32;
                    for (i, ch) in RETRY.chars().enumerate() {
                        // The middle of each letter, so the highlight sits on
                        // characters rather than between them.
                        let along = (i as f32 + 0.5) / n;
                        let lit = sweep(along, phase) as f64;
                        buf[(r.x + i as u16, r.y)]
                            .set_char(ch)
                            .set_style(Style::default().fg(rgb(t.dim.mix(t.accent, lit))));
                    }
                }
            }
        }
    }
}

fn file_name(p: &std::path::Path) -> Option<&str> {
    p.file_name().and_then(|n| n.to_str())
}

/// Upper half block: two rows of pixels in one cell, the top from the
/// foreground and the bottom from the background.
const HALF: char = '\u{2580}';

/// Draw the cover by sampling it into half-block cells.
///
/// Every cell carries two pixels, so a six-row rect is twelve pixels tall. It
/// is coarse, and it works in every terminal without a graphics protocol,
/// which is what makes it the floor rather than the ceiling.
fn draw_cover(area: Rect, buf: &mut Buffer, img: &image::RgbImage, t: &Theme) {
    if area.width == 0 || area.height == 0 || img.width() == 0 || img.height() == 0 {
        return;
    }
    let (iw, ih) = (img.width(), img.height());
    let rows = area.height as u32 * 2;

    for cy in 0..area.height {
        for cx in 0..area.width {
            // Two samples per cell: the upper and lower halves.
            let sample = |half: u32| {
                let py = cy as u32 * 2 + half;
                let sx = (cx as u32 * iw / area.width as u32).min(iw - 1);
                let sy = (py * ih / rows).min(ih - 1);
                let p = img.get_pixel(sx, sy);
                Color::Rgb(p[0], p[1], p[2])
            };
            buf[(area.x + cx, area.y + cy)]
                .set_char(HALF)
                .set_style(Style::default().fg(sample(0)).bg(sample(1)));
        }
    }
    let _ = t;
}

/// A quiet stand-in when there is no cover, rather than a hole in the panel.
fn draw_placeholder(area: Rect, buf: &mut Buffer, t: &Theme) {
    let style = Style::default().fg(rgb(t.vis_grid_fg)).bg(rgb(t.vis_bg));
    for y in 0..area.height {
        for x in 0..area.width {
            buf[(area.x + x, area.y + y)]
                .set_char('\u{2591}')
                .set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::AlbumDetail;
    use crate::theme::builtin;

    fn detail() -> AlbumDetail {
        AlbumDetail {
            album: Some("Dragonchaser".into()),
            artist: Some("At Vance".into()),
            year: Some(2001),
            codec: Some("flac".into()),
            track_count: 12,
            total_ms: 3_080_000,
            dir_id: 1,
            file_rel: "At Vance/Dragonchaser/01.flac".into(),
            track_title: None,
            track_artist: None,
        }
    }

    fn draw(album: Option<Album>, w: u16, h: u16) -> Vec<String> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        AlbumView {
            theme: &theme,
            album: album.as_ref(),
            fallback_album: None,
            fallback_artist: None,
            protocol: None,
            show_cover: true,
            cell_aspect: CELL_ASPECT,
            retry: Retry::Idle,
            focused: false,
        }
        .render(area, &mut buf);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn album_with(detail: Option<AlbumDetail>, img: Option<image::RgbImage>) -> Album {
        Album {
            uri: "A/B/01.flac".into(),
            detail,
            art: None,
            image: img.map(std::sync::Arc::new),
            source: None,
            choice: 0,
            choices: 1,
            labels: vec!["cover.jpg".into()],
            offers: Vec::new(),
        }
    }

    #[test]
    fn the_cover_is_square_on_the_terminal_it_is_actually_drawn_on() {
        // Assuming a cell is exactly twice as tall as it is wide leaves the
        // picture letterboxed inside its own rect on any font where it is not.
        assert_eq!(cover_cols(6, 2.0), 12);
        assert_eq!(cover_cols(6, 2.4), 14, "a taller cell needs more columns");
        assert_eq!(cover_cols(6, 1.8), 11, "and a squarer one fewer");
        // Nonsense from a terminal that answered badly falls back rather than
        // producing a one-column cover.
        assert_eq!(cover_cols(6, 0.0), 12);
        assert_eq!(cover_cols(6, f32::NAN), 12);
        assert_eq!(cover_cols(6, -3.0), 12);
    }

    #[test]
    fn a_measured_cell_changes_the_cover_s_width() {
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let wide = geometry(area, true, 2.4).unwrap();
        let narrow = geometry(area, true, 2.0).unwrap();
        assert!(wide.art.width > narrow.art.width);
        // And the text still starts after it, whichever it is.
        assert!(wide.text.x >= wide.art.x + wide.art.width);
    }

    #[test]
    fn the_cover_area_is_square_on_screen() {
        // A cell is about twice as tall as it is wide, so a square cover needs
        // twice as many columns as rows.
        let g = geometry(Rect::new(0, 0, 60, PANEL_ROWS), true, CELL_ASPECT).unwrap();
        assert_eq!(g.art.width, g.art.height * 2);
    }

    #[test]
    fn the_text_never_overlaps_the_cover() {
        for w in [18u16, 24, 40, 60, 100] {
            let g = geometry(Rect::new(0, 0, w, PANEL_ROWS), true, CELL_ASPECT).unwrap();
            assert!(
                g.text.x >= g.art.x + g.art.width,
                "width {w}: text starts inside the cover"
            );
            assert!(
                g.text.x + g.text.width <= g.inner.x + g.inner.width,
                "width {w}: text runs past the panel"
            );
        }
    }

    #[test]
    fn a_panel_too_narrow_has_no_geometry() {
        assert!(geometry(Rect::new(0, 0, 12, PANEL_ROWS), true, CELL_ASPECT).is_none());
    }

    #[test]
    fn the_album_and_artist_are_shown() {
        let rows = draw(Some(album_with(Some(detail()), None)), 60, PANEL_ROWS);
        let all = rows.join("\n");
        assert!(all.contains("Dragonchaser"), "{all}");
        assert!(all.contains("At Vance"), "{all}");
        assert!(all.contains("2001"), "{all}");
        assert!(all.contains("12 tracks"), "{all}");
        assert!(all.contains("FLAC"), "{all}");
    }

    #[test]
    fn the_retry_is_offered_only_when_it_would_do_something() {
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        // A known album with no cover: worth another look.
        let no_cover = album_with(Some(detail()), None);
        assert!(can_retry(Some(&no_cover)));
        assert!(retry_rect(area, true, CELL_ASPECT, Some(&no_cover), None, None).is_some());

        // One that already has a cover, and one that is not in the index at
        // all. Retrying either achieves nothing.
        let mut has_cover = album_with(Some(detail()), Some(image::RgbImage::new(2, 2)));
        has_cover.source = Some(Source::Sidecar);
        assert!(!can_retry(Some(&has_cover)));
        assert!(!can_retry(Some(&album_with(None, None))));
        assert!(!can_retry(None), "nothing resolved yet is not a miss");
    }

    #[test]
    fn the_retry_reads_as_part_of_the_line() {
        let rows = draw(Some(album_with(Some(detail()), None)), 60, PANEL_ROWS);
        let joined = rows.join("\n");
        assert!(
            joined.contains("no cover found \u{b7} retry"),
            "the separator needs a space either side: {joined}"
        );
    }

    /// Render the panel with the retry word in a given state and return the
    /// colour of each of its letters.
    fn retry_colours(retry: Retry) -> Vec<Color> {
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        let album = album_with(Some(detail()), None);
        AlbumView {
            theme: &theme,
            album: Some(&album),
            fallback_album: None,
            fallback_artist: None,
            protocol: None,
            show_cover: true,
            cell_aspect: CELL_ASPECT,
            retry,
            focused: false,
        }
        .render(area, &mut buf);
        let r = retry_rect(area, true, CELL_ASPECT, Some(&album), None, None).unwrap();
        (0..r.width)
            .map(|i| buf[(r.x + i, r.y)].style().fg.unwrap())
            .collect()
    }

    #[test]
    fn a_lookup_in_flight_lights_the_word_unevenly() {
        // The point of the sweep: at any instant some letters are brighter
        // than others, which is what reads as movement across frames.
        let idle = retry_colours(Retry::Idle);
        assert!(
            idle.iter().all(|c| *c == idle[0]),
            "the resting word should be one colour: {idle:?}"
        );

        let mid = retry_colours(Retry::Working(0.5));
        assert!(
            mid.iter().any(|c| *c != mid[0]),
            "a working word should not be flat: {mid:?}"
        );
    }

    #[test]
    fn the_highlight_travels_across_the_word() {
        // Early in the sweep the leading letters are the bright ones; later,
        // the trailing ones. Without that it is a flash, not a sweep.
        let brightness = |c: Color| match c {
            Color::Rgb(r, g, b) => r as u32 + g as u32 + b as u32,
            _ => 0,
        };
        let early = retry_colours(Retry::Working(0.15));
        let late = retry_colours(Retry::Working(0.85));
        let n = early.len();
        let first = |v: &Vec<Color>| brightness(v[0]);
        let last = |v: &Vec<Color>| brightness(v[n - 1]);
        assert!(
            first(&early) > last(&early),
            "early in the sweep the front should lead: {early:?}"
        );
        assert!(
            last(&late) > first(&late),
            "late in the sweep the back should lead: {late:?}"
        );
    }

    #[test]
    fn with_motion_off_the_word_still_says_it_was_heard() {
        // Bold rather than animated, and never the resting appearance -- a
        // click that changes nothing reads as a click that did nothing.
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let album = album_with(Some(detail()), None);
        let modifier = |retry| {
            let mut buf = Buffer::empty(area);
            AlbumView {
                theme: &theme,
                album: Some(&album),
                fallback_album: None,
                fallback_artist: None,
                protocol: None,
                show_cover: true,
                cell_aspect: CELL_ASPECT,
                retry,
                focused: false,
            }
            .render(area, &mut buf);
            let r = retry_rect(area, true, CELL_ASPECT, Some(&album), None, None).unwrap();
            buf[(r.x, r.y)].style().add_modifier
        };
        assert_ne!(modifier(Retry::Waiting), modifier(Retry::Idle));
    }

    #[test]
    fn the_sweep_enters_and_leaves_rather_than_appearing() {
        // At both ends of the cycle the word is dark, so the highlight arrives
        // from off one edge and departs off the other.
        assert_eq!(sweep(0.5, 0.0), 0.0, "lit before it has entered");
        assert_eq!(sweep(0.5, 1.0), 0.0, "still lit after it has left");
        assert!(sweep(0.5, 0.5) > 0.9, "should peak as it passes the middle");
        // And it never reports more than the full range.
        for p in 0..=20 {
            for a in 0..=10 {
                let v = sweep(a as f32 / 10.0, p as f32 / 20.0);
                assert!((0.0..=1.0).contains(&v), "sweep out of range: {v}");
            }
        }
    }

    #[test]
    fn the_retry_word_is_where_it_is_drawn() {
        // The click has to land on the word. Both come from the same layout,
        // and this is what proves they have not drifted apart.
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let album = album_with(Some(detail()), None);
        let r = retry_rect(area, true, CELL_ASPECT, Some(&album), None, None).expect("offered");
        let rows = draw(Some(album), 60, PANEL_ROWS);
        let line: String = rows[r.y as usize].chars().collect();
        let at: String = line
            .chars()
            .skip(r.x as usize)
            .take(RETRY.chars().count())
            .collect();
        assert_eq!(at, RETRY, "the hit box does not sit on the word: {line:?}");
    }

    #[test]
    fn the_retry_is_dropped_rather_than_overflowing_a_narrow_panel() {
        let album = album_with(Some(detail()), None);
        // Wide enough for the text and not for the word after it.
        let narrow = Rect::new(0, 0, 34, PANEL_ROWS);
        let r = retry_rect(narrow, true, CELL_ASPECT, Some(&album), None, None);
        if let Some(r) = r {
            let g = geometry(narrow, true, CELL_ASPECT).unwrap();
            assert!(
                r.x + r.width <= g.inner.x + g.inner.width,
                "the retry ran past the panel"
            );
        }
    }

    #[test]
    fn a_track_outside_the_index_still_draws() {
        // A playlist can name a file the scan has never seen; the panel must
        // not go blank over it.
        let rows = draw(Some(album_with(None, None)), 60, PANEL_ROWS);
        let all = rows.join("\n");
        assert!(all.contains("not in the library index"), "{all}");
    }

    #[test]
    fn nothing_at_all_still_draws_a_frame() {
        let rows = draw(None, 60, PANEL_ROWS);
        assert!(rows[0].contains("ALBUM"), "{rows:?}");
        assert!(rows.join("\n").contains("looking"), "{rows:?}");
    }

    #[test]
    fn a_cover_fills_its_rect_with_half_blocks() {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 40, 40]));
        let rows = draw(Some(album_with(Some(detail()), Some(img))), 60, PANEL_ROWS);
        let g = geometry(Rect::new(0, 0, 60, PANEL_ROWS), true, CELL_ASPECT).unwrap();
        for y in g.art.y..g.art.y + g.art.height {
            let line: Vec<char> = rows[y as usize].chars().collect();
            for x in g.art.x..g.art.x + g.art.width {
                assert_eq!(
                    line[x as usize], HALF,
                    "cover cell {x},{y} is not a half block"
                );
            }
        }
    }

    #[test]
    fn a_cover_reproduces_its_colours() {
        // Two solid halves: the top of the image must reach the top of the
        // rect, and sampling must not smear one into the other.
        let mut img = image::RgbImage::new(4, 4);
        for (_, y, p) in img.enumerate_pixels_mut() {
            *p = if y < 2 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            };
        }
        let theme = builtin::load("cosmic").unwrap();
        let area = Rect::new(0, 0, 60, PANEL_ROWS);
        let mut buf = Buffer::empty(area);
        AlbumView {
            theme: &theme,
            album: Some(&album_with(Some(detail()), Some(img))),
            fallback_album: None,
            fallback_artist: None,
            protocol: None,
            show_cover: true,
            cell_aspect: CELL_ASPECT,
            retry: Retry::Idle,
            focused: false,
        }
        .render(area, &mut buf);
        let g = geometry(area, true, CELL_ASPECT).unwrap();
        let top = buf[(g.art.x, g.art.y)].style().fg.unwrap();
        let bottom = buf[(g.art.x, g.art.y + g.art.height - 1)]
            .style()
            .bg
            .unwrap();
        assert_eq!(top, Color::Rgb(255, 0, 0), "top of the cover is wrong");
        assert_eq!(
            bottom,
            Color::Rgb(0, 0, 255),
            "bottom of the cover is wrong"
        );
    }

    #[test]
    fn with_covers_off_the_text_has_the_panel_to_itself() {
        let g = geometry(Rect::new(0, 0, 60, PANEL_ROWS), false, CELL_ASPECT).unwrap();
        assert_eq!(g.art.width, 0, "no space reserved for a picture");
        assert_eq!(g.text.x, g.inner.x + 1);
        assert_eq!(g.text.width, g.inner.width - 1);
    }

    #[test]
    fn a_missing_cover_draws_a_placeholder_not_a_hole() {
        let rows = draw(Some(album_with(Some(detail()), None)), 60, PANEL_ROWS);
        let g = geometry(Rect::new(0, 0, 60, PANEL_ROWS), true, CELL_ASPECT).unwrap();
        let line: Vec<char> = rows[g.art.y as usize].chars().collect();
        assert_eq!(line[g.art.x as usize], '\u{2591}');
    }
}
