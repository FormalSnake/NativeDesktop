//! SVG to drawing segments: the subset Lucide uses (path, circle, ellipse, rect, line, polyline,
//! polygon). Shared by the build script, which bakes the vendored set, and by `register`, which
//! parses an app's own icons at run time.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Control one, control two, end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub filled: bool,
    pub segs: Vec<Seg>,
}

/// The pieces of `svg`, in document order, in its own coordinates, plus its viewBox size.
pub fn parse(svg: &str) -> Result<(Vec<Piece>, f32), String> {
    let mut pieces = Vec::new();
    let mut view = 24.;
    let mut rest = svg;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        if rest.starts_with("!--") {
            rest = rest.find("-->").map_or("", |i| &rest[i + 3..]);
            continue;
        }
        let end = rest.find('>').ok_or("unterminated tag")?;
        let tag = rest[..end].trim_end_matches('/');
        rest = &rest[end + 1..];
        let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
        let (name, attrs) = (&tag[..name_end], Attrs(&tag[name_end..]));
        let number = |key: &str| attrs.get(key).map(|v| v.trim().parse::<f32>().map_err(|_| format!("bad {key}")));
        let num = |key: &str| number(key).transpose().map(|v| v.unwrap_or(0.));
        let filled = attrs.get("fill").is_some_and(|f| f != "none");
        let segs = match name {
            "svg" => {
                if let Some(vb) = attrs.get("viewBox") {
                    let parts: Vec<f32> = vb.split_whitespace().filter_map(|p| p.parse().ok()).collect();
                    if parts.len() == 4 && parts[0] == 0. && parts[1] == 0. && parts[2] == parts[3] {
                        view = parts[2];
                    } else {
                        return Err(format!("viewBox must be square and start at 0 0: {vb}"));
                    }
                }
                continue;
            }
            "path" => path(attrs.get("d").ok_or("path without d")?)?,
            "circle" => ellipse(num("cx")?, num("cy")?, num("r")?, num("r")?),
            "ellipse" => ellipse(num("cx")?, num("cy")?, num("rx")?, num("ry")?),
            "rect" => {
                let (rx, ry) = (number("rx").transpose()?, number("ry").transpose()?);
                let (rx, ry) = (rx.or(ry).unwrap_or(0.), ry.or(rx).unwrap_or(0.));
                rect(num("x")?, num("y")?, num("width")?, num("height")?, rx, ry)
            }
            "line" => vec![Seg::Move(num("x1")?, num("y1")?), Seg::Line(num("x2")?, num("y2")?)],
            "polyline" | "polygon" => {
                let nums: Vec<f32> = attrs
                    .get("points")
                    .ok_or("poly without points")?
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().map_err(|_| "bad point"))
                    .collect::<Result<_, _>>()?;
                let mut segs: Vec<Seg> = nums
                    .chunks_exact(2)
                    .enumerate()
                    .map(|(i, p)| if i == 0 { Seg::Move(p[0], p[1]) } else { Seg::Line(p[0], p[1]) })
                    .collect();
                if name == "polygon" {
                    segs.push(Seg::Close);
                }
                segs
            }
            _ => continue,
        };
        pieces.push(Piece { filled, segs });
    }
    if pieces.is_empty() {
        return Err("no drawable elements".into());
    }
    Ok((pieces, view))
}

struct Attrs<'a>(&'a str);

impl Attrs<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        let mut rest = self.0;
        while let Some(eq) = rest.find('=') {
            let name = rest[..eq].trim();
            let after = rest[eq + 1..].trim_start();
            let quote = after.chars().next()?;
            let close = after[1..].find(quote)?;
            let value = &after[1..1 + close];
            if name == key {
                return Some(value);
            }
            rest = &after[close + 2..];
        }
        None
    }
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<Seg> {
    let mut segs = vec![Seg::Move(cx + rx, cy)];
    for (sx, sy) in [(0., 1.), (-1., 0.), (0., -1.), (1., 0.)] {
        arc(&mut segs, (cx + rx * sx, cy + ry * sy), rx, ry, false, true);
    }
    segs.push(Seg::Close);
    segs
}

fn rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<Seg> {
    let (rx, ry) = (rx.min(w / 2.), ry.min(h / 2.));
    if rx <= 0. || ry <= 0. {
        return vec![Seg::Move(x, y), Seg::Line(x + w, y), Seg::Line(x + w, y + h), Seg::Line(x, y + h), Seg::Close];
    }
    let mut s = vec![Seg::Move(x + rx, y), Seg::Line(x + w - rx, y)];
    arc(&mut s, (x + w, y + ry), rx, ry, false, true);
    s.push(Seg::Line(x + w, y + h - ry));
    arc(&mut s, (x + w - rx, y + h), rx, ry, false, true);
    s.push(Seg::Line(x + rx, y + h));
    arc(&mut s, (x, y + h - ry), rx, ry, false, true);
    s.push(Seg::Line(x, y + ry));
    arc(&mut s, (x + rx, y), rx, ry, false, true);
    s.push(Seg::Close);
    s
}

struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Scanner<'_> {
    fn skip(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| b.is_ascii_whitespace() || *b == b',') {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let b = *self.bytes.get(self.at)?;
        b.is_ascii_alphabetic().then(|| {
            self.at += 1;
            b
        })
    }

    fn more_numbers(&mut self) -> bool {
        self.skip();
        self.bytes.get(self.at).is_some_and(|b| !b.is_ascii_alphabetic())
    }

    fn number(&mut self) -> Result<f32, String> {
        self.skip();
        let start = self.at;
        if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        let mut dot = false;
        while let Some(&b) = self.bytes.get(self.at) {
            match b {
                b'0'..=b'9' => self.at += 1,
                b'.' if !dot => {
                    dot = true;
                    self.at += 1;
                }
                b'e' | b'E' => {
                    self.at += 1;
                    if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                        self.at += 1;
                    }
                }
                _ => break,
            }
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("bad number at {start}"))
    }

    /// An arc flag: one digit, which the compact form writes with no separator.
    fn flag(&mut self) -> Result<bool, String> {
        self.skip();
        let flag = match self.bytes.get(self.at) {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err("bad arc flag".into()),
        };
        self.at += 1;
        Ok(flag)
    }
}

fn path(d: &str) -> Result<Vec<Seg>, String> {
    let mut sc = Scanner { bytes: d.as_bytes(), at: 0 };
    let mut out = Vec::new();
    let (mut x, mut y) = (0f32, 0f32);
    let (mut sx, mut sy) = (0f32, 0f32);
    // The last control point, mirrored by S and T; `None` after any other command.
    let mut ctrl: Option<(f32, f32)> = None;
    let mut quad_ctrl: Option<(f32, f32)> = None;
    while let Some(cmd) = sc.command() {
        let rel = cmd.is_ascii_lowercase();
        let upper = cmd.to_ascii_uppercase();
        if upper == b'Z' {
            out.push(Seg::Close);
            (x, y) = (sx, sy);
            ctrl = None;
            quad_ctrl = None;
            continue;
        }
        let mut first = true;
        while first || sc.more_numbers() {
            let implicit_line = !first;
            first = false;
            let (ox, oy) = if rel { (x, y) } else { (0., 0.) };
            let (mut next_ctrl, mut next_quad) = (None, None);
            match upper {
                b'M' => {
                    let (nx, ny) = (ox + sc.number()?, oy + sc.number()?);
                    if implicit_line {
                        out.push(Seg::Line(nx, ny));
                    } else {
                        out.push(Seg::Move(nx, ny));
                        (sx, sy) = (nx, ny);
                    }
                    (x, y) = (nx, ny);
                }
                b'L' => {
                    (x, y) = (ox + sc.number()?, oy + sc.number()?);
                    out.push(Seg::Line(x, y));
                }
                b'H' => {
                    x = ox + sc.number()?;
                    out.push(Seg::Line(x, y));
                }
                b'V' => {
                    y = oy + sc.number()?;
                    out.push(Seg::Line(x, y));
                }
                b'C' => {
                    let c1 = (ox + sc.number()?, oy + sc.number()?);
                    let c2 = (ox + sc.number()?, oy + sc.number()?);
                    let e = (ox + sc.number()?, oy + sc.number()?);
                    out.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, e.0, e.1));
                    next_ctrl = Some(c2);
                    (x, y) = e;
                }
                b'S' => {
                    let c1 = ctrl.map_or((x, y), |c| (2. * x - c.0, 2. * y - c.1));
                    let c2 = (ox + sc.number()?, oy + sc.number()?);
                    let e = (ox + sc.number()?, oy + sc.number()?);
                    out.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, e.0, e.1));
                    next_ctrl = Some(c2);
                    (x, y) = e;
                }
                b'Q' | b'T' => {
                    let c = if upper == b'Q' {
                        (ox + sc.number()?, oy + sc.number()?)
                    } else {
                        quad_ctrl.map_or((x, y), |c| (2. * x - c.0, 2. * y - c.1))
                    };
                    let e = (ox + sc.number()?, oy + sc.number()?);
                    out.push(Seg::Cubic(
                        x + 2. / 3. * (c.0 - x),
                        y + 2. / 3. * (c.1 - y),
                        e.0 + 2. / 3. * (c.0 - e.0),
                        e.1 + 2. / 3. * (c.1 - e.1),
                        e.0,
                        e.1,
                    ));
                    next_quad = Some(c);
                    (x, y) = e;
                }
                b'A' => {
                    let (rx, ry, rot) = (sc.number()?, sc.number()?, sc.number()?);
                    let (large, sweep) = (sc.flag()?, sc.flag()?);
                    let e = (ox + sc.number()?, oy + sc.number()?);
                    arc_from(&mut out, (x, y), e, (rx, ry), rot, large, sweep);
                    (x, y) = e;
                }
                other => return Err(format!("unsupported path command {}", other as char)),
            }
            ctrl = next_ctrl;
            quad_ctrl = next_quad;
        }
    }
    Ok(out)
}

/// An unrotated arc from the last point of `out` to `to`.
fn arc(out: &mut Vec<Seg>, to: (f32, f32), rx: f32, ry: f32, large: bool, sweep: bool) {
    let from = out.iter().rev().find_map(|s| match *s {
        Seg::Move(x, y) | Seg::Line(x, y) | Seg::Cubic(_, _, _, _, x, y) => Some((x, y)),
        Seg::Close => None,
    });
    if let Some(from) = from {
        arc_from(out, from, to, (rx, ry), 0., large, sweep);
    }
}

/// SVG's endpoint arc as cubic segments (SVG 1.1 implementation notes, F.6).
fn arc_from(out: &mut Vec<Seg>, from: (f32, f32), to: (f32, f32), radii: (f32, f32), rot_deg: f32, large: bool, sweep: bool) {
    let (mut rx, mut ry) = (radii.0.abs() as f64, radii.1.abs() as f64);
    if from == to {
        return;
    }
    if rx == 0. || ry == 0. {
        out.push(Seg::Line(to.0, to.1));
        return;
    }
    let (sin, cos) = (rot_deg as f64).to_radians().sin_cos();
    let (dx, dy) = ((from.0 - to.0) as f64 / 2., (from.1 - to.1) as f64 / 2.);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1. {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = (num / den).max(0.).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let (cxp, cyp) = (coef * rx * y1 / ry, -coef * ry * x1 / rx);
    let cx = cos * cxp - sin * cyp + (from.0 + to.0) as f64 / 2.;
    let cy = sin * cxp + cos * cyp + (from.1 + to.1) as f64 / 2.;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let theta = angle(1., 0., (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut delta = angle((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && delta > 0. {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0. {
        delta += std::f64::consts::TAU;
    }
    let n = (delta.abs() / (std::f64::consts::FRAC_PI_2 + 1e-9)).ceil().max(1.) as usize;
    let step = delta / n as f64;
    let k = 4. / 3. * (step / 4.).tan();
    let map = |(px, py): (f64, f64)| ((cos * px - sin * py + cx) as f32, (sin * px + cos * py + cy) as f32);
    for i in 0..n {
        let (a, b) = (theta + step * i as f64, theta + step * (i + 1) as f64);
        let (p0, p3) = ((rx * a.cos(), ry * a.sin()), (rx * b.cos(), ry * b.sin()));
        let d0 = (-rx * a.sin(), ry * a.cos());
        let d3 = (-rx * b.sin(), ry * b.cos());
        let c1 = map((p0.0 + k * d0.0, p0.1 + k * d0.1));
        let c2 = map((p3.0 - k * d3.0, p3.1 - k * d3.1));
        let e = if i + 1 == n { to } else { map(p3) };
        out.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, e.0, e.1));
    }
}
