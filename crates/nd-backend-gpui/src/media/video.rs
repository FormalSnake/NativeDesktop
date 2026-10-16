//! `video()` on GPUI: no video surface yet (decoding needs a GStreamer or Media Foundation
//! pipeline painting frames), so the node draws its placeholder and reports [`VideoFailed`] once
//! per source, and the app falls back to the system player.

use nd_components::{VideoFailed, VideoProps};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, ElementId, InteractiveElement, IntoElement, SharedString, StatefulInteractiveElement, Styled, div,
};

use crate::forms::with_style;
use crate::nav::Marks;
use crate::render::Build;

const UNSUPPORTED: &str = "this backend has no video surface";

pub(super) fn video(
    b: &Build,
    id: NodeId,
    p: &VideoProps,
    marks: &Marks,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> AnyElement {
    let (ctl, reported) = b.ctl(id, false, window, cx, || None::<Option<SharedString>>);
    if reported.borrow().as_ref() != Some(&p.source) {
        *reported.borrow_mut() = Some(p.source.clone());
        if p.source.is_some() {
            ctl.sink.send(VideoFailed { error: UNSUPPORTED.into() }, cx);
        }
    }
    let fill = b.theme().surface.raised;
    marks.fill("video", fill, &[]);
    let root = div().id(ElementId::Integer(id.0)).role(gpui::Role::Image).aria_label(p.alt.clone().unwrap_or_default());
    with_style(root, &p.style).bg(fill).into_any_element()
}
