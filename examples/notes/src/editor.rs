use nd::prelude::*;

use crate::model::{Library, NoteId};
use crate::settings::NotesSettings;

pub const BODY: &str = "body";

pub struct Editor {
    library: Entity<Library>,
    note: Option<NoteId>,
    text: Entity<TextFieldState>,
    zoom: f32,
}

impl Editor {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextFieldState::new(window, cx).placeholder("Start writing"));
        // Only user edits write back. `set_text` in `open` records `Replaced`, so opening a note
        // never touches its timestamp or its place in the list.
        cx.observe(&text, |this, text, cx| {
            let state = text.read(cx);
            if let (Some(TextEvent::Edited), Some(id)) = (state.last_event(), this.note) {
                let body = state.text().to_string();
                this.library.update(cx, |library, cx| library.set_body(id, body, cx));
            }
        })
        .detach();
        Self { library, note: None, text, zoom: 1.0 }
    }

    pub fn open(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        self.note = Some(id);
        let body = self.library.read(cx).body(&id);
        self.text.update(cx, |text, cx| text.set_text(body, window, cx));
        cx.notify();
    }

    /// Puts the caret in the body.
    pub fn focus(window: &Window, cx: &mut App) {
        window.focus_id(cx, BODY);
    }

    // docs:begin pinch
    fn pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let before = self.zoom;
        self.zoom = (self.zoom * event.scale).clamp(0.5, 3.0);
        // A detent at 100%, felt on Force Touch trackpads: passing through it lands on it.
        if (before - 1.0) * (self.zoom - 1.0) < 0.0 {
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
        let settings = cx.settings::<NotesSettings>();
        let font_size = px(settings.editor_font_size * self.zoom);

        v_flex()
            .size_full()
            .id("editor")
            .bg(theme.surface.base)
            .on_pinch(cx.listener(Self::pinch))
            .child(
                text_area(BODY)
                    .bind(&self.text)
                    .min_lines(3)
                    .flex_1()
                    .w_full()
                    .px(theme.spacing.content_margin)
                    .py_4()
                    .text_size(font_size)
                    .max_w(theme.spacing.content_width)
                    .mx_auto(),
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
