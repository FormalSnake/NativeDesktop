//! `glass()`: a container on the system material (ARCHITECTURE.md 4). macOS 26 and later map it
//! to `NSGlassEffectView` (`NSGlassEffectContainerView` for [`glass_group`]), macOS 14 and 15
//! to `NSVisualEffectView`; Windows to a Mica or Acrylic region; Linux to compositor blur or a
//! translucent surface. Never a replica (3.10).

use std::panic::Location;

use nd_theme::GlassVariant;
use nd_ui::prelude::*;
use nd_ui::{ContainerElement, DispatchProps, InteractiveElement, Key, StyleRefinement};

use crate::overrides::{HasScope, ScopedContainer, SubtreeScope};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlassProps {
    pub variant: GlassVariant,
    pub tint: Option<Hsla>,
    /// Reacts to the pointer (macOS's interactive glass), where the OS offers it.
    pub interactive: bool,
    /// The element's style: corner radius becomes the glass shape's.
    pub style: StyleRefinement,
    pub scope: SubtreeScope,
}

impl HasScope for GlassProps {
    fn scope_mut(&mut self) -> &mut SubtreeScope {
        &mut self.scope
    }
}

pub struct Glass(ContainerElement<GlassProps>);

/// A glass surface. Lays its children out like a `div`.
#[track_caller]
pub fn glass() -> Glass {
    let el = ContainerElement::new::<Glass>(Key::Site(Location::caller()), GlassProps::default());
    Glass(el.flex().flex_col())
}

impl Glass {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn variant(mut self, variant: GlassVariant) -> Self {
        self.0.props_mut().variant = variant;
        self
    }

    pub fn tint(mut self, tint: impl Into<Hsla>) -> Self {
        self.0.props_mut().tint = Some(tint.into());
        self
    }

    pub fn interactive(mut self) -> Self {
        self.0.props_mut().interactive = true;
        self
    }
}

impl Styled for Glass {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for Glass {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl FluentBuilder for Glass {}

impl InteractiveElement for Glass {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

impl ScopedContainer for Glass {
    fn update_scope(self, f: impl FnOnce(&mut SubtreeScope)) -> Self {
        Glass(self.0.update_scope(f))
    }

    fn on_native<E: 'static>(self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        Glass(self.0.on(handler))
    }
}

impl IntoElement for Glass {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlassGroupProps {
    /// Glass shapes closer than this merge, where the OS merges glass.
    pub spacing: f32,
}

/// Lets nearby `glass()` children merge and morph together where the OS offers it.
pub struct GlassGroup(ContainerElement<GlassGroupProps>);

#[track_caller]
pub fn glass_group() -> GlassGroup {
    let el = ContainerElement::new::<GlassGroup>(Key::Site(Location::caller()), GlassGroupProps::default());
    GlassGroup(el.flex())
}

impl GlassGroup {
    pub fn spacing(mut self, spacing: Pixels) -> Self {
        self.0.props_mut().spacing = spacing.as_f32();
        self
    }
}

impl Styled for GlassGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for GlassGroup {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl FluentBuilder for GlassGroup {}

impl IntoElement for GlassGroup {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}
