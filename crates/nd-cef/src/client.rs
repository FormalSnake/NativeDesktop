//! The CEF handler objects. Each holds only its page's id: CEF may release (and with some
//! handlers call) them off the main thread, so they never own main-thread state; every call
//! looks its page up in the main thread's registry and does the work there.

use std::os::raw::c_int;

use cef::*;

use super::page::{self, CefPage};

/// `CEF_SCHEME_OPTION_STANDARD | CORS_ENABLED | FETCH_ENABLED`: app schemes behave like http
/// (origins, relative URLs, `fetch`).
const SCHEME_OPTIONS: c_int = 1 | 16 | 64;

fn with<R: Default>(id: u64, f: impl FnOnce(&CefPage) -> R) -> R {
    page::get(id).map(|p| f(&p)).unwrap_or_default()
}

pub(crate) fn app(schemes: Vec<String>) -> App {
    NdApp::new(schemes)
}

wrap_app! {
    struct NdApp {
        schemes: Vec<String>,
    }

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let browser = process_type.is_none_or(|t| t.to_string().is_empty());
            if let (true, Some(line)) = (browser, command_line) {
                crate::runtime::chrome_switches(line);
                crate::pipe::switches(line);
                crate::extensions::switches(line);
                crate::platform::command_line(line);
            }
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            if let Some(registrar) = registrar {
                for scheme in &self.schemes {
                    registrar.add_custom_scheme(Some(&CefString::from(scheme.as_str())), SCHEME_OPTIONS);
                }
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(NdBrowserProcess::new(self.schemes.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct NdBrowserProcess {
        schemes: Vec<String>,
    }

    impl BrowserProcessHandler {
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            super::pump::schedule(delay_ms);
        }

        /// Chrome style would answer a second launch on the same profile by opening a window of
        /// its own.
        fn on_already_running_app_relaunch(&self, _command_line: Option<&mut CommandLine>, _current_directory: Option<&CefString>) -> c_int {
            1
        }

        fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
            if let (false, Some(line)) = (self.schemes.is_empty(), command_line) {
                line.append_switch_with_value(
                    Some(&CefString::from(super::SCHEMES_SWITCH)),
                    Some(&CefString::from(self.schemes.join(",").as_str())),
                );
            }
        }
    }
}

/// The DevTools window's browser: Chromium draws and handles it itself.
pub(crate) fn devtools_client() -> Client {
    NdDevtoolsClient::new()
}

wrap_client! {
    struct NdDevtoolsClient;

    impl Client {}
}

/// A page's handlers. A Chrome-style page is windowed: Chromium draws and takes input itself, so
/// it has no render handler.
pub(crate) fn client(id: u64, style: nd_webview::CefStyle) -> Client {
    let windowed = style == nd_webview::CefStyle::Chrome;
    NdClient::new(Handlers {
        render: (!windowed).then(|| NdRender::new(id)),
        command: NdCommand::new(id),
        life_span: NdLifeSpan::new(id),
        load: NdLoad::new(id),
        display: NdDisplay::new(id),
        context_menu: NdContextMenu::new(id),
        download: NdDownload::new(id),
        js_dialog: NdJsDialog::new(id),
        dialog: NdDialog::new(id),
        permission: NdPermission::new(id),
        find: NdFind::new(id),
        request: NdRequest::new(id),
        focus: NdFocus::new(id),
        drag: NdDrag::new(id),
        keyboard: NdKeyboard::new(id),
    })
}

/// One page's handler objects, made once and handed out on every lookup.
#[derive(Clone)]
struct Handlers {
    render: Option<RenderHandler>,
    command: CommandHandler,
    life_span: LifeSpanHandler,
    load: LoadHandler,
    display: DisplayHandler,
    context_menu: ContextMenuHandler,
    download: DownloadHandler,
    js_dialog: JsdialogHandler,
    dialog: DialogHandler,
    permission: PermissionHandler,
    find: FindHandler,
    request: RequestHandler,
    focus: FocusHandler,
    drag: DragHandler,
    keyboard: KeyboardHandler,
}

wrap_client! {
    struct NdClient {
        h: Handlers,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            self.h.render.clone()
        }

        fn command_handler(&self) -> Option<CommandHandler> {
            Some(self.h.command.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.h.life_span.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.h.load.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.h.display.clone())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.h.context_menu.clone())
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(self.h.download.clone())
        }

        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(self.h.js_dialog.clone())
        }

        fn dialog_handler(&self) -> Option<DialogHandler> {
            Some(self.h.dialog.clone())
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(self.h.permission.clone())
        }

        fn find_handler(&self) -> Option<FindHandler> {
            Some(self.h.find.clone())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.h.request.clone())
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(self.h.focus.clone())
        }

        fn drag_handler(&self) -> Option<DragHandler> {
            Some(self.h.drag.clone())
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(self.h.keyboard.clone())
        }
    }
}

wrap_render_handler! {
    struct NdRender {
        id: u64,
    }

    impl RenderHandler {
        fn accessibility_handler(&self) -> Option<AccessibilityHandler> {
            Some(NdAccessibility::new(self.id))
        }

        fn root_screen_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) -> c_int {
            with(self.id, |p| p.view.screen_rect().map(|r| rect.map(|out| *out = r)).is_some()) as c_int
        }

        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(out) = rect {
                *out = with(self.id, |p| Some(p.view.view_rect())).unwrap_or(Rect { x: 0, y: 0, width: 1, height: 1 });
            }
        }

        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: c_int,
            view_y: c_int,
            screen_x: Option<&mut c_int>,
            screen_y: Option<&mut c_int>,
        ) -> c_int {
            let Some((x, y)) = with(self.id, |p| p.view.screen_point(view_x, view_y)) else { return 0 };
            if let (Some(sx), Some(sy)) = (screen_x, screen_y) {
                *sx = x;
                *sy = y;
            }
            1
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, screen_info: Option<&mut ScreenInfo>) -> c_int {
            let (Some(out), Some((scale, rect))) = (screen_info, with(self.id, |p| p.view.screen_info())) else {
                return 0;
            };
            out.device_scale_factor = scale;
            out.depth = 24;
            out.depth_per_component = 8;
            out.rect = rect.clone();
            out.available_rect = rect;
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: c_int) {
            with(self.id, |p| p.view.popup_show(show != 0));
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            if let Some(rect) = rect {
                let rect = rect.clone();
                with(self.id, |p| p.view.popup_size(rect));
            }
        }

        #[cfg(target_os = "macos")]
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            if let Some(info) = info {
                let popup = type_ == PaintElementType::POPUP;
                let surface = info.shared_texture_io_surface;
                with(self.id, |p| p.view.present(popup, surface));
            }
        }

        #[cfg(target_os = "linux")]
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: c_int,
            height: c_int,
        ) {
            let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else { return };
            if buffer.is_null() || width == 0 || height == 0 {
                return;
            }
            // SAFETY: CEF hands a BGRA buffer of `width * height` pixels, valid for the call.
            let pixels = unsafe { std::slice::from_raw_parts(buffer, width * height * 4) };
            let popup = type_ == PaintElementType::POPUP;
            with(self.id, |p| p.view.present(popup, pixels, width, height));
        }

        fn start_dragging(
            &self,
            _browser: Option<&mut Browser>,
            drag_data: Option<&mut DragData>,
            allowed_ops: DragOperationsMask,
            x: c_int,
            y: c_int,
        ) -> c_int {
            let Some(data) = drag_data else { return 0 };
            let data = <DragData as Clone>::clone(data);
            with(self.id, |p| p.view.start_dragging(&data, allowed_ops, x, y)) as c_int
        }

        fn update_drag_cursor(&self, _browser: Option<&mut Browser>, operation: DragOperationsMask) {
            with(self.id, |p| p.view.set_drag_operation(operation));
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            let bounds = character_bounds.map(<[Rect]>::to_vec).unwrap_or_default();
            with(self.id, |p| p.view.set_composition_bounds(bounds));
        }

        fn on_text_selection_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_text: Option<&CefString>,
            _selected_range: Option<&Range>,
        ) {
            let text = selected_text.map(|t| t.to_string()).unwrap_or_default();
            with(self.id, |p| p.view.set_selection(text));
        }
    }
}

wrap_accessibility_handler! {
    struct NdAccessibility {
        id: u64,
    }

    impl AccessibilityHandler {
        fn on_accessibility_tree_change(&self, value: Option<&mut Value>) {
            if let Some(value) = value {
                let json = super::a11y::to_json(value);
                with(self.id, |p| p.view.accessibility_update(&json));
            }
        }

        fn on_accessibility_location_change(&self, value: Option<&mut Value>) {
            if let Some(value) = value {
                let json = super::a11y::to_json(value);
                with(self.id, |p| p.view.accessibility_locations(&json));
            }
        }
    }
}

wrap_life_span_handler! {
    struct NdLifeSpan {
        id: u64,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: c_int,
            popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut c_int>,
        ) -> c_int {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            let features = popup_features.cloned();
            with(self.id, |p| {
                p.before_popup(url, target_disposition, user_gesture != 0, features, window_info, client, settings)
            }) as c_int
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                let browser = browser.clone();
                with(self.id, |p| p.bind(browser));
            }
        }

        fn do_close(&self, _browser: Option<&mut Browser>) -> c_int {
            with(self.id, |p| p.do_close()) as c_int
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let id = browser.map(|b| b.identifier());
            page::closed(self.id, id);
        }
    }
}

wrap_load_handler! {
    struct NdLoad {
        id: u64,
    }

    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: c_int, can_go_back: c_int, can_go_forward: c_int) {
            with(self.id, |p| p.loading_state(is_loading != 0, can_go_back != 0, can_go_forward != 0));
        }

        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _http_status_code: c_int) {
            if frame.is_some_and(|f| f.is_main() != 0) {
                with(self.id, |p| p.load_end());
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            if frame.is_none_or(|f| f.is_main() == 0) {
                return;
            }
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            let url = failed_url.map(|u| u.to_string()).unwrap_or_default();
            let code = error_code.get_raw();
            with(self.id, |p| p.load_error(code, text, url));
        }
    }
}

wrap_display_handler! {
    struct NdDisplay {
        id: u64,
    }

    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|f| f.is_main() != 0) {
                let url = url.map(|u| u.to_string()).unwrap_or_default();
                with(self.id, |p| p.page.report_url(&url));
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            with(self.id, |p| p.page.report_title(&title));
        }

        fn on_favicon_urlchange(&self, _browser: Option<&mut Browser>, icon_urls: Option<&mut CefStringList>) {
            let urls: Vec<String> = icon_urls.map(|l| l.clone().into_iter().collect()).unwrap_or_default();
            with(self.id, |p| p.favicon(urls));
        }

        fn on_fullscreen_mode_change(&self, _browser: Option<&mut Browser>, fullscreen: c_int) {
            with(self.id, |p| p.page.emit(nd_webview::FullscreenChanged { fullscreen: fullscreen != 0 }));
        }

        fn on_status_message(&self, _browser: Option<&mut Browser>, value: Option<&CefString>) {
            let url = value.map(|v| v.to_string()).unwrap_or_default();
            with(self.id, |p| p.link_hover(url));
        }

        fn on_loading_progress_change(&self, _browser: Option<&mut Browser>, progress: f64) {
            with(self.id, |p| p.page.report_progress(progress));
        }

        #[cfg(target_os = "macos")]
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            cursor: *mut u8,
            _type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> c_int {
            with(self.id, |p| p.view.set_cursor(cursor.cast()));
            1
        }

        #[cfg(target_os = "linux")]
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: std::os::raw::c_ulong,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> c_int {
            with(self.id, |p| p.view.set_cursor(type_));
            1
        }
    }
}

wrap_context_menu_handler! {
    struct NdContextMenu {
        id: u64,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            if let (Some(params), Some(model)) = (params, model) {
                let (params, model) = (params.clone(), model.clone());
                with(self.id, |p| p.before_context_menu(&params, &model));
            }
        }

        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> c_int {
            let (Some(params), Some(model), Some(callback)) = (params, model, callback) else { return 0 };
            let (params, model, callback) = (params.clone(), model.clone(), callback.clone());
            with(self.id, |p| p.run_context_menu(&params, &model, callback)) as c_int
        }
    }
}

wrap_download_handler! {
    struct NdDownload {
        id: u64,
    }

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> c_int {
            1
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> c_int {
            let (Some(item), Some(callback)) = (download_item, callback) else { return 0 };
            // The store's own install download: Chrome style installs it, the app never sees it.
            if crate::store_browser::is_store_package(&crate::text(&item.original_url())) {
                return 0;
            }
            let suggested = suggested_name.map(|s| s.to_string()).unwrap_or_default();
            let callback = callback.clone();
            with(self.id, |p| p.before_download(item, suggested, callback)) as c_int
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            callback: Option<&mut DownloadItemCallback>,
        ) {
            if let (Some(item), Some(callback)) = (download_item, callback) {
                let callback = callback.clone();
                with(self.id, |p| p.download_updated(item, callback));
            }
        }
    }
}

wrap_jsdialog_handler! {
    struct NdJsDialog {
        id: u64,
    }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _browser: Option<&mut Browser>,
            _origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            _suppress_message: Option<&mut c_int>,
        ) -> c_int {
            let Some(callback) = callback else { return 0 };
            let message = message_text.map(|t| t.to_string()).unwrap_or_default();
            let default = default_prompt_text.map(|t| t.to_string()).unwrap_or_default();
            let callback = callback.clone();
            with(self.id, |p| p.js_dialog(dialog_type, message, default, callback)) as c_int
        }

        fn on_before_unload_dialog(
            &self,
            _browser: Option<&mut Browser>,
            _message_text: Option<&CefString>,
            is_reload: c_int,
            callback: Option<&mut JsdialogCallback>,
        ) -> c_int {
            let Some(callback) = callback else { return 0 };
            let callback = callback.clone();
            with(self.id, |p| p.before_unload(is_reload != 0, callback)) as c_int
        }
    }
}

wrap_dialog_handler! {
    struct NdDialog {
        id: u64,
    }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            _browser: Option<&mut Browser>,
            mode: FileDialogMode,
            _title: Option<&CefString>,
            default_file_path: Option<&CefString>,
            _accept_filters: Option<&mut CefStringList>,
            accept_extensions: Option<&mut CefStringList>,
            _accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> c_int {
            let Some(callback) = callback else { return 0 };
            let default = default_file_path.map(|p| p.to_string()).unwrap_or_default();
            let extensions: Vec<String> = accept_extensions.map(|l| l.clone().into_iter().collect()).unwrap_or_default();
            let callback = callback.clone();
            with(self.id, |p| p.file_dialog(mode, default, extensions, callback)) as c_int
        }
    }
}

wrap_permission_handler! {
    struct NdPermission {
        id: u64,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> c_int {
            let Some(callback) = callback else { return 0 };
            let origin = requesting_origin.map(|o| o.to_string()).unwrap_or_default();
            let callback = callback.clone();
            with(self.id, |p| p.media_permission(origin, requested_permissions, callback)) as c_int
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> c_int {
            let Some(callback) = callback else { return 0 };
            let origin = requesting_origin.map(|o| o.to_string()).unwrap_or_default();
            let callback = callback.clone();
            with(self.id, |p| p.permission_prompt(prompt_id, origin, requested_permissions, callback)) as c_int
        }

        fn on_dismiss_permission_prompt(&self, _browser: Option<&mut Browser>, prompt_id: u64, _result: PermissionRequestResult) {
            with(self.id, |p| p.permission_dismissed(prompt_id));
        }
    }
}

wrap_find_handler! {
    struct NdFind {
        id: u64,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: c_int,
            count: c_int,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: c_int,
            final_update: c_int,
        ) {
            with(self.id, |p| {
                p.page.emit(nd_webview::FindResult {
                    match_found: count > 0,
                    match_count: u32::try_from(count).ok(),
                    active_match: u32::try_from(active_match_ordinal).ok().filter(|n| *n > 0),
                    done: final_update != 0,
                })
            });
        }
    }
}

wrap_request_handler! {
    struct NdRequest {
        id: u64,
    }

    impl RequestHandler {
        /// IO thread: the app's request filter, never the page's state on the main thread.
        fn resource_request_handler(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _is_navigation: c_int,
            _is_download: c_int,
            request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut c_int>,
        ) -> Option<ResourceRequestHandler> {
            crate::filter::handler(browser, request, request_initiator)
        }

        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: c_int,
        ) -> c_int {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            with(self.id, |p| p.open_from_tab(url, target_disposition, user_gesture != 0)) as c_int
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            _status: TerminationStatus,
            error_code: c_int,
            error_string: Option<&CefString>,
        ) {
            let reason = error_string.map(|e| e.to_string()).filter(|e| !e.is_empty()).unwrap_or_else(|| format!("exit code {error_code}"));
            with(self.id, |p| p.page.emit(nd_webview::RenderProcessGone { reason }));
        }
    }
}

wrap_command_handler! {
    struct NdCommand {
        id: u64,
    }

    impl CommandHandler {
        fn on_chrome_command(&self, _browser: Option<&mut Browser>, command_id: c_int, _disposition: WindowOpenDisposition) -> c_int {
            with(self.id, |p| p.chrome_command(command_id)) as c_int
        }

        fn is_chrome_app_menu_item_visible(&self, _browser: Option<&mut Browser>, _command_id: c_int) -> c_int {
            0
        }

        fn is_chrome_page_action_icon_visible(&self, _icon_type: ChromePageActionIconType) -> c_int {
            0
        }

        fn is_chrome_toolbar_button_visible(&self, _button_type: ChromeToolbarButtonType) -> c_int {
            0
        }
    }
}

wrap_focus_handler! {
    struct NdFocus {
        id: u64,
    }

    impl FocusHandler {
        fn on_got_focus(&self, _browser: Option<&mut Browser>) {
            with(self.id, |p| {
                p.page.report_focus(true);
                // A click in a Chrome-style page's window reaches Chromium alone: nd focus follows.
                #[cfg(target_os = "linux")]
                if p.view.chrome().is_some() {
                    p.view.focus();
                }
            });
        }

        fn on_take_focus(&self, _browser: Option<&mut Browser>, next: c_int) {
            with(self.id, |p| p.take_focus(next != 0));
        }
    }
}

wrap_drag_handler! {
    struct NdDrag {
        id: u64,
    }

    impl DragHandler {
        fn on_drag_enter(&self, _browser: Option<&mut Browser>, _drag_data: Option<&mut DragData>, _mask: DragOperationsMask) -> c_int {
            0
        }
    }
}

wrap_keyboard_handler! {
    struct NdKeyboard {
        id: u64,
    }

    impl KeyboardHandler {
        /// Keys the page did not handle go back to AppKit, so the main menu's key equivalents
        /// (an app's accelerators) still run when a page has the keyboard. On Linux the app's
        /// bindings see every key before an off-screen page does, and a Chrome-style page's
        /// chords before Chromium does.
        #[cfg(target_os = "macos")]
        fn on_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, os_event: *mut u8) -> c_int {
            let Some(event) = event else { return 0 };
            let down = event.type_ == KeyEventType::RAWKEYDOWN || event.type_ == KeyEventType::KEYDOWN;
            if !down || os_event.is_null() {
                return 0;
            }
            with(self.id, |p| p.view.unhandled_key(os_event.cast())) as c_int
        }

        #[cfg(target_os = "linux")]
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut cef::sys::XEvent>, _is_keyboard_shortcut: Option<&mut c_int>) -> c_int {
            let Some(event) = event else { return 0 };
            with(self.id, |p| p.view.pre_key(event)) as c_int
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_schemes_are_standard_cors_and_fetch() {
        assert_eq!(super::SCHEME_OPTIONS, 1 | 16 | 64);
    }
}
