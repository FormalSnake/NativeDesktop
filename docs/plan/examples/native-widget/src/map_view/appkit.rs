use nd::native::appkit::objc2::rc::Retained;
use nd::native::appkit::objc2::runtime::ProtocolObject;
use nd::native::appkit::objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use nd::native::appkit::objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use nd::native::appkit::{AppKitCx, AppKitWidget, AvailableSize};
use nd::native::*;
use nd::prelude::*;
use objc2_core_location::CLLocationCoordinate2D;
use objc2_map_kit::{MKAnnotation, MKCoordinateRegion, MKCoordinateSpan, MKMapView, MKMapViewDelegate, MKPointAnnotation};

use super::{LatLng, MapEvent, MapProps, MapWidget, PinSelected, RegionChanged};

pub struct MapKit;

impl AppKitWidget<MapWidget> for MapKit {
    type View = MKMapView;

    fn create(&self, props: &MapProps, cx: &mut AppKitCx<MapWidget>) -> Retained<MKMapView> {
        let map = MKMapView::new(cx.mtm());
        let delegate = MapDelegate::new(cx.emitter(), cx.mtm());
        // MKMapView holds its delegate weakly; the node keeps it alive.
        unsafe { map.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        cx.keep_alive(delegate);
        apply(&map, None, props, cx.mtm());
        map
    }

    fn update(&self, map: &MKMapView, old: &MapProps, new: &MapProps, cx: &mut AppKitCx<MapWidget>) {
        apply(map, Some(old), new, cx.mtm());
    }

    fn measure(&self, _map: &MKMapView, _props: &MapProps, _available: AvailableSize) -> Size<Pixels> {
        // No natural size: the app sizes the map with flex or explicit dimensions.
        Size::default()
    }
}

fn apply(map: &MKMapView, old: Option<&MapProps>, new: &MapProps, mtm: MainThreadMarker) {
    if old.is_none_or(|o| o.center != new.center || o.zoom != new.zoom) {
        let region = MKCoordinateRegion { center: to_cl(new.center), span: span_for_zoom(new.zoom) };
        unsafe { map.setRegion_animated(region, old.is_some()) };
    }
    if old.is_none_or(|o| o.pins != new.pins) {
        unsafe {
            map.removeAnnotations(&map.annotations());
            for pin in &new.pins {
                let annotation = MKPointAnnotation::new(mtm);
                annotation.setCoordinate(to_cl(pin.at));
                annotation.setTitle(Some(&NSString::from_str(&pin.title)));
                map.addAnnotation(ProtocolObject::from_ref(&*annotation));
            }
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Emit<MapEvent>]
    struct MapDelegate;

    unsafe impl NSObjectProtocol for MapDelegate {}

    unsafe impl MKMapViewDelegate for MapDelegate {
        #[unsafe(method(mapView:regionDidChangeAnimated:))]
        fn region_did_change(&self, map: &MKMapView, _animated: bool) {
            let region = unsafe { map.region() };
            self.ivars().emit(MapEvent::RegionChanged(RegionChanged {
                center: LatLng { lat: region.center.latitude, lng: region.center.longitude },
                zoom: zoom_for_span(region.span),
            }));
        }

        #[unsafe(method(mapView:didSelectAnnotation:))]
        fn did_select(&self, _map: &MKMapView, annotation: &ProtocolObject<dyn MKAnnotation>) {
            if let Some(title) = unsafe { annotation.title() } {
                self.ivars().emit(MapEvent::PinSelected(PinSelected { title: title.to_string().into() }));
            }
        }
    }
);

impl MapDelegate {
    fn new(emit: Emit<MapEvent>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(emit);
        unsafe { msg_send![super(this), init] }
    }
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
