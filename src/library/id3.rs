//! Reading the picture out of an ID3v2 tag.
//!
//! This exists because the tag library gets it wrong on unsynchronised tags,
//! and the failure is silent and total: the cover comes back as a 512x512
//! rectangle of exactly RGB(128,128,128), which is what a JPEG decoder emits
//! when the entropy stream is unusable but the header parsed.
//!
//! **Unsynchronisation** is ID3's way of making sure a tag can never contain
//! a byte sequence an MP3 decoder would mistake for a frame header. Any `FF`
//! followed by `00` or by a byte from `E0` up gets a `00` inserted after the
//! `FF`. Reversing it means removing exactly one `00` after each `FF`.
//!
//! The case that goes wrong is `FF 00`, which is *also* how JPEG escapes a
//! literal `FF` in its entropy-coded data, and is therefore extremely common
//! inside an embedded cover. In the tag it is stored as `FF 00 00`; removing
//! one zero leaves `FF 00`, which is right, and removing both leaves `FF`,
//! which shifts every subsequent byte and destroys the image from that point.
//! Measured on a real file here, the streams part company at byte 463 of
//! 117,629 and everything after it is noise.
//!
//! Only ID3 needs this. FLAC, Ogg and MP4 have no unsynchronisation scheme,
//! and the tag library reads their pictures correctly.

/// A picture found in a tag.
pub struct Picture {
    pub data: Vec<u8>,
    /// The ID3 picture type: 3 is the front cover.
    pub kind: u8,
}

/// The front cover from a file's ID3v2 tag, if it has one.
///
/// `None` for anything without an ID3v2 tag, which includes every FLAC and
/// most Ogg -- the caller falls back to the tag library for those.
pub fn picture(bytes: &[u8]) -> Option<Picture> {
    let (version, flags, body) = header(bytes)?;

    // In 2.2 and 2.3 the whole tag is unsynchronised as one run, so it has to
    // be reversed before the frames can be walked at all. In 2.4 it is
    // per-frame, and the tag flag is advisory.
    let body = if flags & 0x80 != 0 && version < 4 {
        deunsync(body)
    } else {
        body.to_vec()
    };

    let mut best: Option<Picture> = None;
    for frame in frames(&body, version, flags) {
        let Some(p) = parse(&frame, version) else {
            continue;
        };
        // The one marked as the front wins; otherwise the first picture will
        // do, because a file with only a back cover is better than nothing.
        if p.kind == FRONT_COVER {
            return Some(p);
        }
        best.get_or_insert(p);
    }
    best
}

const FRONT_COVER: u8 = 3;

/// The tag header: version, flags, and the tag body.
fn header(bytes: &[u8]) -> Option<(u8, u8, &[u8])> {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return None;
    }
    let version = bytes[3];
    // 2.2, 2.3 and 2.4 are the versions that exist. A future one would have a
    // different frame layout and guessing at it would be worse than declining.
    if !(2..=4).contains(&version) {
        return None;
    }
    let flags = bytes[5];
    let size = syncsafe(&bytes[6..10])?;
    let end = 10usize.checked_add(size)?.min(bytes.len());
    let mut body = &bytes[10..end];

    // An extended header, if present, sits before the frames.
    if flags & 0x40 != 0 && body.len() >= 4 {
        let ext = if version >= 4 {
            // 2.4 counts the size field itself; 2.3 does not.
            syncsafe(&body[..4])?
        } else {
            u32::from_be_bytes(body[..4].try_into().ok()?) as usize + 4
        };
        body = body.get(ext.min(body.len())..)?;
    }
    Some((version, flags, body))
}

/// Undo unsynchronisation: one `00` goes, after each `FF`.
///
/// The subtlety is entirely in what happens next. After removing the zero the
/// scan continues *past* it, so `FF 00 00` becomes `FF 00` rather than `FF`.
/// Getting that wrong is the bug this module exists for.
fn deunsync(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xFF && data.get(i + 1) == Some(&0x00) {
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// Every picture frame's payload, already de-unsynchronised.
fn frames(body: &[u8], version: u8, tag_flags: u8) -> Vec<Vec<u8>> {
    let (id_len, header_len) = if version == 2 { (3, 6) } else { (4, 10) };
    let want: &[u8] = if version == 2 { b"PIC" } else { b"APIC" };

    let mut out = Vec::new();
    let mut i = 0;
    while i + header_len <= body.len() {
        let id = &body[i..i + id_len];
        // Padding: the rest of the tag is zeroes.
        if id[0] == 0 {
            break;
        }
        let size = match version {
            2 => u32::from_be_bytes([0, body[i + 3], body[i + 4], body[i + 5]]) as usize,
            // 2.3 sizes are plain big-endian; only 2.4 made them syncsafe.
            3 => u32::from_be_bytes(body[i + 4..i + 8].try_into().unwrap()) as usize,
            _ => match syncsafe(&body[i + 4..i + 8]) {
                Some(n) => n,
                None => break,
            },
        };
        let start = i + header_len;
        let end = match start.checked_add(size) {
            Some(e) if e <= body.len() => e,
            _ => break,
        };

        if id == want {
            let flags = if version == 2 { 0 } else { body[i + 9] };
            let mut data = &body[start..end];
            // A data length indicator says how long the frame is once
            // decoded. Four syncsafe bytes, and they are not part of it.
            if version >= 4 && flags & 0x01 != 0 && data.len() >= 4 {
                data = &data[4..];
            }
            // Compressed or encrypted frames are not something to guess at.
            let unusable = if version >= 4 {
                flags & 0x0C != 0
            } else {
                flags & 0xC0 != 0
            };
            if !unusable {
                let per_frame = version >= 4 && flags & 0x02 != 0;
                // In 2.4 the whole-tag flag means every frame is
                // unsynchronised, whether or not each says so itself.
                let whole_tag = version >= 4 && tag_flags & 0x80 != 0;
                out.push(if per_frame || whole_tag {
                    deunsync(data)
                } else {
                    data.to_vec()
                });
            }
        }
        i = end;
    }
    out
}

/// One picture frame's fields.
///
/// `APIC` is encoding, MIME string, picture type, description, data. `PIC`,
/// from 2.2, has a three-character image format instead of the MIME string.
fn parse(frame: &[u8], version: u8) -> Option<Picture> {
    let encoding = *frame.first()?;
    let after_format = if version == 2 {
        // "JPG", "PNG" -- fixed width, no terminator.
        4
    } else {
        let end = frame.iter().skip(1).position(|&b| b == 0)? + 1;
        end + 1
    };
    let kind = *frame.get(after_format)?;
    let desc = frame.get(after_format + 1..)?;

    // The description is text in the frame's own encoding, and UTF-16 ends on
    // a pair of zero bytes rather than one. Reading a single zero would leave
    // the second byte at the front of the image.
    let wide = matches!(encoding, 1 | 2);
    let start = if wide {
        let mut j = 0;
        loop {
            if j + 1 >= desc.len() {
                return None;
            }
            if desc[j] == 0 && desc[j + 1] == 0 {
                break j + 2;
            }
            j += 2;
        }
    } else {
        desc.iter().position(|&b| b == 0)? + 1
    };

    let data = desc.get(start..)?;
    // A picture has to at least look like one. Two bytes of nothing is not a
    // cover, and passing it on only produces a grey rectangle further down.
    if data.len() < 16 {
        return None;
    }
    Some(Picture {
        data: data.to_vec(),
        kind,
    })
}

/// ID3's seven-bits-per-byte integer, used so a size can never contain `FF`.
fn syncsafe(b: &[u8]) -> Option<usize> {
    let b: [u8; 4] = b.get(..4)?.try_into().ok()?;
    // A byte with its top bit set is not syncsafe, and reading it as though it
    // were would give a size that runs off the end of the tag.
    if b.iter().any(|x| x & 0x80 != 0) {
        return None;
    }
    Some(((b[0] as usize) << 21) | ((b[1] as usize) << 14) | ((b[2] as usize) << 7) | b[3] as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply unsynchronisation, the way a tag writer does.
    fn unsync(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, &b) in data.iter().enumerate() {
            out.push(b);
            if b == 0xFF {
                match data.get(i + 1) {
                    Some(&n) if n == 0 || n >= 0xE0 => out.push(0),
                    None => out.push(0),
                    _ => {}
                }
            }
        }
        out
    }

    #[test]
    fn unsynchronisation_survives_a_round_trip() {
        // The middle case is the one that matters: `FF 00` in the source is
        // stored as `FF 00 00`, and reversing it must leave `FF 00` rather
        // than `FF`.
        for original in [
            vec![0xFF, 0x00, 0x12],
            vec![0xFF, 0xE0, 0x11],
            vec![0xFF, 0x00],
            vec![0xFF],
            vec![0x00, 0xFF, 0x00, 0x00, 0xFF, 0xE1],
            vec![0xFF, 0xFF, 0x00, 0xE0],
            vec![1, 2, 3],
            vec![],
        ] {
            let there = unsync(&original);
            let back = deunsync(&there);
            assert_eq!(back, original, "round trip failed for {original:02x?}");
        }
    }

    #[test]
    fn a_stuffed_jpeg_byte_is_not_eaten() {
        // Exactly the failure that prompted this module, in miniature: the
        // library it replaces turned `FF 00 00` into `FF`, dropping both
        // zeros and shifting everything after it.
        assert_eq!(deunsync(&[0xFF, 0x00, 0x00, 0x12]), vec![0xFF, 0x00, 0x12]);
    }

    /// A whole ID3v2 tag with one picture frame, built the way a real tagger
    /// would -- which differs by version in exactly the way that matters.
    ///
    /// In 2.3 the entire tag body is unsynchronised as one run, headers
    /// included, and the frame sizes describe the decoded payload. In 2.4 each
    /// frame is unsynchronised on its own, the size field counts the stored
    /// bytes, and a data length indicator carries the decoded length.
    fn tag(version: u8, tag_unsync: bool, frame_unsync: bool, dli: bool, jpeg: &[u8]) -> Vec<u8> {
        let mut frame = vec![0u8]; // encoding: latin-1
        frame.extend_from_slice(b"image/jpeg\0");
        frame.push(FRONT_COVER);
        frame.push(0); // empty description
        frame.extend_from_slice(jpeg);

        let mut body = Vec::new();
        body.extend_from_slice(b"APIC");
        if version >= 4 {
            let decoded_len = frame.len();
            let mut payload = if tag_unsync || frame_unsync {
                unsync(&frame)
            } else {
                frame
            };
            if dli {
                let mut with = to_syncsafe(decoded_len);
                with.extend_from_slice(&payload);
                payload = with;
            }
            body.extend_from_slice(&to_syncsafe(payload.len()));
            body.push(0);
            body.push(if frame_unsync { 0x02 } else { 0 } | if dli { 0x01 } else { 0 });
            body.extend_from_slice(&payload);
        } else {
            body.extend_from_slice(&(frame.len() as u32).to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&frame);
            if tag_unsync {
                // The whole body, headers and all, as one run.
                body = unsync(&body);
            }
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"ID3");
        out.push(version);
        out.push(0);
        out.push(if tag_unsync { 0x80 } else { 0 });
        out.extend_from_slice(&to_syncsafe(body.len()));
        out.extend_from_slice(&body);
        out
    }

    fn to_syncsafe(n: usize) -> Vec<u8> {
        vec![
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ]
    }

    /// A JPEG-shaped run of bytes containing every sequence unsynchronisation
    /// has to cope with, including the stuffed `FF 00` that broke the library.
    fn jpeg_like() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0");
        for i in 0..64u16 {
            v.extend_from_slice(&[0xFF, 0x00, (i & 0xff) as u8, 0x42]);
        }
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    #[test]
    fn a_picture_comes_back_whole_from_every_tag_shape() {
        let jpeg = jpeg_like();
        // v2.4 with the tag flag set and the frame carrying both its own
        // unsynchronisation flag and a data length indicator is exactly the
        // file that prompted this.
        for (version, tag_u, frame_u, dli) in [
            (4u8, true, true, true),
            (4, false, true, true),
            (4, true, false, false),
            (4, false, false, false),
            (3, true, false, false),
            (3, false, false, false),
        ] {
            let bytes = tag(version, tag_u, frame_u, dli, &jpeg);
            let p = picture(&bytes)
                .unwrap_or_else(|| panic!("no picture for {version} {tag_u} {frame_u} {dli}"));
            assert_eq!(
                p.data, jpeg,
                "picture came back altered for v2.{version} tag_unsync={tag_u} frame_unsync={frame_u} dli={dli}"
            );
            assert_eq!(p.kind, FRONT_COVER);
        }
    }

    #[test]
    fn a_utf16_description_does_not_leak_into_the_image() {
        // UTF-16 terminates on two zero bytes. Stopping at the first leaves a
        // stray byte at the head of the picture, which some decoders survive
        // and others do not.
        let jpeg = jpeg_like();
        let mut frame = vec![1u8]; // UTF-16
        frame.extend_from_slice(b"image/jpeg\0");
        frame.push(FRONT_COVER);
        frame.extend_from_slice(&[0xFF, 0xFE]); // BOM
        frame.extend_from_slice(&[b'h', 0, b'i', 0]);
        frame.extend_from_slice(&[0, 0]); // terminator
        frame.extend_from_slice(&jpeg);

        let p = parse(&frame, 4).expect("parses");
        assert_eq!(p.data, jpeg);
    }

    #[test]
    fn anything_that_is_not_a_tag_is_declined() {
        assert!(picture(b"").is_none());
        assert!(picture(b"not a tag at all, really").is_none());
        // A truncated tag must not panic or read past the end.
        let full = tag(4, true, true, true, &jpeg_like());
        for cut in [10, 20, 40, full.len() / 2, full.len() - 1] {
            let _ = picture(&full[..cut]);
        }
    }

    #[test]
    fn a_front_cover_is_preferred_over_whatever_came_first() {
        let jpeg = jpeg_like();
        let mut back = vec![0u8];
        back.extend_from_slice(b"image/jpeg\0");
        back.push(4); // back cover
        back.push(0);
        back.extend_from_slice(&[0xAB; 64]);

        let mut body = Vec::new();
        for (payload, _) in [
            (back, 4u8),
            (
                {
                    let mut f = vec![0u8];
                    f.extend_from_slice(b"image/jpeg\0");
                    f.push(FRONT_COVER);
                    f.push(0);
                    f.extend_from_slice(&jpeg);
                    f
                },
                FRONT_COVER,
            ),
        ] {
            body.extend_from_slice(b"APIC");
            body.extend_from_slice(&to_syncsafe(payload.len()));
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&payload);
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ID3");
        bytes.extend_from_slice(&[4, 0, 0]);
        bytes.extend_from_slice(&to_syncsafe(body.len()));
        bytes.extend_from_slice(&body);

        let p = picture(&bytes).expect("a picture");
        assert_eq!(p.kind, FRONT_COVER);
        assert_eq!(p.data, jpeg);
    }
}
