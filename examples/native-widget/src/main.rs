mod gauge;
mod map_view;

use nd::prelude::*;

use gauge::gauge;
use map_view::{LatLng, Pin, PinSelected, RegionChanged, map_view};

struct Place {
    name: &'static str,
    at: LatLng,
    visited: bool,
}

struct Places {
    places: Vec<Place>,
    selected: usize,
    center: LatLng,
    zoom: f64,
}

impl Places {
    fn new(_cx: &mut Context<Self>) -> Self {
        let places = vec![
            Place { name: "Antwerpen", at: LatLng { lat: 51.2194, lng: 4.4025 }, visited: true },
            Place { name: "Arrecife", at: LatLng { lat: 28.9630, lng: -13.5477 }, visited: false },
            Place { name: "Las Palmas", at: LatLng { lat: 28.1235, lng: -15.4363 }, visited: true },
        ];
        let (center, zoom) = fit(places.iter().map(|p| p.at));
        Self { places, selected: 0, center, zoom }
    }
}

/// The centre and zoom that show every point with a margin.
fn fit(points: impl Iterator<Item = LatLng> + Clone) -> (LatLng, f64) {
    let (lat0, lat1) = points.clone().fold((90.0f64, -90.0f64), |(lo, hi), p| (lo.min(p.lat), hi.max(p.lat)));
    let (lng0, lng1) = points.fold((180.0f64, -180.0f64), |(lo, hi), p| (lo.min(p.lng), hi.max(p.lng)));
    let span = (lat1 - lat0).max(lng1 - lng0) * 1.6;
    let center = LatLng { lat: (lat0 + lat1) / 2.0, lng: (lng0 + lng1) / 2.0 };
    (center, (360.0 / span.max(1e-3)).log2())
}

impl Render for Places {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visited = self.places.iter().filter(|p| p.visited).count();
        let pins = self.places.iter().map(|p| Pin { at: p.at, title: p.name.into() });
        let items = self.places.iter().enumerate().map(|(ix, p)| {
            source_item(ix.to_string(), p.name).icon(if p.visited { Icon::CircleCheck } else { Icon::MapPin })
        });

        scaffold()
            .toolbar(toolbar().leading(window_controls().embedded()).title("Places"))
            .sidebar(
                sidebar().child(
                    v_flex()
                        .size_full()
                        .child(
                            source_list("places").items(items).selected(self.selected.to_string()).flex_1().on_select(
                                cx.listener(|this, e: &SourceSelected, _window, cx| {
                                    if let Ok(ix) = e.id.parse::<usize>() {
                                        this.selected = ix;
                                        this.center = this.places[ix].at;
                                        cx.notify();
                                    }
                                }),
                            ),
                        )
                        .child(h_flex().p_3().justify_center().child(
                            gauge("visited", visited as f64).range(0.0, self.places.len() as f64).label("Visited"),
                        )),
                ),
            )
            // docs:begin places-view
            .content(
                map_view("map", self.center)
                    .zoom(self.zoom)
                    .pins(pins)
                    .size_full()
                    .on_region_change(cx.listener(|this, e: &RegionChanged, _window, cx| {
                        this.center = e.center;
                        this.zoom = e.zoom;
                        cx.notify();
                    }))
                    .on_pin_select(cx.listener(|this, e: &PinSelected, _window, cx| {
                        if let Some(place) = this.places.iter_mut().find(|p| p.name == e.title.as_ref()) {
                            place.visited = true;
                            cx.notify();
                        }
                    })),
            )
        // docs:end places-view
    }
}

fn main() -> nd::Result<()> {
    Application::new(nd::manifest!())
        .run(|cx| cx.open_window(WindowOptions::main("main").title("Places"), |_window, cx| cx.new(Places::new)))
}
