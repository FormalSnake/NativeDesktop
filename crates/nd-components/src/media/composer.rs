//! `composer`: the message field of a chat window, after messages' composer. A text area that
//! grows from one line to [`COMPOSER_LINES`] and then scrolls, Return to send and Shift-Return
//! for a new line, a row of staged attachments above it, buttons to attach a file, open the
//! emoji picker and send.
//!
//! Composed from the platform's own controls: on AppKit the field is an `NSTextView` (spelling,
//! IME, the Character Viewer for emoji), the buttons `NSButton`s. Pasted or dropped images and
//! files become attachments instead of text.
//!
//! The app owns a [`Draft`] and hands every [`ComposerEvent`] to [`Draft::apply`], which keeps
//! the text and attachments and returns the [`Outgoing`] message when the user sends.

use std::path::PathBuf;
use std::rc::Rc;

use nd_ui::StyleRefinement;
use nd_ui::prelude::*;

use super::image::image;
use super::pipeline::ImageSource;
use crate::controls::{ButtonExt, icon_button};
use crate::forms::{
    FilesDropped, ImagesPasted, LabelTone, PastedImage, TextChanged, TextStyle, TextSubmitted, label, text_area,
};
use crate::icon::Icon;

/// The most lines the field grows to before it scrolls.
pub const COMPOSER_LINES: u16 = 8;
/// A staged attachment's thumbnail.
pub const ATTACHMENT_THUMB: f32 = 32.;
/// The field's height on one line, and the side of every button beside it: a button is as tall
/// as the line it sits on, so icons centre on the text, and stays on the bottom line as the
/// field grows (Messages' layout).
pub const COMPOSER_ROW: f32 = 31.;

/// A file staged to go with the next message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: SharedString,
    pub name: SharedString,
    /// Shown as a thumbnail; a file with none shows its icon.
    pub preview: Option<ImageSource>,
    pub path: Option<PathBuf>,
    pub bytes: Option<std::sync::Arc<[u8]>>,
}

/// Extensions a dropped file is previewed as an image for.
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "heic", "bmp"];

impl Attachment {
    pub fn file(id: impl Into<SharedString>, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let image = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        Self {
            id: id.into(),
            name: name.into(),
            preview: image.then(|| ImageSource::from(path.clone())),
            path: Some(path),
            bytes: None,
        }
    }

    pub fn image(id: impl Into<SharedString>, pasted: &PastedImage) -> Self {
        let id: SharedString = id.into();
        let ext = pasted.mime.strip_prefix("image/").unwrap_or("png");
        Self {
            name: format!("Pasted image.{ext}").into(),
            preview: Some(ImageSource::bytes(id.clone(), pasted.bytes.clone())),
            path: None,
            bytes: Some(pasted.bytes.clone()),
            id,
        }
    }
}

/// What the user did in the composer.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposerEvent {
    Edited(SharedString),
    /// Return, or the send button.
    Send,
    /// The attach button: the app shows its file picker and calls [`Draft::attach`].
    Attach,
    EmojiPicker,
    Remove(SharedString),
    Images(Vec<PastedImage>),
    Files(Vec<PathBuf>),
}

/// The message being written: text, staged attachments, and the emoji picker request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub text: SharedString,
    pub attachments: Vec<Attachment>,
    pub emoji_requests: u64,
    next_id: u64,
}

/// A message the user sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    /// Trimmed; `None` when only attachments go.
    pub text: Option<SharedString>,
    pub attachments: Vec<Attachment>,
}

impl Draft {
    pub fn can_send(&self) -> bool {
        !self.text.trim().is_empty() || !self.attachments.is_empty()
    }

    fn id(&mut self) -> SharedString {
        self.next_id += 1;
        format!("attachment-{}", self.next_id).into()
    }

    pub fn attach(&mut self, path: impl Into<PathBuf>) {
        let id = self.id();
        self.attachments.push(Attachment::file(id, path));
    }

    /// Applies `event`; returns the message when it sends one, and clears the draft then.
    pub fn apply(&mut self, event: &ComposerEvent) -> Option<Outgoing> {
        match event {
            ComposerEvent::Edited(text) => self.text = text.clone(),
            ComposerEvent::Send => {
                if !self.can_send() {
                    return None;
                }
                let text = self.text.trim();
                let out = Outgoing {
                    text: (!text.is_empty()).then(|| SharedString::from(text.to_owned())),
                    attachments: std::mem::take(&mut self.attachments),
                };
                self.text = SharedString::default();
                return Some(out);
            }
            ComposerEvent::Attach => {}
            ComposerEvent::EmojiPicker => self.emoji_requests += 1,
            ComposerEvent::Remove(id) => self.attachments.retain(|a| &a.id != id),
            ComposerEvent::Images(images) => {
                for pasted in images {
                    let id = self.id();
                    self.attachments.push(Attachment::image(id, pasted));
                }
            }
            ComposerEvent::Files(paths) => {
                for path in paths {
                    self.attach(path.clone());
                }
            }
        }
        None
    }
}

type OnEvent = Rc<dyn Fn(&ComposerEvent, &mut Window, &mut App)>;

pub struct Composer {
    id: SharedString,
    draft: Draft,
    placeholder: SharedString,
    enabled: bool,
    attach: bool,
    send_icon: Icon,
    send_tooltip: SharedString,
    send_fill: Option<Hsla>,
    on_event: Option<OnEvent>,
    style: StyleRefinement,
}

/// The message field over `draft`.
pub fn composer(id: impl Into<SharedString>, draft: &Draft) -> Composer {
    Composer {
        id: id.into(),
        draft: draft.clone(),
        placeholder: "Message".into(),
        enabled: true,
        attach: true,
        send_icon: Icon::CircleArrowUp,
        send_tooltip: "Send".into(),
        send_fill: None,
        on_event: None,
        style: StyleRefinement::default(),
    }
}

impl Composer {
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    /// No attach button; pasted and dropped files still stage.
    pub fn without_attach(mut self) -> Self {
        self.attach = false;
        self
    }

    /// The send button's icon and tooltip, for a send that is not a new message: a check while
    /// editing one.
    pub fn send_icon(mut self, icon: Icon, tooltip: impl Into<SharedString>) -> Self {
        self.send_icon = icon;
        self.send_tooltip = tooltip.into();
        self
    }

    /// Fills the send button with `color` instead of the accent: an SMS conversation's green,
    /// read from the app's theme (`theme.fill(FillRole::Success)`).
    pub fn send_fill(mut self, color: Hsla) -> Self {
        self.send_fill = Some(color);
        self
    }

    pub fn on_event(mut self, f: impl Fn(&ComposerEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_event = Some(Rc::new(f));
        self
    }
}

impl FluentBuilder for Composer {}

impl Styled for Composer {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Composer {
    fn into_any_element(self) -> AnyElement {
        let Composer { id, draft, placeholder, enabled, attach, send_icon, send_tooltip, send_fill, on_event, style } =
            self;
        let child = |suffix: &str| SharedString::from(format!("{id}.{suffix}"));
        let send = move |event: ComposerEvent| {
            let on = on_event.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(on) = &on {
                    on(&event, window, cx);
                }
            }
        };
        let sender = send.clone();
        let emit = move |event: ComposerEvent, window: &mut Window, cx: &mut App| sender(event)(window, cx);
        let chips = draft.attachments.iter().map(|a| {
            let thumb = match &a.preview {
                Some(source) => image(source.clone())
                    .id(child(&format!("thumb.{}", a.id)))
                    .alt(a.name.clone())
                    .size(px(ATTACHMENT_THUMB))
                    .flex_none()
                    .rounded_md()
                    .into_any_element(),
                None => label("\u{1f4ce}").text_style(TextStyle::Title3).into_any_element(),
            };
            let remove = icon_button(child(&format!("remove.{}", a.id)), Icon::X)
                .tooltip(format!("Remove {}", a.name))
                .on_click({
                    let f = send(ComposerEvent::Remove(a.id.clone()));
                    move |_, w, cx| f(w, cx)
                });
            h_flex()
                .id(child(&format!("chip.{}", a.id)))
                .gap_1()
                .max_w(px(220.))
                .child(thumb)
                .child(label(a.name.clone()).text_style(TextStyle::Caption1).tone(LabelTone::Secondary).min_w_0())
                .child(remove)
        });
        let field = {
            let (edit, submit, images, files) = (emit.clone(), emit.clone(), emit.clone(), emit.clone());
            text_area(child("field"))
                .text(draft.text.clone())
                .placeholder(placeholder.clone())
                .label(placeholder)
                .min_lines(1)
                .max_lines(COMPOSER_LINES)
                .submit_on_return()
                .accepts_media()
                .emoji_picker(draft.emoji_requests)
                .disabled(!enabled)
                .rounded(px(COMPOSER_ROW / 2.))
                .on_change(move |e: &TextChanged, w, cx| edit(ComposerEvent::Edited(e.text.clone()), w, cx))
                .on_submit(move |_: &TextSubmitted, w, cx| submit(ComposerEvent::Send, w, cx))
                .on_images(move |e: &ImagesPasted, w, cx| images(ComposerEvent::Images(e.images.clone()), w, cx))
                .on_files(move |e: &FilesDropped, w, cx| files(ComposerEvent::Files(e.paths.clone()), w, cx))
        };
        let square = |button: nd_ui::Button| button.size(px(COMPOSER_ROW)).flex_none();
        let attach_button = attach.then(|| {
            square(icon_button(child("attach"), Icon::Plus))
                .bezel(nd_ui::Bezel::Circle)
                .tooltip("Attach Files")
                .disabled(!enabled)
                .on_click({
                    let f = send(ComposerEvent::Attach);
                    move |_, w, cx| f(w, cx)
                })
        });
        let emoji = square(icon_button(child("emoji"), Icon::Smile)).bezel(nd_ui::Bezel::Circle).tooltip("Emoji").disabled(!enabled).on_click({
            let f = send(ComposerEvent::EmojiPicker);
            move |_, w, cx| f(w, cx)
        });
        // Only a message that can go shows the send button, a filled arrow in the accent (or the
        // app's fill), as in Messages; Return sends either way.
        let has_send = enabled && draft.can_send();
        let send_button = has_send.then(|| {
            square(icon_button(child("send"), send_icon))
                .prominent()
                .when_some(send_fill, |button, fill| button.bg(fill))
                .tooltip(send_tooltip)
                .on_click({
                    let f = send(ComposerEvent::Send);
                    move |_, w, cx| f(w, cx)
                })
        });
        let mut root = v_flex()
            .id(id.clone())
            .w_full()
            .gap_2()
            .when(!draft.attachments.is_empty(), |col| col.child(h_flex().flex_wrap().gap_2().children(chips)))
            .child(
                // Messages' bar: 9 pt between the plus button, the field and the emoji button.
                h_flex()
                    .w_full()
                    .items_end()
                    .gap(px(9.))
                    .children(attach_button)
                    // The send button sits inside the field's outline at its trailing end, on the
                    // last line, as Messages draws it; the text wraps before it.
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .child(field.w_full().when(has_send, |f| f.trailing_room(COMPOSER_ROW)))
                            .children(send_button.map(|b| b.absolute().right_0().bottom_0())),
                    )
                    .child(emoji),
            );
        nd_gpui::gpui::Refineable::refine(root.style(), &style);
        root.into_any_element()
    }
}
