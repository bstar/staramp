//! Album-aware chronological playlists built from the canonical library.

use std::cmp::Ordering;
use std::collections::HashMap;

use anyhow::Result;

use super::browse::Model;
use super::db::Db;
use crate::playlist::queue::QueueItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSource {
    /// Filesystem creation time, falling back to modification time.
    File,
    /// The first time STAR/AMP inserted the track into its index.
    Indexed,
    /// The release year carried by the track metadata.
    Release,
}

impl DateSource {
    pub const ALL: [Self; 3] = [Self::File, Self::Indexed, Self::Release];

    pub fn label(self) -> &'static str {
        match self {
            Self::File => "filesystem date (created; modified fallback)",
            Self::Indexed => "first indexed by STAR/AMP",
            Self::Release => "release year from tags",
        }
    }
}

#[derive(Debug, Clone)]
struct TrackDate {
    file_seconds: Option<i64>,
    indexed_seconds: Option<i64>,
}

#[derive(Debug)]
struct DatedAlbum {
    album: usize,
    sort_date: Option<i64>,
    year: Option<i32>,
}

/// Years available for the source, oldest first.
pub fn years(db: &Db, model: &Model, source: DateSource) -> Result<Vec<i32>> {
    let mut years: Vec<i32> = albums(db, model, source)?
        .into_iter()
        .filter_map(|album| album.year)
        .collect();
    years.sort_unstable();
    years.dedup();
    Ok(years)
}

/// Build a full-library timeline. Albums are oldest first; tracks retain the
/// browser model's disc/track/CUE/natural filename order.
pub fn build(
    db: &Db,
    model: &Model,
    source: DateSource,
    selected_year: Option<i32>,
) -> Result<Vec<QueueItem>> {
    let mut albums = albums(db, model, source)?;
    albums.retain(|album| selected_year.is_none() || album.year == selected_year);
    albums.sort_by(|a, b| match (a.sort_date, b.sort_date) {
        (Some(a_date), Some(b_date)) => a_date
            .cmp(&b_date)
            .then_with(|| album_name(model, a.album).cmp(&album_name(model, b.album))),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => album_name(model, a.album).cmp(&album_name(model, b.album)),
    });

    Ok(albums
        .into_iter()
        .flat_map(|dated| {
            let album = &model.albums[dated.album];
            model.tracks[album.tracks.start as usize..album.tracks.end as usize]
                .iter()
                .map(move |track| {
                    let mut item = track.item.clone();
                    item.timeline_date = dated.sort_date;
                    item.timeline_year = dated.year.map(i64::from);
                    item
                })
        })
        .collect())
}

fn album_name(model: &Model, album: usize) -> (&str, &str) {
    let album = &model.albums[album];
    (&model.artists[album.artist as usize].name, &album.title)
}

fn albums(db: &Db, model: &Model, source: DateSource) -> Result<Vec<DatedAlbum>> {
    let dates = track_dates(db)?;
    Ok(model
        .albums
        .iter()
        .enumerate()
        .map(|(album_index, album)| {
            let tracks = &model.tracks[album.tracks.start as usize..album.tracks.end as usize];
            let mut values: Vec<i64> = tracks
                .iter()
                .filter_map(|track| match source {
                    DateSource::File => dates
                        .get(&track.item.uri.to_string())
                        .and_then(|date| date.file_seconds),
                    DateSource::Indexed => dates
                        .get(&track.item.uri.to_string())
                        .and_then(|date| date.indexed_seconds),
                    DateSource::Release => track.item.year,
                })
                .collect();
            values.sort_unstable();
            let sort_date = median(&values);
            let year = match source {
                DateSource::Release => sort_date.and_then(|year| i32::try_from(year).ok()),
                DateSource::File | DateSource::Indexed => sort_date.map(unix_year),
            };
            DatedAlbum {
                album: album_index,
                sort_date,
                year,
            }
        })
        .collect())
}

fn track_dates(db: &Db) -> Result<HashMap<String, TrackDate>> {
    let mut stmt = db.conn.prepare(
        "SELECT t.uri, COALESCE(f.created_ns, f.mtime_ns) / 1000000000, t.added_at
           FROM track t
           JOIN file f ON f.id = t.file_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            TrackDate {
                file_seconds: row.get(1)?,
                indexed_seconds: row.get(2)?,
            },
        ))
    })?;
    Ok(rows.flatten().collect())
}

fn median(values: &[i64]) -> Option<i64> {
    values.get(values.len() / 2).copied()
}

/// Gregorian calendar year for a non-negative Unix timestamp. Howard
/// Hinnant's civil-from-days algorithm, kept local to avoid a date dependency
/// for one integer field.
fn unix_year(seconds: i64) -> i32 {
    let days = seconds.max(0) / 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    i32::try_from(year).unwrap_or(i32::MAX)
}

pub fn year_for_timestamp(seconds: i64) -> i64 {
    i64::from(unix_year(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_resists_one_replaced_track() {
        assert_eq!(median(&[100, 101, 102, 103, 9_999]), Some(102));
    }

    #[test]
    fn unix_year_handles_boundaries_and_leap_years() {
        assert_eq!(unix_year(0), 1970);
        assert_eq!(unix_year(946_684_799), 1999);
        assert_eq!(unix_year(946_684_800), 2000);
        assert_eq!(unix_year(1_709_251_200), 2024);
    }
}
