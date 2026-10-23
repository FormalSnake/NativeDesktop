//! fspec side of the settings: the generated `fspec.star` an app ships at its repo root, option
//! paths and Starlark literals ("Copy as fspec").
//!
//! The module mirrors the home-manager module: `settings` (checked against the schema, written
//! to `fspec.toml`), `theme` (`theme.json`), `autostart` and `environment_file`, plus the package
//! itself. Starlark has no types and no recursion, so the schema becomes a flat table of dotted
//! keys and the checks walk it with bounded loops.

use serde_json::Value;

use crate::schema::{Prop, Ty, root_props};

/// nd-terminal's build script compiles libghostty-vt, which pins a Ghostty commit that needs
/// this Zig. Keep in step with dev.star and flake.nix.
const ZIG: &str = "0.16.0";

/// The build image and the openSUSE packages every nd app needs to compile on Linux: GPUI links
/// xkbcommon and freetype and dlopens the rest, cpal needs ALSA, nd-system D-Bus.
const TUMBLEWEED: &str = "registry.opensuse.org/opensuse/tumbleweed";
const TUMBLEWEED_DEPS: &[&str] = &[
    "rustup",
    "gcc",
    "gcc-c++",
    "make",
    "pkgconf",
    "git-core",
    "curl",
    "tar",
    "xz",
    "fontconfig-devel",
    "freetype2-devel",
    "libxkbcommon-devel",
    "libxkbcommon-x11-devel",
    "wayland-devel",
    "vulkan-devel",
    "libX11-devel",
    "alsa-devel",
    "dbus-1-devel",
    "libopenssl-devel",
];

/// What the generator needs to know about an app beyond its settings schema.
#[derive(Clone, Debug)]
pub struct App {
    name: String,
    display_name: String,
    repo: String,
    package: String,
    build_deps: Vec<String>,
}

impl App {
    /// `name` is the binary, the config directory and the module function, as `pname` is for
    /// the home-manager module. `repo` is the git URL the package builds from.
    pub fn new(name: &str, repo: &str) -> Self {
        Self {
            name: name.to_owned(),
            display_name: name.to_owned(),
            repo: repo.trim_end_matches(".git").to_owned(),
            package: name.to_owned(),
            build_deps: Vec::new(),
        }
    }

    /// The unit description. Defaults to `name`.
    pub fn display_name(mut self, display_name: &str) -> Self {
        display_name.clone_into(&mut self.display_name);
        self
    }

    /// The cargo package that builds the binary. Defaults to `name`.
    pub fn package(mut self, package: &str) -> Self {
        package.clone_into(&mut self.package);
        self
    }

    /// openSUSE packages the app's own crates need on top of nd's (`opus-devel`).
    pub fn build_deps(mut self, deps: &[&str]) -> Self {
        self.build_deps = deps.iter().map(|d| (*d).to_owned()).collect();
        self
    }
}

/// `<app>(settings = {"a": {"b": ...}})`, where `key` is set in the app's fspec module call.
pub fn option_path(app: &str, key: &str) -> String {
    format!("{app}({})", nested(key, "...".into()))
}

/// `settings = {"a": {"b": <value>}}`.
pub fn settings_literal(key: &str, value: &toml::Value) -> String {
    nested(key, literal(value))
}

fn nested(key: &str, leaf: String) -> String {
    let inner = key.rsplit('.').fold(leaf, |acc, segment| format!("{{{}: {acc}}}", string(segment)));
    format!("settings = {inner}")
}

/// A Starlark expression for a TOML value.
pub fn literal(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => string(s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => format!("{f:?}"),
        toml::Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
        toml::Value::Datetime(d) => string(&d.to_string()),
        toml::Value::Array(items) => format!("[{}]", items.iter().map(literal).collect::<Vec<_>>().join(", ")),
        toml::Value::Table(table) => format!(
            "{{{}}}",
            table.iter().map(|(k, v)| format!("{}: {}", string(k), literal(v))).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Starlark and JSON agree on double-quoted string escapes.
fn string(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

/// The `fspec.star` for `app` and its settings schema (the output of [`crate::schema_json`]).
pub fn star_module(app: &App, schema: &Value) -> String {
    let mut keys = Vec::new();
    let mut sections = Vec::new();
    let depth = walk(&root_props(schema), "", &mut keys, &mut sections);
    let name = &app.name;
    let deps: Vec<String> =
        TUMBLEWEED_DEPS.iter().map(|d| (*d).to_owned()).chain(app.build_deps.iter().cloned()).collect();
    // ND_NO_UPDATES: fspec owns the binary, so the in-app updater stays out, as under mkApp.
    let build = format!(
        "set -e; cd \"$FSPEC_SRC\"; \
         if [ -e rust-toolchain.toml ] || [ -e rust-toolchain ]; then rustup toolchain install; else rustup default stable; fi; \
         zig=$(mktemp -d); curl -fsSL \"https://ziglang.org/download/{ZIG}/zig-$(uname -m)-linux-{ZIG}.tar.xz\" | tar -xJ -C \"$zig\"; \
         ND_NO_UPDATES=1 PATH=\"$zig/zig-$(uname -m)-linux-{ZIG}:$PATH\" cargo build --release --locked -p {package}; \
         install -Dm755 target/release/{name} \"$FSPEC_OUT/bin/{name}\"",
        package = app.package,
    );

    let mut out = String::new();
    out.push_str(&format!(
        "# Generated by nd from the settings struct. Edit the Rust struct, not this file.\n\
         #\n\
         #   load(\"gh:{repo}//fspec.star\", \"{name}\")\n\
         #   host(..., modules = [{name}(settings = {{...}}, autostart = True)])\n\
         #\n\
         # settings are checked against the schema and written to fspec.toml in the app's config\n\
         # directory; the app shows those keys read-only. The package builds the commit this file\n\
         # was loaded from unless `rev` or `source` says otherwise.\n\n",
        repo = app.repo.trim_start_matches("https://github.com/"),
    ));
    out.push_str(&format!("_NAME = {}\n", string(name)));
    out.push_str(&format!("_DISPLAY = {}\n", string(&app.display_name)));
    out.push_str(&format!("_REPO = {}\n", string(&app.repo)));
    out.push_str(&format!("_BUILD = {}\n", string(&build)));
    out.push_str(&format!(
        "_IMAGES = {{\n    \"opensuse-tumbleweed\": ({}, {}),\n}}\n",
        string(TUMBLEWEED),
        list(&deps)
    ));
    out.push_str(&format!("_DEPTH = {}\n", depth + 1));
    out.push_str("_SECTIONS = [");
    out.push_str(&sections.iter().map(|s| string(s)).collect::<Vec<_>>().join(", "));
    out.push_str("]\n_KEYS = {\n");
    for (key, desc) in &keys {
        out.push_str(&format!("    {}: {desc},\n", string(key)));
    }
    out.push_str("}\n");
    out.push_str(BODY.replace("__NAME__", name).as_str());
    out
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|i| string(i)).collect::<Vec<_>>().join(", "))
}

/// Fills `keys` with every leaf by dotted key and `sections` with every nested table; returns
/// the deepest section level.
fn walk(props: &[Prop], prefix: &str, keys: &mut Vec<(String, String)>, sections: &mut Vec<String>) -> usize {
    let mut depth = 0;
    for prop in props {
        let key = format!("{prefix}{}", prop.name);
        match &prop.ty {
            Ty::Object(children) => {
                sections.push(key.clone());
                depth = depth.max(1 + walk(children, &format!("{key}."), keys, sections));
            }
            ty => keys.push((key, descriptor(ty, true))),
        }
    }
    depth
}

/// A type descriptor the module's `_check` reads. Lists and maps check one level of items.
fn descriptor(ty: &Ty, outer: bool) -> String {
    let bounds = |t: &str, min: &Option<f64>, max: &Option<f64>| {
        let mut s = format!("{{\"t\": \"{t}\"");
        if let Some(min) = min {
            s.push_str(&format!(", \"min\": {}", number(*min, t == "int")));
        }
        if let Some(max) = max {
            s.push_str(&format!(", \"max\": {}", number(*max, t == "int")));
        }
        s.push('}');
        s
    };
    match ty {
        Ty::Bool => "{\"t\": \"bool\"}".into(),
        Ty::Int { min, max } => bounds("int", min, max),
        Ty::Number { min, max } => bounds("number", min, max),
        Ty::Str => "{\"t\": \"str\"}".into(),
        Ty::Enum(choices) => {
            let values: Vec<String> = choices.iter().map(|c| c.value.clone()).collect();
            format!("{{\"t\": \"enum\", \"of\": {}}}", list(&values))
        }
        Ty::Nullable(inner) => descriptor(inner, outer),
        Ty::List(inner) if outer => format!("{{\"t\": \"list\", \"of\": {}}}", descriptor(inner, false)),
        Ty::Map(inner) if outer => format!("{{\"t\": \"map\", \"of\": {}}}", descriptor(inner, false)),
        Ty::List(_) | Ty::Map(_) | Ty::Object(_) | Ty::Any => "{\"t\": \"any\"}".into(),
    }
}

fn number(v: f64, int: bool) -> String {
    if int || v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

const BODY: &str = r#"
def _expect(d):
    if d["t"] == "enum":
        return "one of " + ", ".join(['"%s"' % v for v in d["of"]])
    return {"bool": "a bool", "int": "an int", "number": "a number", "str": "a string", "list": "a list", "map": "a dict"}[d["t"]]

def _value(key, v, d):
    t = d["t"]
    if t == "any":
        return
    if t == "bool":
        ok = type(v) == "bool"
    elif t == "int":
        ok = type(v) == "int"
    elif t == "number":
        ok = type(v) in ("int", "float")
    elif t == "str":
        ok = type(v) == "string"
    elif t == "enum":
        ok = type(v) == "string" and v in d["of"]
    elif t == "list":
        ok = type(v) == "list"
    else:
        ok = type(v) == "dict"
    if not ok:
        fail("%s: settings.%s is %r, expected %s" % (_NAME, key, v, _expect(d)))
    if "min" in d and v < d["min"]:
        fail("%s: settings.%s is %r, below the minimum %r" % (_NAME, key, v, d["min"]))
    if "max" in d and v > d["max"]:
        fail("%s: settings.%s is %r, above the maximum %r" % (_NAME, key, v, d["max"]))

def _check(key, v, d):
    _value(key, v, d)
    if d["t"] == "list":
        for i, item in enumerate(v):
            _value("%s[%d]" % (key, i), item, d["of"])
    elif d["t"] == "map":
        for k, item in v.items():
            _value(key + "." + k, item, d["of"])

# Dotted key to value, None dropped; `plugins.<name>` sections pass whole, each plugin checks its
# own keys when the app loads them.
def _flatten(settings):
    flat = {}
    todo = [("", settings)]
    for _ in range(_DEPTH):
        nxt = []
        for prefix, table in todo:
            if type(table) != "dict":
                fail("%s: settings.%s must be a dict" % (_NAME, prefix[:-1]))
            for k, v in table.items():
                key = prefix + k
                if v == None:
                    continue
                if key in _SECTIONS:
                    nxt.append((key + ".", v))
                elif key in _KEYS:
                    _check(key, v, _KEYS[key])
                    flat[key] = v
                elif key == "plugins" or key.startswith("plugins."):
                    if type(v) != "dict":
                        fail("%s: settings.%s must be a dict" % (_NAME, key))
                    flat[key] = v
                else:
                    fail("%s: settings has no key `%s`" % (_NAME, key))
        todo = nxt
    return flat

def _nest(flat):
    out = {}
    for key, value in flat.items():
        parts = key.split(".")
        table = out
        for part in parts[:-1]:
            if part not in table:
                table[part] = {}
            table = table[part]
        table[parts[-1]] = value
    return out

def _source(h, rev):
    if rev == None:
        o = origin()
        if o == None:
            fail("%s: loaded from this spec's own tree, so pass rev = \"<commit>\" or source = ..." % _NAME)
        rev = o.rev
    image = _IMAGES.get(h.distro)
    if image == None:
        fail("%s: no build image for %s; pass source = ..." % (_NAME, h.distro))
    return git(_REPO, rev = rev, image = image[0], deps = image[1])

def __NAME__(settings = {}, theme = {}, autostart = False, environment_file = None, rev = None, source = None):
    """The app, its settings and its autostart unit, as a host module.

    settings: the app's config.toml keys, nested dicts by section; None leaves a key unset.
    theme: palette tokens written to theme.json; empty leaves the file to another tool.
    autostart: start with the graphical session (systemd user unit, launchd agent on macOS).
    environment_file: KEY=value lines for the unit, for secrets that stay out of the spec.
    rev: the commit (or branch, for fspec update to track) to build instead of this file's.
    source: any fspec source in place of the build from git.
    """
    toml = _nest(_flatten(settings))
    for k, v in theme.items():
        if type(v) != "string":
            fail("%s: theme.%s must be a string" % (_NAME, k))

    def module(h):
        if h.platform.is_darwin:
            if source == None:
                fail("%s: no macOS build recipe; pass source = ..." % _NAME)
            if environment_file != None:
                fail("%s: environment_file is not supported by the macOS launchd agent" % _NAME)
            config = "~/Library/Application Support/" + _NAME
        else:
            config = "~/.config/" + _NAME
        files = []
        if toml:
            files.append(file(config + "/fspec.toml", content = toml))
        if theme:
            files.append(file(config + "/theme.json", content = theme))
        services = []
        if autostart:
            extra = {}
            if h.platform.is_linux:
                extra["systemd"] = {
                    "unit": {"After": "graphical-session.target", "PartOf": "graphical-session.target"},
                    "install": {"WantedBy": "graphical-session.target"},
                }
                if environment_file != None:
                    extra["systemd"]["EnvironmentFile"] = environment_file
            services.append(service(_NAME, exec = "/opt/fspec/current/bin/" + _NAME, user = True, restart = "on_failure", description = _DISPLAY, **extra))
        build = {"build": ["sh", "-c", _BUILD], "bins": [_NAME]} if source == None else {}
        return [pkg(
            _NAME,
            source = source if source != None else _source(h, rev),
            files = files,
            services = services,
            data = [config],
            **build
        )]

    return module
"#;
