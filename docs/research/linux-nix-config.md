# Linux, Nix, theming, settings, dev storage: research for the Rust rewrite

Date: 2026-10-13. Scope: matugen, Hyprland, Nix helper, settings system, dev storage.
Tags: [local] read from the user's repos on this machine, [doc] fetched from docs/specs today,
[ctx7] Context7, [mem] from memory and not re-verified today (treat as a lead).

## 1. matugen

### What it is and what is installed
- matugen 4.2.0 is current on crates.io (`curl crates.io/api/v1/crates/matugen`), built on the `material-colors` crate (`InioX/matugen Cargo.toml`: `material-colors = 0.4.0`). The nix config and FormalShell test against 4.1.0 [local: FormalShell `crates/fs-theme/src/zenbones.rs:333`].
- Two inputs: `matugen image <file>` and `matugen color hex "#rrggbb"` [doc: github.com/InioX/matugen README].
- Output paths: (a) `--json hex` dumps `colors.{light,dark,amoled}.<role>` plus `colors_android` as JSON [ctx7: /iniox/matugen, wiki Usage]; (b) templates rendered to files from `config.toml`.

### Template system
- Placeholder: `{{colors.<role>.<default|light|dark>.<format>}}`; also `{{image}}` and `{{custom.<kw>}}` [ctx7: wiki Configuration]. Formats seen in use: `hex`, `hex_stripped`, `rgb`, `rgba`, `hsl`, `hsla` [local: noctalia spec `docs/superpowers/specs/2026-06-19-noctalia-dynamic-theming-design.md`; README says colors are objects converted between formats].
- Filters: `grayscale`, `invert`, `set_lightness`, `auto_lightness`, plus hue/saturation/opacity; the engine also has loops, conditionals, includes, arithmetic (Chumsky-built) [ctx7, README].
- `.default` tracks the active mode, so one template file serves light and dark and is simply re-rendered on a flip [local: `matugen-templates/ghostty.tmpl` header].
- `config.toml`: `[config]`, `[config.custom_colors]` (`red = { color = "#ff0000", blend = false }`), `[templates.<name>]` with `input_path`, `output_path`, `pre_hook`, `post_hook` [ctx7: wiki Configuration, Hooks]. `~` expands inside paths [local: `mixins/dms.nix:460`].

### How the user's desktop already drives it [local]
- DMS owns the trigger (wallpaper pick or mode flip) and splices `~/.config/matugen/config.toml` into its own matugen run (`mixins/dms.nix:455-462`). Templates live in `users/kyandesutter/matugen-templates/` and are symlinked by home-manager (`dms.nix:441-453`).
- App reload strategies in use: signal (`post_hook = "pkill -SIGUSR2 ghostty || true"`, `dms.nix:522`), app watches the file itself (neovim, discord: "no post_hook"), or app polls (messages and formalmusic poll mtime once per second, `dms.nix:581-590`).
- FormalShell runs matugen itself (`crates/formalshell-rs/src/services/theme.rs`) and found two traps worth inheriting:
  1. matugen prompts when an image yields several source colours and a child has no TTY, so an unforced run fails (50 of 57 wallpapers on g815). Fix there: a `--dry-run` probe reads the ranking, then the real run pins `--prefer closest-to-fallback --fallback-color <rank0>` (theme.rs:14-35).
  2. Material has no green or yellow. Terminals need `[config.custom_colors]` hues with `blend = false` (`dms.nix:470-490`).
- messages already has a token-map prior art: `messages.json.tmpl` maps `surface -> canvas`, `surface_container_low -> sidebar`, `outline_variant -> separator`, `primary -> accent`, `error -> danger`, `green -> online`; the app loads it in `crates/desktop/src/live_theme.rs` as a partial `PaletteFile` over a baked-in palette, ignoring unknown keys and non-hex values.

### Recommended consumption model
1. The app never invokes matugen. A user-level matugen config writes `~/.config/<app>/theme.json` (or the framework installs a template via its home-manager module). When that file is absent, nd falls back to FormalShell's `$XDG_STATE_HOME/formalshell/theme.json` (shadcn keys plus `mode`). Reason: the desktop shell already owns the trigger; two matugen runners would fight.
2. Read side: `notify` (8.2.0 stable [crates.io]) watching the PARENT directory, debounced ~50 ms. Notify's inotify notes say a watch ends when the watched object is removed and renames surface as Remove/Create [ctx7: notify Event Guide], so watching the file itself dies on atomic-rename writers. Keep a 1 s mtime stat as a safety net (that is what messages ships today).
3. Apply is a diff: parse into `ThemeFile { colors: map }`, compare to the live theme, request one redraw. Unreadable or half-written JSON keeps the previous theme (messages does this).
4. Cross-platform: macOS ignores the file by default and takes system accent plus appearance; the same file format can still be honoured when present (opt-in).

### Token mapping (Material You -> framework tokens)
| Framework token | Material role (`.default`) |
| --- | --- |
| `surface.base` (window bg) | `surface` |
| `surface.sidebar` | `surface_container_low` |
| `surface.raised` / `.raised_hover` | `surface_container` / `surface_container_high` |
| `surface.overlay` (popover, menu) | `surface_container_high` |
| `border.subtle` / `separator` | `outline_variant` |
| `text.primary` / `.secondary` / `.tertiary` | `on_surface` / `on_surface_variant` / `outline` |
| `accent` / `accent.fg` | `primary` / `on_primary` |
| `accent.container` | `primary_container` / `on_primary_container` |
| `danger` | `error` (+ `on_error`) |
| `success`, `warning` | custom colors `green`, `yellow` (Material has none) |
| `selection` | `secondary_container` |
Derived tokens (hover, pressed, focus ring, scrims) are computed in the framework from the base tokens, not read from the file. That is how messages avoids a 70-key file: only base tokens are set, the rest derive.

### Fallback when matugen is absent
Chain, first hit wins: (1) `theme.json` from the user or Nix module; (2) OS appearance: macOS NSAppearance plus accent, Linux `org.freedesktop.appearance` `color-scheme` and `accent-color` via the xdg-desktop-portal Settings interface (`ashpd` 0.13.13 [crates.io]) [mem: portal keys]; (3) baked-in light and dark palettes.
Option for later: generate a scheme in-process from one seed colour using `material-colors` 0.5.0 (same engine matugen uses, so no binary needed). Not a day-one feature.

## 2. Hyprland

### Decorations and window controls
- Hyprland advertises `xdg-decoration` at version 1; so do Sway, niri, COSMIC, labwc, river. GNOME Mutter and Weston do not [doc: wayland.app/protocols/xdg-decoration-unstable-v1].
- Protocol: client calls `get_toplevel_decoration`, may `set_mode`, compositor answers with `configure(mode)` and may override; if the protocol is absent the client must draw its own [doc: same page].
- Hyprland itself draws no title bar and no buttons, so the user's "no controls on Hyprland" is the default behaviour: the app must read the `configure` mode and the `xdg_toplevel.wm_capabilities` set rather than check `$XDG_CURRENT_DESKTOP`. The framework rule: draw controls only when the mode is client-side AND the capability set advertises maximize/minimize/fullscreen; on a tiling compositor hide them. [mem: wm_capabilities event, xdg-shell v5]
- GPUI reference (Zed `crates/gpui_linux/src/linux/wayland/window.rs`, fetched today): binds `get_toplevel_decoration` at creation (L288), defaults `decorations: Client` (L622), flips on `Configure { mode }` and fires `appearance_changed` (L1174-1190). Exposes `start_window_move` (L2085), `start_window_resize(edge)` (L2102), `show_window_menu` (L2072), `request_decorations` (L2176). So drag areas are `xdg_toplevel.move` calls triggered from a pointer-down on a framework "drag region" node. Copy this shape.
- Note: Zed once forced server-side decorations always (PR #8037) as a stopgap and warned compositors may override; do not assume either outcome.

### Gestures
- 0.51 removed `gestures:workspace_swipe*`; the replacement is `gesture = fingers, direction, action[, options]`, e.g. `gesture = 3, horizontal, workspace` [doc: hypr.land/news/update51, wiki 0.54 Gestures]. Directions: swipe, horizontal, vertical, left/right/up/down, pinch, pinchin, pinchout. Actions: dispatcher, workspace, move, resize, special, close, fullscreen, float. Optional `mod:` and `scale:` [doc: wiki 0.54].
- Hyprland says it supports 1:1 (finger-tracking) gestures for some operations [doc: same].
- Conflict rule: the compositor consumes the gesture before the app sees `zwp_pointer_gestures` events. `pointer-gestures-unstable-v1` v3 (swipe, pinch, hold) is supported by Hyprland 0.52.1, Sway, niri, KWin, Mutter [doc: wayland.app/protocols/pointer-gestures-unstable-v1]. Apps get 2-finger scroll and pinch; whichever finger counts the user binds in `gesture =` never reach the app. Framework consequence: standardise app gestures on the finger counts Hyprland leaves free (2-finger scroll, pinch, 2-finger swipe) and document that 3 and 4 finger swipes belong to the compositor on Hyprland, but are app-available on macOS only if the OS does not claim them.
- Gestures are cancellable by the compositor; never commit irreversible actions before `end` [doc: protocol page].

### Scaling, refresh, frame pacing
- `wp_fractional_scale_v1` + `wp_viewporter`: compositor sends `preferred_scale` as numerator over 120; the client renders at the larger buffer and sets the viewport destination to the logical size. Hyprland, Mutter, KWin, niri, Sway all support it [doc: wayland.app/protocols/fractional-scale-v1]. GPUI wires it (`get_fractional_scale` L853, `set_destination` L1538, `rescale` L1547).
- Monitor syntax is `monitor = name, WxH@Hz, pos, scale` [doc: wiki 0.54 Monitors]. Scale is per monitor, so a window dragged between g815's 240 Hz and 144 Hz outputs gets a new `preferred_scale` and a new frame cadence; the surface must not cache either.
- XWayland on fractional scale is blurry unless `force_zero_scaling` is set [doc: wiki.hypr.land/configuring/extra/xwayland]. The framework should be Wayland-native only on Linux (no X11 path), which sidesteps it.
- Frame pacing: use `wl_surface.frame` callbacks as the render clock (GPUI: `pending_frame_callback` L136, `complete_frame` L965, parked/scheduled/ticking loop L1040). A per-monitor refresh falls out for free because the callback fires at the output the surface is on. Add `wp_presentation` (Hyprland 0.52.1 lists version 2 [doc: wayland.app/protocols/presentation-time]) for measured present times: `presented` carries timestamp, predicted `refresh` ns, and `vsync`/`zero_copy` flags, which is the data the profiler and "frame recording" feature need. Idle rule: do not request another frame callback unless something is animating (messages' idle-CPU budget already assumes this).

### Layer-shell, popovers, input leakage
- `wlr-layer-shell` v5 on Hyprland 0.52.1; absent on Mutter [doc: wayland.app/protocols/wlr-layer-shell-unstable-v1]. Layers background < bottom < top < overlay; anchors bitfield; exclusive zone; keyboard interactivity none / exclusive / on_demand. Use for panels, OSDs, launchers only. GPUI maps it to `WindowKind::LayerShell` (window.rs L161-198).
- Popovers and context menus: use `xdg_popup` with a grab (parent can be a layer surface: `layer_surface.get_popup`, GPUI L225-227), not a fake in-window overlay and not a layer surface. A grabbed popup gets dismissed by the compositor on outside click, which is the correct cure for "context menus leak input through". [mem: popup grab semantics]
- `xdg-toplevel-drag` (tab tear-off) is NOT supported on Hyprland 0.52.1 (supported by KWin 6.7, Mutter 51) [doc: wayland.app/protocols/xdg-toplevel-drag-v1]. Tab drag-out needs a fallback: spawn a new window on drop. Flag this against the "no feature loss" requirement.
- Hyprland layer rules (blur, ignorealpha, animation) are user config, not app API; the framework should ship a documented snippet via the home-manager module (FormalShell does: it writes `~/.config/hypr/formalshell.lua`, `nix/hm-module.nix`).

## 3. Nix: what a framework flake helper should provide

### Existing pattern [local]
FormalShell `flake.nix` + `nix/rust-common.nix`:
- `crane.mkLib pkgs`; `src = lib.fileset.toSource {...}`; `commonArgs` (nativeBuildInputs `pkg-config`, `rustPlatform.bindgenHook`; buildInputs `fontconfig libxkbcommon pipewire pam`); one `cargoArtifacts = craneLib.buildDepsOnly` shared by package, clippy and test derivations; own `cargoVendorDir` via `rustPlatform.importCargoLock` because crane's per-crate vendoring makes hundreds of builds on a cold store (rust-common.nix comments).
- Constraint noted there: crane artifacts are profile specific; keep every derivation in release.
- Outputs: `packages`, `homeModules.default`, `nixosModules.*`, plus a `nixos-module-eval` check that evaluates (never builds) the documented NixOS config so a module default that stops resolving fails CI (flake.nix L18-50).
- Exposes systems `x86_64-linux aarch64-linux` plus darwin for pure-crate tests.
- messages and youtubemusic carry the same `nix/hm-module.nix` skeleton: `enable`, `package` (default from `self.packages`), `autostart` systemd user service, `environmentFile` for secrets, a `theme` attrset written to JSON.
- crane docs agree on the shape: `buildDepsOnly` -> `cargoClippy`/`cargoNextest` -> `buildPackage`, and `craneLib.devShell { checks = self.checks.${system}; packages = [...] }` wraps `mkShell` with cargo/clippy/rustc/rustfmt [ctx7: /ipetkov/crane docs/API.md]. Use the flake input `github:ipetkov/crane`.

### Crane vs naersk
Crane is already used in FormalShell and is the one with a documented devShell, nextest, clippy, fmt and doc checks. Pick crane; no evidence for naersk in the user's repos (rg found none).

### What `nativedesktop.lib.mkApp` should return
1. `packages.default`: crane `buildPackage` with the app's runtime libs wrapped (`makeBinaryWrapper --prefix LD_LIBRARY_PATH` for `wayland libxkbcommon vulkan-loader libGL fontconfig`), since winit/wgpu `dlopen` them. [mem: GPUI/winit dlopen wayland, xkbcommon, vulkan]
2. `devShells.default`: `craneLib.devShell` with the same buildInputs, `pkg-config`, `mold` or `lld`, `cargo-nextest`, `just`, and an env that exports `LD_LIBRARY_PATH` and `VK_ICD_FILENAMES` handling for Intel (e1504g: Mesa ANV) and NVIDIA/AMD on g815.
3. `checks`: clippy, nextest, fmt, the NixOS-module-eval trick above, and a "settings schema in sync" check (below).
4. `homeModules.default`: generated from the settings schema (section 4).
5. `nixosModules` only when the app needs system pieces (udev, avahi, firewall); FormalShell splits these the same way.
6. Never build e1504g's closure there; the user's rule is build on g815 and push (`nixos-rebuild --target-host`).
7. Add cachix or attic for `cargoArtifacts`; otherwise every dependency bump rebuilds wgpu and friends on g815.

### Home-manager module from the settings schema
Two options, ranked:
- A (recommended): freeform `settings` typed `(pkgs.formats.toml {}).type`, rendered with `.generate`, plus typed convenience for 5-10 common keys and `lib.mkIf (cfg.settings != {})` on the file. This is what FormalShell does with JSON (`hm-module.nix` L12-16) and what the wallust HM module does with TOML [doc: nixos discourse/pkgs-lib formats]. Caveat: values land in the store, so secrets go through `environmentFile` as messages does.
- B: generate typed options from the Rust schema: emit JSON Schema with `schemars` 1.2.3 (honours serde attrs; doc comments become `description`; defaults become `default`) [ctx7: /gresau/schemars], then a small Nix function `schemaToOptions` turns objects into `mkOption`s. No existing crate does this [not found in search], so it is custom code; keep it to scalars, enums, lists of scalars, nested structs and fall back to freeform for the rest. Check in CI that the committed `settings.schema.json` equals `cargo run -- --print-schema`.
Doing B on top of A gives type errors at nix-eval time instead of a runtime parse error on the user's first login.

## 4. Settings system

### Shape
One Rust struct (`#[derive(Serialize, Deserialize, JsonSchema, Default)]`, `#[serde(default, deny_unknown_fields)]`) is the single source:
- TOML file `~/.config/<app>/config.toml` (XDG; `~/Library/Application Support/<app>` on macOS; `%APPDATA%` on Windows).
- Schema JSON for the HM module, for editor completion (Taplo/Even Better TOML reads `#:schema` URL) and for the in-app settings UI generator (field type -> widget; doc comment -> help text).
- In-app UI writes with `toml_edit` so user comments and ordering survive.

### Crates (versions from crates.io today)
- `serde` 1.0.229, `toml` 1.1.8 (spec 1.1.0; TOML 1.1 parse support arrived in 0.9.10 [doc: toml CHANGELOG], 1B downloads), `toml_edit` for writes, `schemars` 1.2.3.
- `notify` 8.2.0 (+ `notify-debouncer-full` 0.7.0). `inotify` 0.11.5 if Linux-only like FormalShell.
- `figment` 0.10.19: layered providers (`Serialized::defaults` -> `Toml::file` -> `Env::prefixed`), and errors name the source file and key path, e.g. `for key "port" in TOML file Config.toml` [ctx7: /sergiobenitez/figment]. Last release is old; useful but low activity. Recommendation: use figment for layering and diagnostics, or skip it and do three-step `merge` by hand; do not take it for file watching.
- Layers: defaults < system/nix file < user file < env (`APP_*`) < CLI flags. Show the winning layer per key in the in-app UI (figment's `Tagged` metadata gives this [ctx7]).

### Parse speed
- No published TOML-vs-JSON numbers found for the current Rust crates (searched twice; only a self-reported third-party parser claim, unverified). The toml changelog lists allocation and cloning reductions (1.1.6, 1.1.7) but no throughput figure [doc].
- Reasoning instead of a number: app settings files are 1-20 KB; any of these parsers does it in well under a millisecond, so format choice is about ergonomics, not speed. Do not pick a format for performance.
- Action item: add a criterion bench to the framework (`bench_settings_parse`) on a realistic 10 KB file for `toml`, `serde_json`, and `toml_edit`; the framework's own claim in the docs should cite that run.
- Recommendation: TOML for human and Nix-generated files (comments, `pkgs.formats.toml`), JSON only for machine files such as matugen's `theme.json`.

### Live reload
- Watch the parent dir (see section 1); on change: parse -> validate -> diff -> apply per field, with a per-field `reload: live | restart` attribute on the schema so the UI can say "restart required".
- Invalid file: keep last good config, surface the figment-style error with file, key and line in the in-app status area and the log.
- Own writes: tag the write (inode/mtime or a content hash) so the watcher ignores the app's own save.

### Read-only when managed by Nix
Home-manager files are symlinks into `/nix/store` (read-only). Detect with `symlink_metadata` -> target starts with `/nix/store/`, or `read_only` permission on the resolved file. The UI then shows "Managed by Nix" with the Nix option path per key (generated from the schema), disables inputs, and offers "Copy as Nix". For partial management, support a second writable overlay file (`config.local.toml`, merged last) so the UI can still save user-only keys. This is design, not prior art; none of the user's apps does it today (FormalShell: "only reads this file, so home-manager owns it fully", `hm-module.nix` L18).

## 5. Dev storage on the external drive

### Observed state [local, checked today]
- `/Volumes/Kyan Drive`: APFS, USB, solid state, 466G, 356G free, mounted `noowners`; Spotlight indexing enabled (`mdutil -s`).
- `dev-cache/cargo-target` and `dev-cache/sccache` exist and are empty (0 bytes).
- `~/.cargo/config.toml` absent, `CARGO_TARGET_DIR` and `RUSTC_WRAPPER` unset, `sccache` not installed. So nothing points at the drive yet.

### macOS setup
1. Put it in `~/.cargo/config.toml` or the project's `.cargo/config.toml`: `[build] target-dir = "/Volumes/Kyan Drive/dev-cache/cargo-target"` and `rustc-wrapper = "sccache"`; or env `CARGO_TARGET_DIR`, `RUSTC_WRAPPER`, `SCCACHE_DIR` [ctx7: sccache README: `build.rustc-wrapper`, `RUSTC_WRAPPER`].
2. One shared target dir for many workspaces makes cargo serialise on one lock and mixes artifacts; use a per-project subdir (`.../cargo-target/nativedesktop`) set from `just`/direnv.
3. sccache limits [ctx7: sccache README]: crates that invoke the linker (bin, dylib, cdylib, proc-macro) are not cached, and incremental crates are not cached. So it helps dependency compiles, not your own crates; set `CARGO_INCREMENTAL=0` only in CI-like runs. `sccache` size default is 10 GB, set `SCCACHE_CACHE_SIZE` or `[cache.disk] size` (config reference above).
4. Disable Spotlight: `sudo mdutil -i off "/Volumes/Kyan Drive"` or `touch "/Volumes/Kyan Drive/.metadata_never_index"`; Spotlight chewing on millions of `.rlib`/`.o` files is the classic slowdown. [mem]
5. Exclude from Time Machine (`tmutil addexclusion`), optional.

### Pitfalls
- Drive unmounted: `/Volumes` is root-owned, so cargo should fail loudly creating the path instead of silently writing to the boot disk. Wrap in `just` with a `test -d "/Volumes/Kyan Drive/dev-cache" || fail` guard so the error is readable. [mem: behaviour inferred, test it once]
- USB latency: sequential throughput is fine, small random I/O (incremental dep-info files) is the weak spot. Measure a clean and an incremental build on internal vs external before committing; if incremental is slow, keep the project's own `target/` internal and put only `sccache` and registry caches on the drive.
- Linking: macOS default linker is fast enough; on Linux use mold or lld in the dev shell.
- `noowners` mount: files appear owned by the current user, harmless for cargo; breaks nothing seen, but nix builds and codesign steps should not live on it.
- Eject while a build runs corrupts the sccache dir; sccache is rebuildable, cargo's `target/` too, so the cost is time not data.
- APFS volume is Mac only; Linux cannot mount it (needs apfs-fuse, read-only). Do not share this drive as the Linux cache; use a different volume or the network.

### Linux equivalents
- e1504g: do not build there at all (user rule); build on g815 and push the closure. For g815: `CARGO_TARGET_DIR` on a second NVMe or a dedicated ext4/btrfs mount, `SCCACHE_DIR` beside it, exported in the flake dev shell's `shellHook`.
- fstab for removable or secondary disks: `nofail,x-systemd.automount,x-systemd.device-timeout=5` so boot never hangs on a missing drive [mem].
- Nix: the store itself caches crane's `cargoArtifacts`; sccache is for the `cargo build` inner loop only.
- Cleanup: `cargo clean` on a `target/` over 2 GB is pre-approved by the user's CLAUDE.md; the shared dir is cheaper to `rm -rf` once than per project.

## Verdict for NativeDesktop
1. Theme: apps read `theme.json` through `notify` on the parent dir plus a 1 s mtime fallback; the framework ships a matugen template mapping the table above and a baked-in palette; macOS uses OS appearance. Add portal `color-scheme` and `accent-color` as the no-matugen Linux fallback. Never spawn matugen from the app; if the framework ever does, copy FormalShell's rank-0 pinning.
2. Windowing: negotiate `xdg-decoration` and `wm_capabilities`, draw no controls on Hyprland, expose `start_window_move`/`resize` through a drag-region node (GPUI shape). Treat fractional scale (`wp_fractional_scale_v1` + viewporter), `wl_surface.frame` pacing and `wp_presentation` as day-one, not polish.
3. Gestures: define the cross-platform gesture set around pointer-gestures v3 and document that 3- and 4-finger swipes are the compositor's on Hyprland (`gesture =` config, 0.51+).
4. Known gap to decide: Hyprland lacks `xdg-toplevel-drag`, so tab tear-off needs a drop-to-new-window fallback; this is the one place "no feature loss" needs an explicit answer.
5. Nix: crane, shared `cargoArtifacts`, `lib.fileset` source, FormalShell's module-eval check, plus a `mkApp` helper that returns package, devShell, checks and `homeModules.default`. Start with the freeform `settings` option (typed by `pkgs.formats.toml`), then add schemars-generated typed options.
6. Settings: TOML on disk (toml 1.1.8 for reads, toml_edit for UI writes), schemars for schema, figment-style layered errors, read-only UI when the file resolves into `/nix/store`, with a writable `config.local.toml` overlay. Do not claim a parse-speed number until a criterion bench exists.
7. Dev storage: the drive and cache dirs exist but nothing is wired. Wire `target-dir` per project plus sccache, turn Spotlight off, add an "is the drive mounted" guard, and benchmark incremental builds on USB before moving the project's own target dir there.
