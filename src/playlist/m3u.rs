//! M3U reading and writing.
//!
//! The reference library's playlists live in a directory MPD also reads, so this
//! is a two-way contract rather than an import. Two rules follow from that:
//!
//! **Never drop a line.** 243 of the 13,031 entries do not resolve today. If a
//! playlist is modelled as a list of resolved tracks, those entries evaporate
//! the first time it is written back, permanently damaging files curated since
//! 2022. Unresolvable entries are kept verbatim and written back byte-for-byte.
//!
//! **Never normalise a path.** No case folding, no Unicode normalisation, no
//! percent-encoding. Any of those desynchronises us from MPD's view of the same
//! file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::uri::TrackUri;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub uri: TrackUri,
    /// The line exactly as it appeared, for lossless write-back.
    pub raw_line: String,
    /// From `#EXTINF`, when present.
    pub ext_title: Option<String>,
    pub ext_duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    pub name: String,
    pub source_path: Option<PathBuf>,
    pub items: Vec<PlaylistItem>,
    /// Whether the source carried an `#EXTM3U` header, so write-back can match.
    pub extended: bool,
}

impl Playlist {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

pub fn read_file(path: &Path) -> Result<Playlist> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // Playlists are as likely as cue sheets to be legacy-encoded.
    let (text, _) = crate::cue::parser::decode_bytes(&bytes);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("playlist")
        .to_string();
    let mut pl = parse(&text);
    pl.name = name;
    pl.source_path = Some(path.to_path_buf());
    Ok(pl)
}

pub fn parse(text: &str) -> Playlist {
    let mut pl = Playlist::default();
    let mut pending_title: Option<String> = None;
    let mut pending_duration: Option<i64> = None;

    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if trimmed.eq_ignore_ascii_case("#EXTM3U") {
                pl.extended = true;
            } else if let Some(rest) = trimmed.strip_prefix("#EXTINF:") {
                // `#EXTINF:429,Glory Opera - Endless Sin`
                let (dur, title) = match rest.split_once(',') {
                    Some((d, t)) => (d.trim().parse::<f64>().ok(), Some(t.to_string())),
                    None => (rest.trim().parse::<f64>().ok(), None),
                };
                pending_duration = dur.map(|d| d as i64);
                pending_title = title.filter(|t| !t.is_empty());
            }
            continue;
        }

        pl.items.push(PlaylistItem {
            uri: TrackUri::parse(trimmed),
            raw_line: line.to_string(),
            ext_title: pending_title.take(),
            ext_duration_secs: pending_duration.take(),
        });
    }

    pl
}

/// How to write a playlist back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStyle {
    /// Bare library-relative paths, one per line. What MPD reads and writes.
    MpdCompatible,
    /// `#EXTM3U` with `#EXTINF` metadata.
    Extended,
    /// Whatever the source used.
    Preserve,
}

pub fn write_string(pl: &Playlist, style: WriteStyle) -> String {
    let extended = match style {
        WriteStyle::MpdCompatible => false,
        WriteStyle::Extended => true,
        WriteStyle::Preserve => pl.extended,
    };

    let mut out = String::new();
    if extended {
        out.push_str("#EXTM3U\n");
    }
    for item in &pl.items {
        if extended {
            let dur = item.ext_duration_secs.unwrap_or(-1);
            match &item.ext_title {
                Some(t) => out.push_str(&format!("#EXTINF:{dur},{t}\n")),
                None => out.push_str(&format!("#EXTINF:{dur},\n")),
            }
        }
        // The raw line, not a re-rendered URI: that is what makes write-back
        // lossless for entries we could not resolve.
        out.push_str(&item.raw_line);
        out.push('\n');
    }
    out
}

/// Write a playlist, whole or not at all.
///
/// Through a temporary beside the target and a rename, for the reason the
/// config and the session file are: MPD reads this directory, several windows
/// can reach this at once, and a half-written playlist is worse than none. The
/// temporary carries the pid so two writers cannot interleave into one file.
pub fn write_file(pl: &Playlist, path: &Path, style: WriteStyle) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("m3u");
    let tmp = path.with_extension(format!("{ext}.{}", std::process::id()));
    std::fs::write(&tmp, write_string(pl, style))
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

/// Build a playlist from what the queue is holding.
///
/// The queue is the only record of an edit -- adding from the browser changes
/// it and nothing else -- so this is what turns that into a file. Every line is
/// rendered from its `TrackUri`, which is the m3u form exactly.
pub fn from_uris(name: &str, uris: impl Iterator<Item = TrackUri>) -> Playlist {
    Playlist {
        name: name.to_string(),
        source_path: None,
        items: uris
            .map(|uri| PlaylistItem {
                raw_line: uri.to_string(),
                uri,
                ext_title: None,
                ext_duration_secs: None,
            })
            .collect(),
        extended: false,
    }
}

/// Every `.m3u`/`.m3u8` in a directory.
pub fn list_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .flatten()
    {
        let p = entry.path();
        let is_m3u = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8"))
            .unwrap_or(false);
        if is_m3u {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bare_mpd_style_paths() {
        let pl = parse(
            "Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue/track0001\n\
             Orden Ogan/Final Days/01 - December.flac\n",
        );
        assert_eq!(pl.len(), 2);
        assert!(pl.items[0].uri.is_cue());
        assert!(!pl.items[1].uri.is_cue());
        assert!(!pl.extended);
    }

    #[test]
    fn reads_extended_m3u_with_extinf() {
        let pl = parse(
            "#EXTM3U\n\
             #EXTINF:144,Glory Opera - Boto (Intro)\n\
             Glory Opera - Boto (Intro).mp3\n\
             #EXTINF:429,Glory Opera - Endless Sin\n\
             Glory Opera - Endless Sin.mp3\n",
        );
        assert!(pl.extended);
        assert_eq!(pl.len(), 2);
        assert_eq!(pl.items[0].ext_duration_secs, Some(144));
        assert_eq!(
            pl.items[0].ext_title.as_deref(),
            Some("Glory Opera - Boto (Intro)")
        );
    }

    #[test]
    fn round_trips_a_bare_playlist_byte_for_byte() {
        let src = "Running Wild/1995 - Masquerade/rip.cue/track0001\n\
                   Running Wild/1995 - Masquerade/rip.cue/track0002\n\
                   Orden Ogan/Final Days/01 - December.flac\n";
        let pl = parse(src);
        assert_eq!(write_string(&pl, WriteStyle::MpdCompatible), src);
        assert_eq!(write_string(&pl, WriteStyle::Preserve), src);
    }

    #[test]
    fn keeps_entries_that_do_not_resolve() {
        // 243 such entries exist in the reference library. Losing them on
        // write-back would damage files curated since 2022.
        let src = "Gone/Deleted Album/01.flac\nStill Here/01.flac\n";
        let pl = parse(src);
        assert_eq!(pl.len(), 2);
        assert_eq!(write_string(&pl, WriteStyle::MpdCompatible), src);
    }

    #[test]
    fn preserves_unusual_bytes_in_paths() {
        let src = "Crystallion/2009 - Hattïn/01 - Hattïn.flac\n\
                   Хаос/Альбом/01.flac\n\
                   COMMANDMENT [1999] [CD] [EAC-WV]/x.cue/track0004\n";
        let pl = parse(src);
        assert_eq!(write_string(&pl, WriteStyle::MpdCompatible), src);
    }

    #[test]
    fn a_playlist_built_from_a_queue_writes_the_lines_mpd_reads() {
        let uris = [
            "Orden Ogan/Final Days/01 - December.flac",
            "Running Wild/1995 - Masquerade/rip.cue/track0001",
        ];
        let pl = from_uris("new", uris.iter().map(|u| TrackUri::parse(u)));
        assert_eq!(
            write_string(&pl, WriteStyle::MpdCompatible),
            format!("{}\n{}\n", uris[0], uris[1])
        );
    }

    #[test]
    fn a_playlist_is_replaced_whole_or_not_at_all() {
        // Written through a temporary and renamed, so a reader -- MPD, or
        // another window -- never sees half a file. Nothing of ours may be
        // left behind either.
        let dir = std::env::temp_dir().join(format!("staramp-m3u-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.m3u");
        let pl = from_uris("test", ["A/b.flac"].iter().map(|u| TrackUri::parse(u)));
        write_file(&pl, &path, WriteStyle::MpdCompatible).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "A/b.flac\n");

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "test.m3u")
            .collect();
        assert!(leftovers.is_empty(), "temporary left behind: {leftovers:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn blank_lines_and_comments_do_not_become_entries() {
        let pl = parse("#EXTM3U\n\n# a comment\nA/b.flac\n\n");
        assert_eq!(pl.len(), 1);
    }
}
