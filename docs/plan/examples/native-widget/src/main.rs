mod gauge;
mod map_view;

use nd::prelude::*;

use gauge::gauge;
use map_view::{map_view, LatLng, Pin, PinSelected, RegionChanged};

struct Place {
    name: &'static str,
    at: LatLng,
    visited: bool,
}

struct Places {
    places: Vec<Place>,
    list: ListState<usize>,
    center: LatLng,
    zoom: f64,
}

impl Places {
    fn new(cx: &mut Context<Self>) -> Self {
        let places = vec![
            Place { name: "Lisbon", at: LatLng { lat: 38.7223, lng: -9.1393 }, visited: true },
            Place { name: "Porto", at: LatLng { lat: 41.1579, lng: -8.6291 }, visited: false },
            Place { name: "Las Palmas", at: LatLng { lat: 28.1235, lng: -15.4363 }, visited: true },
        ];
        let mut list = ListState::new(cx);
        list.set_items((0..places.len()).collect(), cx);
        let center = places[0].at;
        Self { places, list, center, zoom: 11.0 }
    }

    fn row(&mut self, ix: &usize, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let place = &self.places[*ix];
        source_row(if place.visited { Icon::CircleCheck } else { Icon::MapPin }, place.name)
    }
}

impl Render for Places {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visited = self.places.iter().filter(|p| p.visited).count();
        let pins = self.places.iter().map(|p| Pin { at: p.at, title: p.name.into() });

        scaffold()
            .toolbar(toolbar().leading(window_controls().embedded()).title("Places"))
            .sidebar(
                sidebar().child(
                    v_flex()
                        .size_full()
                        .child(source_list(&self.list, cx.processor(Self::row)).flex_1().on_select(cx.listener(
                            |this, e: &SelectEvent<usize>, _window, cx| {
                                this.center = this.places[e.key].at;
                                cx.notify();
                            },
                        )))
                        .child(
                            h_flex().p_3().justify_center().child(
                                gauge("visited", visited as f64).range(0.0, self.places.len() as f64).label("Visited"),
                            ),
                        ),
                ),
            )
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
    }
}

fn main() -> nd::Result<()> {
    Application::new(nd::manifest!()).run(|cx| {
        cx.open_window(WindowOptions::main("main").title("Places"), |_window, cx| cx.new(Places::new))
    })
}
