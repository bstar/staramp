//! Remembering where you were.
//!
//! Written on quit and periodically while playing, so a crash or a killed
//! terminal does not lose the session either. Restoring is offered rather than
//! done automatically: silently resuming into the middle of a track is
//! startling, and there is no way to say no to it after the fact.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Playlist this came from, for the prompt and to reload it.
    pub playlist: Option<PathBuf>,
    pub playlist_name: String,
    /// Position in the queue, and how long the queue was, for the prompt.
    pub index: usize,
    pub total: usize,
    /// Seconds into the track.
    pub position: f64,
    pub duration: f64,
    pub artist: String,
    pub title: String,
    /// The track URI, so a resume can verify it is still the same track.
    pub uri: String,
    pub shuffle: bool,
    pub repeat: String,
    pub volume: f32,
    /// Where the cursor was, as a position in the order rather than a track:
    /// it is where you were looking, not what was playing.
    #[serde(default)]
    pub cursor: usize,
    /// Records folded shut in the playlist, by album title, lower case.
    ///
    /// The title alone, without the artist that only ever exists to separate
    /// two records of the same name inside one queue -- a fold is meant to
    /// survive being carried to a playlist where the namesake is not there.
    ///
    /// `serde(default)` on both, so a session written by an older build still
    /// loads instead of throwing the whole thing away.
    #[serde(default)]
    pub folded: Vec<String>,
    /// Records arranged by hand, by album title, in the order they play.
    /// Empty when the year order stands.
    #[serde(default)]
    pub album_order: Vec<String>,
    pub saved_at: i64,
}

impl Session {
    pub fn path() -> Result<PathBuf> {
        Ok(crate::paths::data_dir()?.join("session.toml"))
    }

    pub fn load() -> Option<Session> {
        let path = Self::path().ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    /// Write it out, through a temporary file in the same directory.
    ///
    /// This is saved every few seconds while playing, so an interrupted write
    /// is a real possibility -- and a half-written session file is worse than
    /// none, because it parses as far as it got and then loses your place.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = path.with_extension(format!("toml.{}", std::process::id()));
        std::fs::write(&tmp, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
    }

    pub fn clear() {
        if let Ok(p) = Self::path() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Is this worth offering to resume?
    ///
    /// A track barely started is not worth interrupting startup for -- you would
    /// have pressed play anyway -- and one essentially finished would resume
    /// into its last second.
    pub fn worth_resuming(&self) -> bool {
        if self.uri.is_empty() || self.total == 0 {
            return false;
        }
        if self.position < 5.0 {
            return false;
        }
        if self.duration > 0.0 && self.position > self.duration - 5.0 {
            return false;
        }
        true
    }

    /// Does the playlist it came from still exist?
    pub fn playlist_available(&self) -> bool {
        match &self.playlist {
            Some(p) => p.is_file(),
            // No playlist means it came from the library, which is always there.
            None => true,
        }
    }

    /// What the view state adds to the prompt, if anything.
    ///
    /// Worth saying: resuming quietly folds four records away is a surprise,
    /// and the same line explains where they went.
    pub fn describe_view(&self) -> Option<String> {
        let folded = match self.folded.len() {
            0 => None,
            1 => Some("1 record folded".to_string()),
            n => Some(format!("{n} records folded")),
        };
        let arranged = (!self.album_order.is_empty()).then(|| "arranged by hand".to_string());
        match (folded, arranged) {
            (None, None) => None,
            (Some(a), None) | (None, Some(a)) => Some(a),
            (Some(a), Some(b)) => Some(format!("{a} \u{b7} {b}")),
        }
    }

    /// One line for the prompt.
    pub fn describe(&self) -> String {
        let who = match (self.artist.is_empty(), self.title.is_empty()) {
            (false, false) => format!("{} — {}", self.artist, self.title),
            (true, false) => self.title.clone(),
            _ => self.uri.clone(),
        };
        format!("{who}  at {}", crate::ui::digits::clock(self.position))
    }

    pub fn describe_context(&self) -> String {
        format!(
            "{} · track {} of {}",
            self.playlist_name,
            self.index + 1,
            self.total
        )
    }
}

/// Human-readable age, for the prompt.
pub fn age(saved_at: i64, now: i64) -> String {
    let secs = (now - saved_at).max(0);
    match secs {
        s if s < 90 => "just now".to_string(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Where a resume should actually start.
///
/// Backs up a couple of seconds: dropping in exactly where you stopped loses
/// the thread of the music, and every player that gets this right rewinds a
/// little.
pub fn resume_position(saved: f64) -> f64 {
    (saved - 2.0).max(0.0)
}

pub fn exists() -> bool {
    Session::path().map(|p| p.is_file()).unwrap_or(false)
}

pub fn path_matches(session: &Session, playlist: Option<&Path>) -> bool {
    match (&session.playlist, playlist) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            playlist: None,
            playlist_name: "1995".into(),
            index: 41,
            total: 103,
            position: 134.0,
            duration: 300.0,
            artist: "Angra".into(),
            title: "Nova Era".into(),
            uri: "Angra/Rebirth/02.flac".into(),
            shuffle: true,
            repeat: "All".into(),
            volume: 0.8,
            cursor: 41,
            folded: Vec::new(),
            album_order: Vec::new(),
            saved_at: 1_700_000_000,
        }
    }

    #[test]
    fn a_track_in_progress_is_worth_resuming() {
        assert!(sample().worth_resuming());
    }

    #[test]
    fn a_barely_started_track_is_not_worth_interrupting_startup_for() {
        let mut s = sample();
        s.position = 2.0;
        assert!(!s.worth_resuming());
    }

    #[test]
    fn a_track_about_to_end_is_not_offered() {
        let mut s = sample();
        s.position = 298.0;
        assert!(!s.worth_resuming());
    }

    #[test]
    fn an_empty_session_is_not_offered() {
        let mut s = sample();
        s.uri = String::new();
        assert!(!s.worth_resuming());

        let mut s = sample();
        s.total = 0;
        assert!(!s.worth_resuming());
    }

    #[test]
    fn resume_rewinds_a_little_rather_than_dropping_you_mid_phrase() {
        assert_eq!(resume_position(134.0), 132.0);
        // ...but never before the start of the track.
        assert_eq!(resume_position(1.0), 0.0);
        assert_eq!(resume_position(0.0), 0.0);
    }

    #[test]
    fn the_prompt_names_the_track_and_the_time() {
        let d = sample().describe();
        assert!(d.contains("Angra"));
        assert!(d.contains("Nova Era"));
        assert!(d.contains("2:14"), "{d}");
    }

    #[test]
    fn the_prompt_falls_back_to_the_uri_when_untagged() {
        let mut s = sample();
        s.artist = String::new();
        s.title = String::new();
        assert!(s.describe().contains("Angra/Rebirth/02.flac"));
    }

    #[test]
    fn age_reads_naturally() {
        let now = 1_700_000_000;
        assert_eq!(age(now - 10, now), "just now");
        assert_eq!(age(now - 600, now), "10m ago");
        assert_eq!(age(now - 7200, now), "2h ago");
        assert_eq!(age(now - 200_000, now), "2d ago");
        // A clock that moved backwards must not produce nonsense.
        assert_eq!(age(now + 500, now), "just now");
    }

    #[test]
    fn a_missing_playlist_is_detected() {
        let mut s = sample();
        assert!(s.playlist_available(), "no playlist means the library");
        s.playlist = Some(PathBuf::from("/definitely/not/here.m3u"));
        assert!(!s.playlist_available());
    }

    #[test]
    fn round_trips_through_toml() {
        let s = sample();
        let back: Session = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back.uri, s.uri);
        assert_eq!(back.index, s.index);
        assert!((back.position - s.position).abs() < 1e-6);
        assert_eq!(back.shuffle, s.shuffle);
    }

    #[test]
    fn the_view_comes_back_with_the_music() {
        let mut s = sample();
        s.folded = vec!["holy land".into(), "chained".into()];
        s.cursor = 7;
        let back: Session = toml::from_str(&toml::to_string_pretty(&s).unwrap()).unwrap();
        assert_eq!(back.folded, s.folded);
        assert_eq!(back.cursor, 7);
    }

    #[test]
    fn a_session_from_an_older_build_still_loads() {
        // The view state was added after people had session files. Dropping
        // one because it does not mention folded records would lose the place
        // they were actually in, which is the whole point of the file.
        let old = "\
playlist_name = \"1995\"
index = 41
total = 103
position = 134.0
duration = 300.0
artist = \"Angra\"
title = \"Nova Era\"
uri = \"Angra/Rebirth/02.flac\"
shuffle = true
repeat = \"All\"
volume = 0.8
saved_at = 1700000000
";
        let s: Session = toml::from_str(old).expect("an older session should still load");
        assert_eq!(s.index, 41);
        assert!(s.folded.is_empty());
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn the_prompt_says_what_the_view_will_do() {
        let mut s = sample();
        assert_eq!(s.describe_view(), None, "nothing folded, nothing to say");
        s.folded = vec!["holy land".into()];
        assert_eq!(s.describe_view().as_deref(), Some("1 record folded"));
        s.folded.push("chained".into());
        assert_eq!(s.describe_view().as_deref(), Some("2 records folded"));
    }
}
