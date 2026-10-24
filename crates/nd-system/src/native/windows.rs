//! Windows: `IFileDialog` and task dialogs (`rfd`), arboard, Credential Manager, toasts
//! (`notify-rust`), `SHAddToRecentDocs`, `ShellExecuteW` and `SHOpenFolderAndSelectItems`, and
//! a shell thread owning one hidden window for the System Media Transport Controls and the
//! notification area icons, pumping its own messages. There is no audio player yet.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::{ISystemMediaTransportControlsInterop, RO_INIT_SINGLETHREADED, RoInitialize};
use windows::Win32::UI::Shell::{
    GetCurrentProcessExplicitAppUserModelID, ILCreateFromPathW, ILFree, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, OAIF_ALLOW_REGISTRATION, OAIF_EXEC,
    OAIF_REGISTER_EXT, OPENASINFO, SHARD_PATHW, SHAddToRecentDocs, SHOpenFolderAndSelectItems, SHOpenWithDialog,
    SetCurrentProcessExplicitAppUserModelID, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

use super::desktop::{self, ClipboardWorker};
use crate::backend::{Backend, Emitter, Reply};
use crate::{
    AppIdentity, ClipboardImage, Error, MediaCommand, MessageOptions, Notification, NotificationId, NowPlaying,
    OpenOptions, PlaybackState, Result, SaveOptions, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem,
};

/// Posted when jobs are waiting for the shell thread.
const WM_WAKE: u32 = WM_APP;
/// Tray icon mouse messages; the icon's id is in `wparam`.
const WM_TRAY: u32 = WM_APP + 1;

type Job = Box<dyn FnOnce(&mut Shell) + Send>;

pub struct WindowsBackend {
    app: AppIdentity,
    clipboard: ClipboardWorker,
    events: OnceLock<Emitter>,
    shell: OnceLock<std::result::Result<ShellThread, String>>,
}

impl WindowsBackend {
    pub fn new(app: &AppIdentity) -> Self {
        // Toasts, the media flyout and taskbar grouping match the process to its shortcut by
        // AUMID. An installed app already has Velopack's (`velopack.<id>`, the one its shortcuts
        // carry) from `nd_update::startup`, so only an uninstalled run sets the app id.
        let mut app = app.clone();
        match current_aumid() {
            Some(aumid) => app.id = aumid,
            // SAFETY: a valid string for the duration of the call.
            None => unsafe {
                let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(app.id.as_str()));
            },
        }
        Self { app, clipboard: ClipboardWorker::new(), events: OnceLock::new(), shell: OnceLock::new() }
    }

    fn emitter(&self) -> Emitter {
        self.events.get().cloned().unwrap_or_else(Emitter::detached)
    }

    /// Runs `job` on the shell thread, starting it on first use.
    fn on_shell<T: Send + 'static>(&self, reply: Reply<T>, job: impl FnOnce(&mut Shell) -> Result<T> + Send + 'static) {
        let shell = self.shell.get_or_init(|| ShellThread::start(self.emitter()));
        match shell {
            Ok(shell) => shell.send(Box::new(move |s| reply.send(job(s)))),
            Err(e) => reply.err(Error::Platform(e.clone())),
        }
    }
}

fn wide(s: &str) -> HSTRING {
    HSTRING::from(s)
}

/// The process's explicit AUMID, if something already set one.
fn current_aumid() -> Option<String> {
    // SAFETY: the returned string is the caller's to free with `CoTaskMemFree`.
    unsafe {
        let id = GetCurrentProcessExplicitAppUserModelID().ok()?;
        let aumid = id.to_string().ok();
        CoTaskMemFree(Some(id.0.cast()));
        aumid.filter(|s| !s.is_empty())
    }
}

impl Backend for WindowsBackend {
    fn attach(&self, events: Emitter) {
        let _ = self.events.set(events);
    }

    fn open_dialog(&self, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        desktop::dialogs::open(options, reply)
    }

    fn save_dialog(&self, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        desktop::dialogs::save(options, reply)
    }

    fn message(&self, options: MessageOptions, reply: Reply<usize>) {
        desktop::dialogs::message(options, reply)
    }

    fn clipboard_read_text(&self, reply: Reply<Option<String>>) {
        self.clipboard.read_text(reply)
    }

    fn clipboard_write_text(&self, text: String, reply: Reply<()>) {
        self.clipboard.write_text(text, reply)
    }

    fn clipboard_read_image(&self, reply: Reply<Option<ClipboardImage>>) {
        self.clipboard.read_image(reply)
    }

    fn clipboard_write_image(&self, image: ClipboardImage, reply: Reply<()>) {
        self.clipboard.write_image(image, reply)
    }

    fn clipboard_read_files(&self, reply: Reply<Option<Vec<PathBuf>>>) {
        self.clipboard.read_files(reply)
    }

    fn clipboard_write_files(&self, files: Vec<PathBuf>, reply: Reply<()>) {
        self.clipboard.write_files(files, reply)
    }

    fn notify(&self, id: NotificationId, notification: Notification, reply: Reply<()>) {
        super::notify_rust::show(&self.app, self.emitter(), id, notification, reply)
    }

    fn recent_add(&self, path: PathBuf, reply: Reply<()>) {
        let path = wide(&path.to_string_lossy());
        // SAFETY: SHARD_PATHW takes a NUL-terminated wide path, alive for the call.
        unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(path.as_ptr().cast())) };
        reply.ok(())
    }

    fn recent_clear(&self, reply: Reply<()>) {
        // SAFETY: a null item clears the list.
        unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, None) };
        reply.ok(())
    }

    fn credential_set(&self, service: String, account: String, secret: String, reply: Reply<()>) {
        desktop::credential_set(service, account, secret, reply)
    }

    fn credential_get(&self, service: String, account: String, reply: Reply<Option<String>>) {
        desktop::credential_get(service, account, reply)
    }

    fn credential_delete(&self, service: String, account: String, reply: Reply<()>) {
        desktop::credential_delete(service, account, reply)
    }

    fn open_url(&self, url: &str) -> Result<()> {
        shell_open(&wide(url))
    }

    fn open_path(&self, path: PathBuf) -> Result<()> {
        let target = wide(&path.to_string_lossy());
        match shell_open(&target) {
            // No default app, or one whose program is gone: Explorer asks which app to use.
            Err(_) if path.is_file() => {
                std::thread::spawn(move || {
                    let info = OPENASINFO {
                        pcszFile: PCWSTR(target.as_ptr()),
                        pcszClass: PCWSTR::null(),
                        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
                    };
                    // SAFETY: COM is initialised on this thread for the call; `target` outlives it.
                    unsafe {
                        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                        let _ = SHOpenWithDialog(None, &info);
                    }
                });
                Ok(())
            }
            result => result,
        }
    }

    fn reveal_path(&self, path: PathBuf) -> Result<()> {
        let path = wide(&path.to_string_lossy());
        std::thread::spawn(move || {
            // SAFETY: COM is initialised on this thread for the call; the item list is freed.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let item = ILCreateFromPathW(PCWSTR(path.as_ptr()));
                if !item.is_null() {
                    let _ = SHOpenFolderAndSelectItems(item, None, 0);
                    ILFree(Some(item));
                }
            }
        });
        Ok(())
    }

    fn tray_set(&self, tray: Tray, reply: Reply<()>) {
        self.on_shell(reply, move |shell| shell.set_tray(tray))
    }

    fn tray_remove(&self, id: TrayId, reply: Reply<()>) {
        self.on_shell(reply, move |shell| shell.remove_tray(&id))
    }

    fn media_set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        self.on_shell(reply, move |shell| shell.set_media(now.as_ref()).map_err(Error::platform))
    }
}

fn shell_open(target: &HSTRING) -> Result<()> {
    // SAFETY: NUL-terminated wide strings alive for the call.
    let result = unsafe { ShellExecuteW(None, w!("open"), target, None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success as a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(Error::Platform(format!("ShellExecuteW failed ({})", result.0 as isize)))
    }
}

struct ShellThread {
    hwnd: isize,
    jobs: Mutex<mpsc::Sender<Job>>,
}

impl ShellThread {
    fn start(events: Emitter) -> std::result::Result<Self, String> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready, window) = mpsc::channel();
        std::thread::Builder::new()
            .name("nd-shell".into())
            .spawn(move || match Shell::create(events, inbox) {
                Ok(hwnd) => {
                    let _ = ready.send(Ok(hwnd.0 as isize));
                    pump();
                }
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                }
            })
            .map_err(|e| e.to_string())?;
        let hwnd = window.recv().map_err(|e| e.to_string())??;
        Ok(Self { hwnd, jobs: Mutex::new(jobs) })
    }

    fn send(&self, job: Job) {
        let _ = self.jobs.lock().expect("shell jobs").send(job);
        // SAFETY: posting to a window that may already be gone only fails.
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

struct TrayEntry {
    uid: u32,
    tray: Tray,
}

struct Shell {
    hwnd: HWND,
    events: Emitter,
    inbox: mpsc::Receiver<Job>,
    smtc: Option<SystemMediaTransportControls>,
    art: Option<String>,
    trays: BTreeMap<String, TrayEntry>,
    next_uid: u32,
    taskbar_created: u32,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

impl Shell {
    fn create(events: Emitter, inbox: mpsc::Receiver<Job>) -> windows::core::Result<HWND> {
        // SAFETY: plain Win32 calls on this thread; the class and window live until it ends.
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("NdSystemShell"),
                ..Default::default()
            };
            RegisterClassW(&class);
            // A real top-level window, never shown: the media controls need one, and a
            // message-only window gets no TaskbarCreated broadcast.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("NdSystemShell"),
                w!("nd-system"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
            SHELL.with_borrow_mut(|s| {
                *s = Some(Shell {
                    hwnd,
                    events,
                    inbox,
                    smtc: None,
                    art: None,
                    trays: BTreeMap::new(),
                    next_uid: 1,
                    taskbar_created,
                })
            });
            Ok(hwnd)
        }
    }

    fn media_controls(&mut self) -> windows::core::Result<&SystemMediaTransportControls> {
        if self.smtc.is_none() {
            let interop =
                windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
            // SAFETY: `hwnd` is this thread's live window.
            let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(self.hwnd)? };
            smtc.SetIsEnabled(true)?;
            smtc.SetIsPlayEnabled(true)?;
            smtc.SetIsPauseEnabled(true)?;
            smtc.SetIsStopEnabled(true)?;
            let events = self.events.clone();
            smtc.ButtonPressed(&TypedEventHandler::new(move |_, args| {
                let args: &windows::Media::SystemMediaTransportControlsButtonPressedEventArgs =
                    windows::core::Ref::ok(&args)?;
                let command = match args.Button()? {
                    SystemMediaTransportControlsButton::Play => MediaCommand::Play,
                    SystemMediaTransportControlsButton::Pause => MediaCommand::Pause,
                    SystemMediaTransportControlsButton::Stop => MediaCommand::Stop,
                    SystemMediaTransportControlsButton::Next => MediaCommand::Next,
                    SystemMediaTransportControlsButton::Previous => MediaCommand::Previous,
                    _ => return Ok(()),
                };
                events.emit(SystemEvent::Media(command));
                Ok(())
            }))?;
            let events = self.events.clone();
            smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(move |_, args| {
                let args: &windows::Media::PlaybackPositionChangeRequestedEventArgs = windows::core::Ref::ok(&args)?;
                let ticks = args.RequestedPlaybackPosition()?.Duration.max(0) as u64;
                events
                    .emit(SystemEvent::Media(MediaCommand::SetPosition(std::time::Duration::from_micros(ticks / 10))));
                Ok(())
            }))?;
            self.smtc = Some(smtc);
        }
        Ok(self.smtc.as_ref().expect("set above"))
    }

    fn set_media(&mut self, now: Option<&NowPlaying>) -> windows::core::Result<()> {
        let Some(now) = now else {
            if let Some(smtc) = &self.smtc {
                smtc.DisplayUpdater()?.ClearAll()?;
                smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed)?;
                smtc.SetIsEnabled(false)?;
            }
            self.art = None;
            return Ok(());
        };
        let smtc = self.media_controls()?.clone();
        smtc.SetIsEnabled(true)?;
        smtc.SetPlaybackStatus(match now.state {
            PlaybackState::Playing => MediaPlaybackStatus::Playing,
            PlaybackState::Paused => MediaPlaybackStatus::Paused,
            PlaybackState::Stopped => MediaPlaybackStatus::Stopped,
        })?;
        smtc.SetIsNextEnabled(now.can_next)?;
        smtc.SetIsPreviousEnabled(now.can_previous)?;
        let updater = smtc.DisplayUpdater()?;
        updater.SetType(MediaPlaybackType::Music)?;
        let music = updater.MusicProperties()?;
        music.SetTitle(&wide(&now.title))?;
        music.SetArtist(&wide(now.artist.as_deref().unwrap_or("")))?;
        music.SetAlbumTitle(&wide(now.album.as_deref().unwrap_or("")))?;
        // Setting the same thumbnail again refetches it and flickers the flyout.
        if now.artwork_url != self.art {
            match &now.artwork_url {
                Some(url) => {
                    updater.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&wide(url))?)?)?
                }
                None => updater.SetThumbnail(None)?,
            }
            self.art = now.artwork_url.clone();
        }
        updater.Update()?;
        let ticks = |d: std::time::Duration| TimeSpan { Duration: (d.as_micros() * 10) as i64 };
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        let duration = now.duration.unwrap_or_default();
        timeline.SetStartTime(ticks(Default::default()))?;
        timeline.SetMinSeekTime(ticks(Default::default()))?;
        timeline.SetEndTime(ticks(duration))?;
        timeline.SetMaxSeekTime(ticks(if now.can_seek { duration } else { Default::default() }))?;
        timeline.SetPosition(ticks(now.position.unwrap_or_default().min(duration)))?;
        smtc.UpdateTimelineProperties(&timeline)
    }

    fn set_tray(&mut self, tray: Tray) -> Result<()> {
        let key = tray.id.0.clone();
        let (verb, uid) = match self.trays.get(&key) {
            Some(entry) => (NIM_MODIFY, entry.uid),
            None => {
                self.next_uid += 1;
                (NIM_ADD, self.next_uid)
            }
        };
        notify_icon(self.hwnd, verb, uid, &tray)?;
        self.trays.insert(key, TrayEntry { uid, tray });
        Ok(())
    }

    fn remove_tray(&mut self, id: &TrayId) -> Result<()> {
        let entry = self.trays.remove(&id.0).ok_or(Error::NotFound)?;
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: entry.uid,
            ..Default::default()
        };
        // SAFETY: `data` names this window's icon.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        Ok(())
    }
}

fn icon(tray: &Tray) -> HICON {
    let size = |m| unsafe { GetSystemMetrics(m) };
    // SAFETY: CreateIconFromResourceEx copies the PNG it is given; LoadImageW reads the
    // executable's first icon resource.
    unsafe {
        if let Some(TrayIcon::Png { bytes, .. }) = &tray.icon
            && let Ok(icon) = CreateIconFromResourceEx(
                bytes,
                true,
                0x0003_0000,
                size(SM_CXSMICON),
                size(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
        {
            return icon;
        }
        GetModuleHandleW(None)
            .ok()
            .and_then(|module| {
                LoadImageW(
                    Some(module.into()),
                    PCWSTR(1 as _),
                    IMAGE_ICON,
                    size(SM_CXSMICON),
                    size(SM_CYSMICON),
                    LR_DEFAULTCOLOR,
                )
                .ok()
            })
            .map(|handle| HICON(handle.0))
            .unwrap_or_default()
    }
}

fn notify_icon(hwnd: HWND, verb: NOTIFY_ICON_MESSAGE, uid: u32, tray: &Tray) -> Result<()> {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: uid,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon(tray),
        ..Default::default()
    };
    let tip = tray.tooltip.as_deref().or(tray.title.as_deref()).unwrap_or("");
    let tip: Vec<u16> = tip.encode_utf16().take(data.szTip.len() - 1).collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    // SAFETY: `data` is fully initialised for this window.
    let ok = unsafe { Shell_NotifyIconW(verb, &data) };
    if ok.as_bool() { Ok(()) } else { Err(Error::Platform("Shell_NotifyIconW failed".into())) }
}

fn pump() {
    let mut message = MSG::default();
    // SAFETY: the standard message loop.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Builds `items` into `menu`; each chosen item maps back to its id through `ids`.
fn append_items(menu: HMENU, items: &[TrayMenuItem], ids: &mut Vec<String>) {
    let text = |s: &str| HSTRING::from(s.replace('&', "&&"));
    for item in items {
        // SAFETY: appending to a menu owned by the caller.
        unsafe {
            match item {
                TrayMenuItem::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                }
                TrayMenuItem::Item { id, label, enabled, checked } => {
                    ids.push(id.clone());
                    let mut flags = MF_STRING;
                    if !enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked == Some(true) {
                        flags |= MF_CHECKED;
                    }
                    let _ = AppendMenuW(menu, flags, ids.len(), &text(label));
                }
                TrayMenuItem::Submenu { label, items } => {
                    if let Ok(sub) = CreatePopupMenu() {
                        append_items(sub, items, ids);
                        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &text(label));
                    }
                }
            }
        }
    }
}

fn show_menu(hwnd: HWND, items: &[TrayMenuItem]) -> Option<String> {
    let mut ids = Vec::new();
    // SAFETY: a popup menu owned and destroyed here.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        append_items(menu, items, &mut ids);
        let mut cursor = Default::default();
        let _ = GetCursorPos(&mut cursor);
        // Without the foreground the menu stays open after a click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let index = chosen.0 as usize;
        (index > 0).then(|| ids.get(index - 1).cloned()).flatten()
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = SHELL.with(|cell| {
        // A message sent while a menu is up or during creation finds the state borrowed or
        // empty and takes the default path.
        let Ok(mut borrowed) = cell.try_borrow_mut() else { return None };
        let shell = borrowed.as_mut()?;
        match message {
            WM_WAKE => {
                while let Ok(job) = shell.inbox.try_recv() {
                    job(shell);
                }
                Some(LRESULT(0))
            }
            WM_TRAY => {
                let uid = wparam.0 as u32;
                let entry = shell.trays.values().find(|e| e.uid == uid)?;
                let (id, menu, events) = (entry.tray.id.clone(), entry.tray.menu.clone(), shell.events.clone());
                match (lparam.0 & 0xffff) as u32 {
                    WM_LBUTTONUP => events.emit(SystemEvent::Tray { tray: id, item: None }),
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        drop(borrowed);
                        if let Some(item) = show_menu(hwnd, &menu) {
                            events.emit(SystemEvent::Tray { tray: id, item: Some(item) });
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            m if m == shell.taskbar_created => {
                // Explorer restarted and forgot the icons.
                for entry in shell.trays.values() {
                    let _ = notify_icon(shell.hwnd, NIM_ADD, entry.uid, &entry.tray);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    });
    // SAFETY: forwarding the message this procedure was called with.
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
}
