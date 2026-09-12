//! Track URIs, in MPD's on-disk playlist form.
//!
//! MPD addresses a track inside a CUE sheet as the cue file's path with a
//! synthetic component appended:
//!
//! ```text
//! Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue/track0001
//! └──────────────── cue path, library-root-relative ────────────┘ └──────┘
//!                                                        1-based ordinal
//! ```
//!
//! Three details are load-bearing, and all three are easy to get wrong:
//!
//! 1. **The ordinal is positional**, assigned by parse order over the flattened
//!    track list — not the `TRACK` number written in the sheet, and not per-FILE.
//!    For a multi-FILE cue the ordinal continues across `FILE` stanzas.
//! 2. **Paths are preserved byte-for-byte.** No percent-encoding, no scheme, no
//!    case folding, no Unicode normalisation. Normalising on write-back would
//!    silently desynchronise the user's playlists from MPD's own view of them.
//! 3. **Parsing accepts `\d{4,}`** even though writing always uses `%04d`, so a
//!    library large enough to need five digits still round-trips.

use std::fmt;

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum TrackUri {
    /// An ordinary audio file, library-root-relative.
    File { rel_path: String },
    /// One virtual track inside a CUE sheet.
    CueTrack { cue_rel_path: String, ordinal: u32 },
}

impl TrackUri {
    /// Parse a playlist line.
    ///
    /// Never fails: anything that is not recognisably a cue reference is a plain
    /// file path. Playlist lines that do not resolve are kept as-is elsewhere in
    /// the pipeline rather than being rejected here.
    pub fn parse(s: &str) -> Self {
        if let Some((head, tail)) = s.rsplit_once('/') {
            if let Some(ordinal) = parse_track_component(tail) {
                // Only a `.cue` parent makes this a cue reference. A real
                // directory called `track0001` under a normal album would
                // otherwise be misread as a virtual track.
                if has_cue_extension(head) {
                    return TrackUri::CueTrack {
                        cue_rel_path: head.to_string(),
                        ordinal,
                    };
                }
            }
        }
        TrackUri::File {
            rel_path: s.to_string(),
        }
    }

    /// The cue sheet this URI lives in, if any.
    pub fn cue_path(&self) -> Option<&str> {
        match self {
            TrackUri::CueTrack { cue_rel_path, .. } => Some(cue_rel_path),
            TrackUri::File { .. } => None,
        }
    }

    /// The path of the file that must exist on disk for this URI to resolve —
    /// the cue sheet itself for a virtual track.
    pub fn backing_path(&self) -> &str {
        match self {
            TrackUri::File { rel_path } => rel_path,
            TrackUri::CueTrack { cue_rel_path, .. } => cue_rel_path,
        }
    }

    pub fn is_cue(&self) -> bool {
        matches!(self, TrackUri::CueTrack { .. })
    }

    /// Does this URI name a file inside the library, rather than somewhere
    /// else on the machine?
    ///
    /// A library URI is relative and goes downward. Playlist lines are not
    /// written by staramp -- an `.m3u` is a text file anyone can edit and
    /// anyone can send you -- and neither is the index a remote library
    /// serves, so `/etc/shadow` and `../../../../etc/shadow` both have to be
    /// recognised as what they are before anything opens them.
    ///
    /// `parse` deliberately still accepts them: a line that does not resolve
    /// is preserved rather than dropped, so that saving a playlist cannot
    /// silently delete somebody's entry. Refusing happens where the bytes
    /// would actually be read.
    pub fn is_library_relative(&self) -> bool {
        is_library_relative(self.backing_path())
    }
}

/// The path rule behind [`TrackUri::is_library_relative`].
///
/// Checked on the string rather than on a `Path`, because that is the form the
/// index and the playlist both store, and because `Path::components` quietly
/// normalises away the `.` that a Windows-authored playlist can contain.
pub fn is_library_relative(path: &str) -> bool {
    use std::path::{Component, Path};
    let p = Path::new(path);
    !p.is_absolute()
        && !path.is_empty()
        // A backslash is a separator on the platform that wrote the playlist
        // even where it is a legal filename byte here, so `..\..\x` has to be
        // read the way its author meant it.
        && !path.split(['/', '\\']).any(|part| part == "..")
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

impl fmt::Display for TrackUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrackUri::File { rel_path } => f.write_str(rel_path),
            TrackUri::CueTrack {
                cue_rel_path,
                ordinal,
            } => write!(f, "{cue_rel_path}/track{ordinal:04}"),
        }
    }
}

/// `track0007` -> `Some(7)`. Requires at least four digits and nothing else.
fn parse_track_component(s: &str) -> Option<u32> {
    let digits = s.strip_prefix("track")?;
    if digits.len() < 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn has_cue_extension(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.eq_ignore_ascii_case("cue"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_real_mpd_cue_entry() {
        let s = "Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue/track0001";
        assert_eq!(
            TrackUri::parse(s),
            TrackUri::CueTrack {
                cue_rel_path: "Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue".into(),
                ordinal: 1,
            }
        );
    }

    #[test]
    fn parses_a_plain_relative_path() {
        let s = "Orden Ogan/Final Days (2022) FLAC/01 - December.flac";
        assert_eq!(TrackUri::parse(s), TrackUri::File { rel_path: s.into() });
    }

    #[test]
    fn round_trips_both_forms() {
        for s in [
            "Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue/track0001",
            "Running Wild/1995 - Masquerade/Running Wild - Masquerade.cue/track0013",
            "Orden Ogan/Final Days (2022) FLAC/01 - December.flac",
            "Weird Band/album.cue/track0099",
        ] {
            assert_eq!(
                TrackUri::parse(s).to_string(),
                s,
                "round-trip failed for {s}"
            );
        }
    }

    #[test]
    fn accepts_more_than_four_digits() {
        let s = "x/y.cue/track12345";
        assert_eq!(
            TrackUri::parse(s),
            TrackUri::CueTrack {
                cue_rel_path: "x/y.cue".into(),
                ordinal: 12345
            }
        );
        // Writing is %04d, so a 5-digit ordinal still renders as 5 digits.
        assert_eq!(TrackUri::parse(s).to_string(), s);
    }

    #[test]
    fn a_directory_named_like_a_track_is_not_a_cue_reference() {
        // Without the .cue check this would be misread as a virtual track.
        let s = "Some Artist/Some Album/track0001";
        assert_eq!(TrackUri::parse(s), TrackUri::File { rel_path: s.into() });
    }

    #[test]
    fn rejects_short_or_non_numeric_track_components() {
        for s in [
            "a/b.cue/track001",  // three digits
            "a/b.cue/track",     // no digits
            "a/b.cue/track00x1", // not all digits
            "a/b.cue/trak0001",  // wrong prefix
        ] {
            assert!(
                !TrackUri::parse(s).is_cue(),
                "{s} should not parse as a cue track"
            );
            assert_eq!(TrackUri::parse(s).to_string(), s);
        }
    }

    #[test]
    fn cue_extension_match_is_case_insensitive() {
        assert!(TrackUri::parse("a/B.CUE/track0001").is_cue());
        assert!(TrackUri::parse("a/b.Cue/track0001").is_cue());
    }

    #[test]
    fn preserves_bytes_exactly() {
        // Unicode, spaces, brackets, and mixed case must survive untouched;
        // normalising any of these desynchronises us from MPD.
        for s in [
            "Crystallion/2009 - Hattïn/01 - Hattïn.flac",
            "COMMANDMENT [1999] [CD] Engraved In Stone [EAC-WV]/x.cue/track0004",
            "Хаос/Альбом/01.flac",
        ] {
            assert_eq!(TrackUri::parse(s).to_string(), s);
        }
    }

    /// A playlist is a text file that arrives with an album, and an index can
    /// arrive from another machine. Neither is a statement about what this
    /// program may open.
    #[test]
    fn a_library_uri_goes_downward_and_nowhere_else() {
        for good in [
            "Artist/Album/01 - Track.flac",
            "Album/rip.cue/track0007",
            "./Artist/Album/01.flac",
            "single.flac",
            "Artist/..hidden-but-not-parent/01.flac",
        ] {
            assert!(
                TrackUri::parse(good).is_library_relative(),
                "{good} should be a library track"
            );
        }
        for bad in [
            "/etc/shadow",
            "../../../../etc/shadow",
            "Artist/../../../etc/shadow",
            "..",
            "..\\..\\windows\\win.ini",
            "",
        ] {
            assert!(
                !TrackUri::parse(bad).is_library_relative(),
                "{bad} should not be a library track"
            );
        }
    }

    /// Recognising an escape is not the same as dropping the line: a playlist
    /// entry staramp cannot play still has to survive being saved.
    #[test]
    fn an_escaping_line_still_parses_and_round_trips() {
        let s = "../../etc/passwd";
        let uri = TrackUri::parse(s);
        assert_eq!(uri.to_string(), s);
        assert!(!uri.is_library_relative());
    }
}
