//! `select` as `NSPopUpButton`, or `NSSegmentedControl` for `Mapping::Segmented` and
//! `Mapping::Radio`. Changing the mapping or the options recreates the control.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{Mapping, SelectChanged, SelectProps};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::Synced;
use nd_ui::prelude::Hsla;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{NSControl, NSPopUpButton, NSSegmentedControl, NSView};
use objc2_foundation::{NSArray, NSRect, NSString};

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::look::Color;

enum View {
    PopUp(Retained<NSPopUpButton>),
    Segmented(Retained<NSSegmentedControl>),
}

pub(crate) struct SelectControl {
    view: View,
    _target: Retained<NdControlTarget>,
    props: SelectProps,
    selected: Rc<RefCell<Synced<usize>>>,
}

fn segmented(mapping: Mapping) -> bool {
    matches!(mapping, Mapping::Segmented | Mapping::Radio)
}

impl SelectControl {
    pub fn new(cx: Ctx, props: &SelectProps) -> Self {
        let mtm = cx.mtm;
        let selected: Rc<RefCell<Synced<usize>>> = Rc::default();
        let reported = selected.clone();
        let target = NdControlTarget::new(mtm, move |sender| {
            let index = if let Some(s) = sender.downcast_ref::<NSSegmentedControl>() {
                s.selectedSegment()
            } else if let Some(p) = sender.downcast_ref::<NSPopUpButton>() {
                p.indexOfSelectedItem()
            } else {
                return;
            };
            if index >= 0 {
                reported.borrow_mut().report(index as usize);
                cx.send(SelectChanged { index: index as usize });
            }
        });
        let titles: Vec<Retained<NSString>> = props.options.iter().map(|o| NSString::from_str(o)).collect();
        let view = if segmented(props.mapping) {
            let control = NSSegmentedControl::new(mtm);
            control.setSegmentCount(titles.len() as isize);
            for (i, title) in titles.iter().enumerate() {
                control.setLabel_forSegment(title, i as isize);
            }
            target.wire(&control);
            View::Segmented(control)
        } else {
            let control = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
            control.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
            target.wire(&control);
            View::PopUp(control)
        };
        let this = Self { view, _target: target, props: props.clone(), selected };
        this.select(props.selected);
        this.apply_common(None, props);
        this
    }

    fn control(&self) -> &NSControl {
        match &self.view {
            View::PopUp(v) => v,
            View::Segmented(v) => v,
        }
    }

    fn apply_common(&self, old: Option<&SelectProps>, new: &SelectProps) {
        apply_common(
            Some(self.control()),
            self.view(),
            old.map(|o| (o.enabled, &o.tooltip)),
            new.enabled,
            &new.tooltip,
        );
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(self.view(), new.label.as_deref());
        }
    }

    fn select(&self, index: usize) {
        let view = &self.view;
        self.selected.borrow_mut().apply(&index, |i| match view {
            View::PopUp(p) => {
                p.selectItemAtIndex(*i as isize);
                p.indexOfSelectedItem().max(0) as usize
            }
            View::Segmented(s) => {
                s.setSelectedSegment(*i as isize);
                s.selectedSegment().max(0) as usize
            }
        });
    }
}

impl FormControl for SelectControl {
    fn view(&self) -> &NSView {
        self.control()
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&SelectProps, &SelectProps) = (props(old), props(new));
        if segmented(old.mapping) != segmented(new.mapping) || old.options != new.options {
            return false;
        }
        self.select(new.selected);
        self.apply_common(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(self.view())
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.control().isEnabled());
        let index = match &self.view {
            View::PopUp(p) => p.indexOfSelectedItem(),
            View::Segmented(s) => s.selectedSegment(),
        };
        probe.value = usize::try_from(index).ok().and_then(|i| self.props.options.get(i)).map(|o| o.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::SetValue(value) = action else { return None };
        let Some(index) = nd_components::forms::option_index(&self.props.options, value) else {
            return Some(Err(ActionError::NotActionable(format!("no option \"{value}\""))));
        };
        match &self.view {
            View::PopUp(p) => p.selectItemAtIndex(index as isize),
            View::Segmented(s) => s.setSelectedSegment(index as isize),
        }
        Some(send_action(self.control()))
    }

    fn set_accent(&self, accent: Option<Hsla>) {
        let color = accent.map(|a| Color::from(a).ns_color());
        match &self.view {
            View::Segmented(s) => s.setSelectedSegmentBezelColor(color.as_deref()),
            View::PopUp(p) => p.setBezelColor(color.as_deref()),
        }
    }
}
