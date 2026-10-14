//! Box drawing, block elements, braille and the Powerline separators, drawn from geometry at
//! the cell's pixel size instead of taken from a font. Fonts draw these a little larger or
//! smaller than the cell, so borders show seams and separators miss their neighbour; geometry
//! tiles exactly (the zig branch's interim fit heuristic left `nerd-AFTER-zoom.png`'s boxes
//! broken). Both presenters rasterize through here into their glyph atlas.

/// A coverage mask, one byte per pixel, row major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

impl Mask {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self { width, height, alpha: vec![0; (width * height) as usize] }
    }

    /// Fills `[x0, x1) x [y0, y1)`, clipped, with full coverage.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.rect_alpha(x0, y0, x1, y1, 255);
    }

    fn rect_alpha(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, a: u8) {
        let (w, h) = (self.width as i32, self.height as i32);
        for y in y0.max(0)..y1.min(h) {
            for x in x0.max(0)..x1.min(w) {
                let p = &mut self.alpha[(y * w + x) as usize];
                *p = (*p).max(a);
            }
        }
    }

    /// Coverage from a predicate over the pixel area, 4 by 4 samples per pixel.
    fn shade(&mut self, inside: impl Fn(f32, f32) -> bool) {
        const N: u32 = 4;
        for y in 0..self.height {
            for x in 0..self.width {
                let mut hits = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let px = x as f32 + (sx as f32 + 0.5) / N as f32;
                        let py = y as f32 + (sy as f32 + 0.5) / N as f32;
                        if inside(px, py) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    let a = (hits * 255 / (N * N)) as u8;
                    let p = &mut self.alpha[(y * self.width + x) as usize];
                    *p = (*p).max(a);
                }
            }
        }
    }
}

/// True for codepoints [`draw`] handles.
pub fn is_sprite(c: char) -> bool {
    matches!(c as u32, 0x2500..=0x259F | 0x2800..=0x28FF | 0xE0B0..=0xE0BF)
}

/// The glyph for `c` in a `width` by `height` pixel cell, with `light` the pixel width of a
/// light stroke. `None` for codepoints outside the sprite ranges.
pub fn draw(c: char, width: u32, height: u32, light: u32) -> Option<Mask> {
    if !is_sprite(c) || width == 0 || height == 0 {
        return None;
    }
    let mut m = Mask::new(width, height);
    let light = light.max(1) as i32;
    let cp = c as u32;
    match cp {
        0x2500..=0x257F => box_drawing(&mut m, cp, light),
        0x2580..=0x259F => block(&mut m, cp),
        0x2800..=0x28FF => braille(&mut m, cp - 0x2800),
        _ => powerline(&mut m, cp, light),
    }
    Some(m)
}

/// Stroke weight of one arm: none, light, heavy, double.
#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    N,
    L,
    H,
    D,
}

/// Arms of each box drawing character, up, right, down, left. Dashes, arcs and diagonals are
/// drawn separately and have `None`.
fn arms(cp: u32) -> Option<[W; 4]> {
    use W::*;
    let table: [[W; 4]; 128] = [
        [N, L, N, L],
        [N, H, N, H],
        [L, N, L, N],
        [H, N, H, N], // 2500
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N], // 2504 dashes
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N], // 2508 dashes
        [N, L, L, N],
        [N, H, L, N],
        [N, L, H, N],
        [N, H, H, N], // 250C
        [N, N, L, L],
        [N, N, L, H],
        [N, N, H, L],
        [N, N, H, H], // 2510
        [L, L, N, N],
        [L, H, N, N],
        [H, L, N, N],
        [H, H, N, N], // 2514
        [L, N, N, L],
        [L, N, N, H],
        [H, N, N, L],
        [H, N, N, H], // 2518
        [L, L, L, N],
        [L, H, L, N],
        [H, L, L, N],
        [L, L, H, N], // 251C
        [H, L, H, N],
        [H, H, L, N],
        [L, H, H, N],
        [H, H, H, N], // 2520
        [L, N, L, L],
        [L, N, L, H],
        [H, N, L, L],
        [L, N, H, L], // 2524
        [H, N, H, L],
        [H, N, L, H],
        [L, N, H, H],
        [H, N, H, H], // 2528
        [N, L, L, L],
        [N, L, L, H],
        [N, H, L, L],
        [N, H, L, H], // 252C
        [N, L, H, L],
        [N, L, H, H],
        [N, H, H, L],
        [N, H, H, H], // 2530
        [L, L, N, L],
        [L, L, N, H],
        [L, H, N, L],
        [L, H, N, H], // 2534
        [H, L, N, L],
        [H, L, N, H],
        [H, H, N, L],
        [H, H, N, H], // 2538
        [L, L, L, L],
        [L, L, L, H],
        [L, H, L, L],
        [L, H, L, H], // 253C
        [H, L, L, L],
        [L, L, H, L],
        [H, L, H, L],
        [H, L, L, H], // 2540
        [H, H, L, L],
        [L, L, H, H],
        [L, H, H, L],
        [H, H, L, H], // 2544
        [L, H, H, H],
        [H, L, H, H],
        [H, H, H, L],
        [H, H, H, H], // 2548
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N], // 254C dashes
        [N, D, N, D],
        [D, N, D, N],
        [N, D, L, N],
        [N, L, D, N], // 2550
        [N, D, D, N],
        [N, N, L, D],
        [N, N, D, L],
        [N, N, D, D], // 2554
        [L, D, N, N],
        [D, L, N, N],
        [D, D, N, N],
        [L, N, N, D], // 2558
        [D, N, N, L],
        [D, N, N, D],
        [L, D, L, N],
        [D, L, D, N], // 255C
        [D, D, D, N],
        [L, N, L, D],
        [D, N, D, L],
        [D, N, D, D], // 2560
        [N, D, L, D],
        [N, L, D, L],
        [N, D, D, D],
        [L, D, N, D], // 2564
        [D, L, N, L],
        [D, D, N, D],
        [L, D, L, D],
        [D, L, D, L], // 2568
        [D, D, D, D],
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N], // 256C, arcs
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N],
        [N, N, N, N], // 2570 arc, diagonals
        [N, N, N, L],
        [L, N, N, N],
        [N, L, N, N],
        [N, N, L, N], // 2574
        [N, N, N, H],
        [H, N, N, N],
        [N, H, N, N],
        [N, N, H, N], // 2578
        [N, H, N, L],
        [L, N, H, N],
        [N, L, N, H],
        [H, N, L, N], // 257C
    ];
    let arms = table[(cp - 0x2500) as usize];
    (arms != [N, N, N, N]).then_some(arms)
}

fn box_drawing(m: &mut Mask, cp: u32, light: i32) {
    let (w, h) = (m.width as i32, m.height as i32);
    let heavy = light * 2;
    // Where a stroke of thickness `t` sits across an axis of length `len`, centred.
    let span = |len: i32, t: i32| {
        let a = (len - t) / 2;
        (a, a + t)
    };
    match cp {
        0x2504..=0x250B | 0x254C..=0x254F => {
            let (count, thick, vertical) = match cp {
                0x2504 => (3, light, false),
                0x2505 => (3, heavy, false),
                0x2506 => (3, light, true),
                0x2507 => (3, heavy, true),
                0x2508 => (4, light, false),
                0x2509 => (4, heavy, false),
                0x250A => (4, light, true),
                0x250B => (4, heavy, true),
                0x254C => (2, light, false),
                0x254D => (2, heavy, false),
                0x254E => (2, light, true),
                _ => (2, heavy, true),
            };
            let len = if vertical { h } else { w };
            let gap = (len / (count * 4)).max(1);
            for i in 0..count {
                let a = i * len / count + gap / 2;
                let b = (i + 1) * len / count - (gap - gap / 2);
                if vertical {
                    let (x0, x1) = span(w, thick);
                    m.rect(x0, a, x1, b);
                } else {
                    let (y0, y1) = span(h, thick);
                    m.rect(a, y0, b, y1);
                }
            }
            return;
        }
        0x256D..=0x2570 => {
            // A quarter circle from the middle of one edge to the middle of the next, its
            // radius the shorter half of the cell.
            let (cx0, _) = span(w, light);
            let (cy0, _) = span(h, light);
            let mx = cx0 as f32 + light as f32 / 2.;
            let my = cy0 as f32 + light as f32 / 2.;
            let r = mx.min(my).min(w as f32 - mx).min(h as f32 - my).max(1.);
            let (ox, oy, sx, sy) = match cp {
                0x256D => (mx + r, my + r, 1., 1.),   // down and right
                0x256E => (mx - r, my + r, -1., 1.),  // down and left
                0x256F => (mx - r, my - r, -1., -1.), // up and left
                _ => (mx + r, my - r, 1., -1.),       // up and right
            };
            let half = light as f32 / 2.;
            m.shade(|x, y| {
                let (dx, dy) = (x - ox, y - oy);
                let in_quadrant = dx * sx <= 0. && dy * sy <= 0.;
                in_quadrant && ((dx * dx + dy * dy).sqrt() - r).abs() <= half
            });
            // Straight runs from the arc's ends to the cell edges.
            match cp {
                0x256D => {
                    m.rect(ox as i32, cy0, w, cy0 + light);
                    m.rect(cx0, oy as i32, cx0 + light, h);
                }
                0x256E => {
                    m.rect(0, cy0, ox.ceil() as i32, cy0 + light);
                    m.rect(cx0, oy as i32, cx0 + light, h);
                }
                0x256F => {
                    m.rect(0, cy0, ox.ceil() as i32, cy0 + light);
                    m.rect(cx0, 0, cx0 + light, oy.ceil() as i32);
                }
                _ => {
                    m.rect(ox as i32, cy0, w, cy0 + light);
                    m.rect(cx0, 0, cx0 + light, oy.ceil() as i32);
                }
            }
            return;
        }
        0x2571..=0x2573 => {
            let half = light as f32 / 2. * 1.2;
            let (fw, fh) = (w as f32, h as f32);
            let len = (fw * fw + fh * fh).sqrt();
            let rising = move |x: f32, y: f32| ((fh * x + fw * y - fw * fh) / len).abs() <= half;
            let falling = move |x: f32, y: f32| ((fh * x - fw * y) / len).abs() <= half;
            match cp {
                0x2571 => m.shade(rising),
                0x2572 => m.shade(falling),
                _ => m.shade(|x, y| rising(x, y) || falling(x, y)),
            }
            return;
        }
        _ => {}
    }
    let Some([up, right, down, left]) = arms(cp) else { return };
    let thick = |a: W| match a {
        W::N => 0,
        W::L => light,
        W::H => heavy,
        W::D => light,
    };
    // Double strokes are two light lines a light stroke apart.
    let off = light / 2 + light;
    let vertical_double = up == W::D || down == W::D;
    let horizontal_double = left == W::D || right == W::D;
    let h_extent = thick(left).max(thick(right));
    let v_extent = thick(up).max(thick(down));
    // The band the horizontal strokes cover across the centre, and the vertical ones'.
    let (hc0, hc1) = if horizontal_double {
        let (a0, a1) = span(h, light);
        (a0 - off, a1 + off)
    } else if h_extent > 0 {
        span(h, h_extent)
    } else {
        (h / 2, h / 2)
    };
    let (vc0, vc1) = if vertical_double {
        let (a0, a1) = span(w, light);
        (a0 - off, a1 + off)
    } else if v_extent > 0 {
        span(w, v_extent)
    } else {
        (w / 2, w / 2)
    };
    let vertical_arm = |m: &mut Mask, arm: W, from: i32, to: i32| match arm {
        W::N => {}
        W::D => {
            let (a0, a1) = span(w, light);
            m.rect(a0 - off, from, a1 - off, to);
            m.rect(a0 + off, from, a1 + off, to);
        }
        _ => {
            let (a0, a1) = span(w, thick(arm));
            m.rect(a0, from, a1, to);
        }
    };
    let horizontal_arm = |m: &mut Mask, arm: W, from: i32, to: i32| match arm {
        W::N => {}
        W::D => {
            let (a0, a1) = span(h, light);
            m.rect(from, a0 - off, to, a1 - off);
            m.rect(from, a0 + off, to, a1 + off);
        }
        _ => {
            let (a0, a1) = span(h, thick(arm));
            m.rect(from, a0, to, a1);
        }
    };
    // Each arm runs from its edge to the far side of the strokes crossing it, so corners close
    // without overshooting; a lone half line stops at the centre.
    vertical_arm(m, up, 0, hc1);
    vertical_arm(m, down, hc0, h);
    horizontal_arm(m, left, 0, vc1);
    horizontal_arm(m, right, vc0, w);
}

fn block(m: &mut Mask, cp: u32) {
    let (w, h) = (m.width as i32, m.height as i32);
    let eighth_h = |n: i32| (h * n + 4) / 8;
    let eighth_w = |n: i32| (w * n + 4) / 8;
    let (hw, hh) = (w / 2, h / 2);
    match cp {
        0x2580 => m.rect(0, 0, w, hh),
        0x2581..=0x2588 => {
            let n = (cp - 0x2580) as i32;
            m.rect(0, h - eighth_h(n), w, h);
        }
        0x2589..=0x258F => {
            let n = 8 - (cp - 0x2588) as i32;
            m.rect(0, 0, eighth_w(n), h);
        }
        0x2590 => m.rect(hw, 0, w, h),
        0x2591 => m.rect_alpha(0, 0, w, h, 64),
        0x2592 => m.rect_alpha(0, 0, w, h, 128),
        0x2593 => m.rect_alpha(0, 0, w, h, 191),
        0x2594 => m.rect(0, 0, w, eighth_h(1)),
        0x2595 => m.rect(w - eighth_w(1), 0, w, h),
        _ => {
            // Quadrants: upper left, upper right, lower left, lower right.
            let q = match cp {
                0x2596 => [false, false, true, false],
                0x2597 => [false, false, false, true],
                0x2598 => [true, false, false, false],
                0x2599 => [true, false, true, true],
                0x259A => [true, false, false, true],
                0x259B => [true, true, true, false],
                0x259C => [true, true, false, true],
                0x259D => [false, true, false, false],
                0x259E => [false, true, true, false],
                _ => [false, true, true, true],
            };
            if q[0] {
                m.rect(0, 0, hw, hh);
            }
            if q[1] {
                m.rect(hw, 0, w, hh);
            }
            if q[2] {
                m.rect(0, hh, hw, h);
            }
            if q[3] {
                m.rect(hw, hh, w, h);
            }
        }
    }
}

fn braille(m: &mut Mask, bits: u32) {
    let (w, h) = (m.width as f32, m.height as f32);
    // Dot n of the pattern: column and row in the 2 by 4 grid.
    const DOTS: [(u32, u32); 8] = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
    let r = (w / 4.).min(h / 8.) * 0.8;
    let centres: Vec<(f32, f32)> = DOTS
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, (c, row))| (w * (0.25 + 0.5 * *c as f32), h * (0.125 + 0.25 * *row as f32)))
        .collect();
    m.shade(|x, y| centres.iter().any(|(cx, cy)| (x - cx).powi(2) + (y - cy).powi(2) <= r * r));
}

fn powerline(m: &mut Mask, cp: u32, light: i32) {
    let (w, h) = (m.width as f32, m.height as f32);
    let half = light as f32 / 2. * 1.2;
    // Distance from (x, y) to the segment a-b.
    let seg = |x: f32, y: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
        ((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt()
    };
    match cp {
        // Solid triangles pointing right and left, full cell height.
        0xE0B0 => m.shade(|x, y| x <= w * (1. - (2. * y / h - 1.).abs())),
        0xE0B2 => m.shade(|x, y| w - x <= w * (1. - (2. * y / h - 1.).abs())),
        // Their outlines.
        0xE0B1 => m.shade(|x, y| seg(x, y, (0., 0.), (w, h / 2.)).min(seg(x, y, (w, h / 2.), (0., h))) <= half),
        0xE0B3 => m.shade(|x, y| seg(x, y, (w, 0.), (0., h / 2.)).min(seg(x, y, (0., h / 2.), (w, h))) <= half),
        // Half discs, flat side on the cell edge.
        0xE0B4 => m.shade(|x, y| (x / w).powi(2) + ((y - h / 2.) / (h / 2.)).powi(2) <= 1.),
        0xE0B6 => m.shade(|x, y| ((w - x) / w).powi(2) + ((y - h / 2.) / (h / 2.)).powi(2) <= 1.),
        0xE0B5 => m.shade(|x, y| {
            let d = ((x / w).powi(2) + ((y - h / 2.) / (h / 2.)).powi(2)).sqrt();
            (d - 1.).abs() * w.min(h / 2.) <= half
        }),
        0xE0B7 => m.shade(|x, y| {
            let d = (((w - x) / w).powi(2) + ((y - h / 2.) / (h / 2.)).powi(2)).sqrt();
            (d - 1.).abs() * w.min(h / 2.) <= half
        }),
        // Corner triangles: lower left, lower right, upper left, upper right.
        0xE0B8 => m.shade(|x, y| y >= h * x / w),
        0xE0BA => m.shade(|x, y| y >= h * (1. - x / w)),
        0xE0BC => m.shade(|x, y| y <= h * (1. - x / w)),
        0xE0BE => m.shade(|x, y| y <= h * x / w),
        0xE0B9 => m.shade(|x, y| seg(x, y, (0., 0.), (w, h)) <= half),
        0xE0BB => m.shade(|x, y| seg(x, y, (w, 0.), (0., h)) <= half),
        0xE0BD => m.shade(|x, y| seg(x, y, (w, 0.), (0., h)) <= half),
        _ => m.shade(|x, y| seg(x, y, (0., 0.), (w, h)) <= half),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(m: &Mask, y: u32) -> Vec<u8> {
        m.alpha[(y * m.width) as usize..((y + 1) * m.width) as usize].to_vec()
    }

    #[test]
    fn horizontal_line_spans_the_cell_edge_to_edge() {
        let m = draw('─', 9, 18, 1).unwrap();
        let lit: Vec<u32> = (0..18).filter(|y| row(&m, *y).iter().all(|a| *a == 255)).collect();
        assert_eq!(lit, [8], "one full-width row at the centre");
    }

    #[test]
    fn corners_and_tees_meet_at_the_centre() {
        let m = draw('┌', 9, 18, 1).unwrap();
        // The horizontal arm starts where the vertical one is.
        let r = row(&m, 8);
        assert_eq!(r[..4], [0, 0, 0, 0]);
        assert!(r[4..].iter().all(|a| *a == 255));
        // The vertical arm runs to the bottom edge, not past the top.
        assert_eq!(m.alpha[(17 * 9 + 4) as usize], 255);
        assert_eq!(m.alpha[4], 0);
    }

    #[test]
    fn full_block_and_halves() {
        assert!(draw('█', 8, 16, 1).unwrap().alpha.iter().all(|a| *a == 255));
        let upper = draw('▀', 8, 16, 1).unwrap();
        assert!(row(&upper, 7).iter().all(|a| *a == 255));
        assert!(row(&upper, 8).iter().all(|a| *a == 0));
    }

    #[test]
    fn powerline_triangle_touches_both_edges() {
        let m = draw('\u{e0b0}', 10, 20, 1).unwrap();
        // Full height at the left edge, a point at the right edge's middle.
        assert!(m.alpha[0] > 0 && m.alpha[(19 * 10) as usize] > 0);
        assert!(row(&m, 10)[9] > 0);
        assert_eq!(row(&m, 0)[9], 0);
    }

    #[test]
    fn non_sprites_are_left_to_fonts() {
        assert!(draw('a', 8, 16, 1).is_none());
        assert!(!is_sprite('\u{f115}'));
    }
}
