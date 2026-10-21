//! Screen geometry shared by every platform: rectangles in physical pixels
//! and monitors (bounds, work area, UI scale).

/// Integer rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
    pub fn from_ltrb(l: i32, t: i32, r: i32, b: i32) -> Self {
        Self::new(l, t, r - l, b - t)
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= l || b <= t {
            IRect::default()
        } else {
            IRect::from_ltrb(l, t, r, b)
        }
    }
    pub fn offset(&self, dx: i32, dy: i32) -> IRect {
        IRect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub rect: IRect,
    pub work: IRect,
    pub scale: f32,
}
