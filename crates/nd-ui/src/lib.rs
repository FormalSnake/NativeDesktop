//! The app vocabulary: entities and views (GPUI-shaped), elements, the retained tree and
//! reconciler, Taffy layout, the native event queue, nd-owned dispatch and the backend seam.

pub mod actions;
mod app;
pub mod backend;
mod container;
pub mod dispatch;
mod element;
pub mod events;
mod layout;
pub mod overlay;
pub mod semantics;
pub mod surface;
#[cfg(feature = "test-support")]
pub mod test_app;
#[cfg(feature = "test-support")]
pub mod test_backend;
pub mod transaction;
mod tree;
mod window;

pub use app::{AnyView, App, Context, Entity, EntityId, Render, Subscription, WeakEntity};
pub use container::ContainerElement;
pub use element::{
    AnyElement, Bezel, Button, ButtonProps, ButtonVariant, ClickEvent, DispatchProps, Div, DivProps,
    InteractiveElement, IntoElement, Key, NativeElement, ParentElement, RootProps, Scope, Text, TextProps,
    WindowControls, WindowControlsProps, button, div, drag_region, h_flex, text, v_flex, window_controls,
};
pub use layout::with_titlebar_insets;
pub use nd_core::semantic::{Role, SemanticNode, SemanticTree, SemanticWindow};
pub use nd_core::{FrameTime, NodeId, Point, Rect, Size, WindowId};
pub use nd_gesture as gesture;
pub use nd_gesture::{
    DoubleTapEvent, DragEvent, ForceClickEvent, GesturePhase, Haptic, LongPressEvent, PinchEvent, RotateEvent,
    SmartMagnifyEvent, SwipeDirection, SwipePageEvent, TapEvent,
};
pub use nd_gpui::gpui::prelude::FluentBuilder;
pub use nd_gpui::gpui::{Action, KeyBinding, KeyContext, Keystroke, Modifiers, StyleRefinement, Styled};
pub use overlay::{
    DismissCause, Dismissed, Modality, OverlayElement, OverlayKind, OverlayProps, Placement, panel, popover, tooltip,
};
pub use surface::{InputCounts, SurfaceProps, SurfaceSource, native_surface, webview};
pub use window::{Titlebar, TitlebarInsets, Window, WindowHandle, WindowKind, WindowOptions};

/// What an app imports: the vocabulary plus GPUI's geometry and colour helpers.
pub mod prelude {
    pub use crate::overlay::{DismissCause, Dismissed, Modality, OverlayKind, Placement, panel, popover, tooltip};
    pub use crate::surface::{SurfaceSource, native_surface, webview};
    pub use crate::{
        AnyElement, AnyView, App, Button, ClickEvent, Context, Div, Entity, EntityId, FluentBuilder,
        InteractiveElement, IntoElement, NativeElement, ParentElement, Render, Styled, Subscription, Text, WeakEntity,
        Window, WindowHandle, WindowOptions, button, div, drag_region, h_flex, text, v_flex, window_controls,
    };
    pub use nd_gesture::{
        DoubleTapEvent, DragEvent, ForceClickEvent, GesturePhase, Haptic, LongPressEvent, PinchEvent, RotateEvent,
        SmartMagnifyEvent, SwipeDirection, SwipePageEvent, TapEvent,
    };
    pub use nd_gpui::gpui::{
        AbsoluteLength, DefiniteLength, FontWeight, Hsla, Length, Pixels, Rems, Rgba, SharedString, hsla, point, px,
        relative, rems, rgb, rgba, size,
    };
}
