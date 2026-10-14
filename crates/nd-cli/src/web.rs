//! `nd build` and `nd serve`: the native release build, and the static web bundle
//! (ARCHITECTURE.md 12): the app's binary compiled for `wasm32-unknown-unknown`, run through the
//! `wasm-bindgen` CLI matching the locked `wasm-bindgen` crate, plus an `index.html` loader.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use nd_config::Config;

pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Target {
    Native,
    Web,
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    #[arg(long, value_enum, default_value = "native")]
    pub target: Target,
    /// The app package. Defaults to the package whose manifest is in the current directory.
    #[arg(short, long = "package")]
    pub package: Option<String>,
    /// Build without optimisations.
    #[arg(long)]
    pub debug: bool,
    /// Where the web bundle goes. Defaults to `target/nd/web/<package>/`.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, value_enum, default_value = "web")]
    pub target: Target,
    #[arg(short, long = "package")]
    pub package: Option<String>,
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    /// Serve the last bundle as it is.
    #[arg(long)]
    pub no_build: bool,
    #[arg(long)]
    pub debug: bool,
    #[arg(long)]
    pub out: Option<PathBuf>,
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// The app package as `cargo metadata` resolves it, with the `wasm-bindgen` version it locks.
#[derive(Debug)]
pub struct WebProject {
    pub name: String,
    pub root: PathBuf,
    pub bin: String,
    pub target_dir: PathBuf,
    pub wasm_bindgen: Option<String>,
    pub config: Option<Config>,
}

impl WebProject {
    pub fn load(package: Option<&str>) -> Result<Self> {
        let output = cargo().args(["metadata", "--format-version", "1"]).output().context("running cargo metadata")?;
        if !output.status.success() {
            bail!("cargo metadata: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        let meta: serde_json::Value = serde_json::from_slice(&output.stdout).context("cargo metadata output")?;
        let members: Vec<&str> =
            meta["workspace_members"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
        let cwd = std::env::current_dir()?.canonicalize()?;
        let packages = meta["packages"].as_array().context("cargo metadata packages")?;
        let manifest_dir = |p: &serde_json::Value| {
            p["manifest_path"].as_str().and_then(|m| Path::new(m).parent()).map(Path::to_path_buf).unwrap_or_default()
        };
        let pkg = packages
            .iter()
            .filter(|p| p["id"].as_str().is_some_and(|id| members.contains(&id)))
            .find(|p| match package {
                Some(name) => p["name"] == name,
                None => manifest_dir(p).canonicalize().is_ok_and(|d| d == cwd),
            })
            .with_context(|| match package {
                Some(name) => format!("no workspace package named `{name}`"),
                None => "no package here; pass -p <package>".into(),
            })?;
        let name = pkg["name"].as_str().context("package name")?.to_owned();
        let bins: Vec<&str> = pkg["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "bin")))
            .filter_map(|t| t["name"].as_str())
            .collect();
        let bin = match bins.as_slice() {
            [one] => one.to_string(),
            [] => bail!("`{name}` has no binary target"),
            many if many.contains(&name.as_str()) => name.clone(),
            many => bail!("`{name}` has several binaries ({}); keep one", many.join(", ")),
        };
        let root = manifest_dir(pkg);
        let config_path = root.join("nativedesktop.toml");
        let config = if config_path.is_file() { Some(Config::load(&config_path)?) } else { None };
        let wasm_bindgen = packages
            .iter()
            .find(|p| p["name"] == "wasm-bindgen")
            .and_then(|p| p["version"].as_str())
            .map(str::to_owned);
        Ok(Self {
            name,
            root,
            bin,
            target_dir: PathBuf::from(meta["target_directory"].as_str().context("target_directory")?),
            wasm_bindgen,
            config,
        })
    }

    pub fn title(&self) -> String {
        match &self.config {
            Some(config) => config.display_name(&self.name).to_owned(),
            None => self.name.clone(),
        }
    }

    pub fn base_path(&self) -> Option<&str> {
        self.config.as_ref()?.web.base_path.as_deref()
    }

    pub fn default_out(&self) -> PathBuf {
        self.target_dir.join("nd").join("web").join(&self.name)
    }
}

pub fn run_build(args: BuildArgs) -> Result<ExitCode> {
    match args.target {
        Target::Native => {
            let mut cmd = cargo();
            cmd.arg("build");
            if !args.debug {
                cmd.arg("--release");
            }
            if let Some(p) = &args.package {
                cmd.args(["-p", p]);
            }
            Ok(if cmd.status().context("running cargo build")?.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Target::Web => {
            let project = WebProject::load(args.package.as_deref())?;
            let out = args.out.unwrap_or_else(|| project.default_out());
            build_web(&project, &out, args.debug)?;
            println!("nd build: {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Compiles the app for the browser and writes the bundle into `out`.
pub fn build_web(project: &WebProject, out: &Path, debug: bool) -> Result<()> {
    let wasm = cargo_wasm(project, debug)?;
    let version = project.wasm_bindgen.as_deref().context("the app does not depend on wasm-bindgen")?;
    let bindgen = wasm_bindgen(version)?;
    // `--out` is the user's path: clear only a folder that is empty or holds an earlier bundle.
    let earlier = out.join("index.html").is_file() && out.join(format!("{}_bg.wasm", project.bin)).is_file();
    let empty = std::fs::read_dir(out).map_or(true, |mut d| d.next().is_none());
    if !earlier && !empty {
        bail!("{} is not empty and holds no earlier `{}` bundle; refusing to clear it", out.display(), project.bin);
    }
    crate::project::fresh_dir(out)?;
    let status = Command::new(&bindgen)
        .args(["--target", "web", "--no-typescript", "--out-name", &project.bin, "--out-dir"])
        .arg(out)
        .arg(&wasm)
        .status()
        .with_context(|| format!("running {}", bindgen.display()))?;
    if !status.success() {
        bail!("wasm-bindgen failed ({status})");
    }
    let bg = out.join(format!("{}_bg.wasm", project.bin));
    if !debug && on_path("wasm-opt") {
        let status = Command::new("wasm-opt").arg("-Oz").arg(&bg).arg("-o").arg(&bg).status()?;
        if !status.success() {
            bail!("wasm-opt failed ({status})");
        }
    }
    crate::project::write(&out.join("index.html"), index_html(&project.title(), &project.bin, project.base_path()))?;
    let assets = project.root.join("assets");
    if assets.is_dir() {
        copy_dir(&assets, &out.join("assets"))?;
    }
    Ok(())
}

fn cargo_wasm(project: &WebProject, debug: bool) -> Result<PathBuf> {
    let mut cmd = cargo();
    cmd.args(["build", "--target", WASM_TARGET, "--message-format=json-render-diagnostics", "-p"])
        .arg(&project.name)
        .args(["--bin", &project.bin])
        .stdout(Stdio::piped());
    if !debug {
        cmd.arg("--release");
    }
    let output = cmd.spawn().context("running cargo build")?.wait_with_output()?;
    if !output.status.success() {
        bail!("cargo build --target {WASM_TARGET} failed ({})", output.status);
    }
    let mut wasm = None;
    for line in output.stdout.split(|b| *b == b'\n') {
        let Ok(msg) = serde_json::from_slice::<serde_json::Value>(line) else { continue };
        if msg["reason"] == "compiler-artifact" && msg["target"]["name"] == project.bin.as_str() {
            wasm = msg["filenames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| f.as_str())
                .find(|f| f.ends_with(".wasm"))
                .map(PathBuf::from)
                .or(wasm);
        }
    }
    wasm.context("cargo build produced no .wasm")
}

fn on_path(tool: &str) -> bool {
    Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn tools_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".cache/nd/tools")
}

/// The `wasm-bindgen` CLI of exactly `version`: the CLI refuses a wasm built against any other.
fn wasm_bindgen(version: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ND_WASM_BINDGEN") {
        return Ok(path.into());
    }
    let root = tools_dir().join(format!("wasm-bindgen-{version}"));
    let cached = root.join("bin/wasm-bindgen");
    if cached.is_file() {
        return Ok(cached);
    }
    if let Ok(out) = Command::new("wasm-bindgen").arg("--version").output()
        && String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1) == Some(version)
    {
        return Ok("wasm-bindgen".into());
    }
    println!("nd build: installing wasm-bindgen-cli {version} into {}", root.display());
    let status = cargo()
        .args(["install", "wasm-bindgen-cli", "--version", &format!("={version}"), "--locked", "--root"])
        .arg(&root)
        .status()
        .context("running cargo install wasm-bindgen-cli")?;
    if !status.success() || !cached.is_file() {
        bail!("installing wasm-bindgen-cli {version} failed");
    }
    Ok(cached)
}

fn html_escape(text: &str) -> String {
    crate::project::xml_escape(text)
}

pub fn index_html(title: &str, bin: &str, base_path: Option<&str>) -> String {
    let base = base_path.map(|b| format!("\n<base href=\"{}\">", html_escape(b))).unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">{base}
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; }}
html, body {{ margin: 0; height: 100%; overflow: hidden; background: #ffffff; }}
@media (prefers-color-scheme: dark) {{ html, body {{ background: #1e1e1e; }} }}
</style>
</head>
<body>
<script type="module">import init from "./{bin}.js"; init();</script>
</body>
</html>
"#,
        title = html_escape(title),
    )
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        let dest = to.join(path.file_name().unwrap_or_default());
        if path.is_dir() {
            copy_dir(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).with_context(|| format!("copying {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
        "wasm" => "application/wasm",
        "js" | "mjs" => "text/javascript",
        "html" => "text/html; charset=utf-8",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ttf" => "font/ttf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub fn run_serve(args: ServeArgs) -> Result<ExitCode> {
    if args.target != Target::Web {
        bail!("`nd serve` serves the web bundle; pass --target web");
    }
    let project = WebProject::load(args.package.as_deref())?;
    let out = args.out.clone().unwrap_or_else(|| project.default_out());
    if !args.no_build {
        build_web(&project, &out, args.debug)?;
    }
    if !out.join("index.html").is_file() {
        bail!("no bundle at {}; run without --no-build", out.display());
    }
    serve_dir(&out, args.port)
}

fn serve_dir(dir: &Path, port: u16) -> Result<ExitCode> {
    let server =
        tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("binding port {port}: {e}"))?;
    println!("nd serve: http://127.0.0.1:{port}/");
    let root = dir.canonicalize()?;
    for request in server.incoming_requests() {
        let url = request.url().split(['?', '#']).next().unwrap_or("/").to_owned();
        let relative = url.trim_start_matches('/');
        let mut path = root.join(if relative.is_empty() { "index.html" } else { relative });
        if path.is_dir() {
            path = path.join("index.html");
        }
        // Reject `..` escapes out of the bundle.
        let inside = path.canonicalize().is_ok_and(|p| p.starts_with(&root));
        let response = match std::fs::read(&path) {
            Ok(body) if inside => {
                let header = tiny_http::Header::from_bytes("Content-Type", content_type(&path))
                    .map_err(|()| anyhow::anyhow!("bad header"))?;
                tiny_http::Response::from_data(body).with_header(header).with_status_code(200)
            }
            _ => tiny_http::Response::from_data(b"not found".to_vec()).with_status_code(404),
        };
        let _ = request.respond(response);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_html_carries_title_loader_and_base() {
        let html = index_html("Notes & Co", "notes", Some("/notes/"));
        assert!(html.contains("<title>Notes &amp; Co</title>"));
        assert!(html.contains("<base href=\"/notes/\">"));
        assert!(html.contains(r#"import init from "./notes.js"; init();"#));
        assert!(html.contains("prefers-color-scheme: dark"));
        assert!(!index_html("hello", "hello", None).contains("<base"));
    }

    #[test]
    fn wasm_is_served_as_wasm() {
        assert_eq!(content_type(Path::new("a/app_bg.wasm")), "application/wasm");
        assert_eq!(content_type(Path::new("app.js")), "text/javascript");
    }
}
