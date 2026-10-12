//! Original album chronology for presentation, separate from edition metadata.
use std::path::Path;

use lofty::{file::TaggedFileExt, tag::ItemKey};

fn tagged_year(value: &str) -> Option<i64> {
    let value = value.trim();
    let year = value.get(..4)?.parse().ok()?;
    ((1000..=2099).contains(&year) && value.as_bytes().get(4).is_none_or(|b| !b.is_ascii_digit()))
        .then_some(year)
}

/// Read only metadata on the library owner, once per cached album. Never fetch
/// a remote audio file just to determine its date; CUE callers pass the backing file.
pub fn read_original(path: &Path) -> Option<i64> {
    let file = lofty::probe::Probe::open(path)
        .ok()?
        .options(
            lofty::config::ParseOptions::new()
                .read_properties(false)
                .read_cover_art(false),
        )
        .read()
        .ok()?;
    file.tags().iter().find_map(original_tag)
}

fn original_tag(tag: &lofty::tag::Tag) -> Option<i64> {
    tag.get_strings(&ItemKey::OriginalReleaseDate)
        .find_map(tagged_year)
}

/// A leading album-folder year can precede the edition date, e.g. `1980 -
/// Blizzard Of Ozz (2007)`. Avoid artist names, filenames, and box-set year ranges.
pub fn from_context(file_rel: &str, edition_year: Option<i64>) -> Option<i64> {
    let parent = file_rel.rsplit_once('/').map(|(parent, _)| parent)?;
    let album = super::infer::album_dir(parent).rsplit('/').next()?;
    let name = album.trim_start_matches([' ', '[', '(']);
    let leading = tagged_year(name).filter(|year| (1900..=2099).contains(year));
    let range = name.get(4..).is_some_and(|tail| {
        let tail = tail.trim_start();
        tail.strip_prefix(['-', '–', '—'])
            .is_some_and(|tail| tagged_year(tail.trim_start()).is_some())
    });
    let lower = album.to_lowercase();
    let edition_named = ["remaster", "reissue", "re-release", "rerelease"]
        .iter()
        .any(|word| lower.contains(word));
    if let Some(year) = leading.filter(|_| !range) {
        if !edition_named
            || edition_year.is_some_and(|edition| year < edition)
            || super::infer::year_in(name.get(4..).unwrap_or_default())
                .is_some_and(|edition| year < edition)
        {
            return Some(year);
        }
    }
    // An ordinary date alone does not establish the original date of a known reissue.
    if edition_named || range {
        None
    } else {
        edition_year
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_date_survives_real_file_tags_without_changing_the_edition() {
        use lofty::tag::{Tag, TagExt, TagType};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remaster.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&52u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&88200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&[0; 16]);
        std::fs::write(&path, wav).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::RecordingDate, "2007".into());
        tag.insert_text(ItemKey::OriginalReleaseDate, "1980-09-20".into());
        tag.save_to_path(&path, lofty::config::WriteOptions::default())
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(read_original(&path), Some(1980));
        let tagged = lofty::probe::Probe::open(&path).unwrap().read().unwrap();
        assert_eq!(
            tagged
                .primary_tag()
                .unwrap()
                .get_string(&ItemKey::RecordingDate),
            Some("2007")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn explicit_original_date_is_read_separately_from_the_edition() {
        let mut tag = lofty::tag::Tag::new(lofty::tag::TagType::VorbisComments);
        tag.insert_text(ItemKey::RecordingDate, "2007".into());
        tag.insert_text(ItemKey::OriginalReleaseDate, "1980-09-20".into());
        assert_eq!(original_tag(&tag), Some(1980));
        tag.insert_text(ItemKey::OriginalReleaseDate, "unknown".into());
        assert_eq!(original_tag(&tag), None);
    }
    #[test]
    fn original_folder_year_wins_over_remaster_and_cue_backing_year() {
        assert_eq!(
            from_context(
                "Ozzy/1980 - Blizzard Of Ozz (2007)/Blizzard.flac",
                Some(2007)
            ),
            Some(1980)
        );
        assert_eq!(
            from_context(
                "Artist/1980 - Album (2007 remaster)/CD 1/image.flac",
                Some(2007)
            ),
            Some(1980)
        );
        assert_eq!(
            from_context("Artist/Album (2025 Remastered)/01.flac", Some(2025)),
            None
        );
        assert_eq!(
            from_context("Artist/1974-1982 Box Set/01.flac", Some(2022)),
            None
        );
        assert_eq!(
            from_context("Artist/Album/01 - 1980.flac", Some(2001)),
            Some(2001)
        );
    }
}
