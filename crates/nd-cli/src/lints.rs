//! The syntactic framework lints of ARCHITECTURE.md 15.4, run over an app's `src/` with `syn`.
//!
//! A line is exempt with `// nd-allow(rule-id): reason` on it or the line above. Rules that need
//! types are tier 1 audits, not here. The `web-*` rules run only under `nd check --target web`.
//! Not implemented yet: px margins beside `window_controls` (needs the M2 `titlebar()` insets).

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::visit::Visit;

pub const RULES: &[(&str, &str)] = &[
    ("framework-internals", "gpui and objc2 are named only in native widget modules or cfg(target_os) items"),
    ("cfg-target-os", "cfg(target_os) outside a native widget module needs a reason comment"),
    ("colour-literal", "colour constructors belong in theme files; read tokens from `cx.theme()`"),
    ("font-family-literal", "font families come from the OS through `cx.theme().fonts`; never name one"),
    ("clock-in-ui", "Instant::now, thread::sleep and smol::Timer are banned in UI code; use executor timers"),
    ("inline-svg", "no inline SVG; use the icon set"),
    ("scroll-list", "a scrolling container fed by `.children(..)` is a hand-rolled list; use `list()`"),
    ("floating-element", "floating elements go through `cx.overlay` or a built-in, not deferred/anchored"),
    ("drawn-reason", "`.drawn()` needs a reason comment"),
];

/// Rules added by `--target web`: std APIs that compile on `wasm32-unknown-unknown` and panic
/// when called, and framework APIs marked `#[unavailable(web)]`.
pub const WEB_RULES: &[(&str, &str)] = &[
    ("web-std-time", "std::time::Instant and SystemTime panic in a browser; use `nd::time`"),
    ("web-std-thread", "std::thread cannot spawn in a browser; use the executors"),
    ("web-std-fs", "std::fs has no file system in a browser; use `Store` or the file dialogs"),
    ("web-unavailable", "an API unavailable on the web needs an `nd::platform::has(Feature::..)` check"),
];

#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub rule: &'static str,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: [{}] {}", self.file.display(), self.line, self.column, self.rule, self.message)
    }
}

/// Lints every `.rs` file under `dir`, reporting paths relative to `base`.
pub fn lint_dir(base: &Path, dir: &Path) -> std::io::Result<Vec<Finding>> {
    lint_dir_for(base, dir, None)
}

/// As [`lint_dir`], adding the web rules when `web` holds the names of the framework items
/// marked unavailable on the web ([`unavailable_on_web`]).
pub fn lint_dir_for(base: &Path, dir: &Path, web: Option<&HashSet<String>>) -> std::io::Result<Vec<Finding>> {
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let mut findings = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file)?;
        let shown = file.strip_prefix(base).unwrap_or(&file);
        findings.extend(lint_source_for(shown, &source, web));
    }
    Ok(findings)
}

/// Names of the functions and methods under `dirs` carrying `#[unavailable(web)]` (any path
/// ending in `unavailable`). Unparsable files are skipped: they are not the app's to fix.
pub fn unavailable_on_web(dirs: &[PathBuf]) -> std::io::Result<HashSet<String>> {
    let mut files = Vec::new();
    for dir in dirs {
        collect(dir, &mut files)?;
    }
    let mut names = HashSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)?;
        if !source.contains("unavailable") {
            continue;
        }
        if let Ok(parsed) = syn::parse_file(&source) {
            let mut finder = UnavailableFinder(&mut names);
            finder.visit_file(&parsed);
        }
    }
    Ok(names)
}

struct UnavailableFinder<'a>(&'a mut HashSet<String>);

fn marks_unavailable_on_web(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().segments.last().is_some_and(|s| s.ident == "unavailable")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string().split(|c: char| !c.is_alphanumeric()).any(|t| t == "web"))
    })
}

impl<'ast> Visit<'ast> for UnavailableFinder<'_> {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if marks_unavailable_on_web(&item.attrs) {
            self.0.insert(item.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if marks_unavailable_on_web(&item.attrs) {
            self.0.insert(item.sig.ident.to_string());
        }
        syn::visit::visit_impl_item_fn(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if marks_unavailable_on_web(&item.attrs) {
            self.0.insert(item.sig.ident.to_string());
        }
        syn::visit::visit_trait_item_fn(self, item);
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

pub fn lint_source(path: &Path, source: &str) -> Vec<Finding> {
    lint_source_for(path, source, None)
}

pub fn lint_source_for(path: &Path, source: &str, web: Option<&HashSet<String>>) -> Vec<Finding> {
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => {
            let start = error.span().start();
            return vec![Finding {
                file: path.to_path_buf(),
                line: start.line,
                column: start.column + 1,
                rule: "parse",
                message: error.to_string(),
            }];
        }
    };
    let mut linter = Linter {
        path,
        lines: source.lines().collect(),
        native_file: is_native_file(path, source),
        theme_file: is_theme_file(path),
        native_cfg: 0,
        web,
        guarded: Vec::new(),
        seen: HashSet::new(),
        findings: Vec::new(),
    };
    linter.visit_file(&file);
    linter.findings
}

fn is_native_file(path: &Path, source: &str) -> bool {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let in_native_dir = path.components().any(|c| c.as_os_str() == "native");
    in_native_dir
        || matches!(stem, "appkit" | "swift" | "win32" | "wayland")
        || source.contains("impl NativeWidget for")
}

fn is_theme_file(path: &Path) -> bool {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    stem == "theme" || path.components().any(|c| c.as_os_str() == "theme")
}

struct Linter<'a> {
    path: &'a Path,
    lines: Vec<&'a str>,
    native_file: bool,
    theme_file: bool,
    /// Depth of enclosing items carrying `cfg(.. target_os ..)`.
    native_cfg: usize,
    /// Set by `--target web`: the names marked unavailable on the web.
    web: Option<&'a HashSet<String>>,
    /// Per enclosing fn: whether its body checks `has(Feature::..)`.
    guarded: Vec<bool>,
    seen: HashSet<(usize, usize, &'static str)>,
    findings: Vec<Finding>,
}

impl Linter<'_> {
    fn report(&mut self, span: proc_macro2::Span, rule: &'static str, message: impl Into<String>) {
        let start = span.start();
        if !self.seen.insert((start.line, start.column, rule)) || self.allowed(start.line, rule) {
            return;
        }
        self.findings.push(Finding {
            file: self.path.to_path_buf(),
            line: start.line,
            column: start.column + 1,
            rule,
            message: message.into(),
        });
    }

    fn line(&self, number: usize) -> &str {
        number.checked_sub(1).and_then(|i| self.lines.get(i)).copied().unwrap_or_default()
    }

    fn allowed(&self, line: usize, rule: &str) -> bool {
        let pragma = format!("nd-allow({rule})");
        self.line(line).contains(&pragma) || self.line(line.saturating_sub(1)).contains(&pragma)
    }

    /// A `//` comment on the line or in the six lines above it.
    fn has_reason(&self, line: usize) -> bool {
        (line.saturating_sub(6)..=line)
            .any(|n| self.line(n).trim_start().starts_with("//") || self.line(n).contains(" //"))
    }

    fn native_allowed(&self) -> bool {
        self.native_file || self.native_cfg > 0
    }

    fn check_cfg_reason(&mut self, attr: &syn::Attribute) {
        if is_target_os_cfg(attr) && !self.native_allowed() {
            let line = attr.span().start().line;
            if !self.has_reason(line.saturating_sub(1)) && !self.line(line).contains("//") {
                self.report(
                    attr.span(),
                    "cfg-target-os",
                    "cfg(target_os) outside a native widget module needs a reason comment",
                );
            }
        }
    }

    /// Checks the item's own attributes against the enclosing context, then visits it with the
    /// item's `cfg(target_os)` (if any) counting as a native region.
    fn with_attrs(&mut self, attrs: &[syn::Attribute], visit: impl FnOnce(&mut Self)) {
        attrs.iter().for_each(|attr| self.check_cfg_reason(attr));
        let native = attrs.iter().any(is_target_os_cfg);
        self.native_cfg += usize::from(native);
        visit(self);
        self.native_cfg -= usize::from(native);
    }

    fn check_root_ident(&mut self, ident: &syn::Ident) {
        let name = ident.to_string();
        if (name == "gpui" || name == "nd_gpui" || name.starts_with("objc2")) && !self.native_allowed() {
            self.report(
                ident.span(),
                "framework-internals",
                format!("`{name}` is named outside a native widget module"),
            );
        }
    }

    /// `use std::time::{Duration, Instant}` and the like, flattened to full paths.
    fn check_web_use(&mut self, prefix: &str, tree: &syn::UseTree) {
        let join = |name: &syn::Ident| if prefix.is_empty() { name.to_string() } else { format!("{prefix}::{name}") };
        match tree {
            syn::UseTree::Path(p) => self.check_web_use(&join(&p.ident), &p.tree),
            syn::UseTree::Name(n) => self.check_web_path(n.ident.span(), &join(&n.ident)),
            syn::UseTree::Rename(r) => self.check_web_path(r.ident.span(), &join(&r.ident)),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| self.check_web_use(prefix, t)),
            syn::UseTree::Glob(g) => self.check_web_path(g.star_token.span(), &format!("{prefix}::*")),
        }
    }

    fn check_web_path(&mut self, span: proc_macro2::Span, full: &str) {
        if self.web.is_none() {
            return;
        }
        let rule = if full.starts_with("std::time::Instant") || full.starts_with("std::time::SystemTime") {
            "web-std-time"
        } else if full == "std::thread" || full.starts_with("std::thread::") {
            "web-std-thread"
        } else if full == "std::fs" || full.starts_with("std::fs::") {
            "web-std-fs"
        } else {
            return;
        };
        let advice = WEB_RULES.iter().find(|(id, _)| *id == rule).map_or("", |(_, why)| why);
        self.report(span, rule, format!("`{full}`: {advice}"));
    }

    fn check_unavailable(&mut self, ident: &syn::Ident) {
        let Some(names) = self.web else { return };
        let name = ident.to_string();
        if names.contains(&name) && !self.guarded.last().copied().unwrap_or(false) {
            self.report(
                ident.span(),
                "web-unavailable",
                format!("`{name}` is unavailable on the web; check `nd::platform::has(Feature::..)` first"),
            );
        }
    }

    fn in_fn(&mut self, block: &syn::Block, visit: impl FnOnce(&mut Self)) {
        let span = block.brace_token.span.join();
        let body: String = (span.start().line..=span.end().line)
            .map(|n| self.line(n))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        self.guarded.push(body.contains("has(") && body.contains("Feature::"));
        visit(self);
        self.guarded.pop();
    }

    fn check_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.check_root_ident(&p.ident),
            syn::UseTree::Name(n) => self.check_root_ident(&n.ident),
            syn::UseTree::Rename(r) => self.check_root_ident(&r.ident),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| self.check_use_tree(t)),
            syn::UseTree::Glob(_) => {}
        }
    }
}

/// A string literal, bare or through `.into()` / `SharedString::from(..)`: the family the app
/// hardcodes. An empty string names none.
fn names_a_family(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => !s.value().is_empty(),
        syn::Expr::MethodCall(call) if call.args.is_empty() => names_a_family(&call.receiver),
        syn::Expr::Call(call) if call.args.len() == 1 => names_a_family(&call.args[0]),
        syn::Expr::Reference(r) => names_a_family(&r.expr),
        _ => false,
    }
}

fn is_target_os_cfg(attr: &syn::Attribute) -> bool {
    (attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        && attr.meta.require_list().is_ok_and(|list| list.tokens.to_string().contains("target_os"))
}

fn path_string(path: &syn::Path) -> String {
    path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::")
}

impl<'ast> Visit<'ast> for Linter<'_> {
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.check_cfg_reason(attr);
        syn::visit::visit_attribute(self, attr);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.with_attrs(&item.attrs, |this| syn::visit::visit_item_mod(this, item));
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.with_attrs(&item.attrs, |this| {
            this.check_use_tree(&item.tree);
            this.check_web_use("", &item.tree);
            syn::visit::visit_item_use(this, item);
        });
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.with_attrs(&item.attrs, |this| this.in_fn(&item.block, |this| syn::visit::visit_item_fn(this, item)));
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.with_attrs(&item.attrs, |this| syn::visit::visit_item_impl(this, item));
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.with_attrs(&item.attrs, |this| this.in_fn(&item.block, |this| syn::visit::visit_impl_item_fn(this, item)));
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.with_attrs(&item.attrs, |this| syn::visit::visit_item_struct(this, item));
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.with_attrs(&item.attrs, |this| syn::visit::visit_item_enum(this, item));
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.with_attrs(&local.attrs, |this| syn::visit::visit_local(this, local));
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first() {
            self.check_root_ident(&first.ident);
        }
        let full = path_string(path);
        if !path.segments.is_empty() {
            self.check_web_path(path.span(), &full);
        }
        if full.ends_with("Instant::now") || full.ends_with("thread::sleep") || full.contains("smol::Timer") {
            self.report(path.span(), "clock-in-ui", format!("`{full}` in UI code; use the executor's timers"));
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*call.func
            && let Some(last) = p.path.segments.last()
        {
            let name = last.ident.to_string();
            if matches!(name.as_str(), "rgb" | "rgba" | "hsl" | "hsla" | "oklch" | "oklcha") && !self.theme_file {
                self.report(
                    last.ident.span(),
                    "colour-literal",
                    format!("`{name}(..)` outside a theme file; use a token"),
                );
            }
            if name == "font" && call.args.first().is_some_and(names_a_family) && !self.theme_file {
                self.report(
                    last.ident.span(),
                    "font-family-literal",
                    "`font(\"..\")` names a font family; use a font token",
                );
            }
            if matches!(name.as_str(), "deferred" | "anchored") {
                self.report(last.ident.span(), "floating-element", format!("`{name}(..)` creates a floating element"));
            }
            self.check_unavailable(&last.ident);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "drawn" && call.args.is_empty() {
            let line = call.method.span().start().line;
            if !self.has_reason(line) {
                self.report(call.method.span(), "drawn-reason", "`.drawn()` without a reason comment");
            }
        }
        if call.method == "font_family" && call.args.first().is_some_and(names_a_family) && !self.theme_file {
            self.report(
                call.method.span(),
                "font-family-literal",
                "`.font_family(\"..\")` names a font family; use a font token",
            );
        }
        self.check_unavailable(&call.method);
        let mut scroll = None;
        let mut feeds_children = false;
        let mut chain = Some(call);
        while let Some(link) = chain {
            if matches!(link.method.to_string().as_str(), "overflow_y_scroll" | "overflow_x_scroll" | "overflow_scroll")
            {
                scroll = Some(link.method.span());
            }
            feeds_children |= link.method == "children";
            chain = match &*link.receiver {
                syn::Expr::MethodCall(inner) => Some(inner),
                _ => None,
            };
        }
        if let (Some(span), true) = (scroll, feeds_children) {
            self.report(span, "scroll-list", "scrolling container with `.children(..)`; use `list()`");
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        let is_font = expr.path.segments.last().is_some_and(|s| s.ident == "Font");
        for field in &expr.fields {
            if is_font
                && matches!(&field.member, syn::Member::Named(m) if m == "family")
                && names_a_family(&field.expr)
                && !self.theme_file
            {
                self.report(
                    field.expr.span(),
                    "font-family-literal",
                    "`Font { family }` names a font family; use a font token",
                );
            }
        }
        syn::visit::visit_expr_struct(self, expr);
    }

    fn visit_expr_lit(&mut self, lit: &'ast syn::ExprLit) {
        if let syn::Lit::Str(s) = &lit.lit
            && s.value().contains("<svg")
        {
            self.report(s.span(), "inline-svg", "inline SVG in a string literal");
        }
        syn::visit::visit_expr_lit(self, lit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(source: &str) -> Vec<&'static str> {
        lint_source(Path::new("src/app.rs"), source).iter().map(|f| f.rule).collect()
    }

    #[test]
    fn clean_source_has_no_findings() {
        assert!(rules("fn main() { let x = 1; }").is_empty());
    }

    #[test]
    fn flags_gpui_and_objc2_outside_native_modules() {
        assert_eq!(rules("use gpui::px;"), ["framework-internals"]);
        assert_eq!(rules("fn f() { objc2_foundation::NSString::new(); }"), ["framework-internals"]);
    }

    #[test]
    fn native_cfg_items_and_files_may_name_them() {
        assert!(rules("// macOS only\n#[cfg(target_os = \"macos\")]\nuse objc2_foundation::NSString;").is_empty());
        let found = lint_source(Path::new("src/map/appkit.rs"), "use objc2_foundation::NSString;");
        assert!(found.is_empty());
    }

    #[test]
    fn cfg_target_os_needs_a_reason() {
        assert_eq!(rules("#[cfg(target_os = \"linux\")]\nfn f() {}"), ["cfg-target-os"]);
        assert!(rules("// Wayland only: no AppKit equivalent.\n#[cfg(target_os = \"linux\")]\nfn f() {}").is_empty());
        assert!(rules("#[cfg(target_os = \"linux\")] // reason\nfn f() {}").is_empty());
    }

    #[test]
    fn flags_colour_literals_except_in_theme_files() {
        assert_eq!(rules("fn f() { let c = rgb(0xff0000); }"), ["colour-literal"]);
        assert!(lint_source(Path::new("src/theme.rs"), "fn f() { let c = rgb(0xff0000); }").is_empty());
    }

    #[test]
    fn flags_hardcoded_font_families_except_in_theme_files() {
        assert_eq!(rules("fn f() { div().font_family(\"Inter\"); }"), ["font-family-literal"]);
        assert_eq!(rules("fn f() { let f = font(\"SF Pro\"); }"), ["font-family-literal"]);
        assert_eq!(
            rules("fn f() { let f = Font { family: \"Menlo\".into(), ..font(\"\") }; }"),
            ["font-family-literal"]
        );
        assert!(rules("fn f(cx: &App) { div().font_family(cx.theme().fonts.mono.family.clone()); }").is_empty());
        assert!(rules("fn f(t: &Theme) { div().font(t.fonts.body.clone()); }").is_empty());
        assert!(lint_source(Path::new("src/theme.rs"), "fn f() { let f = font(\"Inter\"); }").is_empty());
    }

    #[test]
    fn flags_clocks_svg_scroll_lists_and_floats() {
        assert_eq!(rules("fn f() { std::time::Instant::now(); }"), ["clock-in-ui"]);
        assert_eq!(rules("fn f() { std::thread::sleep(d); }"), ["clock-in-ui"]);
        assert_eq!(rules("fn f() -> &'static str { \"<svg></svg>\" }"), ["inline-svg"]);
        assert_eq!(rules("fn f() { div().overflow_y_scroll().children(rows); }"), ["scroll-list"]);
        assert_eq!(rules("fn f() { deferred(div()); }"), ["floating-element"]);
    }

    #[test]
    fn drawn_needs_a_reason() {
        assert_eq!(rules("fn f() { slider().drawn(); }"), ["drawn-reason"]);
        assert!(rules("fn f() {\n // Native track cannot take this radius.\n slider().drawn(); }").is_empty());
    }

    #[test]
    fn nd_allow_silences_one_rule_on_a_line() {
        assert!(rules("// nd-allow(clock-in-ui): benchmark harness\nfn f() { std::time::Instant::now(); }").is_empty());
    }

    fn web_rules(source: &str) -> Vec<&'static str> {
        let names: HashSet<String> = ["open_window_tab", "set_tray"].map(String::from).into();
        lint_source_for(Path::new("src/app.rs"), source, Some(&names)).iter().map(|f| f.rule).collect()
    }

    #[test]
    fn web_rules_only_run_for_the_web() {
        assert!(rules("use std::fs;\nfn f() { cx.set_tray(t); }").is_empty());
    }

    #[test]
    fn web_flags_std_time_but_not_duration() {
        assert_eq!(web_rules("use std::time::{Duration, Instant};"), ["web-std-time"]);
        assert_eq!(web_rules("fn f() { let t = std::time::SystemTime::now(); }"), ["web-std-time"]);
        assert!(web_rules("use std::time::Duration;\nuse nd::time::Instant;").is_empty());
    }

    #[test]
    fn web_flags_std_thread_and_fs() {
        assert_eq!(web_rules("use std::thread;"), ["web-std-thread"]);
        assert_eq!(web_rules("fn f() { std::fs::read(p); }"), ["web-std-fs"]);
        assert!(web_rules("use std::rc::Rc;\nfn f() { nd::store::Store::open(); }").is_empty());
    }

    #[test]
    fn web_unavailable_needs_a_feature_check() {
        assert_eq!(web_rules("fn f(cx: &mut App) { cx.set_tray(t); }"), ["web-unavailable"]);
        assert_eq!(web_rules("fn f() { open_window_tab(); }"), ["web-unavailable"]);
        let guarded = "fn f(cx: &mut App) { if nd::platform::has(Feature::Tray) { cx.set_tray(t); } }";
        assert!(web_rules(guarded).is_empty());
        assert!(web_rules("fn f(cx: &mut App) { cx.set_title(t); }").is_empty());
    }

    #[test]
    fn web_unavailable_list_reads_framework_attributes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = "#[unavailable(web)]\npub fn set_tray() {}\nimpl A {\n#[nd_macros::unavailable(web)]\npub fn open_tab(&self) {}\n#[unavailable(linux)]\npub fn other(&self) {}\n}\npub fn plain() {}";
        std::fs::write(dir.path().join("lib.rs"), source).expect("write");
        let names = unavailable_on_web(&[dir.path().to_path_buf()]).expect("scan");
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        assert_eq!(names, ["open_tab", "set_tray"]);
    }

    #[test]
    fn unparsable_source_is_a_finding() {
        assert_eq!(rules("fn ("), ["parse"]);
    }
}
