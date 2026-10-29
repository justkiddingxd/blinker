//! Files blinker writes carry the picture or the video and nothing else:
//! no time they were made, no version of Windows, no names of programs.
//! Media Foundation stamps every MP4 with its build (a `uuid` box) and the
//! time in UTC; AVFoundation adds its own; text, EXIF and comments may come
//! with a picture. On by default (`Config::strip_metadata`).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Takes the metadata out of `path` (just written by blinker) if the
/// setting says so. A failure is logged and the file stays as it was.
pub fn file(path: &Path) {
    if !crate::config::peek().strip_metadata {
        return;
    }
    if let Err(e) = clean(path) {
        crate::config::log::error(format!("metadata: {}: {e}", path.display()));
    }
}

/// The metadata out of `path`, whatever the setting.
pub fn clean(path: &Path) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        // the boxes are patched in place: a recording isn't read whole
        "mp4" | "mov" | "m4a" => mp4(path).map_err(|e| e.to_string()),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "mp3" => {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            let out = match ext.as_str() {
                "png" => png(&data),
                "jpg" | "jpeg" => jpeg(&data),
                "webp" => webp(&data),
                "gif" => gif(&data),
                _ => mp3(&data),
            }?;
            match out {
                Some(clean) => replace(path, &clean),
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// `path` becomes `data`, through a file next to it (never half written).
fn replace(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("strip");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

// ---------------------------------------------------------------------------
// MP4 / MOV: boxes that only describe become `free` (same size, so nothing
// moves and the offsets to the media stay right); the times in the headers
// become zero.

/// Boxes with nothing but metadata in them.
const META_BOXES: [&[u8; 4]; 3] = [b"uuid", b"udta", b"meta"];

fn mp4(path: &Path) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let len = f.metadata()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        let (size, kind, head) = box_at(&mut f, at, len)?;
        if size < 8 {
            break;
        }
        if META_BOXES.contains(&&kind) {
            // renamed, and what it said wiped too
            f.seek(SeekFrom::Start(at + 4))?;
            f.write_all(b"free")?;
            if size - head <= 1 << 20 {
                f.seek(SeekFrom::Start(at + head))?;
                f.write_all(&vec![0u8; (size - head) as usize])?;
            }
        } else if &kind == b"moov" && size <= 64 << 20 {
            let mut moov = vec![0u8; size as usize];
            f.seek(SeekFrom::Start(at))?;
            f.read_exact(&mut moov)?;
            if clean_boxes(&mut moov[head as usize..]) {
                f.seek(SeekFrom::Start(at))?;
                f.write_all(&moov)?;
            }
        }
        at += size;
    }
    f.flush()
}

/// A box's size, type and header length at `at`.
fn box_at(f: &mut std::fs::File, at: u64, len: u64) -> std::io::Result<(u64, [u8; 4], u64)> {
    let mut h = [0u8; 16];
    f.seek(SeekFrom::Start(at))?;
    let n = if at + 16 <= len { 16 } else { 8 };
    f.read_exact(&mut h[..n])?;
    let kind = [h[4], h[5], h[6], h[7]];
    let size = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
    Ok(match size {
        0 => (len - at, kind, 8),
        1 if n == 16 => (u64::from_be_bytes(h[8..16].try_into().unwrap()), kind, 16),
        _ => (size, kind, 8),
    })
}

/// The boxes in `buf` (and in the containers among them): metadata boxes
/// turned to `free`, the times of the movie, its tracks and media zeroed.
/// Whether anything changed.
fn clean_boxes(buf: &mut [u8]) -> bool {
    let mut changed = false;
    let mut at = 0usize;
    while at + 8 <= buf.len() {
        let mut size = u32::from_be_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        let mut head = 8;
        if size == 1 && at + 16 <= buf.len() {
            size = u64::from_be_bytes(buf[at + 8..at + 16].try_into().unwrap()) as usize;
            head = 16;
        } else if size == 0 {
            size = buf.len() - at;
        }
        if size < head || at + size > buf.len() {
            break;
        }
        let kind: [u8; 4] = buf[at + 4..at + 8].try_into().unwrap();
        let body = at + head;
        match &kind {
            k if META_BOXES.contains(&k) => {
                buf[at + 4..at + 8].copy_from_slice(b"free");
                buf[body..at + size].fill(0);
                changed = true;
            }
            b"mvhd" | b"tkhd" | b"mdhd" if size >= head + 4 => {
                // version 0: two 32-bit times; version 1: two 64-bit ones
                let times = if buf[body] == 1 { 16 } else { 8 };
                if body + 4 + times <= at + size {
                    let t = &mut buf[body + 4..body + 4 + times];
                    if t.iter().any(|&b| b != 0) {
                        t.fill(0);
                        changed = true;
                    }
                }
            }
            b"trak" | b"mdia" | b"minf" | b"edts" | b"moov" => {
                changed |= clean_boxes(&mut buf[body..at + size]);
            }
            _ => {}
        }
        at += size;
    }
    changed
}

// ---------------------------------------------------------------------------
// Pictures. Each returns the file without its metadata, or None if it had
// none (then the file isn't touched).

/// PNG: text, time and EXIF chunks out.
fn png(d: &[u8]) -> Result<Option<Vec<u8>>, String> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !d.starts_with(SIG) {
        return Err("not a png".into());
    }
    let mut out = SIG.to_vec();
    let mut dropped = false;
    let mut at = SIG.len();
    while at + 12 <= d.len() {
        let len = u32::from_be_bytes(d[at..at + 4].try_into().unwrap()) as usize;
        let end = at + 12 + len;
        if end > d.len() {
            return Err("a broken chunk".into());
        }
        let kind = &d[at + 4..at + 8];
        if matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" | b"eXIf") {
            dropped = true;
        } else {
            out.extend_from_slice(&d[at..end]);
        }
        at = end;
    }
    Ok(dropped.then_some(out))
}

/// JPEG: EXIF and XMP (APP1), Photoshop's IPTC (APP13) and comments out;
/// the color profile and Adobe's marker stay (the colors depend on them).
fn jpeg(d: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if !d.starts_with(&[0xFF, 0xD8]) {
        return Err("not a jpeg".into());
    }
    let mut out = vec![0xFF, 0xD8];
    let mut dropped = false;
    let mut at = 2;
    while at + 4 <= d.len() {
        if d[at] != 0xFF {
            return Err("a broken segment".into());
        }
        let marker = d[at + 1];
        // the picture itself: copied as it is to the end
        if marker == 0xDA {
            out.extend_from_slice(&d[at..]);
            break;
        }
        let len = u16::from_be_bytes([d[at + 2], d[at + 3]]) as usize;
        let end = at + 2 + len;
        if end > d.len() {
            return Err("a broken segment".into());
        }
        if matches!(marker, 0xE1 | 0xED | 0xFE) {
            dropped = true;
        } else {
            out.extend_from_slice(&d[at..end]);
        }
        at = end;
    }
    Ok(dropped.then_some(out))
}

/// WebP: the EXIF and XMP chunks out, their flags off.
fn webp(d: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WEBP" {
        return Err("not a webp".into());
    }
    let mut body = Vec::new();
    let mut dropped = false;
    let mut at = 12;
    while at + 8 <= d.len() {
        let len = u32::from_le_bytes(d[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = (at + 8 + len + (len & 1)).min(d.len());
        let kind = &d[at..at + 4];
        if kind == b"EXIF" || kind == b"XMP " {
            dropped = true;
        } else {
            body.extend_from_slice(&d[at..end]);
        }
        at = end;
    }
    if !dropped {
        return Ok(None);
    }
    // VP8X says which chunks follow: no EXIF (bit 3), no XMP (bit 2)
    if body.starts_with(b"VP8X") && body.len() > 8 {
        body[8] &= !0x0C;
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok(Some(out))
}

/// GIF: comments and other programs' blocks out; the looping block stays.
fn gif(d: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if d.len() < 13 || !d.starts_with(b"GIF8") {
        return Err("not a gif".into());
    }
    let table = |flags: u8| if flags & 0x80 != 0 { 3usize << ((flags & 7) + 1) } else { 0 };
    let mut at = 13 + table(d[10]);
    let mut out = d[..at.min(d.len())].to_vec();
    let mut dropped = false;
    // the length of the sub-blocks starting at `i`, the empty one included
    let blocks = |mut i: usize| -> Option<usize> {
        let from = i;
        loop {
            let n = *d.get(i)? as usize;
            i += 1 + n;
            if n == 0 {
                return Some(i - from);
            }
        }
    };
    while at < d.len() {
        match d[at] {
            0x3B => {
                out.push(0x3B);
                break;
            }
            0x21 if at + 2 <= d.len() => {
                let label = d[at + 1];
                let len = 2 + blocks(at + 2).ok_or("a broken block")?;
                let app = &d[at + 2..(at + 14).min(d.len())];
                let keep = match label {
                    0xFE => false,
                    0xFF => app.get(1..12).is_some_and(|id| id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0"),
                    _ => true,
                };
                if keep {
                    out.extend_from_slice(&d[at..at + len]);
                } else {
                    dropped = true;
                }
                at += len;
            }
            0x2C if at + 10 <= d.len() => {
                let head = 10 + table(d[at + 9]) + 1;
                let len = head + blocks(at + head).ok_or("a broken frame")?;
                out.extend_from_slice(&d[at..at + len]);
                at += len;
            }
            _ => return Err("a broken block".into()),
        }
    }
    Ok(dropped.then_some(out))
}

/// MP3: the ID3 tags at the start and the end out.
fn mp3(d: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut from = 0;
    if d.len() >= 10 && d.starts_with(b"ID3") {
        let size = d[6..10].iter().fold(0usize, |s, &b| (s << 7) | (b & 0x7F) as usize);
        let footer = if d[5] & 0x10 != 0 { 10 } else { 0 };
        from = (10 + size + footer).min(d.len());
    }
    let mut to = d.len();
    if to >= from + 128 && &d[to - 128..to - 125] == b"TAG" {
        to -= 128;
    }
    Ok((from > 0 || to < d.len()).then(|| d[from..to].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("blinker-strip-{}-{name}", std::process::id()))
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    /// As Media Foundation writes them: its build in a `uuid`, the time in
    /// the headers, an empty `udta`. After: all `free`, the times zero, the
    /// media where it was.
    #[test]
    fn mp4_boxes() {
        let header = |v: u8| {
            let mut b = vec![v, 0, 0, 0];
            b.extend_from_slice(&0xE1u32.to_be_bytes().repeat(2));
            b.extend_from_slice(&[7; 12]);
            b
        };
        let trak = mp4_box(b"trak", &[mp4_box(b"tkhd", &header(0)), mp4_box(b"mdia", &mp4_box(b"mdhd", &header(0)))].concat());
        let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &header(0)), trak, mp4_box(b"udta", &mp4_box(b"meta", b"\0\0\0\0x"))].concat());
        let mdat = mp4_box(b"mdat", b"the pictures");
        let file = [mp4_box(b"ftyp", b"isom\0\0\0\0"), mp4_box(b"uuid", b"6.2.09200.0"), mdat.clone(), moov].concat();
        let path = tmp("a.mp4");
        std::fs::write(&path, &file).unwrap();
        clean(&path).unwrap();
        let out = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(out.len(), file.len());
        assert!(!out.windows(4).any(|w| w == b"uuid" || w == b"udta"));
        assert!(!out.windows(5).any(|w| w == b"09200"), "the build is left");
        assert!(!out.windows(4).any(|w| w == [0, 0, 0, 0xE1]), "a time is left");
        let at = file.windows(4).position(|w| w == b"mdat").unwrap() - 4;
        assert_eq!(&out[at..at + mdat.len()], &mdat[..]);
    }

    #[test]
    fn pictures() {
        // a PNG with a text chunk
        let mut png_data = Vec::new();
        image::RgbImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png_data), image::ImageFormat::Png).unwrap();
        let mut text = (5u32).to_be_bytes().to_vec();
        text.extend_from_slice(b"tEXtAuth\0");
        text.extend_from_slice(&[0, 0, 0, 0]);
        let with = [&png_data[..33], &text, &png_data[33..]].concat();
        let out = png(&with).unwrap().expect("the text goes");
        assert_eq!(out, png_data);
        assert!(png(&png_data).unwrap().is_none());
        assert!(image::load_from_memory(&out).is_ok());

        // a JPEG with EXIF and a comment
        let mut jpg = Vec::new();
        image::RgbImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        let exif = [&[0xFF, 0xE1, 0, 8][..], b"Exif\0\0"].concat();
        let com = [&[0xFF, 0xFE, 0, 5][..], b"hi!"].concat();
        let with = [&jpg[..2], &exif, &com, &jpg[2..]].concat();
        let out = jpeg(&with).unwrap().expect("exif goes");
        assert_eq!(out, jpg);
        assert!(image::load_from_memory(&out).is_ok());

        // ID3 at both ends of an MP3
        let frames = vec![0xFFu8, 0xFB, 1, 2, 3];
        let mut tagged = b"ID3\x04\0\0\0\0\0\x02ab".to_vec();
        tagged.extend_from_slice(&frames);
        tagged.extend_from_slice(b"TAG");
        tagged.extend_from_slice(&[0; 125]);
        assert_eq!(mp3(&tagged).unwrap().unwrap(), frames);
    }

    /// `BLINKER_TEST_STRIP` = a file to clean (a copy: it changes in place);
    /// a video must still open after.
    #[test]
    fn real_file() {
        let Ok(path) = std::env::var("BLINKER_TEST_STRIP") else { return };
        let path = std::path::PathBuf::from(path);
        clean(&path).unwrap();
        if path.extension().is_some_and(|e| e == "mp4") {
            let (mut r, _) = crate::media::VideoReader::open(&path, 320, 180).expect("opens");
            assert!(r.next().unwrap().is_some(), "a frame");
        }
    }

    #[test]
    fn gif_comment() {
        let mut g = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff".to_vec();
        let loops = b"\x21\xff\x0bNETSCAPE2.0\x03\x01\0\0\0".to_vec();
        let comment = b"\x21\xfe\x03hey\0".to_vec();
        let frame = b"\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0".to_vec();
        let plain = [g.clone(), loops.clone(), frame.clone(), vec![0x3B]].concat();
        g.extend_from_slice(&loops);
        g.extend_from_slice(&comment);
        g.extend_from_slice(&frame);
        g.push(0x3B);
        assert_eq!(gif(&g).unwrap().unwrap(), plain);
        assert!(image::load_from_memory(&plain).is_ok());
    }
}
