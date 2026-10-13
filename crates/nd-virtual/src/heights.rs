//! Row heights with prefix sums: a Fenwick tree over `f64`, so a row's top, the row at an
//! offset and a height change cost O(log n). Offsets are `f64` because a 100k-row list runs past
//! the 2^24 points where `f32` stops resolving half a pixel.

#[derive(Clone, Debug, Default)]
pub(crate) struct Heights {
    heights: Vec<f64>,
    tree: Vec<f64>,
}

impl Heights {
    pub fn from_vec(heights: Vec<f64>) -> Self {
        let mut tree = heights.clone();
        for i in 0..tree.len() {
            let parent = i | (i + 1);
            if parent < tree.len() {
                tree[parent] += tree[i];
            }
        }
        Self { heights, tree }
    }

    pub fn get(&self, ix: usize) -> f64 {
        self.heights[ix]
    }

    pub fn set(&mut self, ix: usize, height: f64) {
        let delta = height - self.heights[ix];
        if delta == 0. {
            return;
        }
        self.heights[ix] = height;
        let mut i = ix;
        while i < self.tree.len() {
            self.tree[i] += delta;
            i |= i + 1;
        }
    }

    /// Sum of the heights of rows `0..ix`: the top of row `ix`.
    pub fn top(&self, ix: usize) -> f64 {
        let mut sum = 0.;
        let mut i = ix.min(self.tree.len());
        while i > 0 {
            sum += self.tree[i - 1];
            i &= i - 1;
        }
        sum
    }

    pub fn total(&self) -> f64 {
        self.top(self.heights.len())
    }

    /// The row containing offset `y`: the last row whose top is at or above it. Clamped to the
    /// rows there are; `None` for an empty list.
    pub fn index_at(&self, y: f64) -> Option<usize> {
        let n = self.tree.len();
        if n == 0 {
            return None;
        }
        if y <= 0. {
            return Some(0);
        }
        // Largest `pos` with top(pos) <= y, by descending the implicit tree.
        let mut pos = 0;
        let mut rest = y;
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next <= n && self.tree[next - 1] <= rest {
                pos = next;
                rest -= self.tree[next - 1];
            }
            step >>= 1;
        }
        Some(pos.min(n - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tops_and_lookups_match_a_linear_scan() {
        let hs: Vec<f64> = (0..1000).map(|i| ((i * 37) % 91 + 1) as f64).collect();
        let mut h = Heights::from_vec(hs.clone());
        h.set(500, 300.);
        let mut hs = hs;
        hs[500] = 300.;
        let mut top = 0.;
        for (i, height) in hs.iter().enumerate() {
            assert_eq!(h.top(i), top);
            assert_eq!(h.index_at(top), Some(i));
            assert_eq!(h.index_at(top + height - 0.5), Some(i));
            top += height;
        }
        assert_eq!(h.total(), top);
        assert_eq!(h.index_at(top + 10.), Some(999));
    }
}
