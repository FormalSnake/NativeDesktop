//! `nd diag`: framework measurements against the budgets in ARCHITECTURE.md 9.5.
//!
//! `nd diag scroll --rows 100000` launches the `scroll-bench` example, which scrolls a
//! virtualized list of that many variable-height rows by a fixed step every display frame, and
//! reads the host's frame interval line back. It passes when no frame was missed beyond the
//! first second's warm-up allowance and p95 stays within a quarter period of the display's rate.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct DiagArgs {
    #[command(subcommand)]
    pub command: DiagCommand,
}

#[derive(Subcommand, Debug)]
pub enum DiagCommand {
    /// Scroll a virtualized list every frame and check the frame budget.
    Scroll(ScrollArgs),
}

#[derive(Args, Debug)]
pub struct ScrollArgs {
    /// Rows in the list.
    #[arg(long, default_value_t = 100_000)]
    pub rows: u32,
    /// How long to scroll, in seconds.
    #[arg(long, default_value_t = 10.)]
    pub seconds: f64,
    /// Points scrolled per display frame.
    #[arg(long, default_value_t = 40.)]
    pub step: f64,
    /// The `scroll-bench` binary; by default the one next to this `nd`, then `$ND_SCROLL_BIN`.
    #[arg(long, env = "ND_SCROLL_BIN")]
    pub bin: Option<PathBuf>,
}

/// One `nd-budget: frames ...` line.
#[derive(Debug, Default, PartialEq)]
pub struct Frames {
    pub n: u64,
    pub refresh_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub missed: u64,
}

impl Frames {
    pub fn parse(line: &str) -> Option<Self> {
        let rest = line.strip_prefix("nd-budget: frames ")?;
        let mut f = Frames::default();
        for pair in rest.split_whitespace() {
            let (k, v) = pair.split_once('=')?;
            match k {
                "n" => f.n = v.parse().ok()?,
                "refresh_ms" => f.refresh_ms = v.parse().ok()?,
                "p50_ms" => f.p50_ms = v.parse().ok()?,
                "p95_ms" => f.p95_ms = v.parse().ok()?,
                "p99_ms" => f.p99_ms = v.parse().ok()?,
                "max_ms" => f.max_ms = v.parse().ok()?,
                "missed" => f.missed = v.parse().ok()?,
                _ => {}
            }
        }
        Some(f)
    }

    /// Frames missed beyond the warm-up allowance (the first second at the display's rate may
    /// miss up to 2 while rows and glyphs are first built), and p95 within 1.25 periods.
    pub fn within_budget(&self) -> bool {
        self.missed <= 2 && self.p95_ms <= self.refresh_ms * 1.25
    }
}

fn bench_binary(args: &ScrollArgs) -> Result<PathBuf> {
    if let Some(bin) = &args.bin {
        return Ok(bin.clone());
    }
    let exe = std::env::current_exe().context("locating nd")?;
    let sibling = exe.with_file_name(format!("scroll-bench{}", std::env::consts::EXE_SUFFIX));
    if sibling.exists() {
        return Ok(sibling);
    }
    bail!("no scroll-bench next to {}; build it (`cargo build --release -p scroll-bench`) or pass --bin", exe.display())
}

pub fn run(args: DiagArgs) -> Result<ExitCode> {
    match args.command {
        DiagCommand::Scroll(args) => scroll(args),
    }
}

fn scroll(args: ScrollArgs) -> Result<ExitCode> {
    let bin = bench_binary(&args)?;
    // The frame line comes from the GPUI host's `ND_ANIMATE` budget mode; on macOS that is the
    // GPUI backend (a `scroll-bench` built with `--features gpui`).
    let mut child = Command::new(&bin)
        .env("ND_BACKEND", "gpui")
        .env("ND_ROWS", args.rows.to_string())
        .env("ND_SCROLL_STEP", args.step.to_string())
        .env("ND_ANIMATE", args.seconds.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", bin.display()))?;
    let stdout = child.stdout.take().expect("piped stdout");
    // A host without the budget mode never quits on its own: stop it well after the run's end.
    let pid = child.id();
    let deadline = std::time::Duration::from_secs_f64(args.seconds + 30.);
    std::thread::spawn(move || {
        std::thread::sleep(deadline);
        let _ = Command::new("kill").arg(pid.to_string()).status();
    });
    let mut frames = None;
    let mut passes = 0;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if let Some(f) = Frames::parse(&line) {
            println!("{line}");
            frames = Some(f);
        } else if line.starts_with("nd-scroll: reached_end") {
            passes += 1;
        }
    }
    let status = child.wait()?;
    let Some(frames) = frames else {
        bail!(
            "{} exited ({status}) without a frame line; on macOS build it with `--features gpui`, the AppKit host has no frame budget mode",
            bin.display()
        );
    };
    let ok = frames.within_budget();
    println!(
        "nd-diag scroll: rows={} step={} seconds={} reached_end={passes} {}",
        args.rows,
        args.step,
        args.seconds,
        if ok { "within budget" } else { "OVER BUDGET" }
    );
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_frame_line() {
        let f = Frames::parse(
            "nd-budget: frames n=600 refresh_ms=16.67 p50_ms=16.66 p95_ms=17.02 p99_ms=17.3 max_ms=19.8 missed=0",
        )
        .expect("frames");
        assert_eq!(f.n, 600);
        assert_eq!(f.missed, 0);
        assert!(f.within_budget());
        assert!(!Frames { missed: 3, ..f }.within_budget());
    }
}
