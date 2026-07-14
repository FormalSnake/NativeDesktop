import AppKit
import CNd

/// Bind generated NativeView calls to this shell's context-owned plugin manager.
/// Keeping the context at the shell seam lets generated widget code stay backend-shaped.
func nd_plugin_view_create(_ viewKind: String, _ propsJSON: String) -> nd_widget? {
    guard let ctx = gCtx else { return nil }
    return viewKind.withCString { kind in
        propsJSON.withCString { props in
            CNd.nd_plugin_view_create(ctx, kind, props)
        }
    }
}

func nd_plugin_view_apply_props(_ viewKind: String, _ view: nd_widget, _ propsJSON: String) {
    guard let ctx = gCtx else { return }
    viewKind.withCString { kind in
        propsJSON.withCString { props in
            CNd.nd_plugin_view_apply_props(ctx, kind, view, props)
        }
    }
}
