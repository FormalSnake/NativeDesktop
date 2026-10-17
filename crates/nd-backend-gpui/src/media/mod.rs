//! The media and content kinds (`nd_components::media`) drawn on GPUI from theme tokens: images
//! from the shared pipeline, the seek bar, synced lyrics, the sideways shelf, chat bubbles,
//! avatars and video. The models in `nd-components` hold every rule; this
//! module draws them and turns input into their calls. Per-node state lives with the forms
//! controls' (`Build::ctl`), so a released node drops it.

mod chat;
mod image;
mod lyrics;
pub(crate) mod paint;
mod seek;
mod shelf;
mod video;

use std::any::TypeId;

use nd_components::{
    AvatarKind, ChatBubbleKind, IconKind, IconProps, ImageKind, LyricsKind, SeekBarKind, SeekBarProps, ShelfKind,
    VideoKind,
};
use nd_core::NodeId;
use nd_core::time::Instant;
use nd_gpui::gpui::{self, AbsoluteLength, AnyElement, DefiniteLength, Length};
use nd_ui::backend::WidgetKind;

use crate::backend::{GpuiBackend, Node};
use crate::nav::Marks;
use crate::render::Build;

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

/// A style length in points, when it is a fixed one.
pub(crate) fn fixed(length: Option<Length>) -> Option<f32> {
    match length? {
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(p))) => Some(p.as_f32()),
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Rems(r))) => Some(r.0 * 16.),
        _ => None,
    }
}

impl Build<'_> {
    /// The drawn element for a media kind, or `None` when `kind` is none of them.
    pub(crate) fn media(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        let marks = Marks::default();
        let props = &node.props;
        let element = if is::<ImageKind>(kind) {
            image::image(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<SeekBarKind>(kind) {
            seek::seek_bar(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<LyricsKind>(kind) {
            lyrics::lyrics(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<ShelfKind>(kind) {
            let children = node.children.iter().map(|c| self.node(*c, window, cx)).collect();
            shelf::shelf(self, id, props.downcast_ref()?, children, &marks, window, cx)
        } else if is::<ChatBubbleKind>(kind) {
            chat::bubble(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<AvatarKind>(kind) {
            chat::avatar(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<VideoKind>(kind) {
            video::video(self, id, props.downcast_ref()?, &marks, window, cx)
        } else if is::<IconKind>(kind) {
            let props: &IconProps = props.downcast_ref()?;
            let color = props.style.text.color.unwrap_or(self.theme().text.secondary);
            let size = gpui::px(props.size);
            let stroke = gpui::px((props.size / 24. * nd_icons::STROKE).max(1.));
            use gpui::IntoElement as _;
            crate::forms::glyph::glyph(crate::forms::glyph::Glyph::of(props.icon), size, color, stroke)
                .into_any_element()
        } else {
            return None;
        };
        self.record_marks(id, marks.take());
        Some(element)
    }
}

impl GpuiBackend {
    /// What a drawn media node shows that its props do not say: the seek bar's position while
    /// the user drags it.
    pub(crate) fn probe_media(&self, node: NodeId, probe: &mut nd_ui::semantics::Probe) {
        let Ok(state) = self.state.try_borrow() else { return };
        let Some(props) = state.nodes.get(&node).and_then(|n| n.props.downcast_ref::<SeekBarProps>()) else {
            return;
        };
        let Ok(drawn) = self.form_drawn.try_borrow() else { return };
        let Some(seek) =
            drawn.get(&node).and_then(|d| d.state.clone().downcast::<std::cell::RefCell<seek::Seek>>().ok())
        else {
            return;
        };
        let Ok(seek) = seek.try_borrow() else { return };
        if seek.model.dragging() {
            probe.value = Some(seek::describe(props, &mut seek.model.clone(), Instant::now()));
        }
    }
}
