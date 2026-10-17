//! `nd docs`: the docs site's checks, build and captures (ARCHITECTURE.md 13).
//!
//! - `check`: every page loads (snippets, examples and directives resolve), the link check, the
//!   prose check over pages and `nd new` templates, and the capture report. `--compile` builds
//!   every registered example (and the web ones for wasm), `--web` loads the docs app and each
//!   web example in headless Chrome and fails on a blank canvas, `--require macos` fails on a
//!   missing or stale macOS capture.
//! - `build`: the static site (an HTML page and the raw markdown per page, `llms.txt`,
//!   `llms-full.txt`) with the docs app's web bundle under `app/`. `serve` builds and serves it.
//! - `capture`: runs each example through the test harness on this OS, light and dark, and
//!   records the captures with the example's source hash.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use nd_docs::captures::{self, Capture, Manifest, Os, State};
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
    Ok(out)
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
    Ok(ok)
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

/// Loads `url` in headless Chrome and returns the screenshot's distinct colours (capped).
fn colours_at(url: &str, work: &Path, name: &str) -> Result<usize> {
    let chrome = chrome().context("no Chrome: set ND_CHROME")?;
    let shot = work.join(format!("{name}.png"));
    let mut command = system_command(chrome);
    command
        .args([
            "--headless=new",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--enable-unsafe-webgpu",
            "--window-size=1100,760",
            "--virtual-time-budget=15000",
        ])
        .arg(format!("--user-data-dir={}", work.join(format!("profile-{name}")).display()))
        .arg(format!("--screenshot={}", shot.display()));
    // Chrome 137 and later no longer fall back to SwiftShader by themselves, so a runner without a
    // GPU draws no canvas unless software GL is asked for.
    if cfg!(target_os = "linux") {
        command.args(["--use-angle=swiftshader", "--enable-unsafe-swiftshader"]);
    }
    let output = command.arg(url).output().context("running Chrome")?;
    if !output.status.success() {
        bail!("Chrome exited with {}", output.status);
    }
    let image = image::open(&shot).with_context(|| format!("reading {}", shot.display()))?.to_rgba8();
    let mut seen = std::collections::HashSet::new();
    for pixel in image.pixels() {
        seen.insert(pixel.0);
        if seen.len() > 64 {
            break;
        }
    }
    Ok(seen.len())
}

fn web_embeds(repo: &Path, site: &nd_docs::Site) -> Result<bool> {
    let work = repo.join("target/nd/docs-check");
    std::fs::create_dir_all(&work)?;
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

#[cfg(not(unix))]
fn capture(_repo: &Path, _args: &CaptureArgs) -> Result<ExitCode> {
    bail!("`nd docs capture` talks to the example over a Unix socket; on Windows it is not written yet")
}

#[cfg(unix)]
use capture::capture;

/// Captures through the test harness: launch with automation on, wait for the window, capture.
#[cfg(unix)]
mod capture {
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
                    let recorded =
                        shoot(bin, example, scheme, &work, &file, motion.map(|m| (m, gif.as_path())), args.settle)?;
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
    fn shoot(
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
        // Unix socket paths are capped near 104 bytes, which a worktree path can pass.
        let socket = std::env::temp_dir().join(format!("nd-docs-{}-{scheme}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut cmd = Command::new(bin);
        cmd.envs(&example.env)
            .env("NATIVE_AUTOMATION", "1")
            .env("ND_AUTOMATION_SOCKET", &socket)
            .env("ND_APP_DATA_DIR", run.join("data"))
            .env("ND_THEME_TOML", &theme)
            .env("ND_THEME_JSON", "")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| format!("launching {}", bin.display()))?;
        let result = (|| -> Result<bool> {
            let stderr = child.stderr.take().context("stderr")?;
            let socket = wait_ready(stderr, &socket, Duration::from_secs(30))?;
            let mut client = Rpc::connect(&socket)?;
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
            let recorded = composited(child.id(), png, &mut client, window_id, motion)?;
            Ok(recorded)
        })();
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&socket);
        result
    }

    /// Waits for the automation server's ready line on `stderr`, then keeps draining it so the app
    /// never blocks on a full pipe.
    fn wait_ready(stderr: std::process::ChildStderr, socket: &Path, timeout: Duration) -> Result<PathBuf> {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !sent && let Some(path) = line.split("ND_AUTOMATION_LISTENING path=").nth(1) {
                    let _ = send.send(PathBuf::from(path.trim()));
                    sent = true;
                }
            }
        });
        match receive.recv_timeout(timeout) {
            Ok(path) => Ok(path),
            Err(_) if socket.exists() => Ok(socket.to_path_buf()),
            Err(_) => bail!("the app never reported its automation socket"),
        }
    }

    #[cfg(target_os = "macos")]
    fn composited(
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

    #[cfg(not(target_os = "macos"))]
    fn composited(
        _pid: u32,
        png: &Path,
        client: &mut Rpc,
        window: Option<u64>,
        _motion: Option<(&nd_docs::examples::Motion, &Path)>,
    ) -> Result<bool> {
        // GPUI draws the whole window, chrome included, so the backend's own capture is the picture.
        client.call("screenshot", serde_json::json!({ "path": png, "window": window }))?;
        Ok(false)
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
        stream: std::os::unix::net::UnixStream,
        next: u64,
    }

    impl Rpc {
        fn connect(path: &Path) -> Result<Self> {
            let stream = std::os::unix::net::UnixStream::connect(path)
                .with_context(|| format!("connecting to {}", path.display()))?;
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
        Os::Macos => format!("macOS {}", run("sw_vers", &["-productVersion"]).unwrap_or_default()).trim().to_owned(),
        Os::Linux => {
            let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            let name = release
                .lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
                .unwrap_or_else(|| "Linux".into());
            match std::env::var("XDG_CURRENT_DESKTOP") {
                Ok(desktop) if !desktop.is_empty() => format!("{name}, {desktop}"),
                _ => name,
            }
        }
        Os::Windows => "Windows".into(),
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
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_740), "2026-10-14");
    }

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
