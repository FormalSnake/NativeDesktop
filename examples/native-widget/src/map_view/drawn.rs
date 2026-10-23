//! Fallback for Linux, Windows and the web: the OpenStreetMap tiles that cover the view, pins drawn
//! on top, drag to pan. The map is controlled: panning emits `RegionChanged` and the app feeds the
//! new centre back in. Tiles download once into the image pipeline's disk cache.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use nd::native::*;
use nd::prelude::*;

use super::{LatLng, MapEvent, MapProps, RegionChanged};

const TILE: f64 = 256.0;
const MAX_TILE_ZOOM: f64 = 19.0;
const MAX_LAT: f64 = 85.0511;

pub fn render(props: &MapProps, emit: Emit<MapEvent>, _window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let (center, zoom) = (props.center, props.zoom);
    let pins: Vec<(f64, f64)> = props.pins.iter().map(|p| project(p.at)).collect();
    let (pin_fill, pin_ring) = (theme.accent, theme.surface.raised);
    // World width in pixels at the size the map was last painted, for turning a drag into degrees.
    let world = Rc::new(Cell::new(0.0));

    div()
        .size_full()
        .overflow_hidden()
        .relative()
        .bg(theme.surface.raised)
        .on_drag_move({
            let world = world.clone();
            move |drag: &DragMoveEvent, _window, _cx| {
                let w = world.get();
                if w <= 0.0 {
                    return;
                }
                let (x, y) = project(center);
                let moved = unproject(x - f64::from(drag.delta.x) / w, y - f64::from(drag.delta.y) / w);
                emit.emit(MapEvent::RegionChanged(RegionChanged { center: moved, zoom }));
            }
        })
        .child(
            canvas(move |bounds, paint, cx| {
                let w = world_width(center, zoom, bounds.size);
                world.set(w);
                let view = View::new(center, w, bounds);
                let z = (w / TILE).log2().round().clamp(0.0, MAX_TILE_ZOOM);
                let count = 2f64.powf(z);
                let (x0, y0) = view.world_at(0.0, 0.0);
                let (x1, y1) = view.world_at(f64::from(bounds.size.width), f64::from(bounds.size.height));
                let (ty0, ty1) =
                    (((y0 * count).floor()).max(0.0) as i64, ((y1 * count).floor()).min(count - 1.0) as i64);
                for ty in ty0..=ty1 {
                    for tx in (x0 * count).floor() as i64..=(x1 * count).floor() as i64 {
                        let wrapped = tx.rem_euclid(count as i64);
                        let url = format!("https://tile.openstreetmap.org/{}/{wrapped}/{ty}.png", z as u32);
                        // Edges snapped to whole pixels from both sides, so neighbours share them
                        // and no seam shows between tiles at a fractional scale.
                        let a = view.screen(tx as f64 / count, ty as f64 / count);
                        let b = view.screen((tx + 1) as f64 / count, (ty + 1) as f64 / count);
                        let (left, top) = (f32::from(a.x).round(), f32::from(a.y).round());
                        let (right, bottom) = (f32::from(b.x).round(), f32::from(b.y).round());
                        let rect = Bounds::new(point(px(left), px(top)), size(px(right - left), px(bottom - top)));
                        paint.image(cx.image(&url), rect);
                    }
                }
            })
            .size_full(),
        )
        // A second canvas so the pins paint over the tile images.
        .child(
            canvas(move |bounds, paint, _cx| {
                let view = View::new(center, world_width(center, zoom, bounds.size), bounds);
                for &(x, y) in &pins {
                    let at = view.screen(x, y);
                    paint.fill_circle(at, px(8.), pin_ring);
                    paint.fill_circle(at, px(6.), pin_fill);
                }
            })
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .child(
            text("© OpenStreetMap contributors")
                .absolute()
                .bottom_1()
                .right_2()
                .px_1()
                .bg(theme.surface.raised)
                .text_size(theme.type_scale.caption)
                .text_color(theme.text.secondary),
        )
        .into_any_element()
}

/// Maps the unit Web Mercator square onto the canvas, `center` in the middle.
struct View {
    center: (f64, f64),
    world: f64,
    bounds: Bounds<Pixels>,
}

impl View {
    fn new(center: LatLng, world: f64, bounds: Bounds<Pixels>) -> Self {
        Self { center: project(center), world, bounds }
    }

    fn screen(&self, x: f64, y: f64) -> Point<Pixels> {
        let mid = self.bounds.center();
        point(
            mid.x + px(((x - self.center.0) * self.world) as f32),
            mid.y + px(((y - self.center.1) * self.world) as f32),
        )
    }

    fn world_at(&self, sx: f64, sy: f64) -> (f64, f64) {
        let (w, h) = (f64::from(self.bounds.size.width), f64::from(self.bounds.size.height));
        (self.center.0 + (sx - w / 2.0) / self.world, self.center.1 + (sy - h / 2.0) / self.world)
    }
}

/// The world width in pixels that fits `360 / 2^zoom` degrees of longitude and of latitude around
/// `center` into `size`, as MapKit fits a region to its view.
fn world_width(center: LatLng, zoom: f64, size: Size<Pixels>) -> f64 {
    let span = 360.0 / 2f64.powf(zoom);
    let (_, top) = project(LatLng { lat: center.lat + span / 2.0, lng: 0.0 });
    let (_, bottom) = project(LatLng { lat: center.lat - span / 2.0, lng: 0.0 });
    let across = f64::from(size.width) / (span / 360.0).min(1.0);
    let down = f64::from(size.height) / (bottom - top).max(1e-9);
    across.min(down).max(TILE)
}

/// Web Mercator, as a fraction of the world: x from the antimeridian east, y from the top.
fn project(at: LatLng) -> (f64, f64) {
    let x = (at.lng + 180.0) / 360.0;
    let lat = at.lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0;
    (x, y)
}

fn unproject(x: f64, y: f64) -> LatLng {
    let lng = x * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y)).sinh().atan().to_degrees();
    LatLng { lat, lng }
}
