use nd::prelude::*;

/// A mounted page: its view lives as long as the gallery, so its state survives switching away.
pub type Mount = Box<dyn Fn() -> AnyElement>;

pub fn mount<V: Render>(view: Entity<V>) -> Mount {
    Box::new(move || view.clone().into_any_element())
}

pub struct Page {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: Icon,
    pub mount: fn(&mut App) -> Mount,
}

/// The pages in sidebar order. A new page is a module with a `page()` and one line here.
pub fn all() -> Vec<Page> {
    vec![crate::forms::page()]
}
