//! Shared by the tier 2 tests: the `printf` the goldens show, golden comparison, and a
//! libtest-compatible runner for `harness = false` binaries.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nd_terminal::Command;

/// Colours, wide characters, Nerd Font icons, sprites and decorations, printed by `printf`.
pub fn printf_command() -> Command {
    let lines = [
        "\\033[31mred\\033[0m \\033[32mgreen\\033[0m \\033[1;34mbold blue\\033[0m \\033[38;5;208m208\\033[0m \\033[48;2;40;80;160;38;2;255;255;255m truecolor \\033[0m",
        "wide: 你好世界  nerd: \u{f115} \u{e0a0} main \u{f0e7} done",
        "box: ┌──┬──┐ ▀▄█ ░▒▓ \u{e0b0}\u{e0b2} ╭─╮",
        "     └──┴──┘ \\033[4munder\\033[0m \\033[3mitalic\\033[0m \\033[9mstrike\\033[0m \\033[4:3mcurly\\033[0m",
    ];
    Command::new("printf").arg(lines.join("\\n"))
}

pub fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

pub fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/goldens/{name}.png"))
}

/// Per channel, out of 255, and the share of pixels allowed past it: antialiasing drifts.
const CHANNEL_TOLERANCE: i16 = 24;
const PIXEL_TOLERANCE: f64 = 0.004;

/// Compares `image` with the golden `name`, recording it when missing or `ND_UPDATE_GOLDENS=1`.
/// A mismatch writes `<golden>.actual.png` beside it.
pub fn compare(name: &str, image: &image::RgbaImage) -> Result<(), String> {
    let path = golden_path(name);
    if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") || !path.exists() {
        std::fs::create_dir_all(path.parent().expect("dir")).map_err(|e| e.to_string())?;
        image.save(&path).map_err(|e| e.to_string())?;
        println!("recorded {}", path.display());
        return Ok(());
    }
    let expected = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
    let actual_path = path.with_extension("actual.png");
    if expected.dimensions() != image.dimensions() {
        let _ = image.save(&actual_path);
        return Err(format!("{}: size {:?}, golden {:?}", path.display(), image.dimensions(), expected.dimensions()));
    }
    let off = expected
        .pixels()
        .zip(image.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| (*x as i16 - *y as i16).abs() > CHANNEL_TOLERANCE))
        .count();
    let share = off as f64 / f64::from((image.width() * image.height()).max(1));
    if share > PIXEL_TOLERANCE {
        let _ = image.save(&actual_path);
        return Err(format!(
            "{}: {:.2}% of pixels differ (see {})",
            path.display(),
            share * 100.,
            actual_path.display()
        ));
    }
    let _ = std::fs::remove_file(&actual_path);
    println!("ok   {} ({:.3}% off)", path.display(), share * 100.);
    Ok(())
}

/// Runs `tests` the way libtest would for nextest: `--list`, filters, `--exact`, `--ignored`.
pub fn run(tests: &[(&str, fn())]) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, test) in tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
