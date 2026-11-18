pub use glam::Vec2;

/// Integer rectangle, half-open: `left <= x < right`, `top <= y < bottom`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RectI {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RectI {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
    pub fn intersect(&self, o: &RectI) -> RectI {
        RectI::new(
            self.left.max(o.left),
            self.top.max(o.top),
            self.right.min(o.right),
            self.bottom.min(o.bottom),
        )
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    pub fn to_f(&self) -> RectF {
        RectF::new(self.left as f32, self.top as f32, self.right as f32, self.bottom as f32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self { left, top, right, bottom }
    }
    /// Rect from feet-center position (x, bottom y) and size.
    pub fn from_feet(feet: Vec2, size: Vec2) -> Self {
        Self::new(feet.x - size.x * 0.5, feet.y - size.y, feet.x + size.x * 0.5, feet.y)
    }
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
    pub fn center(&self) -> Vec2 {
        Vec2::new((self.left + self.right) * 0.5, (self.top + self.bottom) * 0.5)
    }
    pub fn overlaps(&self, o: &RectF) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
}
