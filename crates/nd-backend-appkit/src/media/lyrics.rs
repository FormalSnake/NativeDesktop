//! `lyrics_view` on AppKit: an `NSScrollView` (the OS's momentum and rubber band) over a column
//! of text layers. Following, the clip view is animated so the anchor row sits at the anchor
//! line; any scroll that is not ours ends following and shows the resync button. Rows fade with
//! distance and shrink when unlit; the anchor row fills word by word.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::time::Duration;

use block2::RcBlock;
use nd_components::media::{
    LyricClicked, LyricRow, LyricRowKind, LyricsFollowChanged, LyricsModel, LyricsProps, SCALE_ANIM, SCROLL,
    SCROLL_CURVE, UNLIT_SCALE, anchor_y, depth_opacity, follow_offset, interlude_fill, word_fill,
};
use nd_core::Size;
use nd_core::time::Instant;
use nd_theme::Theme;
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAttributedStringNSExtendedStringDrawing,
    NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSEvent, NSFont, NSFontAttributeName, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSScrollView, NSScrollViewWillStartLiveScrollNotification, NSStringDrawingOptions,
    NSView, NSViewBoundsDidChangeNotification,
};
use objc2_foundation::{
    NSAttributedString, NSMutableAttributedString, NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint,
    NSRange, NSRect, NSSize, NSString,
};
use objc2_quartz_core::{
    CAMediaTimingFunction, CATextLayer, CATransaction, CATransform3D, kCAAlignmentLeft, kCAAlignmentRight,
};

use super::{Input, NdMediaView, Ticker, cg, ns, reduce_motion, scale_of};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget, Waker};

const PAD_X: f64 = 12.;
const PAD_Y: f64 = 6.;

pub(crate) struct LyricsWidget;

impl CustomWidget for LyricsWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let mtm = cx.mtm();
        let props = props(cx).clone();
        let view = NdMediaView::new(mtm);
        let scroll = NSScrollView::new(mtm);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let document = NdMediaView::new(mtm);
        scroll.setDocumentView(Some(&document));
        view.addSubview(&scroll);
        let resync = NSButton::new(mtm);
        resync.setImage(crate::icons::symbol("locate-fixed").as_deref());
        resync.setBezelStyle(NSBezelStyle::Push);
        resync.setToolTip(Some(&NSString::from_str("Follow the song")));
        resync.setHidden(true);
        view.addSubview(&resync);
        let empty = super::seek::text_layer(false);
        unsafe { empty.setAlignmentMode(objc2_quartz_core::kCAAlignmentCenter) };
        view.layer().expect("layer-backed").addSublayer(&empty);
        let inner = Rc::new(LyricsInner {
            view,
            scroll,
            document,
            resync,
            empty,
            rows: RefCell::new(Vec::new()),
            props: RefCell::new(props),
            theme: RefCell::new(cx.theme().clone()),
            model: RefCell::new(LyricsModel::default()),
            programmatic: Rc::new(Cell::new(false)),
            snapped: Cell::new(false),
            ticker: Rc::default(),
            observers: RefCell::new(Vec::new()),
            target: RefCell::new(None),
            document_input: RefCell::new(None),
            emit: cx.emitter(),
            wake: cx.waker(),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        let document = Rc::new(Document(Rc::downgrade(&inner)));
        inner.document.set_input(Rc::downgrade(&document) as Weak<dyn Input>);
        *inner.document_input.borrow_mut() = Some(document);
        inner.wire(cx);
        inner.set_lyrics();
        inner.run();
        Box::new(LyricsNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a LyricsProps {
    cx.props().downcast_ref::<LyricsProps>().expect("lyrics node needs LyricsProps")
}

struct RowLayer {
    layer: Retained<CATextLayer>,
    /// Top and height inside the document.
    top: f64,
    height: f64,
    /// Words filled when the text was last built, so a tick only rebuilds on a change.
    filled: Cell<Option<usize>>,
}

struct LyricsInner {
    view: Retained<NdMediaView>,
    scroll: Retained<NSScrollView>,
    document: Retained<NdMediaView>,
    resync: Retained<NSButton>,
    empty: Retained<CATextLayer>,
    rows: RefCell<Vec<RowLayer>>,
    props: RefCell<LyricsProps>,
    theme: RefCell<Theme>,
    model: RefCell<LyricsModel>,
    /// The clip view moves because we moved it.
    programmatic: Rc<Cell<bool>>,
    /// The first placement of new lyrics jumps instead of animating.
    snapped: Cell<bool>,
    ticker: Rc<Ticker>,
    observers: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>>,
    target: RefCell<Option<Retained<crate::forms::NdControlTarget>>>,
    document_input: RefCell<Option<Rc<Document>>>,
    emit: Emit,
    wake: Waker,
}

impl Drop for LyricsInner {
    fn drop(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        for o in self.observers.get_mut().drain(..) {
            unsafe { center.removeObserver(o.as_ref()) };
        }
    }
}

fn row_font(theme: &Theme, kind: LyricRowKind) -> Retained<NSFont> {
    let size = match kind {
        LyricRowKind::Background => theme.type_scale.headline,
        _ => theme.type_scale.title,
    };
    NSFont::systemFontOfSize_weight(f64::from(size.as_f32()), unsafe { NSFontWeightSemibold })
}

impl LyricsInner {
    fn wire(self: &Rc<Self>, cx: &CustomCx<'_>) {
        let clip = self.scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        let center = NSNotificationCenter::defaultCenter();
        let me = Rc::downgrade(self);
        let moved = RcBlock::new(move |_n: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade()
                && !me.programmatic.get()
            {
                me.user_scrolled();
            }
        });
        let me = Rc::downgrade(self);
        let live = RcBlock::new(move |_n: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade() {
                me.user_scrolled();
            }
        });
        let clip_obj: &AnyObject = &clip;
        let scroll_obj: &AnyObject = &self.scroll;
        let observers = unsafe {
            vec![
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSViewBoundsDidChangeNotification),
                    Some(clip_obj),
                    None,
                    &moved,
                ),
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSScrollViewWillStartLiveScrollNotification),
                    Some(scroll_obj),
                    None,
                    &live,
                ),
            ]
        };
        *self.observers.borrow_mut() = observers;
        let me = Rc::downgrade(self);
        let target = crate::forms::NdControlTarget::new(cx.mtm(), move |_| {
            if let Some(me) = me.upgrade() {
                me.resync();
            }
        });
        target.wire(&self.resync);
        *self.target.borrow_mut() = Some(target);
    }

    fn user_scrolled(&self) {
        let synced = self.model.borrow().synced();
        if !synced {
            return;
        }
        let ended = self.model.borrow_mut().follow.user_scrolled(Instant::now());
        if ended {
            self.resync.setHidden(false);
            self.emit.emit(LyricsFollowChanged { following: false });
            self.wake.wake();
        }
    }

    fn resync(self: &Rc<Self>) {
        if self.model.borrow_mut().follow.resync() {
            self.resync.setHidden(true);
            self.emit.emit(LyricsFollowChanged { following: true });
            self.wake.wake();
            self.follow(true);
        }
    }

    fn set_lyrics(&self) {
        let props = self.props.borrow();
        let changed = self.model.borrow_mut().set_lyrics(&props.lyrics);
        drop(props);
        if changed {
            self.snapped.set(false);
            self.resync.setHidden(true);
            self.model.borrow_mut().advance(&self.props.borrow(), Instant::now());
            self.build_rows();
        }
    }

    /// One text layer per row, laid out down the document at the view's width.
    fn build_rows(&self) {
        let root = self.document.layer().expect("layer-backed");
        for row in self.rows.borrow_mut().drain(..) {
            row.layer.removeFromSuperlayer();
        }
        let model = self.model.borrow();
        let mut rows = Vec::with_capacity(model.rows.len());
        for row in &model.rows {
            let layer = CATextLayer::new();
            layer.setWrapped(true);
            unsafe { layer.setAlignmentMode(if row.opposite { kCAAlignmentRight } else { kCAAlignmentLeft }) };
            layer.setAnchorPoint(NSPoint::new(if row.opposite { 1. } else { 0. }, 0.5));
            root.addSublayer(&layer);
            rows.push(RowLayer { layer, top: 0., height: 0., filled: Cell::new(None) });
        }
        drop(model);
        *self.rows.borrow_mut() = rows;
        self.layout_rows();
    }

    fn text_for(&self, row: &LyricRow, lit: bool, anchor: bool, t: f64) -> (Retained<NSAttributedString>, usize) {
        let theme = self.theme.borrow();
        let synced = self.model.borrow().synced();
        let font = row_font(&theme, row.kind);
        let (sung, unsung) =
            if synced { (theme.text.primary, theme.text.secondary) } else { (theme.text.primary, theme.text.primary) };
        let text = if row.kind == LyricRowKind::Interlude {
            "\u{2022} \u{2022} \u{2022}".to_owned()
        } else {
            row.text.to_string()
        };
        let ns_text = NSString::from_str(&text);
        let s = NSMutableAttributedString::from_nsstring(&ns_text);
        let all = NSRange::new(0, ns_text.length());
        let base = if lit && !anchor { sung } else { unsung };
        unsafe {
            s.addAttribute_value_range(NSFontAttributeName, &font, all);
            s.addAttribute_value_range(NSForegroundColorAttributeName, &ns(base), all);
        }
        let mut filled = 0;
        if anchor && synced {
            if row.kind == LyricRowKind::Interlude {
                let dots = (interlude_fill(row, t) * 3.).floor() as usize;
                filled = dots;
                // Each dot is two UTF-16 units apart.
                let len = (dots * 2).min(ns_text.length());
                unsafe { s.addAttribute_value_range(NSForegroundColorAttributeName, &ns(sung), NSRange::new(0, len)) };
            } else if row.words.is_empty() {
                filled = 1;
                unsafe { s.addAttribute_value_range(NSForegroundColorAttributeName, &ns(sung), all) };
            } else {
                filled = row.words.iter().filter(|w| word_fill(w, t) > 0.).count();
                // Words are spread through the text in order; colour through the last started one.
                let mut end = 0usize;
                let mut from = 0usize;
                for word in row.words.iter().take(filled) {
                    if let Some(i) = text[from..].find(word.text.as_ref()) {
                        end = from + i + word.text.len();
                        from = end;
                    }
                }
                let utf16 = text[..end].encode_utf16().count();
                unsafe {
                    s.addAttribute_value_range(NSForegroundColorAttributeName, &ns(sung), NSRange::new(0, utf16))
                };
            }
        }
        (Retained::into_super(s), filled)
    }

    fn layout_rows(&self) {
        // Resizing the document moves the clip view; that is not the user scrolling.
        let was = self.programmatic.replace(true);
        self.layout_rows_inner();
        self.programmatic.set(was);
    }

    fn layout_rows_inner(&self) {
        let width = self.scroll.contentSize().width;
        let height = self.scroll.contentSize().height;
        let model = self.model.borrow();
        let synced = model.synced();
        let top = if synced { f64::from(anchor_y(height as f32)) } else { PAD_Y };
        let bottom = if synced { height - top } else { PAD_Y };
        let scale = scale_of(&self.view);
        let mut y = top;
        let t = LyricsModel::time(&self.props.borrow(), Instant::now());
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let mut rows = self.rows.borrow_mut();
        let inner_w = (width - 2. * PAD_X).max(1.);
        for (i, (row, layer)) in model.rows.iter().zip(rows.iter_mut()).enumerate() {
            let (text, filled) = self.text_for(row, model.lit.contains(i), i == model.lit.anchor, t);
            let bounds = text.boundingRectWithSize_options_context(
                NSSize::new(inner_w, 1e6),
                NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::UsesFontLeading,
                None,
            );
            let h = bounds.size.height.ceil();
            layer.layer.setTransform(CATransform3D::new_scale(1., 1., 1.));
            layer.layer.setFrame(NSRect::new(NSPoint::new(PAD_X, y + PAD_Y), NSSize::new(inner_w, h)));
            layer.layer.setContentsScale(scale);
            let object: &AnyObject = &text;
            unsafe { layer.layer.setString(Some(object)) };
            layer.filled.set(Some(filled));
            layer.top = y;
            layer.height = h + 2. * PAD_Y;
            y += layer.height;
        }
        drop((rows, model));
        let doc = NSSize::new(width, (y + bottom).max(height));
        if self.document.frame().size != doc {
            self.document.setFrameSize(doc);
        }
        self.style_rows(false);
        // The empty state, centred.
        let props = self.props.borrow();
        let theme = self.theme.borrow();
        let empty = self.rows.borrow().is_empty();
        self.empty.setHidden(!empty);
        if empty {
            let font = NSFont::systemFontOfSize(f64::from(theme.type_scale.body.as_f32()));
            super::seek::set_font(&self.empty, &font);
            self.empty.setForegroundColor(Some(&cg(theme.text.secondary)));
            super::seek::set_string(&self.empty, &props.empty_text);
            self.empty.setContentsScale(scale);
            let b = self.view.bounds();
            let line = (font.ascender() - font.descender()).ceil();
            self.empty.setFrame(NSRect::new(
                NSPoint::new(0., ((b.size.height - line) / 2.).round()),
                NSSize::new(b.size.width, line),
            ));
        }
        CATransaction::commit();
    }

    /// Opacity and size by lit state and distance from the anchor.
    fn style_rows(&self, animated: bool) {
        let model = self.model.borrow();
        if !model.synced() {
            return;
        }
        let rows = self.rows.borrow();
        let height = self.scroll.contentSize().height as f32;
        let pitch =
            if rows.is_empty() { 1. } else { rows.iter().map(|r| r.height).sum::<f64>() as f32 / rows.len() as f32 };
        let above = (anchor_y(height) / pitch).max(1.);
        let below = ((height - anchor_y(height)) / pitch).max(1.);
        let animate = animated && !reduce_motion();
        CATransaction::begin();
        CATransaction::setDisableActions(!animate);
        CATransaction::setAnimationDuration(SCALE_ANIM.as_secs_f64());
        let anchor = model.lit.anchor;
        for (i, row) in rows.iter().enumerate() {
            let lit = model.lit.contains(i);
            let distance = i.abs_diff(anchor);
            let span = if i < anchor { above } else { below };
            let mut opacity = if lit { 1. } else { depth_opacity(distance, span) };
            if model.rows[i].kind == LyricRowKind::Background {
                opacity *= 0.7;
            }
            row.layer.setOpacity(opacity);
            let s = if lit { 1. } else { f64::from(UNLIT_SCALE) };
            row.layer.setTransform(CATransform3D::new_scale(s, s, 1.));
        }
        CATransaction::commit();
    }

    /// Rebuilds the text of rows whose lit state or word fill changed.
    fn refresh_text(&self) {
        let model = self.model.borrow();
        let rows = self.rows.borrow();
        let t = LyricsModel::time(&self.props.borrow(), Instant::now());
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for (i, (row, layer)) in model.rows.iter().zip(rows.iter()).enumerate() {
            let (text, filled) = self.text_for(row, model.lit.contains(i), i == model.lit.anchor, t);
            let key =
                filled + usize::from(model.lit.contains(i)) * 10_000 + usize::from(i == model.lit.anchor) * 20_000;
            if layer.filled.get() != Some(key) {
                let object: &AnyObject = &text;
                unsafe { layer.layer.setString(Some(object)) };
                layer.filled.set(Some(key));
            }
        }
        CATransaction::commit();
    }

    /// Moves the clip view so the anchor row's top sits at the anchor line.
    fn follow(&self, animated: bool) {
        let model = self.model.borrow();
        if !model.synced() || !model.follow.following() {
            return;
        }
        let Some(row) = self.rows.borrow().get(model.lit.anchor).map(|r| r.top) else { return };
        drop(model);
        let height = self.scroll.contentSize().height as f32;
        let doc_h = self.document.frame().size.height;
        let y = f64::from(follow_offset(row as f32, height)).clamp(0., (doc_h - f64::from(height)).max(0.));
        let clip = self.scroll.contentView();
        if (clip.bounds().origin.y - y).abs() < 0.5 {
            return;
        }
        let target = NSPoint::new(0., y);
        self.programmatic.set(true);
        if !animated || reduce_motion() {
            clip.setBoundsOrigin(target);
            self.scroll.reflectScrolledClipView(&clip);
            self.programmatic.set(false);
            return;
        }
        let [a, b, c, d] = SCROLL_CURVE;
        let curve = CAMediaTimingFunction::functionWithControlPoints(a, b, c, d);
        let scroll = self.scroll.clone();
        let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
            let ctx = unsafe { ctx.as_ref() };
            ctx.setDuration(SCROLL.as_secs_f64());
            ctx.setTimingFunction(Some(&curve));
            let clip = scroll.contentView();
            clip.animator().setBoundsOrigin(target);
            scroll.reflectScrolledClipView(&clip);
        });
        let programmatic = self.programmatic.clone();
        let done = RcBlock::new(move || programmatic.set(false));
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    /// Ticks while playing, and while a resync timeout is pending.
    fn run(self: &Rc<Self>) {
        let wanted = {
            let props = self.props.borrow();
            let model = self.model.borrow();
            model.synced() && (props.clock.playing || (!model.follow.following() && props.resync_after.is_some()))
        };
        self.tick();
        if !wanted {
            self.ticker.stop();
            return;
        }
        let me = Rc::downgrade(self);
        self.ticker.start(30., move || {
            let Some(me) = me.upgrade() else { return false };
            if me.view.window().is_none() {
                return false;
            }
            me.tick();
            let props = me.props.borrow();
            let model = me.model.borrow();
            props.clock.playing || (!model.follow.following() && props.resync_after.is_some())
        });
    }

    fn tick(self: &Rc<Self>) {
        let now = Instant::now();
        let resync_after = self.props.borrow().resync_after;
        if self.model.borrow_mut().follow.tick(now, resync_after) {
            self.resync.setHidden(true);
            self.emit.emit(LyricsFollowChanged { following: true });
            self.wake.wake();
            self.follow(true);
        }
        let changed = self.model.borrow_mut().advance(&self.props.borrow(), now);
        self.refresh_text();
        if changed {
            self.style_rows(true);
            let first = !self.snapped.replace(true);
            self.follow(!first);
        }
    }

    fn click(&self, at: NSPoint) {
        let model = self.model.borrow();
        if !model.synced() {
            return;
        }
        let rows = self.rows.borrow();
        let Some(i) = rows.iter().position(|r| at.y >= r.top && at.y < r.top + r.height) else { return };
        let row = &model.rows[i];
        self.emit.emit(LyricClicked { line: row.line, start: Duration::from_secs_f64(row.start.max(0.)) });
        self.wake.wake();
    }
}

impl Input for LyricsInner {
    fn layout(&self) {
        let b = self.view.bounds();
        if self.scroll.frame() != b {
            self.scroll.setFrame(b);
        }
        let size = self.resync.fittingSize();
        let inset = 12.;
        self.resync.setFrame(NSRect::new(
            NSPoint::new(b.size.width - size.width - inset, b.size.height - size.height - inset),
            size,
        ));
        self.layout_rows();
        let was = self.programmatic.replace(true);
        self.follow(false);
        self.programmatic.set(was);
    }
}

/// Clicks on the document view seek to the row under the pointer.
struct Document(Weak<LyricsInner>);

impl Input for Document {
    fn mouse_up(&self, at: NSPoint, _event: &NSEvent) {
        if let Some(inner) = self.0.upgrade() {
            inner.click(at);
        }
    }
}

struct LyricsNative {
    inner: Rc<LyricsInner>,
}

impl CustomNative for LyricsNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = props(cx).clone();
        self.inner.set_lyrics();
        self.inner.run();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.layout_rows();
    }

    fn measure(&self, width: Option<f32>) -> Size {
        Size { width: width.unwrap_or(320.), height: 240. }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        let theme = self.inner.theme.borrow();
        let model = self.inner.model.borrow();
        let size = theme.type_scale.title.as_f32();
        if model.rows.is_empty() {
            return vec![CustomPaint {
                label: self.inner.props.borrow().empty_text.to_string(),
                color: theme.text.secondary,
                under: Vec::new(),
                text: Some((theme.type_scale.body.as_f32(), 400.)),
            }];
        }
        let mut out = vec![CustomPaint {
            label: "lyric".into(),
            color: theme.text.primary,
            under: Vec::new(),
            text: Some((size, 600.)),
        }];
        if model.synced() {
            out.push(CustomPaint {
                label: "lyric unlit".into(),
                color: theme.text.secondary,
                under: Vec::new(),
                text: Some((size, 600.)),
            });
        }
        out
    }
}
