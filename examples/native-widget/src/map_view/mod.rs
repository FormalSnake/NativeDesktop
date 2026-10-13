//! A map: MapKit's `MKMapView` on macOS, OpenStreetMap tiles drawn by the framework elsewhere.
//! Call sites use `map_view(..)` like any built-in element.

#[cfg(target_os = "macos")]
mod appkit;
mod drawn;

use nd::native::*;
use nd::prelude::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Pin {
    pub at: LatLng,
    pub title: SharedString,
}

#[derive(Clone, PartialEq)]
pub struct MapProps {
    pub center: LatLng,
    /// Web map zoom level: 0 is the whole world, 18 is a street.
    pub zoom: f64,
    pub pins: Vec<Pin>,
}

#[derive(Clone, Debug)]
pub struct RegionChanged {
    pub center: LatLng,
    pub zoom: f64,
}

#[derive(Clone, Debug)]
pub struct PinSelected {
    pub title: SharedString,
}

#[derive(Clone, Debug)]
pub enum MapEvent {
    RegionChanged(RegionChanged),
    PinSelected(PinSelected),
}

pub struct MapWidget;

impl NativeWidget for MapWidget {
    type Props = MapProps;
    type Event = MapEvent;

    fn semantics(props: &MapProps) -> Semantics {
        Semantics::new(Role::Group).label("Map").value(format!("{} places", props.pins.len()))
    }

    fn drawn(props: &MapProps, emit: Emit<MapEvent>, window: &mut Window, cx: &mut App) -> AnyElement {
        drawn::render(props, emit, window, cx)
    }

    fn backends() -> Backends<Self> {
        let backends = Backends::drawn_only();
        #[cfg(target_os = "macos")]
        let backends = backends.appkit(appkit::MapKit);
        backends
    }
}

#[derive(IntoElement)]
pub struct MapView {
    base: NativeBase<MapWidget>,
    props: MapProps,
}

pub fn map_view(id: impl Into<ElementId>, center: LatLng) -> MapView {
    MapView { base: NativeBase::new(id), props: MapProps { center, zoom: 12.0, pins: Vec::new() } }
}

impl MapView {
    pub fn zoom(mut self, zoom: f64) -> Self {
        self.props.zoom = zoom.clamp(0.0, 18.0);
        self
    }

    pub fn pins(mut self, pins: impl IntoIterator<Item = Pin>) -> Self {
        self.props.pins = pins.into_iter().collect();
        self
    }

    pub fn on_region_change(mut self, f: impl Fn(&RegionChanged, &mut Window, &mut App) + 'static) -> Self {
        self.base.on_event(move |event, window, cx| {
            if let MapEvent::RegionChanged(e) = event {
                f(e, window, cx);
            }
        });
        self
    }

    pub fn on_pin_select(mut self, f: impl Fn(&PinSelected, &mut Window, &mut App) + 'static) -> Self {
        self.base.on_event(move |event, window, cx| {
            if let MapEvent::PinSelected(e) = event {
                f(e, window, cx);
            }
        });
        self
    }
}

impl NativeElement for MapView {
    type Widget = MapWidget;

    fn base_mut(&mut self) -> &mut NativeBase<MapWidget> {
        &mut self.base
    }

    fn into_parts(self) -> (NativeBase<MapWidget>, MapProps) {
        (self.base, self.props)
    }
}
