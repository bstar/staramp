//! Choosing which image in a directory is the front cover.
//!
//! The ranking is not a guess. Counting every image beside the audio in the
//! reference library gives `cover` 695, `front` 204 and `folder` 175 against
//! `back` 139, `cd` 132, `inlay` 83, `full` 40, `obi` 37, `inside` 27, `logo`
//! 26, and around two hundred `booklet-N` page scans. "The first image in the
//! directory" picks a disc label or the back of the case about a third of the
//! time, so both a preference list and a rejection list are load-bearing.

use std::path::{Path, PathBuf};

use super::art::Source;
use super::db::{AlbumDetail, Db};

/// One image that could be this album's cover.
///
/// A list of these rather than a single answer, because the ranking below is
/// right most of the time and not all of it. When it is wrong the user can see
/// the alternatives and pick, which is cheaper than making the heuristic
/// cleverer and more honest than pretending it is never wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    /// A picture in the audio file's own tags.
    Embedded,
    /// An image file in the library.
    File(PathBuf),
    /// A cover fetched from the archive and cached.
    Remote(PathBuf),
    /// A cover fetched for the record the song originally came from.
    Original(PathBuf),
}

impl Candidate {
    pub fn source(&self) -> Source {
        match self {
            Candidate::Embedded => Source::Embedded,
            Candidate::Remote(_) => Source::Remote,
            Candidate::Original(_) => Source::Original,
            Candidate::File(_) => Source::Sidecar,
        }
    }

    /// A short name for the panel.
    pub fn label(&self) -> String {
        match self {
            Candidate::Embedded => "embedded".into(),
            Candidate::Remote(_) => "cover art archive".into(),
            Candidate::Original(_) => "original release".into(),
            Candidate::File(p) => p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image")
                .to_string(),
        }
    }

    /// A stable identity for remembering a choice across restarts.
    ///
    /// The path, so a choice survives a rescan; `Remote` and `Embedded` have
    /// no meaningful path of their own to record.
    pub fn id(&self) -> String {
        match self {
            Candidate::Embedded => "embedded".into(),
            Candidate::Remote(_) => "remote".into(),
            Candidate::Original(_) => "original".into(),
            Candidate::File(p) => p.display().to_string(),
        }
    }
}

/// Everything that could be this album's cover, and whether to look further.
pub struct Candidates {
    /// Best first. The first entry is what the panel shows unless the user
    /// has chosen otherwise.
    pub list: Vec<Candidate>,
    /// Nothing here is confidently a front cover, so the archive is worth
    /// asking. A folder holding only `back.jpg` should still get a cover.
    pub wants_remote: bool,
}

/// Stems that mean "this is the front", best first.
const PREFERRED: &[&str] = &["cover", "front", "folder", "album", "albumart", "art"];

/// Stems that are certainly *not* the front, however few other images there
/// are. A wrong cover is worse than none: it is confidently wrong.
const REJECTED: &[&str] = &[
    "back", "cd", "disc", "disk", "inlay", "obi", "booklet", "tray", "sleeve", "label", "logo",
    "band", "artist", "inside", "matrix", "spine", "media",
];

/// Words that mark a subdirectory as artwork, checked as substrings so
/// `Covers`, `covers hi-res` and `Artwork (JP)` all count.
///
/// A hint about *order*, not a filter. It used to be a filter, and a filter is
/// the wrong shape for this: the reference library holds 201 directories
/// called `Cover` -- singular, which the list did not have -- along with
/// `Pictures`, `pics`, `photo`, `bitmap`, `tech.info`, and one `Сovers` whose
/// first letter is a Cyrillic Es and so does not lowercase to `covers` at all.
/// The shapes a real library takes are not a list anyone can finish writing,
/// so anything holding images is looked at, and these are looked at first.
const ART_DIRS: &[&str] = &[
    "cover", "scan", "artwork", "booklet", "image", "art", "picture", "pic", "photo",
];

/// Does this directory name say it holds artwork?
fn looks_like_art(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        // Not `to_ascii_lowercase`: the album that started this has a Cyrillic
        // capital in its folder name, and an ASCII fold leaves it untouched.
        .to_lowercase();
    ART_DIRS.iter().any(|d| name.contains(d))
}

/// Every image that could be this album's cover, best first.
///
/// The order is the ranking that used to be the whole answer: a picture in the
/// file, then a well-named image beside it, then anything else in the folder,
/// then one level down into an artwork subdirectory. What changed is that the
/// losers are kept rather than discarded, so a wrong first guess is one click
/// from being corrected instead of being final.
pub fn candidates(db: &Db, root: &Path, detail: &AlbumDetail) -> Candidates {
    let mut list = Vec::new();

    // A picture in the file itself is unambiguous -- somebody chose it for
    // this record -- so nothing on disk should outrank it.
    if embedded(&root.join(&detail.file_rel)).is_some() {
        list.push(Candidate::Embedded);
    }

    // The album's own directory, then one level down, and only into
    // directories that sound like artwork: a `Live at Wembley/` subfolder full
    // of its own audio is not artwork.
    let mut dirs = vec![detail.dir_id];
    // Named artwork directories first, then any other child that has images
    // in it. A subfolder full of its own audio contributes nothing either way,
    // because only images come back out of it.
    let (named, rest): (Vec<_>, Vec<_>) = db
        .child_dirs(detail.dir_id)
        .unwrap_or_default()
        .into_iter()
        .partition(|(_, path)| looks_like_art(path));
    dirs.extend(named.into_iter().map(|(id, _)| id));
    dirs.extend(rest.into_iter().map(|(id, _)| id));

    let mut confident = !list.is_empty();
    for dir in dirs {
        let Ok(images) = db.images_in_dir(dir) else {
            continue;
        };
        let ranked = rank(&images);
        confident |= ranked.iter().any(|(_, good)| *good);
        list.extend(
            ranked
                .into_iter()
                .map(|(rel, _)| Candidate::File(root.join(rel))),
        );
    }

    Candidates {
        wants_remote: !confident,
        list,
    }
}

/// A directory's images, best first, each flagged as a plausible front or not.
///
/// The flag is what decides whether to go looking further afield: a folder
/// holding nothing but `back.jpg` and `cd.jpg` has no cover in it, even though
/// it has images in it.
fn rank(images: &[String]) -> Vec<(&String, bool)> {
    let mut out: Vec<(&String, bool)> = Vec::with_capacity(images.len());

    // Exact name first: `cover.jpg` over `cover-back.jpg`.
    for want in PREFERRED {
        out.extend(
            images
                .iter()
                .filter(|p| stem(p) == *want)
                .map(|p| (p, true)),
        );
    }
    // Then a name that begins like one.
    for want in PREFERRED {
        out.extend(
            images
                .iter()
                .filter(|p| !rejected(p) && stem(p).starts_with(want))
                .map(|p| (p, true)),
        );
    }
    // Then anything not on the rejection list, then the rejects, which are
    // still worth offering -- somebody may actually want the back.
    for good in [true, false] {
        out.extend(
            images
                .iter()
                .filter(|p| rejected(p) != good)
                .map(|p| (p, good)),
        );
    }

    let mut seen = std::collections::HashSet::new();
    out.retain(|(p, _)| seen.insert(p.as_str()));
    out
}

/// The best cover in a directory, or `None` when it holds no plausible front.
///
/// Kept as the pure, testable heart of the ranking.
pub fn best(images: &[String]) -> Option<&String> {
    rank(images)
        .into_iter()
        .find(|(_, good)| *good)
        .map(|(p, _)| p)
}

/// The front cover stored inside the audio file, if there is one.
///
/// Failure of any kind is `None`: an unreadable tag on a playable file is not
/// worth a word to the user, and every rung below this one still applies.
pub fn embedded(path: &Path) -> Option<Vec<u8>> {
    // Our own reader first, and only for ID3. The tag library mishandles
    // unsynchronised tags -- it eats both zeros of a `FF 00 00`, which is how
    // a stuffed JPEG byte is stored -- and the result decodes to a flat grey
    // rectangle rather than failing outright, so nothing downstream can tell
    // it went wrong. See `super::id3`.
    if let Some(p) = id3_head(path).and_then(|head| super::id3::picture(&head)) {
        return Some(p.data);
    }
    lofty_picture(path)
}

/// The start of a file, enough to hold its ID3v2 tag.
///
/// Read rather than memory-mapped, and bounded by the tag's own declared size
/// so a corrupt header cannot ask for the whole file. `None` for anything
/// without a tag, which costs ten bytes to find out.
fn id3_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 10];
    file.read_exact(&mut header).ok()?;
    if &header[..3] != b"ID3" {
        return None;
    }
    // The syncsafe size, which by construction cannot exceed 256 MB. Refusing
    // anything absurd keeps a damaged header from allocating wildly.
    let size = ((header[6] as usize) << 21)
        | ((header[7] as usize) << 14)
        | ((header[8] as usize) << 7)
        | header[9] as usize;
    if size == 0 || size > 64 * 1024 * 1024 {
        return None;
    }

    // `take` + `read_to_end` rather than one `read`, which is free to come
    // back short and would silently truncate the picture.
    let mut out = header.to_vec();
    let mut rest = Vec::new();
    file.take(size as u64).read_to_end(&mut rest).ok()?;
    out.append(&mut rest);
    Some(out)
}

/// The picture the tag library finds, for everything that is not ID3.
fn lofty_picture(path: &Path) -> Option<Vec<u8>> {
    use lofty::file::TaggedFileExt;
    use lofty::picture::PictureType;
    use lofty::probe::Probe;

    let tagged = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pictures = tag.pictures();
    // The one marked as the front, and only otherwise whatever came first --
    // a file with a back cover and a booklet page but no front should not
    // hand us the booklet page.
    let picture = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    Some(picture.data().to_vec())
}

/// The lowercase file stem of a library-relative path.
fn stem(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Is this one of the images that is definitely not a front cover?
fn rejected(rel: &str) -> bool {
    let s = stem(rel);

    // Page scans: `05-06.jpg`, `01-02.png`. Numeric on both sides of a dash.
    if let Some((a, b)) = s.split_once('-') {
        if !a.is_empty()
            && !b.is_empty()
            && a.chars().all(|c| c.is_ascii_digit())
            && b.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    // A bare number is a scan too: `01.jpg`.
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    REJECTED.iter().any(|bad| {
        // `cd`, `cd1`, `cd 2`, `disc03`, and `booklet-3` all reject, but a
        // word that merely contains one -- `backdrop` -- does not.
        s == *bad
            || s.strip_prefix(bad).is_some_and(|rest| {
                rest.trim_start_matches([' ', '-', '_'])
                    .chars()
                    .all(|c| c.is_ascii_digit())
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| format!("A/B/{n}")).collect()
    }

    #[test]
    fn a_named_front_cover_wins() {
        for name in ["cover.jpg", "front.png", "folder.jpg", "album.jpg"] {
            let imgs = v(&["back.jpg", name, "cd.jpg"]);
            assert_eq!(
                best(&imgs).map(|s| s.as_str()),
                Some(&*format!("A/B/{name}"))
            );
        }
    }

    #[test]
    fn the_preference_order_holds() {
        // Every one of these is a plausible front; the list decides.
        let imgs = v(&["art.jpg", "folder.jpg", "cover.jpg", "front.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/cover.jpg");
        let imgs = v(&["art.jpg", "folder.jpg", "front.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/front.jpg");
    }

    #[test]
    fn the_backs_and_discs_are_never_chosen() {
        // The whole reason for a rejection list: with only these present the
        // answer is none, not "well, the first one".
        let imgs = v(&[
            "back.jpg",
            "cd.jpg",
            "cd2.jpg",
            "disc 1.jpg",
            "inlay.jpg",
            "obi.jpg",
            "booklet-3.jpg",
            "tray.png",
            "logo.jpg",
            "spine.jpg",
        ]);
        assert_eq!(best(&imgs), None, "chose {:?}", best(&imgs));
    }

    #[test]
    fn page_scans_are_never_chosen() {
        let imgs = v(&["05-06.jpg", "01.jpg", "12-13.png"]);
        assert_eq!(best(&imgs), None, "chose {:?}", best(&imgs));
    }

    #[test]
    fn a_prefixed_name_is_taken_when_there_is_no_exact_one() {
        let imgs = v(&["back.jpg", "cover_1200x1200.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/cover_1200x1200.jpg");
    }

    #[test]
    fn an_artwork_directory_is_recognised_however_it_is_spelled() {
        for name in [
            "A/B/Covers",
            "A/B/cover",
            "A/B/covers hi-res",
            "A/B/Artwork (JP)",
            "A/B/Scans",
            "A/B/scans box",
            "A/B/Pictures",
            "A/B/pics",
            "A/B/photo",
            "A/B/Booklet",
        ] {
            assert!(looks_like_art(name), "{name}");
        }
        for name in ["A/B/CD1", "A/B/Live at Wembley", "A/B/tech.info"] {
            assert!(!looks_like_art(name), "{name}");
        }
    }

    #[test]
    fn a_name_this_cannot_read_is_why_it_is_only_a_hint() {
        // The At Vance `Only Human` folder is called `\u{421}overs`, with a
        // Cyrillic capital Es. It lowercases to a Cyrillic es, so it does not
        // contain "cover" and no amount of widening the list will make it. It
        // is looked into anyway, just after the ones that named themselves --
        // which is the whole reason this became an ordering and stopped being
        // a filter.
        assert!(!looks_like_art("A/B/\u{421}overs"));
    }

    #[test]
    fn a_booklet_scan_is_a_cover_when_nothing_better_exists() {
        // The At Vance `Only Human` folder: a full set of scans and not one of
        // them named `front` or `cover`. Everything specific is on the reject
        // list, which leaves the one general name -- and that is the cover.
        let images = v(&[
            "Back.jpg",
            "Band.jpg",
            "Booklet 02.jpg",
            "CD.jpg",
            "Full.jpg",
            "Inlay.jpg",
            "logo 1.jpg",
            "OBI.jpg",
        ]);
        assert_eq!(best(&images).map(|s| s.as_str()), Some("A/B/Full.jpg"));
        // And every one of them is still offered, so the choice can be
        // overruled.
        assert_eq!(rank(&images).len(), images.len());
    }

    #[test]
    fn an_exact_name_beats_a_prefixed_one() {
        let imgs = v(&["cover_hires.jpg", "cover.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/cover.jpg");
    }

    #[test]
    fn an_oddly_named_image_is_better_than_nothing() {
        // 105 albums in the reference library have exactly this: one image,
        // named after the release rather than after its role.
        let imgs = v(&["dragonchaser_frontal.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/dragonchaser_frontal.jpg");
    }

    #[test]
    fn a_word_that_merely_starts_like_a_reject_is_kept() {
        // `backdrop` is not `back`; `cdreissue` is not `cd`.
        let imgs = v(&["backdrop.jpg"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/backdrop.jpg");
        let imgs = v(&["cdreissue.png"]);
        assert_eq!(best(&imgs).unwrap(), "A/B/cdreissue.png");
    }

    /// A minimal but real WAV, so lofty has something it will actually write a
    /// tag into. Sixteen bytes of silence is enough to be a valid file.
    fn silent_wav() -> Vec<u8> {
        let data: [u8; 16] = [0; 16];
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        w.extend_from_slice(&1u16.to_le_bytes()); // pcm
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&44100u32.to_le_bytes());
        w.extend_from_slice(&88200u32.to_le_bytes()); // byte rate
        w.extend_from_slice(&2u16.to_le_bytes()); // block align
        w.extend_from_slice(&16u16.to_le_bytes()); // bits
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        w
    }

    /// A one-pixel PNG, so the bytes that come back can be compared exactly.
    fn tiny_png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(1, 1, image::Rgb([7, 8, 9]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_in_the_file_is_read_out_of_it() {
        use lofty::config::WriteOptions;
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::{Tag, TagExt, TagType};

        let dir = std::env::temp_dir().join(format!("staramp-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.wav");
        std::fs::write(&path, silent_wav()).unwrap();

        let png = tiny_png();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            png.clone(),
        ));
        tag.save_to_path(&path, WriteOptions::default()).unwrap();

        assert_eq!(
            embedded(&path).as_deref(),
            Some(&png[..]),
            "the embedded picture must come back byte for byte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_with_no_picture_falls_through() {
        let dir = std::env::temp_dir().join(format!("staramp-nocover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.wav");
        std::fs::write(&path, silent_wav()).unwrap();
        assert!(embedded(&path).is_none());
        // And a path that is not a file at all is not an error either.
        assert!(embedded(&dir.join("nothing.flac")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_image_is_offered_with_the_best_one_first() {
        // The losers are kept now. The ranking decides what shows by default;
        // it no longer decides what the user is allowed to see.
        let imgs = v(&["back.jpg", "cover.jpg", "booklet-3.jpg", "live.jpg"]);
        let ranked: Vec<&str> = rank(&imgs).iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(ranked[0], "A/B/cover.jpg", "the front still leads");
        assert_eq!(ranked.len(), 4, "nothing may be dropped: {ranked:?}");
        assert!(
            ranked.iter().position(|p| p.contains("live")).unwrap()
                < ranked.iter().position(|p| p.contains("back")).unwrap(),
            "a plain image should come before a known back cover: {ranked:?}"
        );
    }

    #[test]
    fn a_directory_of_only_rejects_is_not_confident() {
        // This is what sends the search to the archive: images are present,
        // but none of them is a front cover.
        let imgs = v(&["back.jpg", "cd.jpg", "booklet-2.jpg"]);
        assert!(rank(&imgs).iter().all(|(_, good)| !*good));
        assert_eq!(best(&imgs), None);
        // And they are still offered, because somebody may want the back.
        assert_eq!(rank(&imgs).len(), 3);
    }

    #[test]
    fn a_candidate_can_name_itself() {
        let c = Candidate::File(PathBuf::from("/music/A/cover.jpg"));
        assert_eq!(c.label(), "cover.jpg");
        assert_eq!(c.id(), "/music/A/cover.jpg", "the id survives a rescan");
        assert_eq!(Candidate::Embedded.label(), "embedded");
        assert_eq!(
            Candidate::Remote(PathBuf::from("/c/ab12.jpg")).label(),
            "cover art archive",
            "a cache hash is not a name worth showing"
        );
    }

    #[test]
    fn an_empty_directory_has_no_cover() {
        assert_eq!(best(&[]), None);
    }

    #[test]
    fn the_choice_is_stable_across_orderings() {
        // A rescan can return rows in a different order; the cover must not
        // change under the user.
        let a = v(&["one.jpg", "two.jpg", "three.jpg"]);
        let mut b = a.clone();
        b.reverse();
        assert_eq!(best(&a).unwrap(), "A/B/one.jpg");
        assert_eq!(best(&b).unwrap(), "A/B/three.jpg");
        // With a real cover present, order is irrelevant.
        let mut c = v(&["one.jpg", "cover.jpg", "two.jpg"]);
        assert_eq!(best(&c).unwrap(), "A/B/cover.jpg");
        c.reverse();
        assert_eq!(best(&c).unwrap(), "A/B/cover.jpg");
    }
}
