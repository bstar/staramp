//! Adding one track to an existing playlist without losing its other entries.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use unicode_normalization::UnicodeNormalization;

use super::m3u::{self, Playlist, PlaylistItem, WriteStyle};
use super::queue::QueueItem;
use crate::library::browse::{Model, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Similar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMatch {
    pub index: usize,
    pub kind: MatchKind,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub codec: String,
}

#[derive(Debug, Clone)]
pub struct Destination {
    pub name: String,
    pub path: PathBuf,
    pub tracks: usize,
    pub matches: Vec<TrackMatch>,
}

fn indexed<'a>(model: &'a Model, uri: &str) -> Option<&'a Track> {
    let span = model.by_uri.get(uri)?;
    let index = *model.by_span.get(span)? as usize;
    model.tracks.get(index)
}

fn normalized(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn artist(item: &QueueItem) -> Option<&str> {
    item.artist
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            item.album_artist
                .as_deref()
                .filter(|s| !s.trim().is_empty())
        })
}

fn similar(source: &Track, candidate: &Track) -> bool {
    if source
        .mb_recording_id
        .as_deref()
        .zip(candidate.mb_recording_id.as_deref())
        .is_some_and(|(a, b)| !a.trim().is_empty() && a.eq_ignore_ascii_case(b))
    {
        return true;
    }

    let Some(source_title) = source.item.title.as_deref() else {
        return false;
    };
    let Some(candidate_title) = candidate.item.title.as_deref() else {
        return false;
    };
    let Some(source_artist) = artist(&source.item) else {
        return false;
    };
    let Some(candidate_artist) = artist(&candidate.item) else {
        return false;
    };
    let Some(source_duration) = source.item.duration_secs else {
        return false;
    };
    let Some(candidate_duration) = candidate.item.duration_secs else {
        return false;
    };

    normalized(source_title) == normalized(candidate_title)
        && normalized(source_artist) == normalized(candidate_artist)
        && source_duration.abs_diff(candidate_duration) <= 3
}

pub fn matches(model: &Model, source: &QueueItem, playlist: &Playlist) -> Vec<TrackMatch> {
    let source_uri = source.uri.to_string();
    let source_track = indexed(model, &source_uri);
    playlist
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let uri = item.uri.to_string();
            let candidate = indexed(model, &uri);
            let kind = if uri == source_uri
                || source_track
                    .zip(candidate)
                    .is_some_and(|(a, b)| a.span == b.span)
            {
                MatchKind::Exact
            } else if source_track
                .zip(candidate)
                .is_some_and(|(a, b)| similar(a, b))
            {
                MatchKind::Similar
            } else {
                return None;
            };
            let track = candidate;
            Some(TrackMatch {
                index,
                kind,
                uri,
                title: track
                    .and_then(|t| t.item.title.clone())
                    .or_else(|| item.ext_title.clone())
                    .unwrap_or_else(|| item.raw_line.clone()),
                artist: track
                    .and_then(|t| artist(&t.item).map(str::to_string))
                    .unwrap_or_default(),
                codec: track.map(|t| t.codec.to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn scan(dir: &Path, model: &Model, source: &QueueItem) -> Result<Vec<Destination>> {
    let mut out = Vec::new();
    for path in m3u::list_dir(dir)? {
        let Ok(playlist) = m3u::read_file(&path) else {
            continue;
        };
        out.push(Destination {
            name: playlist.name.clone(),
            path,
            tracks: playlist.items.len(),
            matches: matches(model, source, &playlist),
        });
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    Ok(out)
}

pub fn playlist_item(source: &QueueItem) -> PlaylistItem {
    let title = match (source.artist.as_deref(), source.title.as_deref()) {
        (Some(artist), Some(title)) if !artist.is_empty() => Some(format!("{artist} - {title}")),
        (_, title) => title.map(str::to_string),
    };
    PlaylistItem {
        uri: source.uri.clone(),
        before: Vec::new(),
        raw_line: source.uri.to_string(),
        ext_title: title,
        ext_duration_secs: source.duration_secs,
    }
}

pub fn append(path: &Path, source: &QueueItem) -> Result<usize> {
    let mut playlist = m3u::read_file(path)?;
    playlist.items.push(playlist_item(source));
    m3u::write_file(&playlist, path, WriteStyle::Preserve)
        .with_context(|| format!("adding to {}", path.display()))?;
    Ok(playlist.items.len())
}

pub fn replace(path: &Path, expected: &TrackMatch, source: &QueueItem) -> Result<usize> {
    let mut playlist = m3u::read_file(path)?;
    let Some(item) = playlist.items.get_mut(expected.index) else {
        anyhow::bail!("playlist changed; choose the track again");
    };
    if item.uri.to_string() != expected.uri {
        anyhow::bail!("playlist changed; choose the track again");
    }
    let before = std::mem::take(&mut item.before);
    *item = playlist_item(source);
    item.before = before;
    m3u::write_file(&playlist, path, WriteStyle::Preserve)
        .with_context(|| format!("updating {}", path.display()))?;
    Ok(expected.index)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::library::browse::{Model, Span, Track};
    use crate::library::infer::Source;
    use crate::playlist::uri::TrackUri;

    type ModelRow<'a> = (
        &'a str,
        &'a str,
        &'a str,
        i64,
        &'a str,
        Option<&'a str>,
        Span,
    );

    fn item(uri: &str, artist: &str, title: &str, duration: i64) -> QueueItem {
        let mut item = QueueItem::new(TrackUri::parse(uri));
        item.artist = Some(artist.into());
        item.title = Some(title.into());
        item.duration_secs = Some(duration);
        item
    }

    fn model(rows: Vec<ModelRow<'_>>) -> Model {
        let mut tracks = Vec::new();
        let mut by_uri = HashMap::new();
        let mut by_span = HashMap::new();
        for (uri, artist, title, duration, codec, mbid, span) in rows {
            let index = tracks.len() as u32;
            by_uri.insert(uri.to_string(), span);
            by_span.insert(span, index);
            tracks.push(Track {
                item: item(uri, artist, title, duration),
                span,
                dir_id: 1,
                dir: Arc::from("Artist/Album"),
                sheet: None,
                codec: Arc::from(codec),
                mb_recording_id: mbid.map(Arc::from),
                album: 0,
                artist_from: Source::Tag,
                album_from: Source::Tag,
                year_from: Source::Tag,
            });
        }
        Model {
            tracks,
            albums: Vec::new(),
            artists: Vec::new(),
            by_uri,
            by_span,
            generation: 1,
        }
    }

    fn playlist(uris: &[&str]) -> Playlist {
        m3u::parse(
            &uris
                .iter()
                .map(|uri| format!("{uri}\n"))
                .collect::<String>(),
        )
    }

    #[test]
    fn exact_uri_matches_without_index_metadata() {
        let model = model(Vec::new());
        let source = QueueItem::new(TrackUri::parse("A/song.flac"));
        let found = matches(&model, &source, &playlist(&["A/song.flac"]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, MatchKind::Exact);
    }

    #[test]
    fn a_shared_audio_span_is_exact() {
        let span = Span {
            file_id: 7,
            start_frame: 0,
            end_frame: None,
        };
        let mut model = model(vec![("A/song.flac", "A", "Song", 200, "flac", None, span)]);
        model.by_uri.insert("A/disc.cue/track0001".into(), span);
        let source = item("A/disc.cue/track0001", "A", "Song", 200);
        let found = matches(&model, &source, &playlist(&["A/song.flac"]));
        assert_eq!(found[0].kind, MatchKind::Exact);
    }

    #[test]
    fn metadata_finds_an_alternate_encoding_conservatively() {
        let model = model(vec![
            (
                "A/song.flac",
                "  The  Artist ",
                "The Song",
                201,
                "flac",
                None,
                Span {
                    file_id: 1,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
            (
                "A/song.mp3",
                "the artist",
                "the song",
                199,
                "mp3",
                None,
                Span {
                    file_id: 2,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
        ]);
        let source = item("A/song.flac", "The Artist", "The Song", 201);
        let found = matches(&model, &source, &playlist(&["A/song.mp3"]));
        assert_eq!(found[0].kind, MatchKind::Similar);
        assert_eq!(found[0].codec, "mp3");
    }

    #[test]
    fn duration_difference_rejects_a_fuzzy_match() {
        let model = model(vec![
            (
                "new.flac",
                "Artist",
                "Song",
                200,
                "flac",
                None,
                Span {
                    file_id: 1,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
            (
                "live.mp3",
                "Artist",
                "Song",
                204,
                "mp3",
                None,
                Span {
                    file_id: 2,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
        ]);
        assert!(matches(
            &model,
            &item("new.flac", "Artist", "Song", 200),
            &playlist(&["live.mp3"])
        )
        .is_empty());
    }

    #[test]
    fn musicbrainz_identity_matches_even_when_release_tags_differ() {
        let model = model(vec![
            (
                "new.flac",
                "Artist",
                "Album Mix",
                200,
                "flac",
                Some("recording-1"),
                Span {
                    file_id: 1,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
            (
                "old.mp3",
                "Artist",
                "Original Mix",
                190,
                "mp3",
                Some("recording-1"),
                Span {
                    file_id: 2,
                    start_frame: 0,
                    end_frame: None,
                },
            ),
        ]);
        let found = matches(
            &model,
            &item("new.flac", "Artist", "Album Mix", 200),
            &playlist(&["old.mp3"]),
        );
        assert_eq!(found[0].kind, MatchKind::Similar);
    }

    #[test]
    fn replacement_keeps_position_and_other_unresolved_entries() {
        let dir = std::env::temp_dir().join(format!("staramp-add-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("epic.m3u");
        std::fs::write(&path, "missing/one.ape\nold/song.mp3\nmissing/two.wv\n").unwrap();
        let expected = TrackMatch {
            index: 1,
            kind: MatchKind::Similar,
            uri: "old/song.mp3".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            codec: "mp3".into(),
        };
        replace(
            &path,
            &expected,
            &item("new/song.flac", "Artist", "Song", 200),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "missing/one.ape\nnew/song.flac\nmissing/two.wv\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stale_replacement_is_refused() {
        let dir = std::env::temp_dir().join(format!("staramp-add-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("epic.m3u");
        std::fs::write(&path, "someone/else.flac\n").unwrap();
        let expected = TrackMatch {
            index: 0,
            kind: MatchKind::Similar,
            uri: "old/song.mp3".into(),
            title: String::new(),
            artist: String::new(),
            codec: String::new(),
        };
        assert!(replace(
            &path,
            &expected,
            &item("new/song.flac", "Artist", "Song", 200)
        )
        .is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "someone/else.flac\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
