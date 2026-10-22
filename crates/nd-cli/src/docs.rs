//! `nd docs`: the docs site's checks, build and captures (ARCHITECTURE.md 13).
//!
//! - `check`: every page loads (snippets, examples and directives resolve), the link check, the
//!   prose check over pages and `nd new` templates, and the capture report. `--compile` builds
//!   every registered example (and the web ones for wasm), `--web` loads the docs app and each
//!   web example in headless Chrome and fails on a blank canvas, `--require macos` fails on a
//!   missing or stale macOS capture.
//! - `build`: the static site (an HTML page and the raw markdown per page, `llms.txt`,
//!   `llms-full.txt`) with the docs app's web bundle under `app/`, code blocks highlighted by
//!   `scripts/docs-highlight` (Node). `serve` builds and serves it.
//! - `capture`: runs each example through the test harness on this OS, light and dark, and
//!   records the captures with the example's source hash. On Windows the examples are launched
//!   and captured by `ndshell agent` in the desktop session, so it also runs over ssh.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use nd_docs::captures::{self, Manifest, Os, State};
use nd_docs::examples::Example;

use crate::web::{self, WebProject};

const APP_PACKAGE: &str = "nd-docs-app";

#[derive(Args, Debug)]
pub struct DocsArgs {
    #[command(subcommand)]
    pub command: DocsCommand,
}

#[derive(Subcommand, Debug)]
pub enum DocsCommand {
    /// Pages, links, prose and captures; `--compile` and `--web` for the slower checks.
    Check(CheckArgs),
    /// The static site (HTML, raw markdown, llms.txt) with the docs app under app/.
    Build(BuildArgs),
    /// Build, then serve the site locally.
    Serve(ServeArgs),
    /// Capture every example (or the ones named) on this OS, light and dark.
    Capture(CaptureArgs),
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Fail when a capture for this OS is missing or stale (CI on that OS).
    #[arg(long, value_parser = parse_os)]
    pub require: Vec<Os>,
    /// `cargo check` every registered example, and the web ones for wasm32.
    #[arg(long)]
    pub compile: bool,
    /// Build the docs app and each web example for the browser and load them in headless Chrome.
    #[arg(long)]
    pub web: bool,
    /// Also list external links (they are not fetched).
    #[arg(long)]
    pub external: bool,
}

#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Defaults to `target/nd/docs-site/`.
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub debug: bool,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub debug: bool,
    /// Serve the last build as it is.
    #[arg(long)]
    pub no_build: bool,
}

#[derive(Args, Debug)]
pub struct CaptureArgs {
    /// Example names from docs/examples.toml; all of them when empty.
    pub examples: Vec<String>,
    /// Only examples whose capture for this OS is missing or stale.
    #[arg(long)]
    pub stale: bool,
    /// Seconds to let a window settle before the capture.
    #[arg(long, default_value_t = 2.0)]
    pub settle: f32,
}

fn parse_os(text: &str) -> Result<Os, String> {
    Os::parse(text).ok_or_else(|| format!("`{text}` is not macos, linux or windows"))
}

pub fn run(args: DocsArgs) -> Result<ExitCode> {
    let repo = repo()?;
    match args.command {
        DocsCommand::Check(a) => check(&repo, &a),
        DocsCommand::Build(a) => {
            let out = build(&repo, a.out, a.debug)?;
            println!("nd docs: {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        DocsCommand::Serve(a) => {
            let out = match (a.no_build, a.out) {
                (true, out) => out.unwrap_or_else(|| default_out(&repo)),
                (false, out) => build(&repo, out, a.debug)?,
            };
            web::serve_dir(&out, a.port)
        }
        DocsCommand::Capture(a) => capture(&repo, &a),
    }
}

fn repo() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    nd_docs::find_repo(&cwd).context("no docs/content/nav.toml above the current directory")
}

fn default_out(repo: &Path) -> PathBuf {
    repo.join("target/nd/docs-site")
}

fn build(repo: &Path, out: Option<PathBuf>, debug: bool) -> Result<PathBuf> {
    let site = nd_docs::load(repo)?;
    let out = out.unwrap_or_else(|| default_out(repo));
    let project = in_dir(repo, || WebProject::load(Some(APP_PACKAGE)))?;
    web::build_web(&project, &out.join("app"), debug)?;
    nd_docs::export(&site, &Manifest::load(repo)?, &out)?;
    highlight(repo, &out)?;
    Ok(out)
}

/// Highlights the exported pages' code blocks with `scripts/docs-highlight`, a Node script around
/// @pierre/highlights.
fn highlight(repo: &Path, out: &Path) -> Result<()> {
    let dir = repo.join("scripts/docs-highlight");
    let mut install = system_command("npm");
    install.arg("ci").arg("--prefix").arg(&dir).args([
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--loglevel=error",
    ]);
    let mut run = system_command("node");
    run.arg(dir.join("highlight.mjs")).arg(out);
    for mut command in [install, run] {
        let program = command.get_program().to_string_lossy().into_owned();
        let status =
            command.status().with_context(|| format!("running {program}: the docs build needs Node.js and npm"))?;
        if !status.success() {
            bail!("{program} failed ({status})");
        }
    }
    Ok(())
}

/// Runs `f` with the current directory at `dir`, for helpers that ask cargo about "here".
fn in_dir<T>(dir: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let before = std::env::current_dir()?;
    std::env::set_current_dir(dir)?;
    let result = f();
    std::env::set_current_dir(before)?;
    result
}

fn check(repo: &Path, args: &CheckArgs) -> Result<ExitCode> {
    let mut ok = true;
    println!("nd docs: pages");
    let site = nd_docs::load(repo)?;
    println!("  {} pages in {} sections, {} examples", site.pages.len(), site.sections.len(), site.examples.len());

    println!("nd docs: links");
    let (findings, external) = nd_docs::links::check(repo, &site);
    for finding in &findings {
        println!("  {finding}");
    }
    ok &= findings.is_empty();
    println!("  {} broken, {} external (not fetched)", findings.len(), external.len());
    if args.external {
        for url in &external {
            println!("  external: {url}");
        }
    }

    println!("nd docs: prose");
    let mut prose = 0;
    for page in &site.pages {
        for problem in nd_docs::prose::check_page(page) {
            println!("  {problem}");
            prose += 1;
        }
    }
    for file in templates(repo)? {
        let text = std::fs::read_to_string(&file)?;
        for (line, problem) in nd_docs::prose::check_text(&text) {
            println!("  {}:{line}: {problem}", file.strip_prefix(repo).unwrap_or(&file).display());
            prose += 1;
        }
    }
    ok &= prose == 0;

    println!("nd docs: captures");
    let report = captures::report(repo, &site.examples)?;
    for os in Os::ALL {
        let of_os: Vec<_> = report.iter().filter(|(_, o, _)| *o == os).collect();
        let fresh = of_os.iter().filter(|(_, _, s)| matches!(s, State::Fresh(_))).count();
        let stale: Vec<&str> =
            of_os.iter().filter(|(_, _, s)| matches!(s, State::Stale { .. })).map(|(n, _, _)| n.as_str()).collect();
        let missing: Vec<&str> =
            of_os.iter().filter(|(_, _, s)| matches!(s, State::Missing)).map(|(n, _, _)| n.as_str()).collect();
        println!("  {}: {fresh}/{} fresh", os.label(), of_os.len());
        if !stale.is_empty() {
            println!("    stale: {}", stale.join(", "));
        }
        if !missing.is_empty() {
            println!("    missing: {}", missing.join(", "));
        }
        if args.require.contains(&os) && fresh < of_os.len() {
            println!("  {} captures are required: run `nd docs capture --stale` on {}", os.label(), os.label());
            ok = false;
        }
    }

    if args.compile {
        ok &= compile(repo, &site.examples)?;
    }
    if args.web {
        ok &= web_embeds(repo, &site)?;
    }
    println!("nd docs: {}", if ok { "ok" } else { "FAILED" });
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// The markdown `nd new` writes: its README, CLAUDE.md and agent docs.
fn templates(repo: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name.ends_with(".md") || name.ends_with(".md.tmpl") {
                    out.push(path);
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let dir = repo.join("crates/nd-cli/templates");
    if dir.is_dir() {
        walk(&dir, &mut out)?;
    }
    out.push(repo.join("docs/WRITING.md"));
    out.sort();
    Ok(out)
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn feature_flags(examples: &[&Example]) -> Vec<String> {
    let features: Vec<String> =
        examples.iter().flat_map(|e| e.features.iter().map(move |f| format!("{}/{f}", e.package))).collect();
    if features.is_empty() { Vec::new() } else { vec!["--features".into(), features.join(",")] }
}

fn compile(repo: &Path, examples: &[Example]) -> Result<bool> {
    let all: Vec<&Example> = examples.iter().collect();
    println!("nd docs: compile {} examples and the docs app", all.len());
    let mut native = cargo();
    native.current_dir(repo).args(["check", "--all-targets", "-p", APP_PACKAGE]);
    for e in &all {
        native.args(["-p", &e.package]);
    }
    native.args(feature_flags(&all));
    let mut ok = native.status().context("cargo check")?.success();
    let web: Vec<&Example> = examples.iter().filter(|e| e.web).collect();
    println!("nd docs: compile {} web examples and the docs app for {}", web.len(), web::WASM_TARGET);
    let mut wasm = cargo();
    wasm.current_dir(repo).args(["check", "--lib", "--bins", "--target", web::WASM_TARGET, "-p", APP_PACKAGE]);
    for e in &web {
        wasm.args(["-p", &e.package]);
    }
    wasm.args(feature_flags(&web));
    ok &= wasm.status().context("cargo check --target wasm32")?.success();
    ok &= std_clocks_in_web_crates(repo, &web, &feature_flags(&web))?;
    Ok(ok)
}

/// `std::time::Instant::now` and `SystemTime::now` compile for wasm32 and panic when the page runs.
/// Clippy with `clippy-web/clippy.toml` over every workspace crate the web examples and the docs
/// app link reports a direct use of std's types.
fn std_clocks_in_web_crates(repo: &Path, web: &[&Example], features: &[String]) -> Result<bool> {
    let mut tree = cargo();
    tree.current_dir(repo).args([
        "tree",
        "--target",
        web::WASM_TARGET,
        "-e",
        "normal",
        "--prefix",
        "none",
        "-p",
        APP_PACKAGE,
    ]);
    for e in web {
        tree.args(["-p", &e.package]);
    }
    tree.args(features);
    let output = tree.output().context("cargo tree")?;
    if !output.status.success() {
        bail!("cargo tree failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let root = repo.to_string_lossy().into_owned();
    let mut packages: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.contains(&format!("({root}")) && !l.contains("proc-macro") && !l.contains("/vendor/"))
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect();
    packages.sort();
    packages.dedup();
    println!("nd docs: std clocks in the {} crates the web build links", packages.len());
    let mut clippy = cargo();
    clippy
        .current_dir(repo)
        .env("CLIPPY_CONF_DIR", repo.join("clippy-web"))
        .args(["clippy", "--lib", "--bins", "--no-deps", "--target", web::WASM_TARGET])
        .args(features);
    for package in &packages {
        clippy.args(["-p", package]);
    }
    clippy.args(["--", "-A", "clippy::all", "-D", "clippy::disallowed_types"]);
    Ok(clippy.status().context("cargo clippy --target wasm32")?.success())
}

fn chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ND_CHROME") {
        return Some(path.into());
    }
    let mac = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if mac.exists() {
        return Some(mac.into());
    }
    ["google-chrome", "chromium"]
        .iter()
        .map(PathBuf::from)
        .find(|p| system_command(p).arg("--version").stdout(Stdio::null()).status().is_ok())
}

/// A command for a binary the system installed. The dev shell's `LD_LIBRARY_PATH` points at nix
/// store libraries built against a newer glibc than the system's, which a system binary cannot load.
fn system_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.env_remove("LD_LIBRARY_PATH");
    command
}

/// Headless Chrome with the flags every web check shares.
fn chrome_command(work: &Path, name: &str) -> Result<Command> {
    let chrome = chrome().context("no Chrome: set ND_CHROME")?;
    let mut command = system_command(chrome);
    command
        .args([
            "--headless=new",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--enable-unsafe-webgpu",
            "--window-size=1100,760",
        ])
        .arg(format!("--user-data-dir={}", work.join(format!("profile-{name}")).display()));
    // Chrome 137 and later no longer fall back to SwiftShader by themselves, so a runner without a
    // GPU draws no canvas unless software GL is asked for. Linux Chrome also keeps Vulkan off,
    // and without it a WebGPU canvas gets no shared image to present through ("Could not find
    // SharedImageBackingFactory"), so every frame is dropped and the page stays its background.
    if cfg!(target_os = "linux") {
        command.args([
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--enable-features=Vulkan",
            "--use-vulkan=swiftshader",
        ]);
    }
    Ok(command)
}

const PROBE_PAGE: &str = r#"<!doctype html><meta charset="utf-8"><pre id="out">probe did not run</pre><script>
(async () => {
  const lines = [];
  lines.push('navigator.gpu: ' + (navigator.gpu ? 'present' : 'absent'));
  if (navigator.gpu) {
    try {
      const adapter = await navigator.gpu.requestAdapter();
      lines.push('webgpu adapter: ' + (adapter ? JSON.stringify(adapter.info ? { vendor: adapter.info.vendor, architecture: adapter.info.architecture, description: adapter.info.description } : 'no info') : 'null'));
    } catch (e) { lines.push('webgpu adapter error: ' + e); }
  }
  const gl = document.createElement('canvas').getContext('webgl2');
  if (!gl) lines.push('webgl2: unavailable');
  else {
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    lines.push('webgl2 renderer: ' + (ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER)));
  }
  document.getElementById('out').textContent = lines.join('\n');
})();
</script>"#;

/// Prints why a page drew nothing: what the browser offers for graphics, and what the page logged.
fn explain_blank(dir: &Path, url: &str, work: &Path, name: &str) {
    let probe = dir.join("nd-probe.html");
    if std::fs::write(&probe, PROBE_PAGE).is_ok()
        && let Ok(mut command) = chrome_command(work, &format!("{name}-probe"))
        && let Ok(output) = command.args(["--dump-dom", &format!("{url}nd-probe.html")]).output()
    {
        let dom = String::from_utf8_lossy(&output.stdout);
        let text = dom.split("<pre id=\"out\">").nth(1).and_then(|t| t.split("</pre>").next()).unwrap_or("no output");
        for line in text.lines() {
            println!("    {line}");
        }
        let _ = std::fs::remove_file(&probe);
    }
    if let Ok(mut command) = chrome_command(work, &format!("{name}-log"))
        && let Ok(output) = command
            .args(["--virtual-time-budget=15000", "--enable-logging=stderr", "--v=0", "--dump-dom", url])
            .output()
    {
        let log = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = log.lines().filter(|l| l.contains("CONSOLE") || l.contains("Uncaught")).collect();
        println!("    page console: {} line(s)", lines.len());
        for line in lines.iter().take(20) {
            println!("    {}", line.chars().take(400).collect::<String>());
        }
    }
}

/// Loads `url` in headless Chrome and returns the distinct colours (capped) of its viewport once
/// it has drawn: screenshots every half second until the page shows more than the flat
/// background or 30 s pass.
fn colours_at(url: &str, work: &Path, name: &str) -> Result<usize> {
    let mut page = crate::cdp::Page::open(chrome_command(work, name)?, url)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let image = image::load_from_memory(&page.screenshot()?)?.to_rgba8();
        let mut seen = std::collections::HashSet::new();
        for pixel in image.pixels() {
            seen.insert(pixel.0);
            if seen.len() > 64 {
                break;
            }
        }
        if seen.len() > 4 || Instant::now() > deadline {
            return Ok(seen.len());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn web_embeds(repo: &Path, site: &nd_docs::Site) -> Result<bool> {
    let work = repo.join("target/nd/docs-check");
    // nd owns this folder; a run killed mid-build (or restored from a CI cache) leaves a bundle
    // without index.html, which build_web would refuse to clear forever.
    crate::project::fresh_dir(&work)?;
    let mut ok = true;
    let site_dir = build(repo, Some(work.join("site")), false)?;
    // Every fresh capture the app shows must be in the bundle it loads them from.
    let manifest = Manifest::load(repo)?;
    for example in &site.examples {
        for os in Os::ALL {
            if let State::Fresh(c) = manifest.state(example, os) {
                for file in [Some(&c.light), Some(&c.dark), c.recording.as_ref()].into_iter().flatten() {
                    let path = site_dir.join("app/assets/captures").join(&example.name).join(file);
                    if !path.is_file() {
                        println!("  missing from the bundle: {}", path.display());
                        ok = false;
                    }
                }
            }
        }
    }
    let mut targets = vec![("docs app".to_owned(), site_dir.join("app"))];
    for example in site.examples.iter().filter(|e| e.web) {
        let project = in_dir(repo, || WebProject::load(Some(&example.package)))?;
        let out = work.join("live").join(&example.name);
        web::build_web(&project, &out, false)?;
        targets.push((example.name.clone(), out));
    }
    for (name, dir) in targets {
        let (port, _server) = serve_background(&dir)?;
        let colours = colours_at(&format!("http://127.0.0.1:{port}/"), &work, &name.replace(' ', "-"))?;
        // A canvas that never drew is one flat colour: the page background.
        let loaded = colours > 4;
        println!("  {} {name} ({colours} colours)", if loaded { "ok  " } else { "FAIL" });
        if !loaded {
            explain_blank(&dir, &format!("http://127.0.0.1:{port}/"), &work, &name.replace(' ', "-"));
        }
        ok &= loaded;
    }
    Ok(ok)
}

/// Serves `dir` on a free port from a background thread for the life of the process.
fn serve_background(dir: &Path) -> Result<(u16, std::thread::JoinHandle<()>)> {
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| anyhow::anyhow!("binding: {e}"))?;
    let port = server.server_addr().to_ip().context("server address")?.port();
    let root = dir.to_path_buf();
    let handle = std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let relative = request.url().trim_start_matches('/').split(['?', '#']).next().unwrap_or("").to_owned();
            let path = root.join(if relative.is_empty() { "index.html" } else { &relative });
            let response = match std::fs::read(&path) {
                Ok(bytes) if !relative.contains("..") => {
                    let mut response = tiny_http::Response::from_data(bytes);
                    if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", web::content_type(&path)) {
                        response = response.with_header(header);
                    }
                    response
                }
                _ => tiny_http::Response::from_data(Vec::new()).with_status_code(404),
            };
            let _ = request.respond(response);
        }
    });
    Ok((port, handle))
}

#[cfg(not(feature = "capture"))]
fn capture(_repo: &Path, _args: &CaptureArgs) -> Result<ExitCode> {
    bail!("this nd was built without the `capture` feature")
}

#[cfg(feature = "capture")]
use capture::capture;

/// Captures through the test harness: launch with automation on, wait for the window, capture.
#[cfg(feature = "capture")]
mod capture {
    use nd_docs::captures::Capture;

    use super::*;

    pub fn capture(repo: &Path, args: &CaptureArgs) -> Result<ExitCode> {
        let os = Os::current().context("captures run on macOS, Linux or Windows")?;
        let site_examples = nd_docs::examples::load(repo, &nd_docs::framework_version(repo)?)?;
        let mut manifest = Manifest::load(repo)?;
        for name in &args.examples {
            if !site_examples.iter().any(|e| &e.name == name) {
                bail!("no example `{name}` in docs/examples.toml");
            }
        }
        let chosen: Vec<&Example> = site_examples
            .iter()
            .filter(|e| !e.headless)
            .filter(|e| args.examples.is_empty() || args.examples.contains(&e.name))
            .filter(|e| !args.stale || !matches!(manifest.state(e, os), State::Fresh(_)))
            .collect();
        if chosen.is_empty() {
            println!("nd docs capture: nothing to capture on {}", os.label());
            return Ok(ExitCode::SUCCESS);
        }
        let bins = build_bins(repo, &chosen)?;
        let _lock = ndshell::lock::acquire("nd docs capture").context("the machine-wide window lock")?;
        let session = session()?;
        let captured_on = os_version(os);
        let date = today();
        let work = repo.join("target/nd/docs-capture");
        let mut failed = Vec::new();
        for example in chosen {
            let bin = &bins[&example.name];
            let dir = repo.join(captures::DIR).join(&example.name);
            std::fs::create_dir_all(&dir)?;
            println!("nd docs capture: {} on {}", example.name, os.label());
            let result = (|| -> Result<Capture> {
                let mut recording = None;
                for scheme in ["light", "dark"] {
                    let file = dir.join(format!("{}-{scheme}.png", os.key()));
                    let motion = if scheme == "light" { example.motion.as_ref() } else { None };
                    let gif = dir.join(format!("{}.gif", os.key()));
                    let motion = motion.map(|m| (m, gif.as_path()));
                    let recorded = shoot(&session, bin, example, scheme, &work, &file, motion, args.settle)?;
                    if recorded {
                        recording = Some(format!("{}.gif", os.key()));
                    }
                    println!("  {}", file.strip_prefix(repo).unwrap_or(&file).display());
                }
                Ok(Capture {
                    hash: example.hash.clone(),
                    captured_on: captured_on.clone(),
                    date: date.clone(),
                    light: format!("{}-light.png", os.key()),
                    dark: format!("{}-dark.png", os.key()),
                    recording,
                })
            })();
            match result {
                Ok(capture) => {
                    manifest.set(&example.name, os, capture);
                    manifest.save(repo)?;
                }
                Err(e) => {
                    println!("  FAILED: {e:#}");
                    failed.push(example.name.clone());
                }
            }
        }
        if failed.is_empty() {
            Ok(ExitCode::SUCCESS)
        } else {
            println!("nd docs capture: failed: {}", failed.join(", "));
            Ok(ExitCode::FAILURE)
        }
    }

    /// Builds the examples' binaries in one cargo run and maps example name to binary.
    fn build_bins(repo: &Path, examples: &[&Example]) -> Result<std::collections::HashMap<String, PathBuf>> {
        let mut cmd = cargo();
        cmd.current_dir(repo).args(["build", "--bins", "--message-format=json-render-diagnostics"]);
        for e in examples {
            cmd.args(["-p", &e.package]);
        }
        cmd.args(feature_flags(examples)).stdout(Stdio::piped());
        let output = cmd.spawn().context("cargo build")?.wait_with_output()?;
        if !output.status.success() {
            bail!("cargo build of the examples failed ({})", output.status);
        }
        let mut bins = std::collections::HashMap::new();
        for line in output.stdout.split(|b| *b == b'\n') {
            let Ok(msg) = serde_json::from_slice::<serde_json::Value>(line) else { continue };
            let Some(exe) = msg["executable"].as_str() else { continue };
            let target = msg["target"]["name"].as_str().unwrap_or_default();
            let package_id = msg["package_id"].as_str().unwrap_or_default();
            for e in examples {
                let wanted = e.bin.as_deref().unwrap_or(&e.package);
                let ours = package_id.contains(&format!("/examples/{}#", e.name));
                if target == wanted && ours {
                    bins.insert(e.name.clone(), PathBuf::from(exe));
                }
            }
        }
        for e in examples {
            if !bins.contains_key(&e.name) {
                bail!("no binary `{}` built for example {}", e.bin.as_deref().unwrap_or(&e.package), e.name);
            }
        }
        Ok(bins)
    }

    /// One launch of `bin` in `scheme`: waits for its window, captures it into `png`, and when
    /// `motion` is given records it into the GIF as well. Returns whether a recording was written.
    #[allow(clippy::too_many_arguments)]
    fn shoot(
        session: &Session,
        bin: &Path,
        example: &Example,
        scheme: &str,
        work: &Path,
        png: &Path,
        motion: Option<(&nd_docs::examples::Motion, &Path)>,
        settle: f32,
    ) -> Result<bool> {
        let run = work.join(format!("{}-{scheme}", example.name));
        crate::project::fresh_dir(&run)?;
        let theme = run.join("theme.toml");
        std::fs::write(&theme, format!("appearance = \"{scheme}\"\n"))?;
        // Unix socket paths are capped near 104 bytes, which a worktree path can pass. On Windows
        // the app listens on a named pipe named after this path.
        let socket = std::env::temp_dir().join(format!("nd-docs-{}-{scheme}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut env: Vec<(String, String)> = example.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        env.extend([
            ("NATIVE_AUTOMATION".into(), "1".into()),
            ("ND_AUTOMATION_SOCKET".into(), socket.display().to_string()),
            ("ND_APP_DATA_DIR".into(), run.join("data").display().to_string()),
            ("ND_THEME_TOML".into(), theme.display().to_string()),
            ("ND_THEME_JSON".into(), String::new()),
        ]);
        let app = launch(session, bin, &env, &socket, Duration::from_secs(30))?;
        let result = (|| -> Result<bool> {
            let mut client = Rpc::connect(&app.socket)?;
            let deadline = Instant::now() + Duration::from_secs(20);
            let window = loop {
                let windows = client.call("windows", serde_json::json!({}))?;
                let visible = windows.as_array().into_iter().flatten().find(|w| w["visible"] == true).cloned();
                if let Some(w) = visible {
                    break w;
                }
                if Instant::now() > deadline {
                    bail!("no visible window after 20 s");
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            std::thread::sleep(Duration::from_secs_f32(settle));
            let window_id = window["id"].as_u64();
            let recorded = composited(session, app.pid, png, &mut client, window_id, motion)?;
            Ok(recorded)
        })();
        stop(session, app);
        let _ = std::fs::remove_file(&socket);
        result
    }

    /// A launched example and the automation socket it reported.
    struct Launched {
        pid: u32,
        socket: PathBuf,
        #[cfg(unix)]
        child: std::process::Child,
    }

    /// The socket path in the automation server's ready line, if `line` is one.
    fn ready_path(line: &str) -> Option<PathBuf> {
        line.split("ND_AUTOMATION_LISTENING path=").nth(1).map(|path| PathBuf::from(path.trim()))
    }

    /// Linux captures run in a private headless Hyprland, so they draw nothing on the user's session,
    /// work over ssh, and get the same output and scale on every run.
    #[cfg(target_os = "linux")]
    type Session = ndshell::hyprland::Hyprland;
    #[cfg(target_os = "macos")]
    struct Session;
    /// Windows: processes ssh starts have no desktop to show a window on, so `ndshell agent`, which
    /// runs in the logged-in user's session, launches the examples and captures their windows.
    #[cfg(windows)]
    struct Session(std::cell::RefCell<ndshell::agent::Client>);

    #[cfg(windows)]
    impl Session {
        fn call(&self, request: serde_json::Value) -> Result<serde_json::Value> {
            self.0.borrow_mut().call(request).map_err(|e| anyhow::anyhow!("ndshell agent: {e}"))
        }
    }

    #[cfg(target_os = "linux")]
    fn session() -> Result<Session> {
        ndshell::hyprland::Hyprland::start(Default::default()).map_err(|e| anyhow::anyhow!("headless Hyprland: {e}"))
    }

    #[cfg(target_os = "macos")]
    fn session() -> Result<Session> {
        Ok(Session)
    }

    #[cfg(windows)]
    fn session() -> Result<Session> {
        let client = ndshell::agent::Client::connect().map_err(anyhow::Error::msg)?;
        Ok(Session(std::cell::RefCell::new(client)))
    }

    /// Starts `bin` and waits for its automation server's ready line on stderr, which a thread
    /// keeps draining afterwards so the app never blocks on a full pipe.
    #[cfg(unix)]
    fn launch(
        session: &Session,
        bin: &Path,
        env: &[(String, String)],
        socket: &Path,
        timeout: Duration,
    ) -> Result<Launched> {
        use std::io::{BufRead, BufReader};
        #[cfg(target_os = "linux")]
        let mut cmd = session.command(bin);
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let _ = session;
            Command::new(bin)
        };
        cmd.envs(env.iter().map(|(k, v)| (k, v))).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| format!("launching {}", bin.display()))?;
        let stderr = child.stderr.take().context("stderr")?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !sent && let Some(path) = ready_path(&line) {
                    let _ = send.send(path);
                    sent = true;
                }
            }
        });
        let socket = match receive.recv_timeout(timeout) {
            Ok(path) => path,
            Err(_) if socket.exists() => socket.to_path_buf(),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("the app never reported its automation socket");
            }
        };
        Ok(Launched { pid: child.id(), socket, child })
    }

    #[cfg(unix)]
    fn stop(_session: &Session, mut app: Launched) {
        let _ = app.child.kill();
        let _ = app.child.wait();
    }

    /// Starts `bin` through the agent and polls the output it keeps for the ready line.
    #[cfg(windows)]
    fn launch(
        session: &Session,
        bin: &Path,
        env: &[(String, String)],
        _socket: &Path,
        timeout: Duration,
    ) -> Result<Launched> {
        let env: serde_json::Map<String, serde_json::Value> =
            env.iter().map(|(k, v)| (k.clone(), serde_json::Value::from(v.as_str()))).collect();
        let launched =
            session.call(serde_json::json!({ "cmd": "launch", "exe": bin.display().to_string(), "env": env }))?;
        let pid = launched["pid"].as_u64().context("the agent reported no pid")? as u32;
        let deadline = Instant::now() + timeout;
        loop {
            let output = session.call(serde_json::json!({ "cmd": "output", "pid": pid }))?;
            let lines: Vec<&str> =
                output["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).collect();
            if let Some(socket) = lines.iter().find_map(|l| ready_path(l)) {
                return Ok(Launched { pid, socket });
            }
            if !output["exited"].is_null() || Instant::now() > deadline {
                let _ = session.call(serde_json::json!({ "cmd": "kill", "pid": pid }));
                let tail = lines[lines.len().saturating_sub(20)..].join("\n");
                bail!("the app never reported its automation socket (exit {}):\n{tail}", output["exited"]);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[cfg(windows)]
    fn stop(session: &Session, app: Launched) {
        let _ = session.call(serde_json::json!({ "cmd": "kill", "pid": app.pid }));
    }

    #[cfg(target_os = "macos")]
    fn composited(
        _session: &Session,
        pid: u32,
        png: &Path,
        client: &mut Rpc,
        _window: Option<u64>,
        motion: Option<(&nd_docs::examples::Motion, &Path)>,
    ) -> Result<bool> {
        // An app launched in the background is not active, and AppKit draws an inactive window's
        // controls and selection differently.
        let deadline = Instant::now() + Duration::from_secs(5);
        let info = loop {
            ndshell::input::activate(pid);
            std::thread::sleep(Duration::from_millis(300));
            if let Some(info) = ndshell::window_list::of_pid(pid).into_iter().next() {
                break info;
            }
            if Instant::now() > deadline {
                bail!("the window server lists no window for pid {pid}");
            }
        };
        std::thread::sleep(Duration::from_millis(400));
        ndshell::window_list::capture(info.number, png).map_err(anyhow::Error::msg)?;
        let Some((motion, gif)) = motion else { return Ok(false) };
        // Frames of the window alone (`screencapture -l`), so nothing else on screen, such as
        // a system alert, ends up in the recording. Wheel steps post from their own thread while
        // this one captures; automation steps run between frames.
        if gif.exists() {
            std::fs::remove_file(gif)?;
        }
        let frames = png.with_extension("frames");
        crate::project::fresh_dir(&frames)?;
        let end = Instant::now() + Duration::from_secs_f32(motion.seconds);
        let wheel: Vec<i32> = motion
            .steps
            .iter()
            .filter(|s| s.method == "wheel")
            .map(|s| s.params["dy"].as_i64().unwrap_or(-120) as i32)
            .collect();
        let (x, y, w, h) = info.bounds;
        let interval = Duration::from_millis(motion.interval_ms);
        let poster = std::thread::spawn(move || {
            let mut steps = wheel.iter().cycle();
            while Instant::now() < end {
                if let Some(dy) = steps.next() {
                    ndshell::input::scroll((x + w / 2., y + h / 2.), *dy, 4);
                }
                std::thread::sleep(interval);
            }
        });
        let mut calls = motion.steps.iter().filter(|s| s.method != "wheel").cycle();
        let has_calls = motion.steps.iter().any(|s| s.method != "wheel");
        let started = Instant::now();
        let mut count = 0u32;
        while Instant::now() < end {
            if has_calls && let Some(step) = calls.next() {
                client.call(&step.method, step.params.clone())?;
            }
            ndshell::window_list::capture(info.number, &frames.join(format!("{count:04}.png")))
                .map_err(anyhow::Error::msg)?;
            count += 1;
        }
        let _ = poster.join();
        let fps = f64::from(count) / started.elapsed().as_secs_f64();
        let made = to_gif(&frames, fps, gif);
        std::fs::remove_dir_all(&frames)?;
        made
    }

    /// The window's rectangle cut from a frame of the headless output. GPUI draws the whole window,
    /// chrome included, so the client area is the picture; the app's own `screenshot` needs GPUI's
    /// test-support readback, which only headless GPUI windows have.
    #[cfg(target_os = "linux")]
    fn composited(
        session: &Session,
        pid: u32,
        png: &Path,
        _client: &mut Rpc,
        _window: Option<u64>,
        _motion: Option<(&nd_docs::examples::Motion, &Path)>,
    ) -> Result<bool> {
        let window = session.wait_for_client(pid, Duration::from_secs(5)).map_err(anyhow::Error::msg)?;
        // A frame or two for the window to present at its settled size.
        std::thread::sleep(Duration::from_millis(400));
        let frame = ndshell::wayland::screenshot(session).map_err(anyhow::Error::msg)?;
        let (x, y) = (window.at.0.max(0) as u32, window.at.1.max(0) as u32);
        let width = (window.size.0.max(0) as u32).min(frame.width.saturating_sub(x));
        let height = (window.size.1.max(0) as u32).min(frame.height.saturating_sub(y));
        let image = image::RgbImage::from_fn(width, height, |dx, dy| image::Rgb(frame.rgb(x + dx, y + dy)));
        image.save(png).with_context(|| format!("writing {}", png.display()))?;
        Ok(false)
    }

    /// The window as DWM composes it (Windows.Graphics.Capture through the agent), frame
    /// included, so the Mica backdrop and the title bar are in the picture.
    #[cfg(windows)]
    fn composited(
        session: &Session,
        pid: u32,
        png: &Path,
        _client: &mut Rpc,
        _window: Option<u64>,
        _motion: Option<(&nd_docs::examples::Motion, &Path)>,
    ) -> Result<bool> {
        let mut info = session.call(serde_json::json!({ "cmd": "window", "pid": pid, "timeout_ms": 5000 }))?;
        let hwnd = info["hwnd"].as_i64().context("the agent reported no window handle")?;
        // Windows keeps a process the agent starts out of the foreground, and an inactive window
        // draws its title bar and backdrop differently. A click on the caption activates it
        // without pressing a control.
        if info["foreground"] != true {
            let frame: Vec<i64> = info["frame"].as_array().into_iter().flatten().filter_map(|v| v.as_i64()).collect();
            if let [x, y, w, _] = frame[..] {
                let caption = caption_points(x, y, w).into_iter().find(|at| {
                    let hit = session.call(serde_json::json!({ "cmd": "hit_test", "hwnd": hwnd, "at": at }));
                    hit.is_ok_and(|h| h["code"] == HTCAPTION)
                });
                if let Some(at) = caption {
                    session.call(serde_json::json!({ "cmd": "click", "at": at }))?;
                    std::thread::sleep(Duration::from_millis(300));
                    info = session.call(serde_json::json!({ "cmd": "window_info", "hwnd": hwnd }))?;
                }
            }
        }
        if info["foreground"] != true {
            println!("  the window is not in the foreground: it is captured with its inactive look");
        }
        std::thread::sleep(Duration::from_millis(400));
        session.call(serde_json::json!({ "cmd": "capture", "hwnd": hwnd, "out": png.display().to_string() }))?;
        Ok(false)
    }

    /// `WM_NCHITTEST`'s answer for the caption.
    #[cfg(windows)]
    const HTCAPTION: i64 = 2;

    /// Points along the top of a window frame at (`x`, `y`), `w` wide, where an empty stretch of
    /// caption is likeliest: rows below the resize border, from the middle outwards.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn caption_points(x: i64, y: i64, w: i64) -> Vec<[i64; 2]> {
        let mut points = Vec::new();
        for dy in [12, 20, 6, 28] {
            for step in 0..8 {
                let offset = (step + 1) / 2 * w / 16 * if step % 2 == 0 { 1 } else { -1 };
                points.push([x + w / 2 + offset, y + dy]);
            }
        }
        points
    }

    #[cfg(target_os = "macos")]
    fn to_gif(frames: &Path, fps: f64, gif: &Path) -> Result<bool> {
        let ffmpeg = Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status();
        if !ffmpeg.is_ok_and(|s| s.success()) {
            println!("  no ffmpeg on PATH: recording skipped");
            return Ok(false);
        }
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-framerate", &format!("{fps:.2}"), "-i"])
            .arg(frames.join("%04d.png"))
            .args(["-vf", "scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse"])
            .arg(gif)
            .status()
            .context("ffmpeg")?;
        Ok(status.success() && gif.is_file())
    }

    /// A blocking client for the app's automation socket.
    struct Rpc {
        stream: nd_automation::protocol::Stream,
        next: u64,
    }

    impl Rpc {
        fn connect(path: &Path) -> Result<Self> {
            let stream =
                nd_automation::protocol::connect(path).with_context(|| format!("connecting to {}", path.display()))?;
            #[cfg(unix)]
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            Ok(Self { stream, next: 1 })
        }

        fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
            use nd_automation::protocol::{parse_response, read_frame, request, write_frame};
            let id = self.next;
            self.next += 1;
            write_frame(&mut self.stream, &request(id, method, params))?;
            loop {
                let frame = read_frame(&mut self.stream)?.context("the app closed the automation socket")?;
                let (reply, result) = parse_response(frame)?;
                if reply.as_u64() == Some(id) {
                    return result.map_err(|e| anyhow::anyhow!("{method}: {e}"));
                }
            }
        }
    }

    fn os_version(os: Os) -> String {
        let run = |cmd: &str, args: &[&str]| {
            Command::new(cmd)
                .args(args)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        match os {
            Os::Macos => {
                format!("macOS {}", run("sw_vers", &["-productVersion"]).unwrap_or_default()).trim().to_owned()
            }
            Os::Linux => {
                let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
                let name = release
                    .lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_owned())
                    .unwrap_or_else(|| "Linux".into());
                format!("{name}, Hyprland")
            }
            Os::Windows => windows_version(
                &run("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion"]).unwrap_or_default(),
            ),
        }
    }

    /// "Windows 11 25H2" from `reg query` of the CurrentVersion key. The key's ProductName still
    /// says Windows 10 on Windows 11, so the major version comes from the build number.
    fn windows_version(reg: &str) -> String {
        let value = |name: &str| {
            reg.lines().find_map(|line| {
                let mut words = line.split_whitespace();
                (words.next() == Some(name)).then(|| words.nth(1)).flatten().map(str::to_owned)
            })
        };
        let major = match value("CurrentBuild").and_then(|b| b.parse::<u32>().ok()) {
            Some(build) if build >= 22_000 => "Windows 11",
            Some(_) => "Windows 10",
            None => return "Windows".into(),
        };
        match value("DisplayVersion") {
            Some(version) => format!("{major} {version}"),
            None => major.into(),
        }
    }

    /// Today's date in UTC as `YYYY-MM-DD` (Howard Hinnant's days-to-civil).
    fn today() -> String {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        civil(secs as i64 / 86_400)
    }

    fn civil(days: i64) -> String {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02}")
    }

    #[cfg(test)]
    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_740), "2026-10-14");
    }

    #[cfg(test)]
    #[test]
    fn ready_lines_name_the_socket() {
        let line = r"ND_AUTOMATION_LISTENING path=\\.\pipe\nd-automation-00ff ";
        assert_eq!(ready_path(line), Some(PathBuf::from(r"\\.\pipe\nd-automation-00ff")));
        assert_eq!(ready_path("[nd] window opened"), None);
    }

    #[cfg(test)]
    #[test]
    fn caption_points_start_mid_window_and_stay_inside_it() {
        let points = caption_points(100, 50, 800);
        assert_eq!(points[0], [500, 62]);
        assert_eq!(points[1], [450, 62]);
        assert_eq!(points[2], [550, 62]);
        assert!(points.iter().all(|[x, y]| (100..900).contains(x) && (50..80).contains(y)));
    }

    #[cfg(test)]
    #[test]
    fn windows_versions_come_from_the_build_number() {
        let reg = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\r\n    \
                   CurrentBuild    REG_SZ    26200\r\n    DisplayVersion    REG_SZ    25H2\r\n    \
                   ProductName    REG_SZ    Windows 10 Pro\r\n";
        assert_eq!(windows_version(reg), "Windows 11 25H2");
        assert_eq!(windows_version("    CurrentBuild    REG_SZ    19045\r\n"), "Windows 10");
        assert_eq!(windows_version(""), "Windows");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_package_qualified() {
        let notes = Example { package: "notes".into(), features: vec!["m6".into()], ..Example::default() };
        let hello = Example { package: "hello".into(), ..Example::default() };
        assert_eq!(feature_flags(&[&notes, &hello]), ["--features", "notes/m6"]);
        assert!(feature_flags(&[&hello]).is_empty());
    }

    #[test]
    fn system_binaries_do_not_inherit_the_dev_shell_library_path() {
        let command = system_command("google-chrome");
        let removed = command.get_envs().find(|(key, _)| *key == "LD_LIBRARY_PATH");
        assert_eq!(removed, Some((std::ffi::OsStr::new("LD_LIBRARY_PATH"), None)));
    }
}
