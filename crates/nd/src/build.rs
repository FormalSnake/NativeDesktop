//! Build-script helper for apps: `nd::build::app()` in the app's `build.rs`.
//!
//! On macOS it collects the Swift packages of the framework (`DEP_ND_SWIFT_*_SWIFT_PACKAGE`
//! metadata of the app's direct dependencies, see each crate's `links`) and of the app
//! (`[macos] swift_packages` in `nativedesktop.toml`), generates one aggregate SwiftPM package in
//! `OUT_DIR`, builds it into one static library and emits the link arguments for the app's
//! binaries. Every package must export a library product named like its package. Packages that
//! depend on the framework package take its path from the `ND_SWIFT_PATH` environment variable.
//! On every other target it does nothing.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PINNED: &str = include_str!("../../../swift-toolchain.toml");
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

pub fn app() {
    if let Err(message) = bake_updates() {
        println!("cargo::error=nd::build::app: {message}");
        std::process::exit(1);
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if let Err(message) = build() {
        println!("cargo::error=nd::build::app: {message}");
        std::process::exit(1);
    }
}

fn build() -> Result<(), String> {
    let manifest_dir = PathBuf::from(var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(var("OUT_DIR")?);
    println!("cargo::rerun-if-changed=build.rs");
    // A missing path counts as changed on every build, which would rerun the Swift build after
    // each edit; an app that adds the file later has to touch build.rs once.
    if manifest_dir.join("nativedesktop.toml").exists() {
        println!("cargo::rerun-if-changed=nativedesktop.toml");
    }

    let sdk = check_toolchain()?;
    let packages = collect_packages(&manifest_dir)?;
    if packages.is_empty() {
        return Err("no Swift packages found (no framework metadata, no [macos] swift_packages)".into());
    }
    for (_, path) in &packages {
        println!("cargo::rerun-if-changed={}", path.display());
    }

    let framework = packages.iter().find(|(name, _)| name == "NDSwift").map(|(_, path)| path.clone());
    let aggregate = out_dir.join("nd-swift/aggregate");
    write_aggregate(&aggregate, &packages)?;

    let configuration = if var("PROFILE")? == "release" { "release" } else { "debug" };
    let scratch = out_dir.join("nd-swift/scratch");
    let mut swift = system_command("/usr/bin/swift");
    swift
        .args(["build", "-c", configuration, "--product", "NDAggregate"])
        .arg("--package-path")
        .arg(&aggregate)
        .arg("--scratch-path")
        .arg(&scratch);
    if let Some(framework) = &framework {
        swift.env("ND_SWIFT_PATH", framework);
    }
    run(&mut swift, "swift build")?;

    let bin_path = output(
        "/usr/bin/swift",
        &[
            "build",
            "-c",
            configuration,
            "--show-bin-path",
            "--package-path",
            aggregate.to_str().ok_or("non-UTF-8 path")?,
            "--scratch-path",
            scratch.to_str().ok_or("non-UTF-8 path")?,
        ],
    )?;
    let lib = PathBuf::from(bin_path).join("libNDAggregate.a");
    if !lib.exists() {
        return Err(format!("swift build produced no {}", lib.display()));
    }

    println!("cargo::rustc-link-arg=-Wl,-force_load,{}", lib.display());
    for dir in swift_library_dirs(&sdk)? {
        println!("cargo::rustc-link-arg=-L{}", dir.display());
    }
    println!("cargo::rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    for symbol in exported_symbols(&lib)? {
        println!("cargo::rustc-link-arg=-Wl,-u,{symbol}");
    }
    Ok(())
}

/// Bakes `[updates] feed` and `public_key` into the app as `ND_UPDATE_FEED` and
/// `ND_UPDATE_PUBLIC_KEY`, read back by `nd::manifest!()`. Skipped for wasm and when
/// `ND_NO_UPDATES=1` (Nix builds, where the store owns the files).
fn bake_updates() -> Result<(), String> {
    println!("cargo::rerun-if-env-changed=ND_NO_UPDATES");
    let config = PathBuf::from(var("CARGO_MANIFEST_DIR")?).join("nativedesktop.toml");
    if env::var("ND_NO_UPDATES").is_ok_and(|v| v == "1") || env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32")
    {
        return Ok(());
    }
    let Ok(text) = fs::read_to_string(&config) else { return Ok(()) };
    println!("cargo::rerun-if-changed=nativedesktop.toml");
    let table: toml::Table = text.parse().map_err(|e| format!("{}: {e}", config.display()))?;
    let updates = table.get("updates");
    let get = |key: &str| updates.and_then(|u| u.get(key)).and_then(|v| v.as_str());
    if let (Some(feed), Some(key)) = (get("feed"), get("public_key")) {
        println!("cargo::rustc-env=ND_UPDATE_FEED={feed}");
        println!("cargo::rustc-env=ND_UPDATE_PUBLIC_KEY={key}");
    }
    Ok(())
}

fn var(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is not set; call this from a build script"))
}

fn system_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env_remove("SDKROOT").env_remove("DEVELOPER_DIR").env("PATH", SYSTEM_PATH);
    command
}

fn run(command: &mut Command, what: &str) -> Result<(), String> {
    let status = command.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = system_command(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn major(version: &str) -> Option<u32> {
    version.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty())?.parse().ok()
}

/// Checks the installed Swift and SDK against `swift-toolchain.toml` and returns the SDK path.
fn check_toolchain() -> Result<PathBuf, String> {
    let pinned: toml::Table = PINNED.parse().map_err(|e| format!("swift-toolchain.toml: {e}"))?;
    let want = |key: &str| pinned.get(key).and_then(|v| v.as_integer()).map(|v| v as u32).unwrap_or(0);

    let swift = output("/usr/bin/swift", &["--version"])?;
    let swift_major = swift
        .split("Swift version ")
        .nth(1)
        .and_then(major)
        .ok_or_else(|| format!("cannot read the Swift version from `{swift}`"))?;
    if swift_major < want("swift_major") {
        return Err(format!("Swift {swift_major} is installed, swift-toolchain.toml needs {}", want("swift_major")));
    }

    let sdk_version = output("/usr/bin/xcrun", &["--sdk", "macosx", "--show-sdk-version"])?;
    let sdk_major = major(&sdk_version).ok_or_else(|| format!("cannot read the SDK version `{sdk_version}`"))?;
    if sdk_major < want("sdk_major") {
        return Err(format!("macOS SDK {sdk_major} is installed, swift-toolchain.toml needs {}", want("sdk_major")));
    }
    Ok(PathBuf::from(output("/usr/bin/xcrun", &["--sdk", "macosx", "--show-sdk-path"])?))
}

/// `(package name, absolute path)`, framework packages first, deduplicated by path.
fn collect_packages(manifest_dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut paths = BTreeSet::new();
    for (key, value) in env::vars() {
        if key.starts_with("DEP_ND_SWIFT_") && key.ends_with("_SWIFT_PACKAGE") {
            paths.insert(PathBuf::from(value));
        }
    }
    let config = manifest_dir.join("nativedesktop.toml");
    if let Ok(text) = fs::read_to_string(&config) {
        let table: toml::Table = text.parse().map_err(|e| format!("{}: {e}", config.display()))?;
        let listed = table.get("macos").and_then(|m| m.get("swift_packages")).and_then(|v| v.as_array());
        for entry in listed.into_iter().flatten().filter_map(|v| v.as_str()) {
            paths.insert(manifest_dir.join(entry));
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let path = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
            let name = path.file_name().and_then(|n| n.to_str()).ok_or("package path has no name")?.to_owned();
            Ok((name, path))
        })
        .collect()
}

fn write_aggregate(dir: &Path, packages: &[(String, PathBuf)]) -> Result<(), String> {
    let sources = dir.join("Sources/NDAggregate");
    fs::create_dir_all(&sources).map_err(|e| e.to_string())?;
    let dependencies: String =
        packages.iter().map(|(_, p)| format!("        .package(path: \"{}\"),\n", p.display())).collect();
    let products: String = packages
        .iter()
        .map(|(name, _)| {
            let identity = name.to_lowercase();
            format!("            .product(name: \"{name}\", package: \"{identity}\"),\n")
        })
        .collect();
    let manifest = format!(
        "// swift-tools-version: 6.0\n\
         import PackageDescription\n\n\
         let package = Package(\n    name: \"NDAggregate\",\n    platforms: [.macOS(.v14)],\n    \
         products: [.library(name: \"NDAggregate\", type: .static, targets: [\"NDAggregate\"])],\n    \
         dependencies: [\n{dependencies}    ],\n    \
         targets: [.target(name: \"NDAggregate\", dependencies: [\n{products}        ])]\n)\n"
    );
    write_if_changed(&dir.join("Package.swift"), &manifest)?;
    write_if_changed(&sources.join("Aggregate.swift"), "// Generated by nd::build::app().\n")
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Search paths for the libraries Swift objects autolink: the toolchain's runtime and
/// back-deployment archives, and the SDK's `usr/lib/swift` stubs.
fn swift_library_dirs(sdk: &Path) -> Result<Vec<PathBuf>, String> {
    let swiftc = PathBuf::from(output("/usr/bin/xcrun", &["--sdk", "macosx", "-f", "swiftc"])?);
    let toolchain = swiftc.parent().and_then(Path::parent).ok_or("unexpected swiftc location")?;
    let candidates = [toolchain.join("lib/swift/macosx"), sdk.join("usr/lib/swift")];
    Ok(candidates.into_iter().filter(|p| p.is_dir()).collect())
}

/// Global `nd_*` text symbols of the aggregate. `-u` keeps them through dead stripping even when
/// no Rust code names them (symbols exported by an app's own Swift package).
fn exported_symbols(lib: &Path) -> Result<Vec<String>, String> {
    let lib = lib.to_str().ok_or("non-UTF-8 library path")?;
    let listing = output("/usr/bin/nm", &["-g", "--defined-only", lib])?;
    let mut symbols = BTreeSet::new();
    for line in listing.lines() {
        let mut parts = line.split_whitespace().rev();
        if let (Some(name), Some("T")) = (parts.next(), parts.next())
            && name.starts_with("_nd_")
        {
            symbols.insert(name.to_owned());
        }
    }
    Ok(symbols.into_iter().collect())
}
