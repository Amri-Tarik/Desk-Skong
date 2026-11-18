//! Tiny deterministic xorshift RNG (no global state, no wall clock).

#[derive(Clone, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        // Scramble so nearby seeds diverge immediately; never zero.
        Self(seed.wrapping_mul(0x9E37_79B9).rotate_left(13) | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// Weighted pick; `None` when every weight is <= 0.
    pub fn pick_weighted<T: Copy>(&mut self, items: &[(T, f32)]) -> Option<T> {
        let total: f32 = items.iter().map(|(_, w)| w.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut r = self.f32() * total;
        for &(item, w) in items {
            r -= w.max(0.0);
            if r < 0.0 {
                return Some(item);
            }
        }
        items.iter().rev().find(|(_, w)| *w > 0.0).map(|(t, _)| *t)
    }
}
