//! Fallback for Linux, Windows and the web: a 3x3 block of OpenStreetMap tiles around the centre,
//! pins drawn on top, drag to pan. The map is controlled: panning emits `RegionChanged` and the app
//! feeds the new centre back in.

use std::f64::consts::PI;

use nd::native::*;
use nd::prelude::*;

use super::{LatLng, MapEvent, MapProps, RegionChanged};

const TILE: f64 = 256.0;

pub fn render(props: &MapProps, emit: Emit<MapEvent>, _window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let zoom = props.zoom.round();
    let (cx_px, cy_px) = project(props.center, zoom);
    let (tile_x, tile_y) = ((cx_px / TILE).floor() as i64, (cy_px / TILE).floor() as i64);
    let pins: Vec<(f64, f64)> = props.pins.iter().map(|p| project(p.at, zoom)).collect();
    let pin_color = theme.accent;
    let center = props.center;

    div()
        .size_full()
        .overflow_hidden()
        .relative()
        .bg(theme.surface.raised)
        .on_drag_move(move |drag: &DragMoveEvent, _window, _cx| {
            let (x, y) = project(center, zoom);
            let moved = unproject(x - f64::from(drag.delta.x), y - f64::from(drag.delta.y), zoom);
            emit.emit(MapEvent::RegionChanged(RegionChanged { center: moved, zoom }));
        })
        .child(canvas(move |bounds, paint, cx| {
            let origin = |x: f64, y: f64| {
                point(bounds.center().x + px((x - cx_px) as f32), bounds.center().y + px((y - cy_px) as f32))
            };
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (tx, ty) = (tile_x + dx, tile_y + dy);
                    let url = format!("https://tile.openstreetmap.org/{}/{tx}/{ty}.png", zoom as u32);
                    let at = origin(tx as f64 * TILE, ty as f64 * TILE);
                    paint.image(cx.image(&url), Bounds::new(at, size(px(TILE as f32), px(TILE as f32))));
                }
            }
            for &(x, y) in &pins {
                paint.fill_circle(origin(x, y), px(6.), pin_color);
            }
        }))
        .child(
            text("© OpenStreetMap contributors")
                .absolute()
                .bottom_1()
                .right_2()
                .text_size(theme.type_scale.caption)
                .text_color(theme.text.secondary),
        )
        .into_any_element()
}

fn project(at: LatLng, zoom: f64) -> (f64, f64) {
    let scale = TILE * 2f64.powf(zoom);
    let x = (at.lng + 180.0) / 360.0 * scale;
    let lat = at.lat.to_radians();
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * scale;
    (x, y)
}

fn unproject(x: f64, y: f64, zoom: f64) -> LatLng {
    let scale = TILE * 2f64.powf(zoom);
    let lng = x / scale * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y / scale)).sinh().atan().to_degrees();
    LatLng { lat, lng }
}
