//! `chat_bubble` and `avatar`, drawn from theme tokens: sent bubbles in the accent, received on
//! the raised surface, the run's last bubble with a tail filled in its own colour, reactions as
//! pills over its top outer corner; avatars as a photo, a monogram on the accent container, or a
//! group's cluster.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{
    AvatarProps, BUBBLE, BubbleSide, ChatBubbleProps, DecodeRequest, ImageFit, MONOGRAM_MIN, Person, TextStyle,
    group_layout, initials, monogram_size,
};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, PathBuilder,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, canvas, div, point, prelude::FluentBuilder, px,
};
use nd_theme::Theme;

use super::image::{Picture, picture};
use crate::forms::glyph::{Glyph, glyph};
use crate::forms::with_style;
use crate::nav::Marks;
use crate::render::Build;

/// The tail: a lobe curving out of the bubble's bottom outer corner, filling the corner's
/// rounding under it so bubble and tail read as one shape.
fn tail(color: Hsla, sent: bool) -> impl IntoElement {
    let (r, w, h) = (BUBBLE.radius, BUBBLE.tail_width, BUBBLE.tail_height);
    let el = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let at = |x: f32, y: f32| {
                let x = if sent { x } else { r + w - x };
                point(o.x + px(x), o.y + px(y))
            };
            let mut path = PathBuilder::fill();
            path.move_to(at(r, 0.));
            path.curve_to(at(r + w, h), at(r, h * 0.8));
            path.line_to(at(0., h));
            path.line_to(at(0., 0.));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .bottom_0()
    .w(px(r + w))
    .h(px(h));
    if sent { el.right(px(-w)) } else { el.left(px(-w)) }
}

pub(super) fn bubble(
    b: &Build,
    id: NodeId,
    p: &ChatBubbleProps,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (ctl, _) = b.ctl(id, false, window, cx, || ());
    let theme = b.theme();
    let look = ctl.look;
    let sent = p.side == BubbleSide::Sent;
    let (fill, ink) = theme.fill(p.fill_role());
    let mut root = div().id(ElementId::Integer(id.0)).role(gpui::Role::Label).aria_label(p.text.clone()).relative();
    if p.emoji_only {
        let size = px(BUBBLE.emoji_size);
        marks.text(&p.text, theme.text.primary, &[], size, FontWeight::NORMAL);
        root = root.text_size(size).line_height(size * 1.2).text_color(theme.text.primary).child(p.text.clone());
    } else {
        let (size, weight, line) = look.type_of(TextStyle::Body);
        marks.fill("bubble", fill, &[]);
        let body = match &p.rich {
            Some(rich) => {
                let base = crate::rich::Base {
                    theme: &theme,
                    size,
                    leading: line.as_f32() / size.as_f32(),
                    weight: weight.0,
                    color: ink,
                    ink: (p.fill_role() != nd_theme::FillRole::Neutral).then_some(ink),
                };
                for span in rich.spans().iter().filter(|s| !s.text.trim().is_empty()) {
                    let at = size * span.style.scale();
                    marks.text(&span.text, base.color(&span.style), &[fill], at, FontWeight(span.style.weight(weight.0)));
                }
                let on_link = rich.has_links().then(|| b.link_handler(id, p.opens_links));
                crate::rich::element(ElementId::NamedInteger("text".into(), id.0), rich, &base, on_link)
            }
            None => {
                marks.text(&p.text, ink, &[fill], size, weight);
                p.text.clone().into_any_element()
            }
        };
        let [tl, tr, br, bl] = p.corners();
        root = root.child(
            div()
                .max_w(px(BUBBLE.max_width))
                .px(px(BUBBLE.padding_x))
                .py(px(BUBBLE.padding_y))
                .rounded_tl(px(tl))
                .rounded_tr(px(tr))
                .rounded_br(px(br))
                .rounded_bl(px(bl))
                .bg(fill)
                .text_size(size)
                .line_height(line)
                .text_color(ink)
                .child(body),
        );
        if p.has_tail() {
            root = root.child(tail(fill, sent));
        }
    }
    if !p.reactions.is_empty() {
        let behind = b.backdrop();
        let mut pills = div().absolute().top(px(-BUBBLE.reaction_lift)).flex().flex_row();
        pills = if sent { pills.left(px(-8.)) } else { pills.right(px(-8.)) };
        for (i, r) in p.reactions.iter().take(3).enumerate() {
            let (pill, count_ink) = if r.mine {
                (theme.accent_fill, theme.text.on_accent)
            } else {
                (theme.surface.raised_hover, theme.text.secondary)
            };
            marks.fill("reaction", pill, &[]);
            marks.fill("reaction ring", behind, &[]);
            let count = (r.count > 1).then(|| {
                let text = r.count.to_string();
                marks.text(&text, count_ink, &[pill], px(12.), FontWeight::SEMIBOLD);
                div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(count_ink).child(text)
            });
            pills = pills.child(
                div()
                    .when(i > 0, |d| d.ml(px(-6.)))
                    .h(px(BUBBLE.reaction))
                    .min_w(px(BUBBLE.reaction))
                    .px(px(5.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(2.))
                    .rounded_full()
                    .bg(pill)
                    .border_2()
                    .border_color(behind)
                    .text_size(px(13.))
                    .child(r.glyph.clone())
                    .children(count),
            );
        }
        root = root.child(pills);
    }
    with_style(root, &p.style).into_any_element()
}

/// One person's circle: the photo once it decoded, else the monogram.
fn face(
    picture_state: &Rc<RefCell<Picture>>,
    person: &Person,
    size: f32,
    theme: &Theme,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Div {
    let request =
        person.image.clone().map(|s| DecodeRequest::at(s, size, size, window.scale_factor(), ImageFit::Cover));
    Picture::want(picture_state, request, window, cx);
    let circle = div().size(px(size)).rounded_full().overflow_hidden().flex().items_center().justify_center();
    if let Some(image) = Picture::current(picture_state, false, Some(1.), window, cx) {
        let mut round = StyleRefinement::default();
        round.corner_radii.top_left = Some(px(size / 2.).into());
        round.corner_radii.top_right = Some(px(size / 2.).into());
        round.corner_radii.bottom_right = Some(px(size / 2.).into());
        round.corner_radii.bottom_left = Some(px(size / 2.).into());
        return circle.child(picture(image, ImageFit::Cover, &round));
    }
    let (bg, fg) = (theme.accent_container.bg, theme.accent_container.fg);
    marks.fill("monogram", bg, &[]);
    let circle = circle.bg(bg);
    if size < MONOGRAM_MIN {
        return circle;
    }
    let letters = initials(&person.name);
    if letters == "#" {
        marks.fill("monogram glyph", fg, &[bg]);
        return circle.child(glyph(Glyph::Person, px(size * 0.62), fg, px(1.5)));
    }
    let text = px(monogram_size(size));
    marks.text(&letters, fg, &[bg], text, FontWeight::SEMIBOLD);
    circle.text_size(text).line_height(text).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(letters)
}

pub(super) fn avatar(
    b: &Build,
    id: NodeId,
    p: &AvatarProps,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (_, pictures) = b.ctl(id, false, window, cx, Vec::<Rc<RefCell<Picture>>>::new);
    let theme = b.theme();
    let app = b.backend.state.borrow().app.clone();
    let shown = p.people.len().min(4);
    let slots: Vec<Rc<RefCell<Picture>>> = {
        let mut slots = pictures.borrow_mut();
        slots.resize_with(shown.max(1), Rc::default);
        slots.truncate(shown.max(1));
        for slot in slots.iter() {
            slot.borrow_mut().attach(None, app.clone());
        }
        slots.clone()
    };
    let name = p.people.iter().map(|p| p.name.to_string()).collect::<Vec<_>>().join(", ");
    let root = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Image)
        .aria_label(name)
        .relative()
        .flex_none()
        .size(px(p.size))
        .rounded_full();
    let root = match p.people.as_slice() {
        [] => root.child(face(&slots[0], &Person::new(""), p.size, &theme, marks, window, cx)),
        [one] => root.child(face(&slots[0], one, p.size, &theme, marks, window, cx)),
        many => {
            let fill = theme.surface.raised_hover;
            marks.fill("avatar group", fill, &[]);
            let mut root = root.bg(fill);
            for (i, (person, at)) in many.iter().zip(group_layout(shown, p.size)).enumerate() {
                let mini = face(&slots[i], person, at.size.width, &theme, marks, window, cx);
                root = root.child(div().absolute().left(px(at.origin.x)).top(px(at.origin.y)).child(mini));
            }
            root
        }
    };
    with_style(root, &p.style).into_any_element()
}
