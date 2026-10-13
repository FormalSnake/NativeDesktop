//! `glass()` on the system material (ARCHITECTURE.md 4): `NSGlassEffectView` on macOS 26 and
//! later, `NSVisualEffectView` on 14 and 15. The node's children live in an [`NdLayerView`]
//! that is the glass view's content view, so the material draws behind them. The window
//! server composites both materials; they appear in captures of the screen, not of the layer
//! tree.

use nd_components::GlassProps;
use nd_theme::GlassVariant;
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSGlassEffectContainerView, NSGlassEffectView, NSGlassEffectViewStyle, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
};
use objc2_foundation::MainThreadMarker;

use crate::look::{Color, ViewLook, length};
use crate::views::NdLayerView;

/// Liquid Glass is available: macOS 26 or later, where the class exists.
pub(crate) fn has_liquid_glass() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

enum Material {
    Glass(Retained<NSGlassEffectView>),
    Vibrancy(Retained<NSVisualEffectView>),
}

pub(crate) struct GlassNative {
    material: Material,
    pub content: Retained<NdLayerView>,
    props: GlassProps,
    radius_scale: f32,
}

fn radius(props: &GlassProps) -> f32 {
    props.style.corner_radii.top_left.map(length).unwrap_or(0.)
}

impl GlassNative {
    pub fn new(mtm: MainThreadMarker, props: &GlassProps) -> Self {
        let content = NdLayerView::new(mtm, &ViewLook::default());
        content.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let material = if has_liquid_glass() {
            let glass = NSGlassEffectView::new(mtm);
            glass.setContentView(Some(&content));
            Material::Glass(glass)
        } else {
            let effect = NSVisualEffectView::new(mtm);
            effect.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.setWantsLayer(true);
            effect.addSubview(&content);
            Material::Vibrancy(effect)
        };
        let this = Self { material, content, props: props.clone(), radius_scale: 1. };
        this.apply();
        this
    }

    pub fn view(&self) -> &NSView {
        match &self.material {
            Material::Glass(v) => v,
            Material::Vibrancy(v) => v,
        }
    }

    pub fn update(&mut self, props: &GlassProps) {
        if self.props != *props {
            self.props = props.clone();
            self.apply();
        }
    }

    pub fn set_radius_scale(&mut self, scale: f32) {
        if self.radius_scale != scale {
            self.radius_scale = scale;
            self.apply();
        }
    }

    fn apply(&self) {
        let r = f64::from(radius(&self.props) * self.radius_scale);
        let tint = self.props.tint.map(|t| Color::from(t).ns_color());
        match &self.material {
            Material::Glass(glass) => {
                glass.setCornerRadius(r);
                glass.setTintColor(tint.as_deref());
                glass.setStyle(match self.props.variant {
                    GlassVariant::Regular => NSGlassEffectViewStyle::Regular,
                    GlassVariant::Clear => NSGlassEffectViewStyle::Clear,
                });
            }
            Material::Vibrancy(effect) => {
                effect.setMaterial(match self.props.variant {
                    GlassVariant::Regular => NSVisualEffectMaterial::Popover,
                    GlassVariant::Clear => NSVisualEffectMaterial::HUDWindow,
                });
                if let Some(layer) = effect.layer() {
                    layer.setCornerRadius(r);
                    layer.setMasksToBounds(r > 0.);
                }
                // The material has no tint of its own below 26: the tint lays over it.
                let look = ViewLook {
                    background: self.props.tint.map(Color::from),
                    corner_radius: radius(&self.props),
                    ..ViewLook::default()
                };
                self.content.set_props(&look);
                self.content.set_radius_scale(self.radius_scale);
            }
        }
    }
}

/// `glass_group()`: `NSGlassEffectContainerView` on 26 and later, a plain container below.
pub(crate) struct GlassGroupNative {
    outer: Retained<NSView>,
    pub content: Retained<NdLayerView>,
}

impl GlassGroupNative {
    pub fn new(mtm: MainThreadMarker, spacing: f32) -> Self {
        let content = NdLayerView::new(mtm, &ViewLook::default());
        content.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let outer: Retained<NSView> = if has_liquid_glass() {
            let group = NSGlassEffectContainerView::new(mtm);
            group.setContentView(Some(&content));
            group.setSpacing(spacing.into());
            Retained::into_super(group)
        } else {
            let outer = NSView::new(mtm);
            outer.addSubview(&content);
            outer
        };
        Self { outer, content }
    }

    pub fn view(&self) -> &NSView {
        &self.outer
    }

    pub fn set_spacing(&self, spacing: f32) {
        if let Some(group) = self.outer.downcast_ref::<NSGlassEffectContainerView>() {
            group.setSpacing(spacing.into());
        }
    }
}
