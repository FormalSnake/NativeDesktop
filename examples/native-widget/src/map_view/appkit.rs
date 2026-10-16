use std::cell::RefCell;

use nd::native::appkit::objc2::rc::Retained;
use nd::native::appkit::objc2::runtime::ProtocolObject;
use nd::native::appkit::objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use nd::native::appkit::objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use nd::native::appkit::{AppKitCx, AppKitWidget, AvailableSize};
use nd::native::*;
use nd::prelude::*;
use objc2_core_location::CLLocationCoordinate2D;
use objc2_map_kit::{
    MKAnnotation, MKCoordinateRegion, MKCoordinateSpan, MKMapView, MKMapViewDelegate, MKPointAnnotation,
};

use super::{LatLng, MapEvent, MapProps, MapWidget, Pin, PinSelected, RegionChanged};

pub struct MapKit;

// docs:begin map-viewport
/// What the map shows, in the props' own terms. MKMapView fits any region to its aspect ratio,
/// so the region it reports differs from the one we set; `Synced` remembers the reported one and
/// the app feeding it back causes no second `setRegion`.
type Viewport = (LatLng, f64);
// docs:end map-viewport

impl AppKitWidget<MapWidget> for MapKit {
    type View = MKMapView;
    type State = Retained<MapDelegate>;

    // docs:begin map-create
    fn create(&self, props: &MapProps, cx: &mut AppKitCx<MapWidget>) -> (Retained<MKMapView>, Self::State) {
        let map = unsafe { MKMapView::new(cx.mtm()) };
        let delegate = MapDelegate::new(cx.emitter(), cx.mtm());
        // MKMapView holds its delegate weakly; the node state keeps it alive.
        unsafe { map.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        apply(&map, &delegate, None, props, cx);
        (map, delegate)
    }
    // docs:end map-create

    fn update(
        &self,
        map: &MKMapView,
        delegate: &mut Self::State,
        old: &MapProps,
        new: &MapProps,
        cx: &mut AppKitCx<MapWidget>,
    ) {
        apply(map, delegate, Some(old), new, cx);
    }

    fn measure(&self, _map: &MKMapView, _props: &MapProps, _available: AvailableSize) -> Size<Pixels> {
        // No natural size: the app sizes the map with flex or explicit dimensions.
        Size::default()
    }
}

// docs:begin map-apply
fn apply(
    map: &MKMapView,
    delegate: &MapDelegate,
    old: Option<&MapProps>,
    new: &MapProps,
    cx: &mut AppKitCx<MapWidget>,
) {
    let viewport: Viewport = (new.center, new.zoom);
    delegate.ivars().viewport.borrow_mut().apply(&viewport, |&(center, zoom)| {
        let region = MKCoordinateRegion { center: to_cl(center), span: span_for_zoom(zoom) };
        // Programmatic: the regionDidChange this triggers is reported, not emitted.
        cx.programmatic(|| unsafe { map.setRegion_animated(region, old.is_some()) });
        shown(map)
    });
    if old.is_none_or(|o| o.pins != new.pins) {
        set_pins(map, &new.pins);
    }
}
// docs:end map-apply

fn set_pins(map: &MKMapView, pins: &[Pin]) {
    unsafe {
        map.removeAnnotations(&map.annotations());
        for pin in pins {
            let annotation = MKPointAnnotation::new();
            annotation.setCoordinate(to_cl(pin.at));
            annotation.setTitle(Some(&NSString::from_str(&pin.title)));
            map.addAnnotation(ProtocolObject::from_ref(&*annotation));
        }
    }
}

pub struct MapIvars {
    emit: Emit<MapEvent>,
    viewport: RefCell<Synced<Viewport>>,
}

// docs:begin map-delegate
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MapIvars]
    pub struct MapDelegate;

    unsafe impl NSObjectProtocol for MapDelegate {}

    unsafe impl MKMapViewDelegate for MapDelegate {
        #[unsafe(method(mapView:regionDidChangeAnimated:))]
        fn region_did_change(&self, map: &MKMapView, _animated: bool) {
            let (center, zoom) = shown(map);
            // Busy while `apply` runs the setter, which records what the map shows itself.
            if let Ok(mut viewport) = self.ivars().viewport.try_borrow_mut() {
                viewport.report((center, zoom));
            }
            // Queued, never delivered inside this callback; dropped if a programmatic set caused it.
            self.ivars().emit.emit(MapEvent::RegionChanged(RegionChanged { center, zoom }));
        }

        #[unsafe(method(mapView:didSelectAnnotation:))]
        fn did_select(&self, _map: &MKMapView, annotation: &ProtocolObject<dyn MKAnnotation>) {
            if let Some(title) = unsafe { annotation.title() } {
                self.ivars().emit.emit(MapEvent::PinSelected(PinSelected { title: title.to_string().into() }));
            }
        }
    }
);
// docs:end map-delegate

impl MapDelegate {
    fn new(emit: Emit<MapEvent>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MapIvars { emit, viewport: RefCell::new(Synced::default()) });
        unsafe { msg_send![super(this), init] }
    }
}

fn shown(map: &MKMapView) -> Viewport {
    let region = unsafe { map.region() };
    (LatLng { lat: region.center.latitude, lng: region.center.longitude }, zoom_for_span(region.span))
}

fn to_cl(at: LatLng) -> CLLocationCoordinate2D {
    CLLocationCoordinate2D { latitude: at.lat, longitude: at.lng }
}

fn span_for_zoom(zoom: f64) -> MKCoordinateSpan {
    let degrees = 360.0 / 2f64.powf(zoom);
    MKCoordinateSpan { latitudeDelta: degrees, longitudeDelta: degrees }
}

fn zoom_for_span(span: MKCoordinateSpan) -> f64 {
    (360.0 / span.longitudeDelta.max(1e-6)).log2()
}
