# Windows, packaging, installers, auto-update

Research for the Rust rewrite. Date: 2026-10-13. Every claim carries its source;
"(old repo)" means the Zig/Bun NativeDesktop on branch `zig` and its `nd-win-*`
worktrees. Estimates are marked as such.

## 0. What the old repo already learned on Windows

- The Windows backend was a .NET 10 WinUI 3 host (`windows/NDShell`, C#,
  NativeAOT) loading the Zig core as `nd.dll` over the same vtable
  (`nd-win-cef/docs/windows-backend.md:1-6`). So "WinUI from Rust" was never
  tried; WinUI was reached through C#.
- NativeAOT needed `Microsoft.WinUI` rooted because CsWinRT resolves objects by
  runtime class name, and the `.pri` resource index must be renamed with the
  exe (`nd-win-cef/docs/windows-backend.md:106-111`).
- CEF inside WinUI cost most of the Windows effort: a windowed CEF HWND draws
  over XAML (airspace), so every flyout over a page had to be cut out with
  `SetWindowRgn` (`docs/superpowers/specs/2026-10-08-cef-windows.md`, "Airspace");
  WinUI's content island was raised over the page and took every click
  (commit `572664f4`); WinUI's thread never handed the page windows their mouse
  input (`ac8c6543`) and mouse-in-pointer messages had to be turned back into
  mouse messages (`249f87f6`). The end state was off-screen rendering into a
  swap chain (`a50c68a6`). Lesson: any native-widget toolkit on Windows pays
  this tax for every embedded surface.
- Packaging shipped a zip plus `install.ps1` into `%LOCALAPPDATA%\Programs`,
  registered under HKCU, unsigned, because "an unsigned MSIX needs Developer
  Mode and sideloading" (`nd-win-cef/packages/nd/src/package/windows.ts:1-7`).
  Updates were a minisign-signed full archive plus a signed JSON manifest,
  mac and Linux only (`NativeDesktop/packages/nd/src/package/updates.ts:1-60`).
  No deltas, no channels, no Windows updates.

## 1. WinUI 3 from Rust vs GPUI's Windows backend

### WinUI 3 from Rust: `windows-reactor` (new, official, preview)

- Microsoft's `windows-rs` now ships `windows-reactor`, "a typed declarative UI
  library for building native WinUI 3 applications in Rust", with a React/Elm
  shaped `Component { create, update, view }` API
  ([readme](https://github.com/microsoft/windows-rs/blob/master/crates/libs/reactor/readme.md)).
- Status: "Reactor is currently a preview, so releases may still refine public
  APIs" ([docs/crates/windows-reactor.md:13](https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor.md)).
  crates.io: 0.100.0 published 2026-09-03, 3.5k downloads total
  ([crates.io API](https://crates.io/api/v1/crates/windows-reactor)). Needs
  `rust-version = "1.95"`, edition 2024 (`crates/libs/reactor/Cargo.toml`).
- Churn: launched in #4479, second preview #4848, third preview
  [#5000](https://github.com/microsoft/windows-rs/pull/5000) merged 2026-10-01
  as an internals rewrite (82.5k lines, 52.9k generated); a dozen follow-up PRs
  merged 2026-10-02..12 (#5004 to #5071). Internals are still moving weekly.
- Coverage is real: windows, notification icons and menus, keyed
  virtualization, WebView2, Canvas (Direct2D) and Composition interop, TitleBar,
  icons ([docs](https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor.md),
  lines 296-428).
- Runtime: unpackaged apps need the Windows App SDK framework package
  installed, or `windows-reactor-setup` in `build.rs` to stage it
  self-contained ([reactor-setup readme](https://github.com/microsoft/windows-rs/blob/master/crates/libs/reactor-setup/readme.md)).
  Microsoft says self-contained "significantly increases output size"
  ([Learn: unpackage WinUI app](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/unpackage-winui-app)).
- Older routes are dead: `windows-ui-xaml` 0.22.3 pins `windows` ^0.21
  ([docs.rs](https://docs.rs/windows-ui-xaml)); third-party `winrt-xaml` is
  incomplete ([docs.rs](https://docs.rs/winrt-xaml)). XAML islands from raw
  `windows` bindings is hand-rolled COM with no declarative layer.

Pain against our standards:
1. It is a third UI tree. AppKit + GPUI already means two backends; WinUI adds a
   third control set whose metrics, focus and virtualization differ, while the
   request says "all three platforms must look the same".
2. Embedded surfaces (CEF, video, our own GPU canvases) re-hit airspace exactly
   as in section 0, unless everything goes through Composition interop.
3. Agent testing: our input synthesis and frame recording would have to drive
   XAML through UIA plus a separate capture path; on GPUI we own the scene graph.
4. Preview API on a weekly-changing internals rewrite, plus a WinAppSDK runtime
   dependency for every app.

### GPUI on Windows (DirectX 11 + DirectComposition)

- Zed's Windows build is stable, rendered with DirectX 11 and DirectWrite
  ([Neowin](https://www.neowin.net/news/windows-1011-users-have-a-reason-to-rejoice-as-microsofts-rival-gains-platform-support/));
  Blade was dropped on Windows after ARM64 problems
  ([The Register](https://www.theregister.com/2025/08/22/everything_is_different_on_windows/)).
  Linux moved from Blade to wgpu
  ([zed commit af8ea0d](https://git.secluded.site/zed/commit/af8ea0d6c26192c45f44f473c0d4a7d6f72ed018)).
- The renderer presents through a DirectComposition device, target and visual
  (`crates/gpui_windows/src/directx_renderer.rs:119-121,1294`), which is what
  lets a CEF shared texture or a video surface be a sibling visual with no
  airspace.
- Mica: `MicaBackdrop` sets `DWMSBT_MAINWINDOW`, `MicaAltBackdrop` sets
  `DWMSBT_TABBEDWINDOW`; `Blurred` uses the undocumented
  `SetWindowCompositionAttribute` accent (`gpui_windows/src/window.rs:915-940`).
  Acrylic proper (`DWMSBT_TRANSIENTWINDOW`, [DWM_SYSTEMBACKDROP_TYPE](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type))
  is not wired; a one-line addition.
- Snap layouts: custom caption hit-testing returns `HTMAXBUTTON` for the app's
  `WindowControlArea::Max`, which is what makes Windows 11 show the snap flyout
  on a custom-drawn maximize button (`gpui_windows/src/events.rs:971,1083-1141`).
  So caption buttons stay app-drawn and themable, yet OS-native in behaviour.
- Manifest: `PerMonitorV2` DPI awareness and `SegmentHeap`
  (`crates/gpui/resources/windows/gpui.manifest.xml`); `WM_DPICHANGED` handled
  (`events.rs:106`).

Recommendation: GPUI on Windows. WinUI does not clear "fits our standards"
today (preview, third backend, airspace, test rig). Re-evaluate `windows-reactor`
once it leaves preview, as an optional Fluent control adapter, not a backend.

## 2. Windows input

| Need | GPUI today | Gap / plan |
|---|---|---|
| Precision touchpad pan + pinch | DirectManipulation viewport fed by `PT_TOUCHPAD` pointers, emits `ScrollWheel` and `Pinch` (`gpui_windows/src/direct_manipulation.rs:101,173-307`) | Good base. Rotate and 3/4-finger swipes are OS-reserved on Windows; standardise our gesture API on pan, pinch, momentum, and expose swipe only where the OS gives it. |
| Pen (pressure, tilt, eraser, barrel) | Only `WM_MOUSE*` handled (`events.rs:112-149`); no `WM_POINTER`/`PT_PEN` path, so pen arrives as promoted mouse without pressure | Add a `WM_POINTERUPDATE` path reading `POINTER_PEN_INFO` ([Learn: GetPointerPenInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfo)). Est. 2-3 days. |
| Touch screen | Not seen in the Windows sources | Same `WM_POINTER` path with `PT_TOUCH`. |
| Per-monitor DPI v2 | Manifest + `WM_DPICHANGED` (above) | Covered. Test drag across g815's two monitors at different scales. |
| 144/240 Hz, mixed monitors | One `VSyncProvider` per process: `DwmFlush`, interval from `DwmGetCompositionTimingInfo(HWND::default())`, sleep fallback (`gpui_windows/src/vsync.rs:25-75`) | One global clock. Windows 11 added per-monitor refresh handling for mixed setups ([PCWorld](https://www.pcworld.com/article/2013528/windows-11-will-soon-work-better-with-multiple-gaming-monitors.html)); historically DWM composed at the primary's rate ([Blur Busters](https://forums.blurbusters.com/viewtopic.php?p=96423)). Plan: per-window frame pacing from the window's monitor (DXGI frame-latency waitable swap chain or `DCompositionWaitForCompositorClock`, which GPUI's own comment names, `vsync.rs:45`), gated by a frame-time trace on g815 (240 + 144) before any claim. |

## 3. Installers and signing

- **cargo-packager** (CrabNebula, from tauri-bundler): `app`, `dmg`, `wix` (MSI),
  `nsis`, `deb`, `appimage`, `pacman`; config in `Packager.toml` or Cargo
  metadata ([Context7 /crabnebula-dev/cargo-packager](https://github.com/crabnebula-dev/cargo-packager));
  0.11.8, last release 2025-11-27 ([crates.io](https://crates.io/crates/cargo-packager)).
- **tauri-bundler**: MSI via WiX v3 or NSIS setup exe; MSI only builds on Windows
  ([Tauri: Windows installer](https://v2.tauri.app/distribute/windows-installer/)).
- **cargo-dist**: "we rely on the industry standard WiX v3 toolchain"; WiX v4 not
  supported; aimed at CLI binaries (`bin` subdir only)
  ([cargo-dist MSI](https://axodotdev.github.io/cargo-dist/book/installers/msi.html)).
- **WiX** is at v7 (2026-04-06) under the OSMF EULA; v7 waives the fee under
  US$10k/yr revenue, v6 required it for any revenue use
  ([WiX releases](https://github.com/wixtoolset/wix/releases),
  [What's new](https://docs.firegiant.com/wix/whatsnew/)). Every Rust bundler
  above still emits WiX v3, which is end-of-life.
- **MSIX**: must be signed to install outside Developer Mode (old repo finding,
  section 0). Good for Store, poor for a self-updating per-user app.
- **Velopack Setup.exe**: per-user install to `%LocalAppData%`, no elevation,
  `--splashImage`, `--shortcuts`, icon; optional MSI with
  `PerUser`/`PerMachine`/`Either` ([Velopack installer docs](https://github.com/velopack/velopack.docs/blob/master/docs/packaging/installer.mdx),
  [vpk windows options](https://github.com/velopack/velopack.docs/blob/master/docs/reference/cli/content/vpk-windows.mdx)).
- **Inno Setup**: what Zed ships (`crates/zed/resources/windows/zed.iss`); fully
  brandable, Pascal scripting, no updater.

Branded per-app installer: the brand lives in our project config (name, icon,
splash, publisher, accent colour), and the generator feeds it to Velopack's
`vpk pack` (Setup.exe) by default, MSI only on request. That avoids owning a
WiX/NSIS template, which is where tauri and cargo-dist are stuck on WiX v3.

**Code signing (Windows)**: Azure Artifact Signing (formerly Trusted Signing).
Basic tier 5,000 signatures/month, Premium 100,000; billed per full month
([pricing](https://azure.microsoft.com/pricing/details/artifact-signing/));
about $9.99/month Basic per third-party reports
([melatonin.dev](https://melatonin.dev/blog/code-signing-on-windows-with-azure-trusted-signing/)).
Individuals: US and Canada only; organizations: also EU, UK and others
([Quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart),
[Learn: code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)),
so it must go through CanaryCoders as an organization (Spain/EU). Zed signs
with it from `sign.ps1` (env `ENDPOINT`, `ACCOUNT_NAME`, `CERT_PROFILE_NAME`).
Velopack takes `--azureTrustedSignFile metadata.json` or a jsign
`--signTemplate` that runs on macOS/Linux CI
([Velopack signing](https://github.com/velopack/velopack.docs/blob/master/docs/packaging/signing.mdx)).

## 4. Auto-update

| | Platforms | Delta | Signature | Channels | Notes |
|---|---|---|---|---|---|
| Velopack (`velopack` 1.2.161, 2026-09-29) | Win, mac, Linux (AppImage only) | Yes, falls back to full on failure | SHA1/SHA256 from the feed only (`src/lib-rust/src/manager.rs:55-62`); no publisher key over the feed | Yes, `--channel`, `ExplicitChannel` (`manager.rs:128-138`) | Rust core, installer + updater + signing in one CLI ([docs.rs](https://docs.rs/velopack), [UpdateManager](https://github.com/velopack/velopack.docs/blob/master/docs/getting-started/rust.mdx)) |
| Sparkle 2 | mac only | Yes, auto-generated by `generate_appcast`; new format in 2.7 | EdDSA (ed25519) | Yes, plus phased rollout, critical updates | De-facto mac updater UI ([publishing](https://sparkle-project.org/documentation/publishing/), [upgrading](https://sparkle-project.org/documentation/upgrading/)) |
| tauri-plugin-updater 2.13.3 | Win, mac, Linux | None documented | minisign, pubkey in config, `TAURI_SIGNING_PRIVATE_KEY` | Via endpoint URL | Tied to tauri config ([Tauri updater](https://v2.tauri.app/plugin/updater/)) |
| cargo-packager-updater 0.2.3 | Win, mac, Linux | No | minisign pubkey | Via endpoint | Last release 2025-07-21 ([README](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/updater/README.md)) |
| Old ND updater | mac, Linux | No | minisign over archive + manifest | No | (old repo) `updates.ts` |

Velopack is the only one covering installer, deltas, channels and all three
platforms in Rust. Its gap is feed authenticity: a compromised host can serve a
matching hash. Close it by signing `releases.{channel}.json` with ed25519
(minisign format, reusing the old ND key flow) and verifying in a custom
`UpdateSource` before Velopack sees the feed, plus Authenticode/codesign on the
binaries themselves. On Nix installs the updater is compiled out or disabled
(Nix owns the store path).

## 5. Centralised project config

What to copy:
- One file per app owning identity, icons, bundle ids, file associations, URL
  schemes, signing, update feed and per-platform sections. The old
  `nativedesktop.config.ts` already did identity, file associations and URL
  schemes (`packages/nd/src/package/identity.ts`, CLAUDE.md "File
  associations").
- tauri's per-platform overlay files (`tauri.windows.conf.json` merged over the
  base) map cleanly to `[windows]`, `[macos]`, `[linux]` tables.
- cargo-packager accepts both a standalone file and Cargo metadata
  ([Packager.toml example](https://github.com/crabnebula-dev/cargo-packager/blob/main/examples/wails/Packager.toml)).

What to avoid:
- A config that is also code (electron-builder's JS configs, our old `.ts`):
  agents cannot validate it statically and Nix cannot read it.
- One giant flat JSON with no comments (tauri.conf.json); runtime capability and
  CSP config mixed with packaging.
- Version duplicated outside `Cargo.toml` (old repo published mismatched
  versions, CLAUDE.md "npm publishing"). Read version and name from Cargo.

Proposal: `nativedesktop.toml` beside `Cargo.toml`, deserialized by one Rust
struct with `serde` + `schemars`; the CLI emits a JSON Schema, and the file opens
with `#:schema ./target/nd-schema.json` for taplo-based editors
([taplo-cli](https://crates.io/crates/taplo-cli)).

### App settings format

| Format | Comments | Nix generation | Rust crate | Verdict |
|---|---|---|---|---|
| TOML | Yes | `pkgs.formats.toml` in nixpkgs (`pkgs/pkgs-lib/formats.nix:123`) | `toml` 1.1.8, spec 1.1, 2026-10-09; `toml_edit` for comment-preserving writes | Pick |
| JSON | No | `pkgs.formats.json` (`formats.nix:105`) | `serde_json` | Machine state only |
| KDL | Yes | Only home-manager's `lib.hm.generators.toKDL`, not nixpkgs formats | `kdl` 6.7.1 | Nice, weaker tooling |
| RON | Yes | None | `ron` 0.12.2 | No Nix path |

Parse speed does not decide it: a settings file is a few KB, which every crate
above parses well under a millisecond (estimate; confirm with a criterion bench
in the spike). TOML wins on Nix generation, comments, schema tooling and
familiarity. Writes from the app's settings UI go through `toml_edit` so a
user's comments survive. A home-manager-managed file is a read-only store
symlink, so the app must detect it and show settings as managed rather than fail
on save (layer: Nix base, user overrides file on top).

## 6. Linux packaging on top of Nix

- Flake: one `packages.<system>.<app>` per app built with crane (active,
  pushed 2026-10-11, [ipetkov/crane](https://github.com/ipetkov/crane)), sharing a
  deps-only derivation across apps; GPUI needs vulkan-loader, wayland, libxkbcommon
  and fontconfig on the runtime path.
- `homeManagerModules.<app>`: `programs.<app>.settings` typed with
  `(pkgs.formats.toml {}).type`, written with `.generate` to
  `$XDG_CONFIG_HOME/<app>/settings.toml`; the framework can generate this module
  from the same schemars schema as the app's settings struct.
- Non-Nix users:
  - AppImage: Velopack's Linux output is a self-updating AppImage and nothing
    else ([Velopack packaging overview](https://github.com/velopack/velopack.docs/blob/master/docs/packaging/overview.mdx));
    `nix bundle --bundler github:ralismark/nix-appimage` exists but was last
    pushed 2025-09-15 ([repo](https://github.com/ralismark/nix-appimage)).
  - deb/pacman: cargo-packager formats (section 3).
  - Flatpak: `flatpak-cargo-generator.py` vendors Cargo deps for the offline
    build ([flatpak-builder-tools/cargo](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo)).
    GPUI apps need the GPU and Wayland sockets in `finish-args`.
- Matugen/desktop theming stays a runtime concern of the GPUI theme loader, not
  packaging.

## Verdict for NativeDesktop

1. **Windows renders with GPUI**, sharing the Linux backend. Ship Fluent behaviour
   on it: Mica/MicaAlt/Acrylic via `DWMWA_SYSTEMBACKDROP_TYPE` (add Acrylic),
   app-drawn 32 px caption buttons with `HTMAXBUTTON` hit-testing for snap
   layouts, Segoe Fluent icons, Fluent spacing. `windows-reactor` is a watch item
   until it leaves preview; it fails "fits our standards" today on maturity,
   third-backend cost and airspace.
2. **Input work on Windows**: add a `WM_POINTER` path for pen and touch, and per-window
   frame pacing for mixed 240/144 Hz, each gated by a recorded trace on g815.
   DirectManipulation pan/pinch and PerMonitorV2 DPI come with GPUI.
3. **Updates and installers: Velopack everywhere except Nix**, wrapped by `nd
   package`: Setup.exe (branded from config) on Windows, `.app`/`.pkg` on mac, AppImage
   on Linux; deltas and channels built in. Add an ed25519-signed feed in a custom
   `UpdateSource`. Evaluate Sparkle 2 only if the mac update UI feels foreign.
4. **Signing**: Azure Artifact Signing through the CanaryCoders organization,
   driven by jsign from CI; Apple notarization through `vpk` flags.
5. **Config**: `nativedesktop.toml` per project (identity, icons, signing, feeds,
   per-OS tables), version and name read from Cargo, JSON Schema emitted from
   Rust types. App settings in TOML (`toml` + `toml_edit`), one schemars schema
   feeding the settings UI, validation, and a generated home-manager module.
6. **Nix**: crane-built flake package plus `homeManagerModules` per app; updater
   disabled on Nix builds; deb/Flatpak through cargo-packager formats and
   flatpak-cargo-generator for non-Nix users.
