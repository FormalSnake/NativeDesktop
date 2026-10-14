//! `chat_bubble` and `avatar` on AppKit: shape and text layers painted from the theme. The
//! bubble's text is a selectable `NSTextField` label, so copy and the services menu work.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nd_components::media::{
    AvatarProps, BUBBLE, BubbleSide, ChatBubbleProps, MONOGRAM_MIN, Person, group_layout, initials, monogram_size,
};
use nd_core::Size;
use nd_theme::Theme;
use objc2::rc::Retained;
use objc2_app_kit::{NSFont, NSFontWeightRegular, NSFontWeightSemibold, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATextLayer, CATransaction, kCAAlignmentCenter};

use super::image::ImageSlot;
use super::seek::{set_font, set_string};
use super::{Input, NdMediaView, cg, ns, scale_of};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget};

/// A rounded rect with its own radius per corner, `[top_left, top_right, bottom_right,
/// bottom_left]`, in flipped coordinates.
fn rounded(path: &CGMutablePath, r: NSRect, radii: [f64; 4]) {
    let (x0, y0) = (r.origin.x, r.origin.y);
    let (x1, y1) = (x0 + r.size.width, y0 + r.size.height);
    let max = r.size.width.min(r.size.height) / 2.;
    let [tl, tr, br, bl] = radii.map(|v| v.min(max).max(0.));
    let m = std::ptr::null();
    unsafe {
        CGMutablePath::move_to_point(Some(path), m, x0 + tl, y0);
        CGMutablePath::add_arc_to_point(Some(path), m, x1, y0, x1, y0 + tr, tr);
        CGMutablePath::add_arc_to_point(Some(path), m, x1, y1, x1 - br, y1, br);
        CGMutablePath::add_arc_to_point(Some(path), m, x0, y1, x0, y1 - bl, bl);
        CGMutablePath::add_arc_to_point(Some(path), m, x0, y0, x0 + tl, y0, tl);
    }
    CGMutablePath::close_subpath(Some(path));
}

/// The tail: a lobe leaving the bubble's bottom outer corner and curving out to a point level
/// with its bottom edge.
fn tail(path: &CGMutablePath, bubble: NSRect, side: BubbleSide) {
    let (tw, th) = (f64::from(BUBBLE.tail_width), f64::from(BUBBLE.tail_height));
    let bottom = bubble.origin.y + bubble.size.height;
    let (edge, out) = match side {
        BubbleSide::Sent => (bubble.origin.x + bubble.size.width, 1.),
        BubbleSide::Received => (bubble.origin.x, -1.),
    };
    let m = std::ptr::null();
    unsafe {
        CGMutablePath::move_to_point(Some(path), m, edge - out * th, bottom - th);
        CGMutablePath::add_line_to_point(Some(path), m, edge, bottom - th);
        CGMutablePath::add_quad_curve_to_point(Some(path), m, edge, bottom - 2., edge + out * tw, bottom);
        CGMutablePath::add_quad_curve_to_point(
            Some(path),
            m,
            edge - out * tw * 0.5,
            bottom,
            edge - out * th,
            bottom - 3.,
        );
    }
    CGMutablePath::close_subpath(Some(path));
}

fn bubble_font(theme: &Theme, emoji: bool) -> Retained<NSFont> {
    let size = if emoji { BUBBLE.emoji_size } else { theme.type_scale.body.as_f32() };
    NSFont::systemFontOfSize_weight(f64::from(size), unsafe { NSFontWeightRegular })
}

fn colors(theme: &Theme, side: BubbleSide) -> (nd_ui::prelude::Hsla, nd_ui::prelude::Hsla) {
    match side {
        BubbleSide::Sent => (theme.accent, theme.text.on_accent),
        BubbleSide::Received => (theme.surface.raised_hover, theme.text.primary),
    }
}

pub(crate) struct BubbleWidget;

impl CustomWidget for BubbleWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = bubble_props(cx).clone();
        let view = NdMediaView::new(cx.mtm());
        let shape = CAShapeLayer::new();
        view.layer().expect("layer-backed").addSublayer(&shape);
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(&props.text), cx.mtm());
        label.setSelectable(props.selectable);
        label.setDrawsBackground(false);
        view.addSubview(&label);
        let inner = Rc::new(BubbleInner {
            view,
            shape,
            label,
            pills: RefCell::new(Vec::new()),
            props: RefCell::new(props),
            theme: RefCell::new(cx.theme().clone()),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.restyle();
        Box::new(BubbleNative { inner })
    }
}

fn bubble_props<'a>(cx: &'a CustomCx<'_>) -> &'a ChatBubbleProps {
    cx.props().downcast_ref::<ChatBubbleProps>().expect("bubble node needs ChatBubbleProps")
}

struct BubbleInner {
    view: Retained<NdMediaView>,
    shape: Retained<CAShapeLayer>,
    label: Retained<NSTextField>,
    pills: RefCell<Vec<(Retained<CALayer>, Retained<CATextLayer>)>>,
    props: RefCell<ChatBubbleProps>,
    theme: RefCell<Theme>,
}

impl BubbleInner {
    fn restyle(&self) {
        let props = self.props.borrow();
        let theme = self.theme.borrow();
        let (fill, ink) = colors(&theme, props.side);
        self.label.setStringValue(&NSString::from_str(&props.text));
        self.label.setFont(Some(&bubble_font(&theme, props.emoji_only)));
        self.label.setTextColor(Some(&ns(if props.emoji_only { theme.text.primary } else { ink })));
        self.label.setSelectable(props.selectable);
        self.shape.setFillColor(Some(&cg(fill)));
        self.shape.setHidden(props.emoji_only);
        self.view.setToolTip(props.tooltip.as_deref().map(NSString::from_str).as_deref());
        let root = self.view.layer().expect("layer-backed");
        let mut pills = self.pills.borrow_mut();
        while pills.len() < props.reactions.len() {
            let pill = CALayer::new();
            let text = super::seek::text_layer(false);
            unsafe { text.setAlignmentMode(kCAAlignmentCenter) };
            pill.addSublayer(&text);
            root.addSublayer(&pill);
            pills.push((pill, text));
        }
        while pills.len() > props.reactions.len() {
            if let Some((pill, _)) = pills.pop() {
                pill.removeFromSuperlayer();
            }
        }
        let font = NSFont::systemFontOfSize_weight(f64::from(theme.type_scale.caption.as_f32()), unsafe {
            NSFontWeightSemibold
        });
        // The ring that cuts each pill out of the bubble scales with the pill's text.
        let ring = (font.pointSize() / 6.).round().max(1.);
        for ((pill, text), reaction) in pills.iter().zip(&props.reactions) {
            let (bg, fg) = if reaction.mine {
                (theme.accent, theme.text.on_accent)
            } else {
                (theme.surface.raised_hover, theme.text.primary)
            };
            pill.setBackgroundColor(Some(&cg(bg)));
            pill.setBorderColor(Some(&cg(theme.surface.base)));
            pill.setBorderWidth(ring);
            set_font(text, &font);
            text.setForegroundColor(Some(&cg(fg)));
            let label = if reaction.count > 1 {
                format!("{} {}", reaction.glyph, reaction.count)
            } else {
                reaction.glyph.to_string()
            };
            set_string(text, &label);
        }
        drop((props, theme, pills));
        self.layout();
    }

    /// The bubble's rect inside the view: the tail's width is kept free on the outer side.
    fn bubble_rect(&self) -> NSRect {
        let b = self.view.bounds();
        let props = self.props.borrow();
        if props.emoji_only {
            return b;
        }
        let tw = f64::from(BUBBLE.tail_width);
        let x = if props.side == BubbleSide::Received { tw } else { 0. };
        NSRect::new(NSPoint::new(x, 0.), NSSize::new((b.size.width - tw).max(0.), b.size.height))
    }

    fn text_size(&self, max_text: CGFloat) -> NSSize {
        let cell = self.label.cell().expect("a label has a cell");
        let s = cell.cellSizeForBounds(NSRect::new(NSPoint::ZERO, NSSize::new(max_text.max(1.), 1e6)));
        NSSize::new(s.width.ceil(), s.height.ceil())
    }
}

impl Input for BubbleInner {
    fn layout(&self) {
        let props = self.props.borrow();
        let r = self.bubble_rect();
        let (px, py) =
            if props.emoji_only { (0., 0.) } else { (f64::from(BUBBLE.padding_x), f64::from(BUBBLE.padding_y)) };
        let text = NSRect::new(
            NSPoint::new(r.origin.x + px, r.origin.y + py),
            NSSize::new((r.size.width - 2. * px).max(0.), (r.size.height - 2. * py).max(0.)),
        );
        if self.label.frame() != text {
            self.label.setFrame(text);
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let path = CGMutablePath::new();
        rounded(&path, r, props.corners().map(f64::from));
        if props.has_tail() {
            tail(&path, r, props.side);
        }
        self.shape.setPath(Some(&path));
        let h = f64::from(BUBBLE.reaction);
        let scale = scale_of(&self.view);
        let pills = self.pills.borrow();
        let mut x = match props.side {
            BubbleSide::Sent => r.origin.x + r.size.width,
            BubbleSide::Received => r.origin.x,
        };
        for (i, (pill, text)) in pills.iter().enumerate() {
            let label_w = text_width(text).max(h - 10.);
            let w = label_w + 10.;
            let px = match props.side {
                BubbleSide::Sent => {
                    x -= w;
                    x + 2.
                }
                BubbleSide::Received => {
                    let at = x - 2.;
                    x += w;
                    at
                }
            };
            let overlap = if i == 0 { 0. } else { 6. };
            let px = if props.side == BubbleSide::Sent { px + overlap } else { px - overlap };
            pill.setFrame(NSRect::new(NSPoint::new(px, -f64::from(BUBBLE.reaction_lift)), NSSize::new(w, h)));
            pill.setCornerRadius(h / 2.);
            text.setContentsScale(scale);
            text.setFrame(NSRect::new(NSPoint::new(0., (h - 16.) / 2.), NSSize::new(w, 16.)));
        }
        CATransaction::commit();
    }
}

fn text_width(text: &CATextLayer) -> f64 {
    text.preferredFrameSize().width.ceil()
}

struct BubbleNative {
    inner: Rc<BubbleInner>,
}

impl CustomNative for BubbleNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = bubble_props(cx).clone();
        self.inner.restyle();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.restyle();
    }

    fn measure(&self, width: Option<f32>) -> Size {
        let props = self.inner.props.borrow();
        let (px, py, tw) =
            if props.emoji_only { (0., 0., 0.) } else { (BUBBLE.padding_x, BUBBLE.padding_y, BUBBLE.tail_width) };
        let outer = width.unwrap_or(BUBBLE.max_width + tw).min(BUBBLE.max_width + tw);
        let max_text = f64::from(outer - tw - 2. * px);
        drop(props);
        let text = self.inner.text_size(max_text);
        Size { width: text.width as f32 + 2. * px + tw, height: text.height as f32 + 2. * py }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        let props = self.inner.props.borrow();
        let theme = self.inner.theme.borrow();
        let (fill, ink) = colors(&theme, props.side);
        let size = if props.emoji_only { BUBBLE.emoji_size } else { theme.type_scale.body.as_f32() };
        let mut out = Vec::new();
        if props.emoji_only {
            out.push(CustomPaint {
                label: props.text.to_string(),
                color: theme.text.primary,
                under: Vec::new(),
                text: Some((size, 400.)),
            });
        } else {
            out.push(CustomPaint { label: "bubble".into(), color: fill, under: Vec::new(), text: None });
            out.push(CustomPaint {
                label: props.text.to_string(),
                color: ink,
                under: vec![fill],
                text: Some((size, 400.)),
            });
        }
        for r in &props.reactions {
            let (bg, fg) = if r.mine {
                (theme.accent, theme.text.on_accent)
            } else {
                (theme.surface.raised_hover, theme.text.primary)
            };
            out.push(CustomPaint { label: "reaction".into(), color: bg, under: Vec::new(), text: None });
            if r.count > 1 {
                out.push(CustomPaint {
                    label: r.count.to_string(),
                    color: fg,
                    under: vec![bg],
                    text: Some((theme.type_scale.caption.as_f32(), 600.)),
                });
            }
        }
        out
    }
}

/// One person's disc: a monogram under their photo.
struct Disc {
    circle: Retained<CALayer>,
    monogram: Retained<CATextLayer>,
    photo: ImageSlot,
}

impl Disc {
    fn new(parent: &CALayer) -> Self {
        let circle = CALayer::new();
        circle.setMasksToBounds(true);
        let monogram = super::seek::text_layer(false);
        unsafe { monogram.setAlignmentMode(kCAAlignmentCenter) };
        let photo = ImageSlot::new();
        circle.addSublayer(&monogram);
        circle.addSublayer(&photo.0.layer);
        parent.addSublayer(&circle);
        Self { circle, monogram, photo }
    }

    fn show(&self, person: &Person, frame: NSRect, theme: &Theme, scale: f64) {
        let size = frame.size.width;
        self.circle.setFrame(frame);
        self.circle.setCornerRadius(size / 2.);
        self.circle.setBackgroundColor(Some(&cg(theme.accent_container.bg)));
        let points = monogram_size(size as f32);
        let font = NSFont::systemFontOfSize_weight(f64::from(points), unsafe { NSFontWeightSemibold });
        set_font(&self.monogram, &font);
        self.monogram.setForegroundColor(Some(&cg(theme.accent_container.fg)));
        let monogram = if (size as f32) < MONOGRAM_MIN { String::new() } else { initials(&person.name) };
        set_string(&self.monogram, &monogram);
        self.monogram.setContentsScale(scale);
        let line = (font.ascender() - font.descender()).ceil();
        self.monogram.setFrame(NSRect::new(NSPoint::new(0., ((size - line) / 2.).round()), NSSize::new(size, line)));
        self.photo.set_source(person.image.clone(), nd_components::media::ImageFit::Cover, true);
        self.photo.place(NSRect::new(NSPoint::ZERO, frame.size), scale);
    }

    fn remove(&self) {
        self.circle.removeFromSuperlayer();
    }
}

pub(crate) struct AvatarWidget;

impl CustomWidget for AvatarWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = avatar_props(cx).clone();
        let view = NdMediaView::new(cx.mtm());
        let group = CALayer::new();
        group.setMasksToBounds(true);
        view.layer().expect("layer-backed").addSublayer(&group);
        let inner = Rc::new(AvatarInner {
            view,
            group,
            discs: RefCell::new(Vec::new()),
            props: RefCell::new(props),
            theme: RefCell::new(cx.theme().clone()),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.layout();
        Box::new(AvatarNative { inner })
    }
}

fn avatar_props<'a>(cx: &'a CustomCx<'_>) -> &'a AvatarProps {
    cx.props().downcast_ref::<AvatarProps>().expect("avatar node needs AvatarProps")
}

struct AvatarInner {
    view: Retained<NdMediaView>,
    group: Retained<CALayer>,
    discs: RefCell<Vec<Disc>>,
    props: RefCell<AvatarProps>,
    theme: RefCell<Theme>,
}

impl Input for AvatarInner {
    fn layout(&self) {
        let props = self.props.borrow();
        let theme = self.theme.borrow();
        let size = f64::from(props.size);
        let scale = scale_of(&self.view);
        let people: Vec<&Person> = props.people.iter().take(4).collect();
        let frames: Vec<NSRect> = if people.len() <= 1 {
            vec![NSRect::new(NSPoint::ZERO, NSSize::new(size, size))]
        } else {
            group_layout(people.len(), props.size)
                .iter()
                .map(|r| {
                    NSRect::new(
                        NSPoint::new(f64::from(r.origin.x), f64::from(r.origin.y)),
                        NSSize::new(f64::from(r.size.width), f64::from(r.size.height)),
                    )
                })
                .collect()
        };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.group.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(size, size)));
        self.group.setCornerRadius(size / 2.);
        let fill = (people.len() > 1).then(|| cg(theme.surface.raised_hover));
        self.group.setBackgroundColor(fill.as_deref());
        let mut discs = self.discs.borrow_mut();
        while discs.len() < people.len().max(1) {
            discs.push(Disc::new(&self.group));
        }
        while discs.len() > people.len().max(1) {
            if let Some(d) = discs.pop() {
                d.remove();
            }
        }
        let anonymous = Person::new("");
        for (i, disc) in discs.iter().enumerate() {
            disc.show(people.get(i).copied().unwrap_or(&anonymous), frames[i], &theme, scale);
        }
        CATransaction::commit();
        self.view.setToolTip(props.tooltip.as_deref().map(NSString::from_str).as_deref());
    }
}

struct AvatarNative {
    inner: Rc<AvatarInner>,
}

impl CustomNative for AvatarNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = avatar_props(cx).clone();
        self.inner.layout();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.layout();
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        let s = self.inner.props.borrow().size;
        Size { width: s, height: s }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        let props = self.inner.props.borrow();
        let theme = self.inner.theme.borrow();
        let mut out = Vec::new();
        let group = props.people.len() > 1;
        if group {
            out.push(CustomPaint {
                label: "avatar group".into(),
                color: theme.surface.raised_hover,
                under: Vec::new(),
                text: None,
            });
        }
        let size = if group { (props.size * nd_components::media::AVATAR_GROUP_SCALE).round() } else { props.size };
        for person in props.people.iter().take(4) {
            let under = if group { vec![theme.surface.raised_hover] } else { Vec::new() };
            out.push(CustomPaint {
                label: "monogram".into(),
                color: theme.accent_container.bg,
                under: under.clone(),
                text: None,
            });
            if size >= MONOGRAM_MIN && person.image.is_none() {
                let mut on = under;
                on.push(theme.accent_container.bg);
                out.push(CustomPaint {
                    label: initials(&person.name),
                    color: theme.accent_container.fg,
                    under: on,
                    text: Some((monogram_size(size), 600.)),
                });
            }
        }
        out
    }
}
