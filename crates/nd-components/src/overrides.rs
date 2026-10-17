//! Override layers on containers (ARCHITECTURE.md 3.6): a subtree theme, a pattern choice and a
//! context menu, plus the per-platform closures every element takes.

use nd_theme::{Preset, ThemeRefinement};
use nd_ui::prelude::*;
use nd_ui::{ContainerElement, Scope};

use crate::menu::{ContextMenu, ContextMenuRequested};
use crate::services;

/// How a structural pattern is presented: the OS's own, or drawn by nd.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pattern {
    #[default]
    Native,
    Drawn,
}

/// App-wide pattern choices; `nativedesktop.toml` `[patterns]` sits below these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patterns {
    pub tooltips: Pattern,
    pub menus: Pattern,
    pub popovers: Pattern,
}

/// What a container carries for its subtree. Backends read it from the node's props: on
/// AppKit the theme reaches native controls inside (accent as bezel colour, radius scale, a
/// forced appearance), and `context_menu` makes the view answer right clicks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubtreeScope {
    pub theme: Option<ThemeRefinement>,
    pub pattern: Option<Pattern>,
    pub context_menu: bool,
}

/// A container that can carry a [`SubtreeScope`] and receive native events.
pub trait ScopedContainer: Sized {
    fn update_scope(self, f: impl FnOnce(&mut SubtreeScope)) -> Self;
    fn on_native<E: 'static>(self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self;
}

impl ScopedContainer for Div {
    fn update_scope(self, f: impl FnOnce(&mut SubtreeScope)) -> Self {
        let mut scope = self.scope_ref().and_then(|s| s.get::<SubtreeScope>()).cloned().unwrap_or_default();
        f(&mut scope);
        self.scope(Scope::new(scope))
    }

    fn on_native<E: 'static>(self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        self.on(handler)
    }
}

/// Containers whose props embed a [`SubtreeScope`].
pub trait HasScope {
    fn scope_mut(&mut self) -> &mut SubtreeScope;
}

impl<P: HasScope + nd_ui::backend::AnyProps> ScopedContainer for ContainerElement<P> {
    fn update_scope(mut self, f: impl FnOnce(&mut SubtreeScope)) -> Self {
        f(self.props_mut().scope_mut());
        self
    }

    fn on_native<E: 'static>(self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        self.on(handler)
    }
}

/// Subtree overrides on any scoped container.
pub trait SubtreeOverrides: ScopedContainer {
    /// Everything inside resolves against this refinement on top of the window's theme.
    fn theme(self, refine: impl FnOnce(ThemeRefinement) -> ThemeRefinement) -> Self {
        let refinement = refine(ThemeRefinement::new());
        self.update_scope(|s| s.theme = Some(refinement))
    }

    /// Native or drawn presentation for the patterns inside (menus, tooltips).
    fn pattern(self, pattern: Pattern) -> Self {
        self.update_scope(|s| s.pattern = Some(pattern))
    }

    /// A context menu, built when it opens. Items dispatch their actions from the window's
    /// focused node.
    fn context_menu(self, build: impl Fn(ContextMenu, &mut Window, &mut App) -> ContextMenu + 'static) -> Self {
        self.update_scope(|s| s.context_menu = true).on_native(move |e: &ContextMenuRequested, window, cx| {
            let menu = build(ContextMenu::default(), window, cx);
            services::show_context_menu(cx, window.id(), e.node, e.x, e.y, menu);
        })
    }
}

impl<T: ScopedContainer> SubtreeOverrides for T {}

/// Per-platform adjustments on any element; the closure runs only on that platform, so app
/// code needs no `cfg`. A GPUI build drawing another look (`ND_PRESET=fluent` on Linux) runs the
/// closures of the look it draws.
pub trait OnPlatform: Sized {
    fn on_macos(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_os = "macos") { f(self) } else { self }
    }

    fn on_windows(self, f: impl FnOnce(Self) -> Self) -> Self {
        if !cfg!(target_os = "macos") && Preset::current() == Preset::Fluent { f(self) } else { self }
    }

    fn on_linux(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(all(unix, not(target_os = "macos"), not(target_family = "wasm")))
            && Preset::current() == Preset::Adwaita
        {
            f(self)
        } else {
            self
        }
    }

    fn on_web(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(target_family = "wasm") { f(self) } else { self }
    }
}

impl<T: IntoElement> OnPlatform for T {}
