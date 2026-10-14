//! `select` as `NSPopUpButton`, `NSSegmentedControl` for `Mapping::Segmented`, or a group of
//! radio `NSButton`s in an `NSStackView` for `Mapping::Radio`. Changing the mapping or the
//! options recreates the control. `tab_strip` is a select-one segmented control too.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{Mapping, SelectChanged, SelectProps, TabChanged, TabSpec, TabStripProps};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::Synced;
use nd_ui::prelude::{Hsla, SharedString};
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityRadioGroupRole, NSButton, NSButtonType, NSControl, NSControlStateValueOff,
    NSControlStateValueOn, NSLayoutAttribute, NSPopUpButton, NSSegmentStyle, NSSegmentSwitchTracking,
    NSSegmentedControl, NSStackView, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSRect, NSString};

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::look::Color;

enum View {
    PopUp(Retained<NSPopUpButton>),
    Segmented(Retained<NSSegmentedControl>),
    /// One radio button per option; AppKit makes buttons sharing a superview and an action one group.
    Radio {
        stack: Retained<NSStackView>,
        buttons: Vec<Retained<NSButton>>,
    },
}

#[derive(PartialEq)]
enum Style {
    PopUp,
    Segmented,
    Radio,
}

pub(crate) struct SelectControl {
    view: View,
    _target: Retained<NdControlTarget>,
    props: SelectProps,
    selected: Rc<RefCell<Synced<usize>>>,
}

fn style(mapping: Mapping) -> Style {
    match mapping {
        Mapping::Automatic | Mapping::PopUp => Style::PopUp,
        Mapping::Segmented => Style::Segmented,
        Mapping::Radio => Style::Radio,
    }
}

fn orient(stack: &NSStackView, horizontal: bool) {
    if horizontal {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        stack.setAlignment(NSLayoutAttribute::CenterY);
    } else {
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(NSLayoutAttribute::Leading);
    }
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
            } else if let Some(b) = sender.downcast_ref::<NSButton>() {
                let index = b.tag();
                // Clicking the chosen radio again changes nothing.
                if reported.borrow().value() == Some(&(index as usize)) {
                    return;
                }
                index
            } else {
                return;
            };
            if index >= 0 {
                reported.borrow_mut().report(index as usize);
                cx.send(SelectChanged { index: index as usize });
            }
        });
        let titles: Vec<Retained<NSString>> = props.options.iter().map(|o| NSString::from_str(o)).collect();
        let view = if style(props.mapping) == Style::Radio {
            let stack = NSStackView::new(mtm);
            orient(&stack, props.horizontal);
            stack.setAccessibilityRole(Some(unsafe { NSAccessibilityRadioGroupRole }));
            let buttons: Vec<_> = titles
                .iter()
                .enumerate()
                .map(|(i, title)| {
                    let button = NSButton::new(mtm);
                    button.setButtonType(NSButtonType::Radio);
                    button.setTitle(title);
                    button.setTag(i as isize);
                    target.wire(&button);
                    stack.addArrangedSubview(&button);
                    button
                })
                .collect();
            View::Radio { stack, buttons }
        } else if style(props.mapping) == Style::Segmented {
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

    /// The single native control, absent for a radio group, which is a view of several.
    fn control(&self) -> Option<&NSControl> {
        match &self.view {
            View::PopUp(v) => Some(v),
            View::Segmented(v) => Some(v),
            View::Radio { .. } => None,
        }
    }

    fn apply_common(&self, old: Option<&SelectProps>, new: &SelectProps) {
        let old_common = old.map(|o| (o.enabled, &o.tooltip));
        apply_common(self.control(), self.view(), old_common, new.enabled, &new.tooltip);
        if let View::Radio { buttons, .. } = &self.view
            && old.is_none_or(|o| o.enabled != new.enabled)
        {
            for button in buttons {
                button.setEnabled(new.enabled);
            }
        }
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
            View::Radio { buttons, .. } => {
                for (n, button) in buttons.iter().enumerate() {
                    button.setState(if n == *i { NSControlStateValueOn } else { NSControlStateValueOff });
                }
                *i
            }
        });
    }
}

impl FormControl for SelectControl {
    fn view(&self) -> &NSView {
        match &self.view {
            View::PopUp(v) => v,
            View::Segmented(v) => v,
            View::Radio { stack, .. } => stack,
        }
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&SelectProps, &SelectProps) = (props(old), props(new));
        if style(old.mapping) != style(new.mapping) || old.options != new.options {
            return false;
        }
        if let View::Radio { stack, .. } = &self.view
            && old.horizontal != new.horizontal
        {
            orient(stack, new.horizontal);
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
        probe.enabled = Some(match &self.view {
            View::PopUp(p) => p.isEnabled(),
            View::Segmented(s) => s.isEnabled(),
            View::Radio { buttons, .. } => buttons.iter().all(|b| b.isEnabled()),
        });
        let index = match &self.view {
            View::PopUp(p) => p.indexOfSelectedItem(),
            View::Segmented(s) => s.selectedSegment(),
            View::Radio { buttons, .. } => {
                buttons.iter().position(|b| b.state() == NSControlStateValueOn).map_or(-1, |i| i as isize)
            }
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
            View::Radio { buttons, .. } => {
                for (n, button) in buttons.iter().enumerate() {
                    button.setState(if n == index { NSControlStateValueOn } else { NSControlStateValueOff });
                }
            }
        }
        Some(match (&self.view, self.control()) {
            (View::Radio { buttons, .. }, _) => send_action(&buttons[index]),
            (_, Some(control)) => send_action(control),
            _ => unreachable!("a non-radio select has one control"),
        })
    }

    fn set_accent(&self, accent: Option<Hsla>) {
        let color = accent.map(|a| Color::from(a).ns_color());
        match &self.view {
            View::Segmented(s) => s.setSelectedSegmentBezelColor(color.as_deref()),
            View::PopUp(p) => p.setBezelColor(color.as_deref()),
            View::Radio { buttons, .. } => buttons.iter().for_each(|b| b.setContentTintColor(color.as_deref())),
        }
    }
}

/// `tab_strip` (every presentation) as a select-one `NSSegmentedControl`, what `NSTabView`
/// draws for its tabs. Tabs that change rebuild the segments in place.
pub(crate) struct TabStripControl {
    control: Retained<NSSegmentedControl>,
    _target: Retained<NdControlTarget>,
    props: TabStripProps,
    /// The tab ids by segment, which the target reports by.
    ids: Rc<RefCell<Vec<SharedString>>>,
    selected: Rc<RefCell<Synced<usize>>>,
}

impl TabStripControl {
    pub fn new(cx: Ctx, props: &TabStripProps) -> Self {
        let mtm = cx.mtm;
        let selected: Rc<RefCell<Synced<usize>>> = Rc::default();
        let ids: Rc<RefCell<Vec<SharedString>>> = Rc::default();
        let (reported, by_index) = (selected.clone(), ids.clone());
        let target = NdControlTarget::new(mtm, move |sender| {
            let Some(control) = sender.downcast_ref::<NSSegmentedControl>() else { return };
            let Ok(index) = usize::try_from(control.selectedSegment()) else { return };
            // Clicking the tab already shown changes nothing.
            if reported.borrow().value() == Some(&index) {
                return;
            }
            let Some(id) = by_index.borrow().get(index).cloned() else { return };
            reported.borrow_mut().report(index);
            cx.send(TabChanged { id });
        });
        let control = NSSegmentedControl::new(mtm);
        control.setSegmentStyle(NSSegmentStyle::Automatic);
        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        target.wire(&control);
        let this = Self { control, _target: target, props: props.clone(), ids, selected };
        this.set_tabs(&props.tabs);
        this.select(props);
        apply_common(Some(&this.control), &this.control, None, props.enabled, &props.tooltip);
        set_accessibility_label(&this.control, props.label.as_deref());
        this
    }

    fn set_tabs(&self, tabs: &[TabSpec]) {
        let control = &self.control;
        control.setSegmentCount(tabs.len() as isize);
        for (i, tab) in tabs.iter().enumerate() {
            control.setLabel_forSegment(&NSString::from_str(&tab.title), i as isize);
            let image = tab.icon.and_then(|icon| crate::icons::symbol(icon.lucide()));
            control.setImage_forSegment(image.as_deref(), i as isize);
        }
        *self.ids.borrow_mut() = tabs.iter().map(|t| t.id.clone()).collect();
    }

    fn select(&self, props: &TabStripProps) {
        let Some(index) = props.selected_index() else { return };
        let control = &self.control;
        self.selected.borrow_mut().apply(&index, |i| {
            control.setSelectedSegment(*i as isize);
            control.selectedSegment().max(0) as usize
        });
    }
}

impl FormControl for TabStripControl {
    fn view(&self) -> &NSView {
        &self.control
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&TabStripProps, &TabStripProps) = (props(old), props(new));
        if old.tabs != new.tabs {
            self.set_tabs(&new.tabs);
            // Re-counting the segments drops the selection; the app's is applied again.
            *self.selected.borrow_mut() = Synced::default();
        }
        self.select(new);
        apply_common(Some(&self.control), &self.control, Some((old.enabled, &old.tooltip)), new.enabled, &new.tooltip);
        if old.label != new.label {
            set_accessibility_label(&self.control, new.label.as_deref());
        }
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.control)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.control.isEnabled());
        let index = usize::try_from(self.control.selectedSegment()).ok();
        probe.value = index.and_then(|i| self.props.tabs.get(i)).map(|t| t.title.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::SetValue(value) = action else { return None };
        let Some(index) = self.props.tab_named(value).and_then(|t| self.props.tabs.iter().position(|x| x.id == t.id))
        else {
            return Some(Err(ActionError::NotActionable(format!("no tab \"{value}\""))));
        };
        if !self.control.isEnabled() {
            return Some(Err(ActionError::NotActionable("the tabs are disabled".into())));
        }
        self.control.setSelectedSegment(index as isize);
        Some(send_action(&self.control))
    }
}
