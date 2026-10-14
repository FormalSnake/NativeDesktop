//! The appearance audit (tier 2): renders a window in light, dark and two matugen themes on
//! every backend kind this target links (AppKit and GPUI on macOS, GPUI elsewhere), reads back
//! every text run and fill the backend painted, and fails when a colour is not one of the
//! theme's tokens (or a state derived from one) or when text is under WCAG contrast against
//! what it sits on: 4.5:1, or 3:1 for large text (24 px, or 18.66 px bold).
//!
//! An app's `tests/appearance.rs`, with `harness = false`:
//!
//! ```ignore
//! fn main() {
//!     nd::audit::Audit::new("hello", hello::app).window("main", |cx| hello::open(cx).id()).run()
//! }
//! ```
//!
//! Each backend runs in its own child process, so AppKit and headless GPUI never share one.
//! The matugen themes run on GPUI only: on AppKit native text draws with system colours by
//! design, and matugen is a Linux palette source.

use std::process::{Command, ExitCode};
use std::rc::Rc;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

use nd_components::SubtreeScope;
use nd_theme::color::{close, contrast, over, to_hex};
use nd_theme::{Appearance, MatugenTheme, OsTheme, Scheme, Theme, ThemeRefinement, ThemeRegistry};
use nd_ui::semantics::{Paint, PaintKind};
use nd_ui::{App, WindowId};

use crate::Application;
use crate::theme::{ThemeState, update_registry};

const TEST_NAME: &str = "appearance_audit";
const BACKEND_ENV: &str = "ND_AUDIT_BACKEND";

/// Material You's baseline roles, as matugen writes them for a desktop in each mode.
const MATUGEN_DARK: &str = r##"{"mode":"dark","colors":{
    "surface":"#141218","surface_container_low":"#1d1b20","surface_container":"#211f26",
    "surface_container_high":"#2b2930","outline_variant":"#49454f","on_surface":"#e6e0e9",
    "on_surface_variant":"#cac4d0","outline":"#938f99","primary":"#d0bcff","on_primary":"#381e72",
    "primary_container":"#4f378b","on_primary_container":"#eaddff","error":"#f2b8b5",
    "secondary_container":"#4a4458"}}"##;
const MATUGEN_LIGHT: &str = r##"{"mode":"light","colors":{
    "surface":"#fef7ff","surface_container_low":"#f7f2fa","surface_container":"#f3edf7",
    "surface_container_high":"#ece6f0","outline_variant":"#cac4d0","on_surface":"#1d1b20",
    "on_surface_variant":"#49454f","outline":"#79747e","primary":"#6750a4","on_primary":"#ffffff",
    "primary_container":"#eaddff","on_primary_container":"#21005d","error":"#b3261e",
    "secondary_container":"#e8def8"}}"##;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    AppKit,
    Gpui,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::AppKit => "appkit",
            Kind::Gpui => "gpui",
        }
    }

    fn linked() -> Vec<Kind> {
        if cfg!(target_os = "macos") { vec![Kind::AppKit, Kind::Gpui] } else { vec![Kind::Gpui] }
    }

    fn variants(self) -> Vec<Variant> {
        let mut out = vec![Variant::Os(Scheme::Light), Variant::Os(Scheme::Dark)];
        if self == Kind::Gpui {
            out.extend([Variant::Matugen(Scheme::Light), Variant::Matugen(Scheme::Dark)]);
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
enum Variant {
    /// The desktop in this scheme.
    Os(Scheme),
    /// matugen's `theme.json` in this mode, on a desktop in the same scheme.
    Matugen(Scheme),
}

impl Variant {
    fn name(self) -> String {
        match self {
            Variant::Os(s) => scheme_name(s).into(),
            Variant::Matugen(s) => format!("matugen-{}", scheme_name(s)),
        }
    }

    fn scheme(self) -> Scheme {
        match self {
            Variant::Os(s) | Variant::Matugen(s) => s,
        }
    }

    fn matugen(self) -> Option<MatugenTheme> {
        let Variant::Matugen(scheme) = self else { return None };
        let json = if scheme.is_dark() { MATUGEN_DARK } else { MATUGEN_LIGHT };
        Some(MatugenTheme::parse(json).expect("the audit's matugen themes parse"))
    }
}

fn scheme_name(s: Scheme) -> &'static str {
    if s.is_dark() { "dark" } else { "light" }
}

type Opener = Rc<dyn Fn(&mut App) -> WindowId>;

/// One window's appearance audit. See the module docs.
pub struct Audit {
    name: String,
    app: fn() -> Application,
    windows: Vec<(String, Opener)>,
    allowed: Vec<nd_ui::prelude::Hsla>,
    must_fail: bool,
}

impl Audit {
    /// `app` builds the app as `main` does (its theme override and patterns).
    pub fn new(name: &str, app: fn() -> Application) -> Self {
        Self { name: name.into(), app, windows: Vec::new(), allowed: Vec::new(), must_fail: false }
    }

    /// A window to audit: `open` opens it and returns its id. Each opens once per variant.
    pub fn window(mut self, label: &str, open: impl Fn(&mut App) -> WindowId + 'static) -> Self {
        self.windows.push((label.into(), Rc::new(open)));
        self
    }

    /// Inverts the verdict on every backend, for the audit's own tests: a view built to break
    /// the rules must be caught.
    pub fn must_fail(mut self) -> Self {
        self.must_fail = true;
        self
    }

    /// Colours the app names in its theme file on purpose (a brand fill, a glass tint), which
    /// pass the token check. Contrast is still checked.
    pub fn allow(mut self, colors: impl IntoIterator<Item = nd_ui::prelude::Hsla>) -> Self {
        self.allowed.extend(colors);
        self
    }

    /// Runs the audit as a `harness = false` test binary under nextest or `cargo test`, and
    /// exits.
    pub fn run(self) -> ! {
        std::process::exit(match self.main() {
            ExitCode::SUCCESS => 0,
            _ => 1,
        })
    }

    fn main(self) -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--list") {
            if !args.iter().any(|a| a == "--ignored") {
                println!("{TEST_NAME}: test");
            }
            return ExitCode::SUCCESS;
        }
        let filters: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
        if args.iter().any(|a| a == "--ignored")
            || !(filters.is_empty() || filters.iter().any(|f| TEST_NAME.contains(f.as_str())))
        {
            return ExitCode::SUCCESS;
        }
        match std::env::var(BACKEND_ENV).as_deref() {
            Ok("appkit") => self.in_process(Kind::AppKit),
            Ok("gpui") => self.in_process(Kind::Gpui),
            _ => self.each_backend(),
        }
    }

    fn each_backend(&self) -> ExitCode {
        let exe = std::env::current_exe().expect("the test binary's path");
        let mut ok = true;
        for kind in Kind::linked() {
            let status = Command::new(&exe).arg(TEST_NAME).env(BACKEND_ENV, kind.name()).status();
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    println!("FAIL {} on {}: {s}", self.name, kind.name());
                    ok = false;
                }
                Err(e) => {
                    println!("FAIL {} on {}: could not start: {e}", self.name, kind.name());
                    ok = false;
                }
            }
        }
        if ok {
            println!("ND_APPEARANCE_AUDIT_OK {}", self.name);
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }

    fn in_process(&self, kind: Kind) -> ExitCode {
        // The AppKit variant puts real windows on screen; the GPUI one is headless.
        #[cfg(target_os = "macos")]
        let _window_lock =
            (kind == Kind::AppKit).then(|| ndshell::lock::window_test(&format!("appearance audit {}", self.name)));
        // Isolated from the user's theme files and data, so only the app and the variant apply.
        let data = std::env::temp_dir().join(format!("nd-appearance-audit-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &data);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let findings = match kind {
            Kind::AppKit => self.appkit(),
            Kind::Gpui => self.gpui(),
        };
        let _ = std::fs::remove_dir_all(&data);
        let failed = findings.iter().filter(|f| f.starts_with("FAIL")).count();
        for line in &findings {
            println!("{line}");
        }
        match (failed == 0, self.must_fail) {
            (true, false) | (false, true) => ExitCode::SUCCESS,
            (true, true) => {
                println!("FAIL {} on {}: expected findings, found none", self.name, kind.name());
                ExitCode::FAILURE
            }
            (false, false) => ExitCode::FAILURE,
        }
    }

    /// Audits one variant on an app whose registry and backend already carry it.
    fn check(&self, cx: &App, window: WindowId, label: &str) -> Vec<String> {
        let mut out = Vec::new();
        let paints = match cx.backend().paints(window) {
            Ok(p) => p,
            Err(e) => return vec![format!("FAIL {label}: the backend reported no paints: {e}")],
        };
        // Text nodes with no text paint mean the backend's read-back missed them. Labels a
        // native intent draws itself (a SwiftUI form) have no node and are not counted.
        let has_text_nodes = cx.semantic_window(window).is_some_and(|w| has_text(&w.root));
        if has_text_nodes && !paints.iter().any(|p| matches!(p.kind, PaintKind::Text { .. })) {
            out.push(format!("FAIL {label}: the window has text nodes but no text was painted"));
        }
        let registry = &cx.global::<ThemeState>().0;
        let (mut texts, mut fills) = (0, 0);
        for paint in &paints {
            let theme = theme_for(registry, window, paint);
            if !self.is_token(&theme, paint) {
                out.push(format!(
                    "FAIL {label}: {} \"{}\" is {}, not a {} theme token",
                    kind_name(paint),
                    paint.label,
                    to_hex(paint.color),
                    scheme_name(theme.scheme)
                ));
            }
            match paint.kind {
                PaintKind::Text { size, weight } => {
                    texts += 1;
                    let ratio = contrast(over(paint.color, paint.background), paint.background);
                    let large = size >= 24. || (size >= 18.66 && weight >= 700.);
                    let floor = if large { 3. } else { 4.5 };
                    if ratio < floor {
                        out.push(format!(
                            "FAIL {label}: text \"{}\" {} on {} is {ratio:.2}:1, under {floor}:1",
                            paint.label,
                            to_hex(paint.color),
                            to_hex(paint.background)
                        ));
                    }
                }
                PaintKind::Fill => fills += 1,
            }
        }
        if out.is_empty() {
            out.push(format!("ok   {label}: {texts} text runs, {fills} fills"));
        }
        out
    }

    fn is_token(&self, theme: &Theme, paint: &Paint) -> bool {
        let color = paint.color;
        tokens(theme).into_iter().chain(self.allowed.iter().copied()).any(|t| close(t, color))
            // Text faded with its element (opacity) keeps its token's colour.
            || (matches!(paint.kind, PaintKind::Text { .. })
                && tokens(theme).into_iter().any(|t| close(nd_theme::color::with_alpha(t, color.a), color)))
    }

    fn label(&self, window: &str, kind: Kind, variant: Variant) -> String {
        format!("{}/{window} {}/{}", self.name, kind.name(), variant.name())
    }

    /// Applies `variant` to the registry, as the platform or the matugen file would.
    fn apply(cx: &mut App, kind: Kind, variant: Variant, app_override: &ThemeRefinement) {
        update_registry(cx, |r| {
            r.set_matugen(variant.matugen());
            match kind {
                // AppKit reads the real desktop; the variant forces the app's appearance.
                Kind::AppKit => {
                    let appearance = if variant.scheme().is_dark() { Appearance::Dark } else { Appearance::Light };
                    r.set_app_override(app_override.clone().appearance(appearance));
                }
                Kind::Gpui => r.set_os(OsTheme { scheme: Some(variant.scheme()), ..r.os().clone() }),
            }
        });
    }

    #[cfg(target_os = "macos")]
    fn appkit(&self) -> Vec<String> {
        let host = (self.app)().host(|_cx| {});
        let app_override = host.update(|cx| cx.global::<ThemeState>().0.app_override().clone());
        let mut out = Vec::new();
        for variant in Kind::AppKit.variants() {
            for (name, open) in &self.windows {
                let window = host.update(|cx| {
                    Self::apply(cx, Kind::AppKit, variant, &app_override);
                    open(cx)
                });
                settle_appkit(&host);
                out.extend(host.update(|cx| self.check(cx, window, &self.label(name, Kind::AppKit, variant))));
                host.update(|cx| cx.close_window(window));
                settle_appkit(&host);
            }
        }
        out
    }

    #[cfg(not(target_os = "macos"))]
    fn appkit(&self) -> Vec<String> {
        Vec::new()
    }

    fn gpui(&self) -> Vec<String> {
        use std::sync::Arc;

        use nd_backend_gpui::GpuiHost;
        use nd_gpui::gpui::HeadlessAppContext;

        let application = (self.app)();
        let files = crate::theme::ThemeFiles::for_app(application.manifest().name);
        let app_override = application.theme.clone().unwrap_or_default();
        let patterns = application.patterns.clone();
        let text = nd_gpui::gpui_platform::current_platform(true).text_system();
        let mut cx =
            HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
                crate::runtime::gpui_host::install(app, &files, app_override.clone(), patterns);
                crate::theme::sync(app);
            })
        });
        let mut out = Vec::new();
        for variant in Kind::Gpui.variants() {
            for (name, open) in &self.windows {
                let window = cx.update(|gcx| {
                    host.update(gcx, |app| {
                        Self::apply(app, Kind::Gpui, variant, &app_override);
                        open(app)
                    })
                });
                settle_gpui(&mut cx, &host);
                let found = host.read(|app| self.check(app, window, &self.label(name, Kind::Gpui, variant)));
                out.extend(found);
                cx.update(|gcx| host.update(gcx, |app| app.close_window(window)));
                settle_gpui(&mut cx, &host);
            }
        }
        out
    }
}

#[cfg(target_os = "macos")]
fn settle_appkit(host: &nd_backend_appkit::AppHost) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        host.platform().pump(Duration::from_millis(30), Duration::from_millis(300));
        let busy = host.update(|app| {
            app.frame();
            app.needs_frame()
        });
        if !busy || Instant::now() > deadline {
            break;
        }
    }
    host.platform().pump(Duration::from_millis(50), Duration::from_millis(200));
}

fn settle_gpui(cx: &mut nd_gpui::gpui::HeadlessAppContext, host: &nd_backend_gpui::GpuiHost) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|gcx| {
            host.pump(gcx);
            host.draw(gcx);
        });
    }
    cx.run_until_parked();
}

fn has_text(node: &nd_ui::SemanticNode) -> bool {
    node.role == nd_ui::Role::Text || node.children.iter().any(has_text)
}

fn kind_name(paint: &Paint) -> &'static str {
    match paint.kind {
        PaintKind::Text { .. } => "text",
        PaintKind::Fill => "fill",
    }
}

/// The theme a paint should come from: the window's, under every subtree scope around it.
fn theme_for(registry: &ThemeRegistry, window: WindowId, paint: &Paint) -> Theme {
    let mut composed: Option<ThemeRefinement> = None;
    for scope in &paint.scopes {
        if let Some(refinement) = scope.get::<SubtreeScope>().and_then(|s| s.theme.as_ref()) {
            composed = Some(match composed {
                Some(outer) => refinement.over(&outer),
                None => refinement.clone(),
            });
        }
    }
    match composed {
        Some(r) => registry.resolve_subtree(Some(window), &r),
        None => registry.window_theme(window).clone(),
    }
}

/// Every colour token of `theme`, with the hover, pressed and disabled states derived from
/// each.
fn tokens(theme: &Theme) -> Vec<nd_ui::prelude::Hsla> {
    let t = theme;
    let base = [
        t.surface.base,
        t.surface.sidebar,
        t.surface.raised,
        t.surface.raised_hover,
        t.surface.overlay,
        t.border.subtle,
        t.border.strong,
        t.separator,
        t.text.primary,
        t.text.secondary,
        t.text.tertiary,
        t.text.on_accent,
        t.text.danger,
        t.accent,
        t.accent_container.bg,
        t.accent_container.fg,
        t.selection,
        t.danger,
        t.success,
        t.warning,
        t.focus_ring,
        t.scrim,
    ];
    base.iter().chain(t.terminal.ansi.iter()).flat_map(|c| [*c, t.hover(*c), t.pressed(*c), t.disabled(*c)]).collect()
}
