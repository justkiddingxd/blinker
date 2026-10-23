//! Just enough MP4 for the previews in messengers: the picture's size, and
//! the index (`moov`) moved to the front ("fast start"). Windows writes it
//! after the frames, and Telegram or Discord then won't play the link before
//! the whole file is in.

/// One box: its type, where it starts, its header and total length.
#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    start: usize,
    header: usize,
    len: usize,
}

impl Atom {
    fn body(&self) -> std::ops::Range<usize> {
        self.start + self.header..self.start + self.len
    }
}

/// The boxes in `b[from..to]`, in order; None if one runs past the end.
fn atoms(b: &[u8], from: usize, to: usize) -> Option<Vec<Atom>> {
    let mut out = Vec::new();
    let mut i = from;
    while i + 8 <= to {
        let size = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as u64;
        let kind: [u8; 4] = b[i + 4..i + 8].try_into().ok()?;
        let (header, len) = match size {
            0 => (8, (to - i) as u64),
            1 => (16, u64::from_be_bytes(b.get(i + 8..i + 16)?.try_into().ok()?)),
            n => (8, n),
        };
        if len < header as u64 || i as u64 + len > to as u64 {
            return None;
        }
        out.push(Atom { kind, start: i, header, len: len as usize });
        i += len as usize;
    }
    Some(out)
}

fn child(b: &[u8], parent: &Atom, kind: &[u8; 4]) -> Option<Atom> {
    let r = parent.body();
    atoms(b, r.start, r.end)?.into_iter().find(|a| &a.kind == kind)
}

/// Width and height of the first video track (`tkhd`, 16.16 fixed point).
pub fn video_size(b: &[u8]) -> Option<(u32, u32)> {
    let moov = atoms(b, 0, b.len())?.into_iter().find(|a| &a.kind == b"moov")?;
    let r = moov.body();
    for trak in atoms(b, r.start, r.end)?.into_iter().filter(|a| &a.kind == b"trak") {
        let Some(tkhd) = child(b, &trak, b"tkhd") else { continue };
        let body = &b[tkhd.body()];
        // version 1 has 64-bit times: 12 bytes more before the size
        let at = if body.first() == Some(&1) { 88 } else { 76 };
        let Some(wh) = body.get(at..at + 8) else { continue };
        let w = u32::from_be_bytes(wh[0..4].try_into().ok()?) >> 16;
        let h = u32::from_be_bytes(wh[4..8].try_into().ok()?) >> 16;
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    None
}

/// The file with its `moov` before the frames, the chunk offsets in it moved
/// by as much; unchanged if it already is or anything looks off.
pub fn fast_start(b: Vec<u8>) -> Vec<u8> {
    let Some(top) = atoms(&b, 0, b.len()) else { return b };
    let (Some(mi), Some(di)) = (top.iter().position(|a| &a.kind == b"moov"), top.iter().position(|a| &a.kind == b"mdat")) else {
        return b;
    };
    if mi < di {
        return b;
    }
    let moov = top[mi];
    let mut index = b[moov.start..moov.start + moov.len].to_vec();
    if !shift_offsets(&mut index, moov.len as u64) {
        return b;
    }
    let mut out = Vec::with_capacity(b.len());
    for (i, a) in top.iter().enumerate() {
        if i == di {
            out.extend_from_slice(&index);
        }
        if i != mi {
            out.extend_from_slice(&b[a.start..a.start + a.len]);
        }
    }
    out
}

/// Adds `by` to every chunk offset (`stco`, `co64`) in a `moov` box.
/// False if one wouldn't fit any more.
fn shift_offsets(moov: &mut [u8], by: u64) -> bool {
    fn walk(b: &mut [u8], from: usize, to: usize, by: u64) -> bool {
        let Some(list) = atoms(b, from, to) else { return false };
        for a in list {
            let r = a.body();
            match &a.kind {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" => {
                    if !walk(b, r.start, r.end, by) {
                        return false;
                    }
                }
                b"stco" | b"co64" => {
                    let wide = &a.kind == b"co64";
                    let Some(n) = b.get(r.start + 4..r.start + 8).map(|v| u32::from_be_bytes(v.try_into().unwrap()) as usize) else { return false };
                    let step = if wide { 8 } else { 4 };
                    if r.start + 8 + n * step > r.end {
                        return false;
                    }
                    for k in 0..n {
                        let at = r.start + 8 + k * step;
                        if wide {
                            let v = u64::from_be_bytes(b[at..at + 8].try_into().unwrap()) + by;
                            b[at..at + 8].copy_from_slice(&v.to_be_bytes());
                        } else {
                            let Some(v) = (u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as u64 + by).try_into().ok() else { return false };
                            b[at..at + 4].copy_from_slice(&u32::to_be_bytes(v));
                        }
                    }
                }
                _ => {}
            }
        }
        true
    }
    let len = moov.len();
    walk(moov, 0, len, by)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// ftyp, mdat with two chunks, then a moov pointing at them.
    fn sample() -> (Vec<u8>, usize) {
        let ftyp = atom(b"ftyp", b"isom\0\0\0\0");
        let mdat = atom(b"mdat", b"AAAABBBB");
        let first = ftyp.len() + 8;
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut stco = vec![0, 0, 0, 0, 0, 0, 0, 2];
        stco.extend_from_slice(&(first as u32).to_be_bytes());
        stco.extend_from_slice(&(first as u32 + 4).to_be_bytes());
        let stbl = atom(b"stbl", &atom(b"stco", &stco));
        let mdia = atom(b"mdia", &atom(b"minf", &stbl));
        let trak = atom(b"trak", &[atom(b"tkhd", &tkhd), mdia].concat());
        let moov = atom(b"moov", &trak);
        ([ftyp, mdat, moov].concat(), first)
    }

    /// A real file: `BLINKER_MP4` in, `BLINKER_MP4_OUT` out.
    #[test]
    fn real_file() {
        let (Ok(src), Ok(dst)) = (std::env::var("BLINKER_MP4"), std::env::var("BLINKER_MP4_OUT")) else { return };
        let b = std::fs::read(src).unwrap();
        println!("size {:?}", video_size(&b));
        std::fs::write(dst, fast_start(b)).unwrap();
    }

    #[test]
    fn size_and_fast_start() {
        let (file, first) = sample();
        assert_eq!(video_size(&file), Some((1920, 1080)));
        let fixed = fast_start(file.clone());
        assert_eq!(fixed.len(), file.len());
        let top = atoms(&fixed, 0, fixed.len()).unwrap();
        let kinds: Vec<_> = top.iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [*b"ftyp", *b"moov", *b"mdat"]);
        assert_eq!(video_size(&fixed), Some((1920, 1080)));
        // the offsets still land on the chunks
        let at = fixed.windows(4).position(|w| w == b"stco").unwrap() + 4 + 8;
        let o = u32::from_be_bytes(fixed[at..at + 4].try_into().unwrap()) as usize;
        assert!(o > first);
        assert_eq!(&fixed[o..o + 4], b"AAAA");
        let o2 = u32::from_be_bytes(fixed[at + 4..at + 8].try_into().unwrap()) as usize;
        assert_eq!(&fixed[o2..o2 + 4], b"BBBB");
        // already at the front: left alone
        assert_eq!(fast_start(fixed.clone()), fixed);
    }
}
