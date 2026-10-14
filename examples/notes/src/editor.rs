use nd::prelude::*;

use crate::model::{Library, NoteId};
use crate::settings;

pub struct Editor {
    library: Entity<Library>,
    note: Option<NoteId>,
    zoom: f32,
}

impl Editor {
    pub fn new(library: Entity<Library>, _cx: &mut Context<Self>) -> Self {
        Self { library, note: None, zoom: 1.0 }
    }

    /// Shows `id`. Only user edits write back (`on_change`), so opening a note never touches its
    /// timestamp or its place in the list.
    pub fn open(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.note = Some(id);
        cx.notify();
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    // docs:begin pinch
    fn pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let before = self.zoom;
        self.zoom = (self.zoom * event.scale).clamp(0.5, 3.0);
        // A detent at 100%, felt on Force Touch trackpads.
        if (before < 1.0 && self.zoom > 1.0) || (before > 1.0 && self.zoom < 1.0) {
            self.zoom = 1.0;
            cx.haptic(Haptic::Alignment);
        }
        cx.notify();
    }
    // docs:end pinch
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = settings::current(cx);
        let font_size = px(settings.editor_font_size * self.zoom);
        let body = self.note.and_then(|id| self.library.read(cx).note(&id)).map(|n| n.body).unwrap_or_default();

        v_flex()
            .size_full()
            .id("editor")
            .bg(theme.surface.base)
            .on_pinch(cx.listener(Self::pinch))
            .child(
                text_area("body")
                    .text(body)
                    .placeholder("Start writing")
                    .label("Note")
                    .disabled(self.note.is_none())
                    .flex_1()
                    .px(theme.spacing.content_margin)
                    .py_4()
                    .text_size(font_size)
                    .max_w(theme.spacing.content_width)
                    .mx_auto()
                    .on_change(cx.listener(|this, e: &TextChanged, _window, cx| {
                        let Some(id) = this.note else { return };
                        let body = e.text.to_string();
                        this.library.update(cx, |library, cx| library.set_body(id, body, cx));
                    })),
            )
            .when(self.zoom != 1.0, |col| {
                col.child(
                    glass()
                        .absolute()
                        .bottom_3()
                        .right_3()
                        .rounded_full()
                        .px_2()
                        .py_1()
                        .child(text(format!("{:.0}%", self.zoom * 100.)).tabular_nums().id("zoom-label")),
                )
            })
    }
}
