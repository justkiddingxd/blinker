//! The pointer, drawn into the recording by blinker itself. Windows can put
//! it into the captured frames (`IsCursorCaptureEnabled`), but that turns the
//! system's pointer into a software one, and over some fullscreen games
//! (Minecraft) the pointer then disappears from the game itself. So the
//! capture leaves it out, and each frame gets the pointer as it is: its
//! shape, its place, and nothing while the game hides it.

use std::collections::HashMap;
use std::ptr::null_mut;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::core::Interface;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Which pointer is up and where (its hot spot, in screen pixels).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    handle: isize,
    x: i32,
    y: i32,
}

/// A pointer's picture: premultiplied BGRA, and its hot spot.
struct Shape {
    w: u32,
    h: u32,
    hot: (i32, i32),
    px: Vec<u8>,
}

pub struct Pointer {
    d2d: ID2D1Factory,
    /// Pictures by pointer handle (None: one that couldn't be read).
    shapes: HashMap<isize, Option<Shape>>,
}

impl Pointer {
    pub fn new() -> Option<Pointer> {
        let d2d = unsafe { D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }.ok()?;
        Some(Pointer { d2d, shapes: HashMap::new() })
    }

    /// The pointer now; None when it's hidden (a game took it away).
    pub fn now() -> Option<Seen> {
        unsafe {
            let mut ci: CURSORINFO = std::mem::zeroed();
            ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) == 0 || ci.flags & CURSOR_SHOWING == 0 || ci.hCursor.is_null() {
                return None;
            }
            Some(Seen { handle: ci.hCursor as isize, x: ci.ptScreenPos.x, y: ci.ptScreenPos.y })
        }
    }

    /// Whether `seen` shows on a frame `size` px whose top-left is at
    /// `origin` on the screen.
    pub fn within(&mut self, seen: Seen, size: (u32, u32), origin: (i32, i32)) -> bool {
        let Some(s) = self.shape(seen.handle) else { return false };
        let (x, y) = (seen.x - s.hot.0 - origin.0, seen.y - s.hot.1 - origin.1);
        x < size.0 as i32 && y < size.1 as i32 && x + s.w as i32 > 0 && y + s.h as i32 > 0
    }

    /// Draws `seen` onto `tex`.
    pub fn draw(&mut self, tex: &ID3D11Texture2D, origin: (i32, i32), seen: Seen) {
        let d2d = self.d2d.clone();
        let Some(s) = self.shape(seen.handle) else { return };
        unsafe {
            let Ok(surface) = tex.cast::<IDXGISurface>() else { return };
            let format = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: format,
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let Ok(rt) = d2d.CreateDxgiSurfaceRenderTarget(&surface, &props) else { return };
            let bprops = D2D1_BITMAP_PROPERTIES { pixelFormat: format, dpiX: 96.0, dpiY: 96.0 };
            let Ok(bmp) = rt.CreateBitmap(D2D_SIZE_U { width: s.w, height: s.h }, Some(s.px.as_ptr() as *const _), s.w * 4, &bprops) else { return };
            let (x, y) = ((seen.x - s.hot.0 - origin.0) as f32, (seen.y - s.hot.1 - origin.1) as f32);
            let rect = D2D_RECT_F { left: x, top: y, right: x + s.w as f32, bottom: y + s.h as f32 };
            rt.BeginDraw();
            rt.DrawBitmap(&bmp, Some(&rect), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None);
            let _ = rt.EndDraw(None, None);
        }
    }

    fn shape(&mut self, handle: isize) -> Option<&Shape> {
        self.shapes.entry(handle).or_insert_with(|| unsafe { read_shape(handle as HCURSOR) }).as_ref()
    }
}

/// A bitmap's pixels as 32-bit BGRA rows, top first.
unsafe fn pixels(dc: HDC, bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut b: BITMAP = std::mem::zeroed();
        if GetObjectW(bmp, std::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut _) == 0 || b.bmWidth <= 0 || b.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (b.bmWidth as u32, b.bmHeight as u32);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w as i32;
        info.bmiHeader.biHeight = -(h as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut px = vec![0u8; (w * h * 4) as usize];
        if GetDIBits(dc, bmp, 0, h, px.as_mut_ptr() as *mut _, &mut info, DIB_RGB_COLORS) == 0 {
            return None;
        }
        Some((w, h, px))
    }
}

/// The picture of the pointer `h`: a color one with its alpha (or the mask
/// for an old one without), or a black-and-white one from its two masks.
unsafe fn read_shape(h: HCURSOR) -> Option<Shape> {
    unsafe {
        let mut ii: ICONINFO = std::mem::zeroed();
        if GetIconInfo(h, &mut ii) == 0 {
            return None;
        }
        let dc = GetDC(null_mut());
        let color = if ii.hbmColor.is_null() { None } else { pixels(dc, ii.hbmColor) };
        let mask = if ii.hbmMask.is_null() { None } else { pixels(dc, ii.hbmMask) };
        ReleaseDC(null_mut(), dc);
        if !ii.hbmColor.is_null() {
            DeleteObject(ii.hbmColor);
        }
        if !ii.hbmMask.is_null() {
            DeleteObject(ii.hbmMask);
        }
        let hot = (ii.xHotspot as i32, ii.yHotspot as i32);
        let shape = match (color, mask) {
            (Some((w, hh, mut px)), mask) => {
                if px.chunks_exact(4).all(|p| p[3] == 0) {
                    // no alpha of its own: opaque where the mask is black
                    for (i, p) in px.chunks_exact_mut(4).enumerate() {
                        let clear = mask.as_ref().is_some_and(|(mw, _, m)| *mw == w && m.get(i * 4).is_some_and(|&v| v != 0));
                        p[3] = if clear { 0 } else { 255 };
                    }
                }
                Shape { w, h: hh, hot, px }
            }
            // black and white: the AND mask on top of the XOR one
            (None, Some((w, hh, m))) if hh >= 2 => {
                let h2 = hh / 2;
                let mut px = vec![0u8; (w * h2 * 4) as usize];
                for i in 0..(w * h2) as usize {
                    let and = m[i * 4] != 0;
                    let xor = m[(i + (w * h2) as usize) * 4] != 0;
                    // (inverting pixels are shown white: the I-beam over text)
                    let (v, a) = match (and, xor) {
                        (false, false) => (0, 255),
                        (false, true) => (255, 255),
                        (true, false) => (0, 0),
                        (true, true) => (255, 255),
                    };
                    px[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, a]);
                }
                Shape { w, h: h2, hot, px }
            }
            _ => return None,
        };
        // premultiplied, as Direct2D takes it
        let mut shape = shape;
        for p in shape.px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = (*c as u32 * a / 255) as u8;
            }
        }
        Some(shape)
    }
}

#[cfg(test)]
mod tests {
    /// The arrow drawn on a black frame at a known place shows there: white
    /// and black pixels around its hot spot, nothing far from it.
    #[test]
    fn drawn_on_a_frame() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, LoadCursorW};
        let gpu = crate::record::video::Gpu::new().expect("gpu");
        let (w, h) = (200u32, 120u32);
        let tex = gpu.texture(w, h, false).expect("texture");
        let mut p = super::Pointer::new().expect("d2d");
        let arrow = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) } as isize;
        // the frame's top-left is at (1000, 500) on the screen
        let seen = super::Seen { handle: arrow, x: 1060, y: 540 };
        assert!(p.within(seen, (w, h), (1000, 500)));
        assert!(!p.within(super::Seen { x: 5000, ..seen }, (w, h), (1000, 500)));
        p.draw(&tex, (1000, 500), seen);
        let img = gpu.read_rgba(&tex, w, h).expect("read");
        let lit = |x0: u32, y0: u32, x1: u32, y1: u32| (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img.get_pixel(x, y)[0] > 128).count();
        assert!(lit(60, 40, 90, 80) > 20, "no arrow at its place");
        assert_eq!(lit(150, 0, 200, 30), 0, "paint far from it");
    }

    /// The arrow, the hand, the I-beam: each reads into a picture with
    /// something visible in it and its hot spot inside.
    #[test]
    fn system_pointers() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_HAND, IDC_IBEAM, LoadCursorW};
        for id in [IDC_ARROW, IDC_HAND, IDC_IBEAM] {
            let h = unsafe { LoadCursorW(std::ptr::null_mut(), id) };
            let s = unsafe { super::read_shape(h) }.expect("a picture");
            assert!(s.w > 0 && s.h > 0);
            assert!(s.px.chunks_exact(4).any(|p| p[3] > 0), "all clear");
            assert!(s.hot.0 < s.w as i32 && s.hot.1 < s.h as i32);
        }
    }
}
