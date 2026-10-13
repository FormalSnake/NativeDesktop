//! `overflow_*_scroll()` on AppKit: an `NSScrollView` filling the div's layer view, with a flipped
//! document view holding the div's children at the frames nd lays them out at. The scroll view
//! keeps the OS's own momentum, rubber band and scrollers; the document is sized after each
//! transaction to the children's extent, on the axes that scroll.

use nd_gesture::Axes;
use objc2::rc::Retained;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSScrollView};
use objc2_foundation::{MainThreadMarker, NSSize};

use crate::look::ViewLook;
use crate::views::NdLayerView;

pub(crate) struct ScrollBox {
    pub scroll: Retained<NSScrollView>,
    pub document: Retained<NdLayerView>,
}

impl ScrollBox {
    pub fn new(mtm: MainThreadMarker, outer: &NdLayerView, axes: Axes) -> Self {
        let scroll = NSScrollView::new(mtm);
        scroll.setDrawsBackground(false);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        scroll.setFrame(outer.bounds());
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let document = NdLayerView::new(mtm, &ViewLook::default());
        scroll.setDocumentView(Some(&document));
        outer.addSubview(&scroll);
        let this = Self { scroll, document };
        this.set_axes(axes);
        this
    }

    /// A div that starts scrolling after it was created: its children move into the document.
    pub fn adopt(mtm: MainThreadMarker, outer: &NdLayerView, axes: Axes) -> Self {
        let children = outer.subviews();
        let this = Self::new(mtm, outer, axes);
        for child in children.iter() {
            this.document.addSubview(&child);
        }
        this
    }

    /// The div stops scrolling: its children go back under it.
    pub fn dissolve(self, outer: &NdLayerView) {
        for child in self.document.subviews().iter() {
            outer.addSubview(&child);
        }
        self.scroll.removeFromSuperview();
    }

    pub fn set_axes(&self, axes: Axes) {
        self.scroll.setHasVerticalScroller(axes.vertical);
        self.scroll.setHasHorizontalScroller(axes.horizontal);
        self.scroll.setHorizontalScrollElasticity(if axes.horizontal {
            objc2_app_kit::NSScrollElasticity::Automatic
        } else {
            objc2_app_kit::NSScrollElasticity::None
        });
        self.scroll.setVerticalScrollElasticity(if axes.vertical {
            objc2_app_kit::NSScrollElasticity::Automatic
        } else {
            objc2_app_kit::NSScrollElasticity::None
        });
    }

    /// Sizes the document to its children's extent, never smaller than the visible area, and
    /// pinned to it on an axis that does not scroll.
    pub fn fit_document(&self) {
        let visible = self.scroll.contentSize();
        let (mut w, mut h) = (0f64, 0f64);
        for child in self.document.subviews().iter() {
            let f = child.frame();
            w = w.max(f.origin.x + f.size.width);
            h = h.max(f.origin.y + f.size.height);
        }
        let size = NSSize::new(
            if self.scroll.hasHorizontalScroller() { w.max(visible.width) } else { visible.width },
            if self.scroll.hasVerticalScroller() { h.max(visible.height) } else { visible.height },
        );
        if self.document.frame().size != size {
            self.document.setFrameSize(size);
        }
    }
}
