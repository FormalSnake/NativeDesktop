use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSToolbar, NSToolbarDelegate, NSToolbarItem, NSToolbarItemIdentifier, NSWindow};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSString};

pub struct ToolbarIvars {
    items: Vec<Retained<NSToolbarItem>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToolbarIvars]
    struct NdToolbarDelegate;

    unsafe impl NSObjectProtocol for NdToolbarDelegate {}

    unsafe impl NSToolbarDelegate for NdToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _will_insert: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.ivars().items.iter().find(|item| &*item.itemIdentifier() == identifier).cloned()
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            self.identifiers()
        }
    }
);

impl NdToolbarDelegate {
    fn identifiers(&self) -> Retained<NSArray<NSToolbarItemIdentifier>> {
        let ids: Vec<Retained<NSString>> = self.ivars().items.iter().map(|i| i.itemIdentifier()).collect();
        NSArray::from_retained_slice(&ids)
    }
}

/// An `NSToolbar` serving a fixed set of items. Keep it alive as long as the window: the
/// toolbar holds its delegate weakly.
pub struct ToolbarHost {
    pub toolbar: Retained<NSToolbar>,
    _delegate: Retained<NdToolbarDelegate>,
}

impl ToolbarHost {
    pub fn attach(
        mtm: MainThreadMarker,
        window: &NSWindow,
        identifier: &str,
        items: Vec<Retained<NSToolbarItem>>,
    ) -> Self {
        let delegate: Retained<NdToolbarDelegate> =
            unsafe { msg_send![super(NdToolbarDelegate::alloc(mtm).set_ivars(ToolbarIvars { items })), init] };
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(identifier));
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.setToolbar(Some(&toolbar));
        Self { toolbar, _delegate: delegate }
    }
}
