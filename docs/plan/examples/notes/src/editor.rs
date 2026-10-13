use nd::prelude::*;

use crate::model::{Library, NoteId};
use crate::settings::NotesSettings;

pub struct Editor {
    library: Entity<Library>,
    note: Option<NoteId>,
    text: Entity<TextEditorState>,
    zoom: f32,
}

impl Editor {
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextEditorState::new(window, cx).placeholder("Start writing"));
        cx.subscribe_in(&text, window, |this, text, event: &TextEvent, _window, cx| {
            if let (TextEvent::Changed, Some(id)) = (event, this.note) {
                let body = text.read(cx).text().to_string();
                this.library.update(cx, |library, cx| library.set_body(id, body, cx));
            }
        })
        .detach();
        Self { library, note: None, text, zoom: 1.0 }
    }

    pub fn open(&mut self, id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        self.note = Some(id);
        let body = self.library.read(cx).note(&id).map(|n| n.body.clone()).unwrap_or_default();
        self.text.update(cx, |text, cx| text.set_text(body, window, cx));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.text.read(cx).focus_handle().focus(window);
    }

    fn pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let before = self.zoom;
        self.zoom = (self.zoom * event.scale).clamp(0.5, 3.0);
        // A detent at 100%, felt on Force Touch trackpads.
        if (before < 1.0) != (self.zoom < 1.0) {
            self.zoom = 1.0;
            cx.haptic(Haptic::Alignment);
        }
        cx.notify();
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let settings = cx.settings::<NotesSettings>();
        let font_size = px(settings.editor_font_size * self.zoom);

        v_flex()
            .size_full()
            .test_id("editor")
            .bg(theme.surface.base)
            .on_pinch(cx.listener(Self::pinch))
            .child(
                text_editor(&self.text)
                    .flex_1()
                    .px(theme.spacing.content_margin)
                    .py_4()
                    .font_size(font_size)
                    .line_height(relative(1.5))
                    .spellcheck(settings.spellcheck)
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
                        .child(text(format!("{:.0}%", self.zoom * 100.)).tabular_nums().test_id("zoom-label")),
                )
            })
    }
}
