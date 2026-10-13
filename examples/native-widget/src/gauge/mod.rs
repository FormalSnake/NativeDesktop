//! A circular capacity gauge: SwiftUI's `Gauge` (written in Swift, `swift/Widgets`) on macOS,
//! a drawn ring elsewhere.

use std::convert::Infallible;

use nd::native::*;
use nd::prelude::*;
use serde::Serialize;

#[derive(Clone, PartialEq, Serialize)]
pub struct GaugeProps {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub label: SharedString,
}

pub struct GaugeWidget;

impl NativeWidget for GaugeWidget {
    type Props = GaugeProps;
    type Event = Infallible;

    fn semantics(props: &GaugeProps) -> Semantics {
        Semantics::new(Role::ProgressIndicator)
            .label(props.label.clone())
            .value(SemanticValue::range(props.value, props.min, props.max))
    }

    fn drawn(props: &GaugeProps, _emit: Emit<Infallible>, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = cx.theme();
        let fraction = ((props.value - props.min) / (props.max - props.min)).clamp(0.0, 1.0) as f32;
        let (track, fill) = (theme.border.subtle, theme.accent);
        v_flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .relative()
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        canvas(move |bounds, paint, _cx| {
                            let radius = bounds.size.width.min(bounds.size.height) / 2. - px(3.);
                            paint.stroke_arc(bounds.center(), radius, 0.0, 1.0, px(5.), track);
                            paint.stroke_arc(bounds.center(), radius, 0.0, fraction, px(5.), fill);
                        })
                        .absolute()
                        .inset_0(),
                    )
                    .child(text(format!("{:.0}", props.value)).font_weight(FontWeight::SEMIBOLD).tabular_nums()),
            )
            .child(text(props.label.clone()).text_size(theme.type_scale.caption).text_color(theme.text.secondary))
            .into_any_element()
    }

    fn backends() -> Backends<Self> {
        let backends = Backends::drawn_only();
        #[cfg(target_os = "macos")]
        let backends = backends.swift(SwiftWidget::named("Gauge"));
        backends
    }
}

#[derive(IntoElement)]
pub struct Gauge {
    base: NativeBase<GaugeWidget>,
    props: GaugeProps,
}

pub fn gauge(id: impl Into<ElementId>, value: f64) -> Gauge {
    Gauge { base: NativeBase::new(id), props: GaugeProps { value, min: 0.0, max: 100.0, label: SharedString::default() } }
}

impl Gauge {
    pub fn range(mut self, min: f64, max: f64) -> Self {
        (self.props.min, self.props.max) = (min, max);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.props.label = label.into();
        self
    }
}

impl NativeElement for Gauge {
    type Widget = GaugeWidget;

    fn base_mut(&mut self) -> &mut NativeBase<GaugeWidget> {
        &mut self.base
    }

    fn into_parts(self) -> (NativeBase<GaugeWidget>, GaugeProps) {
        (self.base, self.props)
    }
}
