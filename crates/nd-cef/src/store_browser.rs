//! A windowless browser of nd-cef's own for a Web Store install by id. `completeInstall`
//! downloads the extension from the page that called it, and CEF cancels a download whose page is
//! not a CEF browser (`CefDownloadManagerDelegateImpl::OnDownloadCreated`), which a hidden
//! DevTools page is not. Nothing of it is drawn; the install prompt it raises is a window of its
//! own.

use std::os::raw::c_int;

use cef::*;

/// Where an install's package lands before Chromium unpacks it into the profile.
fn download_dir() -> std::path::PathBuf {
    crate::runtime::root_cache().join("nd-store-downloads")
}

/// A browser at `url`, in the default profile.
pub(crate) fn create(url: &str) -> Result<Browser, String> {
    let info =
        WindowInfo { windowless_rendering_enabled: 1, runtime_style: RuntimeStyle::ALLOY, ..WindowInfo::default() };
    let settings = BrowserSettings { windowless_frame_rate: 1, ..BrowserSettings::default() };
    let mut client = StoreClient::new();
    browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&settings),
        None,
        None,
    )
    .ok_or_else(|| "Chromium could not create the store page".into())
}

pub(crate) fn close(browser: &Browser) {
    if let Some(host) = browser.host() {
        host.close_browser(1);
    }
}

/// The Web Store's package downloads, as `webstorePrivate.completeInstall` starts them.
pub(crate) fn is_store_package(original_url: &str) -> bool {
    original_url.starts_with("https://clients2.google.com/service/update2/crx")
}

wrap_client! {
    struct StoreClient;

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(StoreRender::new())
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(StoreDownload::new())
        }
    }
}

wrap_render_handler! {
    struct StoreRender;

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(out) = rect {
                *out = Rect { x: 0, y: 0, width: 800, height: 600 };
            }
        }
    }
}

wrap_download_handler! {
    struct StoreDownload;

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, url: Option<&CefString>, _request_method: Option<&CefString>) -> c_int {
            c_int::from(url.is_some_and(|u| is_store_package(&u.to_string())))
        }

        /// The package goes to the profile, never the user's downloads; anything else is
        /// cancelled (unhandled in a windowless browser).
        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> c_int {
            let (Some(item), Some(callback)) = (download_item, callback) else { return 0 };
            if !is_store_package(&crate::text(&item.original_url())) {
                return 0;
            }
            // Chromium interrupts a download to a path with `..` in it, and a profile path can
            // have one.
            let dir = download_dir();
            let dir = match std::fs::create_dir_all(&dir).and_then(|()| std::fs::canonicalize(&dir)) {
                Ok(dir) => dir,
                Err(e) => {
                    tracing::warn!("nd-cef: {}: {e}", dir.display());
                    return 0;
                }
            };
            let name = suggested_name.map(|s| s.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "extension.crx".into());
            callback.cont(Some(&CefString::from(dir.join(name).to_string_lossy().as_ref())), 0);
            1
        }
    }
}
