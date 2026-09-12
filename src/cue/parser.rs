//! CUE sheet parsing.
//!
//! Tolerant by design. Real sheets in the wild are inconsistent in every way a
//! text format can be, and the correct response to one bad line is a warning,
//! not a rejected album. Specifically handled, because all of these occur in the
//! reference library:
//!
//! - **Not UTF-8.** 37 of 1,123 sheets are CP1251 or CP1252.
//! - **CRLF line endings**, universally.
//! - **Mixed tabs and spaces**, sometimes within a single stanza.
//! - **Inconsistent quoting**: `REM GENRE "Symphonic Metal"` next to
//!   `REM GENRE Synthpop, Pop Rock, New Wave` — unquoted, with commas.
//! - **Multiple `FILE` stanzas** — 119 sheets, ~10% of the total.

use anyhow::{Context, Result};
use std::path::Path;

use super::model::{CueFile, CueSheet, CueTrack, Msf};

/// Decode bytes to text, detecting the encoding.
///
/// Returns the text and the encoding name for diagnostics.
pub fn decode_bytes(bytes: &[u8]) -> (String, String) {
    // A BOM is authoritative when present.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (
            String::from_utf8_lossy(rest).into_owned(),
            "UTF-8 (BOM)".into(),
        );
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return (text.into_owned(), "UTF-16LE".into());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (text, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return (text.into_owned(), "UTF-16BE".into());
    }

    // Valid UTF-8 is overwhelmingly the common case and needs no guessing.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_owned(), "UTF-8".into());
    }

    // Otherwise guess. chardetng is the same detector Firefox uses.
    let mut det = chardetng::EncodingDetector::new();
    det.feed(bytes, true);
    let encoding = det.guess(None, true);
    let (text, _, _) = encoding.decode(bytes);
    (text.into_owned(), encoding.name().to_string())
}

pub fn parse_file(path: &Path) -> Result<CueSheet> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_bytes(&bytes))
}

pub fn parse_bytes(bytes: &[u8]) -> CueSheet {
    let (text, encoding) = decode_bytes(bytes);
    let mut sheet = parse_str(&text);
    sheet.encoding = encoding;
    sheet
}

pub fn parse_str(text: &str) -> CueSheet {
    let mut sheet = CueSheet::default();

    for (lineno, raw) in text.lines().enumerate() {
        // Trim aggressively: leading indentation is arbitrary, and CRLF leaves a
        // trailing \r that would otherwise end up inside the last token.
        let line = raw.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if line.is_empty() {
            continue;
        }

        let tokens = tokenize(line);
        if tokens.is_empty() {
            continue;
        }
        let cmd = tokens[0].to_ascii_uppercase();
        let args = &tokens[1..];

        match cmd.as_str() {
            "FILE" => {
                if args.is_empty() {
                    sheet
                        .warnings
                        .push(format!("line {}: FILE with no name", lineno + 1));
                    continue;
                }
                sheet.files.push(CueFile {
                    name: args[0].clone(),
                    kind: args.get(1).cloned().unwrap_or_else(|| "WAVE".into()),
                    tracks: Vec::new(),
                });
            }
            "TRACK" => {
                let number = args.first().and_then(|s| s.parse::<u32>().ok());
                let Some(number) = number else {
                    sheet
                        .warnings
                        .push(format!("line {}: TRACK with no number", lineno + 1));
                    continue;
                };
                // A TRACK before any FILE is malformed but recoverable: synthesise
                // a nameless FILE so the tracks are not lost.
                if sheet.files.is_empty() {
                    sheet.warnings.push(format!(
                        "line {}: TRACK before any FILE; assuming an implicit one",
                        lineno + 1
                    ));
                    sheet.files.push(CueFile {
                        name: String::new(),
                        kind: "WAVE".into(),
                        tracks: Vec::new(),
                    });
                }
                let file = sheet.files.last_mut().expect("just ensured non-empty");
                file.tracks.push(CueTrack {
                    number,
                    kind: args.get(1).cloned().unwrap_or_else(|| "AUDIO".into()),
                    ..Default::default()
                });
            }
            "INDEX" => {
                let (Some(n), Some(ts)) = (
                    args.first().and_then(|s| s.parse::<u32>().ok()),
                    args.get(1).and_then(|s| parse_msf(s)),
                ) else {
                    sheet
                        .warnings
                        .push(format!("line {}: malformed INDEX", lineno + 1));
                    continue;
                };
                match current_track(&mut sheet) {
                    Some(t) => {
                        t.indices.insert(n, ts);
                    }
                    None => sheet
                        .warnings
                        .push(format!("line {}: INDEX outside a TRACK", lineno + 1)),
                }
            }
            "PREGAP" | "POSTGAP" => {
                let Some(ts) = args.first().and_then(|s| parse_msf(s)) else {
                    sheet
                        .warnings
                        .push(format!("line {}: malformed {cmd}", lineno + 1));
                    continue;
                };
                if let Some(t) = current_track(&mut sheet) {
                    if cmd == "PREGAP" {
                        t.pregap = Some(ts);
                    } else {
                        t.postgap = Some(ts);
                    }
                }
            }
            "TITLE" | "PERFORMER" | "SONGWRITER" => {
                // These appear at both sheet and track level; whichever we are
                // inside decides.
                let value = args.first().cloned().unwrap_or_default();
                if let Some(t) = current_track(&mut sheet) {
                    match cmd.as_str() {
                        "TITLE" => t.title = Some(value),
                        "PERFORMER" => t.performer = Some(value),
                        _ => t.songwriter = Some(value),
                    }
                } else {
                    match cmd.as_str() {
                        "TITLE" => sheet.title = Some(value),
                        "PERFORMER" => sheet.performer = Some(value),
                        _ => sheet.songwriter = Some(value),
                    }
                }
            }
            "ISRC" => {
                let value = args.first().cloned().unwrap_or_default();
                if let Some(t) = current_track(&mut sheet) {
                    t.isrc = Some(value);
                }
            }
            "FLAGS" => {
                if let Some(t) = current_track(&mut sheet) {
                    t.flags = args.to_vec();
                }
            }
            "CATALOG" => sheet.catalog = args.first().cloned(),
            "REM" => parse_rem(&mut sheet, args),
            // CDTEXTFILE and anything else we do not model is not an error.
            _ => {}
        }
    }

    sheet
}

/// `REM` carries the useful non-standard metadata: genre, date, ReplayGain.
fn parse_rem(sheet: &mut CueSheet, args: &[String]) {
    let Some(key) = args.first().map(|s| s.to_ascii_uppercase()) else {
        return;
    };
    // Unquoted values may contain spaces and commas — `REM GENRE Synthpop, Pop
    // Rock, New Wave` is real — so rejoin everything after the key.
    let value = args[1..].join(" ");
    if value.is_empty() {
        return;
    }
    match key.as_str() {
        "GENRE" => sheet.genre = Some(value),
        "DATE" => sheet.date = Some(value),
        "COMMENT" => sheet.comment = Some(value),
        "REPLAYGAIN_ALBUM_GAIN" => {
            sheet.replaygain_album_gain = parse_gain(&value);
        }
        "REPLAYGAIN_ALBUM_PEAK" => {
            sheet.replaygain_album_peak = value.parse().ok();
        }
        _ => {}
    }
}

/// `-7.86 dB` -> `-7.86`.
fn parse_gain(s: &str) -> Option<f32> {
    s.split_whitespace().next()?.parse().ok()
}

fn current_track(sheet: &mut CueSheet) -> Option<&mut CueTrack> {
    sheet.files.last_mut()?.tracks.last_mut()
}

/// `MM:SS:FF`, where FF is CD frames at 75/second.
fn parse_msf(s: &str) -> Option<Msf> {
    let mut parts = s.split(':');
    let m = parts.next()?.trim().parse().ok()?;
    let sec = parts.next()?.trim().parse().ok()?;
    let f = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(Msf::new(m, sec, f))
}

/// Split a line into tokens, honouring double quotes.
///
/// Quoted tokens keep their internal spaces and are emitted without the quotes;
/// unquoted runs split on any whitespace, which is what makes both quoted and
/// bare `REM GENRE` values work.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut have_token = false;

    for c in line.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                have_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if have_token {
                    out.push(std::mem::take(&mut cur));
                    have_token = false;
                }
            }
            c => {
                cur.push(c);
                have_token = true;
            }
        }
    }
    if have_token {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"REM GENRE "Power Metal"
REM DATE 2001
PERFORMER "Angra"
TITLE "Rebirth"
FILE "Angra - Rebirth.flac" WAVE
  TRACK 01 AUDIO
    TITLE "In Excelsis"
    PERFORMER "Angra"
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    TITLE "Nova Era"
    INDEX 00 01:29:60
    INDEX 01 01:32:15
"#;

    #[test]
    fn parses_a_conventional_sheet() {
        let s = parse_str(SIMPLE);
        assert_eq!(s.performer.as_deref(), Some("Angra"));
        assert_eq!(s.title.as_deref(), Some("Rebirth"));
        assert_eq!(s.genre.as_deref(), Some("Power Metal"));
        assert_eq!(s.date.as_deref(), Some("2001"));
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].name, "Angra - Rebirth.flac");
        assert_eq!(s.track_count(), 2);
        assert_eq!(s.files[0].tracks[0].title.as_deref(), Some("In Excelsis"));
        assert_eq!(s.files[0].tracks[1].start(), Some(Msf::new(1, 32, 15)));
        assert_eq!(
            s.files[0].tracks[1].pregap_start(),
            Some(Msf::new(1, 29, 60))
        );
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    }

    #[test]
    fn handles_crlf_and_mixed_indentation() {
        let text = "PERFORMER \"X\"\r\nFILE \"a.flac\" WAVE\r\n\tTRACK 01 AUDIO\r\n        INDEX 01 00:00:00\r\n";
        let s = parse_str(text);
        assert_eq!(s.performer.as_deref(), Some("X"));
        assert_eq!(s.files[0].name, "a.flac");
        assert_eq!(s.track_count(), 1);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    }

    #[test]
    fn accepts_unquoted_rem_values_containing_commas() {
        // Real sheet: `REM GENRE Synthpop, Pop Rock, New Wave, Art Pop`
        let s = parse_str("REM GENRE Synthpop, Pop Rock, New Wave\nFILE \"a.flac\" WAVE\n");
        assert_eq!(s.genre.as_deref(), Some("Synthpop, Pop Rock, New Wave"));
    }

    #[test]
    fn multi_file_sheets_keep_their_stanzas_separate() {
        let text = r#"FILE "disc - 1.wv" WAVE
  TRACK 01 AUDIO
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 05:00:00
FILE "disc - 2.wv" WAVE
  TRACK 03 AUDIO
    INDEX 01 00:00:00
"#;
        let s = parse_str(text);
        assert!(s.is_multi_file());
        assert_eq!(s.files.len(), 2);
        assert_eq!(s.files[0].tracks.len(), 2);
        assert_eq!(s.files[1].tracks.len(), 1);
        assert_eq!(s.track_count(), 3);
    }

    #[test]
    fn one_bad_line_does_not_lose_the_album() {
        let text =
            "FILE \"a.flac\" WAVE\n  TRACK 01 AUDIO\n    INDEX oops\n    INDEX 01 00:00:00\n";
        let s = parse_str(text);
        assert_eq!(s.track_count(), 1);
        assert_eq!(s.files[0].tracks[0].start(), Some(Msf::new(0, 0, 0)));
        assert_eq!(s.warnings.len(), 1);
    }

    #[test]
    fn track_level_title_does_not_overwrite_the_album_title() {
        let s = parse_str(SIMPLE);
        assert_eq!(s.title.as_deref(), Some("Rebirth"));
        assert_eq!(s.files[0].tracks[0].title.as_deref(), Some("In Excelsis"));
    }

    #[test]
    fn decodes_cp1251_without_failing() {
        // "Ария" in CP1251 — String::from_utf8 rejects these bytes outright.
        let mut bytes = b"PERFORMER \"".to_vec();
        bytes.extend_from_slice(&[0xC0, 0xF0, 0xE8, 0xFF]);
        bytes.extend_from_slice(b"\"\nFILE \"a.flac\" WAVE\n");
        let s = parse_bytes(&bytes);
        assert_eq!(s.files.len(), 1);
        let performer = s.performer.expect("performer decoded");
        assert!(!performer.is_empty());
        assert!(
            !performer.contains('\u{fffd}'),
            "got replacement chars: {performer:?}"
        );
    }

    #[test]
    fn decodes_utf8_with_a_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("TITLE \"Hattïn\"\nFILE \"a.flac\" WAVE\n".as_bytes());
        let s = parse_bytes(&bytes);
        assert_eq!(s.title.as_deref(), Some("Hattïn"));
        assert_eq!(s.encoding, "UTF-8 (BOM)");
    }

    #[test]
    fn msf_conversion_uses_the_backing_files_rate() {
        let t = Msf::new(1, 0, 0); // one minute
        assert_eq!(t.to_audio_frames(44_100), 44_100 * 60);
        // The hi-res vinyl rips in the library are cue-split; a hardcoded 44100
        // would misplace every boundary on them.
        assert_eq!(t.to_audio_frames(96_000), 96_000 * 60);
        assert_eq!(t.cd_frames(), 60 * 75);
    }
}
