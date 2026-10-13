//! The app vocabulary: entities and views (GPUI-shaped), elements, the retained tree and
//! reconciler, Taffy layout, the native event queue, nd-owned dispatch and the backend seam.

pub mod actions;
mod app;
pub mod backend;
pub mod dispatch;
mod element;
pub mod events;
mod layout;
#[cfg(feature = "test-support")]
pub mod test_app;
#[cfg(feature = "test-support")]
pub mod test_backend;
pub mod transaction;
mod tree;
mod window;

pub use app::{AnyView, App, Context, Entity, EntityId, Render, Subscription, WeakEntity};
pub use element::{
    AnyElement, Button, ButtonProps, ClickEvent, DispatchProps, Div, DivProps, InteractiveElement, IntoElement, Key,
    NativeElement, ParentElement, RootProps, Text, TextProps, WindowControls, WindowControlsProps, button, div, h_flex,
    text, v_flex, window_controls,
};
pub use nd_core::{NodeId, Point, Rect, Size, WindowId};
pub use nd_gpui::gpui::prelude::FluentBuilder;
pub use nd_gpui::gpui::{Action, KeyBinding, KeyContext, Keystroke, Modifiers, StyleRefinement, Styled};
pub use window::{Window, WindowHandle, WindowOptions};

/// What an app imports: the vocabulary plus GPUI's geometry and colour helpers.
pub mod prelude {
    pub use crate::{
        AnyElement, AnyView, App, Button, ClickEvent, Context, Div, Entity, EntityId, FluentBuilder,
        InteractiveElement, IntoElement, NativeElement, ParentElement, Render, Styled, Subscription, Text, WeakEntity,
        Window, WindowHandle, WindowOptions, button, div, h_flex, text, v_flex, window_controls,
    };
    pub use nd_gpui::gpui::{
        AbsoluteLength, DefiniteLength, FontWeight, Hsla, Length, Pixels, Rems, Rgba, SharedString, hsla, point, px,
        relative, rems, rgb, rgba, size,
    };
}
