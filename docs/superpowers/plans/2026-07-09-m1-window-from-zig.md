# M1 — Window from Zig (Linux) + Headless CI Proof: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From the existing empty scaffold, produce a Zig 0.16 program that opens a GTK4 window with a clickable button, runs identically under a headless Wayland compositor, and is verified by CI — proving the toolchain pin, the zig-gobject binding path, and the headless story before anything is built on them.

**Architecture:** Single Zig executable (`nd-hello`) using zig-gobject-generated GTK4 bindings, driven by GTK's own main loop. A `--smoke` flag makes the app self-verify (print a marker when the window maps, then auto-quit) so the same binary works interactively and in headless CI under `weston --backend=headless` with `GSK_RENDERER=cairo`.

**Tech Stack:** Zig 0.16.0 (exact), zig-gobject (master, post-April-2026 commit, pinned by hash), GTK4 ≥ 4.20 (Nix devshell provides 4.22.4), weston (headless), Nix flake + direnv (already in repo), GitHub Actions with `DeterminateSystems/nix-installer-action`.

## Global Constraints

- Zig is exactly `0.16.0`; the build fails loudly on any other version (Task 2 enforces this).
- Bun is pinned `1.3.13` (not used in M1, but the flake must not drift it).
- GTK4 ≥ 4.20; devshell currently provides 4.22.4, libadwaita is NOT linked (spec D-record: adwaita is an optional add-on, never core).
- No `@cImport` anywhere — it no longer exists in Zig 0.16; all GTK access goes through zig-gobject modules.
- No hand-written per-widget C bindings (spec D6); only zig-gobject-generated modules.
- Headless CI must use `weston --backend=headless` + `GSK_RENDERER=cairo` — explicitly NOT Broadway and NOT Xvfb/X11 (both deprecated for removal in GTK 5).
- Commit style: short imperative lowercase subject (e.g. `feat: open gtk4 window from zig`). No co-author trailers.
- All commands below run inside the devshell (direnv activates it automatically in the repo; in CI, `nix develop -c`).

---

### Task 1: Extend the flake for binding generation and headless runs

**Files:**
- Modify: `flake.nix` (Linux package list)

**Interfaces:**
- Consumes: existing flake devshell (zig, zls, bun, pkg-config, gtk4, libadwaita, glib, gobject-introspection).
- Produces: devshell additionally exposing `xsltproc` (zig-gobject's codegen dependency), `weston` (headless compositor), and GIR data on `XDG_DATA_DIRS` for binding generation. Later tasks assume `weston` and `xsltproc` are on PATH inside the devshell.

- [ ] **Step 1: Add the packages**

In `flake.nix`, extend the Linux-only package list:

```nix
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              gtk4
              libadwaita
              glib
              gobject-introspection
              libxslt      # provides xsltproc for zig-gobject codegen
              weston       # headless wayland compositor for CI/agent runs
            ];
```

- [ ] **Step 2: Verify the devshell resolves the new tools**

Run: `git add flake.nix && nix develop -c bash -c 'which xsltproc weston && pkg-config --modversion gtk4'`
Expected: paths for both binaries and `4.22.4`. (`git add` first — flakes only see git-tracked/staged files.)

- [ ] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "feat: add xsltproc and weston to devshell for gtk binding codegen and headless ci"
```

### Task 2: Zig project scaffold with enforced toolchain pin

**Files:**
- Create: `build.zig`
- Create: `build.zig.zon`
- Create: `src/main.zig`
- Create: `.zigversion` (single line: `0.16.0`)

**Interfaces:**
- Produces: `zig build` / `zig build run` / `zig build test` entrypoints; a `checkZigVersion(b)` guard in `build.zig` that later milestones keep; executable name `nd-hello`. Task 3 replaces the body of `src/main.zig` but keeps `pub fn main`.

- [ ] **Step 1: Write `.zigversion` and the failing build guard test**

`.zigversion`:

```
0.16.0
```

`build.zig` (thin on purpose — Zig 0.17's build rework lands ~Aug 2026, keep churn surface small):

```zig
const std = @import("std");
const builtin = @import("builtin");

const required_zig = "0.16.0";

pub fn build(b: *std.Build) void {
    checkZigVersion();

    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const exe = b.addExecutable(.{
        .name = "nd-hello",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    b.installArtifact(exe);

    const run_step = b.step("run", "Run nd-hello");
    const run_cmd = b.addRunArtifact(exe);
    if (b.args) |args| run_cmd.addArgs(args);
    run_step.dependOn(&run_cmd.step);

    const tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&b.addRunArtifact(tests).step);
}

fn checkZigVersion() void {
    const required = std.SemanticVersion.parse(required_zig) catch unreachable;
    if (builtin.zig_version.order(required) != .eq) {
        std.debug.panic(
            "NativeDesktop requires Zig {s} exactly (found {f}). Enter the devshell (direnv/nix develop).",
            .{ required_zig, builtin.zig_version },
        );
    }
}
```

Note: if `{f}` is rejected for `std.SemanticVersion` on 0.16, print `builtin.zig_version_string` with `{s}` instead — one-line adjustment, keep the panic message content.

`build.zig.zon`:

```zig
.{
    .name = .nativedesktop,
    .version = "0.0.1",
    .fingerprint = 0x0, // replace with the value `zig build` suggests on first run
    .minimum_zig_version = "0.16.0",
    .paths = .{ "build.zig", "build.zig.zon", "src" },
    .dependencies = .{},
}
```

`src/main.zig` (placeholder body; replaced in Task 3):

```zig
const std = @import("std");

pub fn main() !void {
    std.debug.print("nd-hello scaffold\n", .{});
}

test "toolchain pin matches .zigversion" {
    const builtin = @import("builtin");
    const pinned = std.mem.trim(u8, @embedFile(".zigversion"), " \n\r\t");
    const required = try std.SemanticVersion.parse(pinned);
    try std.testing.expect(builtin.zig_version.order(required) == .eq);
}
```

The `@embedFile(".zigversion")` requires the file to live next to `main.zig`'s module root; add `.zigversion` to the module via a symlink OR simpler: place the real file at `src/.zigversion` and make the repo-root `.zigversion` the symlink. Choose whichever `zig build test` accepts; the test content stays as written.

- [ ] **Step 2: Run the test, expect the fingerprint error first**

Run: `zig build test`
Expected: first run fails telling you the correct `.fingerprint` value — paste it into `build.zig.zon` and re-run. Then: test passes (we are on 0.16.0 inside the devshell).

- [ ] **Step 3: Verify the guard actually guards**

Run: `zig build run`
Expected: `nd-hello scaffold`. (Negative case for the panic path is exercised naturally the first time anyone builds outside the devshell; do not engineer a fake-version test.)

- [ ] **Step 4: Commit**

```bash
git add build.zig build.zig.zon src/ .zigversion
git commit -m "feat: zig scaffold with enforced 0.16.0 toolchain pin"
```

### Task 3: GTK4 window with a clickable button via zig-gobject

**Files:**
- Modify: `build.zig.zon` (add zig-gobject dependency)
- Modify: `build.zig` (wire gtk/gio/glib/gobject modules)
- Modify: `src/main.zig` (real app)

**Interfaces:**
- Consumes: Task 2 scaffold.
- Produces: `zig build run` opens a titled window with one button; clicking prints `ND_CLICKED` on stdout. Module imports named `gtk`, `gio`, `glib`, `gobject` — Task 4 and all later milestones use these names. App ID constant `dev.nativedesktop.hello` exposed as `pub const app_id`.

- [ ] **Step 1: Pin zig-gobject**

Pick the latest master commit of `https://github.com/ianprime0509/zig-gobject` that is ≥ April 2026 (Zig 0.16 support; verify the commit message/CI badge mentions 0.16), then:

Run: `zig fetch --save=gobject https://github.com/ianprime0509/zig-gobject/archive/<COMMIT_SHA>.tar.gz`
Expected: `build.zig.zon` gains a `.gobject` dependency with a content hash (the hash pins it; the URL is just a mirror).

- [ ] **Step 2: Wire the binding modules in `build.zig`**

zig-gobject exposes one Zig module per GIR namespace. Add after `exe` creation (and mirror onto `tests`):

```zig
    const gobject_dep = b.dependency("gobject", .{
        .target = target,
        .optimize = optimize,
    });
    for ([_][]const u8{ "gtk4", "gio2", "glib2", "gobject2" }, [_][]const u8{ "gtk", "gio", "glib", "gobject" }) |mod, name| {
        exe.root_module.addImport(name, gobject_dep.module(mod));
    }
    exe.linkSystemLibrary("gtk4");
    exe.linkLibC();
```

Verification sub-step (module names are the one unstable assumption here): if `gobject_dep.module("gtk4")` fails, run `zig build --verbose 2>&1 | head -40` and check the dependency's exposed modules with `rg '"name"|addModule|module\(' ~/.cache/zig/p/<gobject-pkg-dir>/build.zig | head -30`, then substitute the actual names (historically `gtk4`, `gio2`, `glib2`, `gobject2` for the GNOME 4x binding set). If the pinned commit requires running its codegen step first (README section "Usage"), follow it inside the devshell — `xsltproc` and GIR data are already provided by Task 1.

- [ ] **Step 3: Write the app**

`src/main.zig`:

```zig
const std = @import("std");
const glib = @import("glib");
const gobject = @import("gobject");
const gio = @import("gio");
const gtk = @import("gtk");

pub const app_id = "dev.nativedesktop.hello";

pub fn main() u8 {
    const app = gtk.Application.new(app_id, .{});
    defer app.unref();

    _ = gio.Application.signals.activate.connect(app, ?*anyopaque, &onActivate, null, .{});
    return @intCast(gio.Application.run(app.as(gio.Application), 0, null));
}

fn onActivate(app: *gtk.Application, _: ?*anyopaque) callconv(.c) void {
    const window = gtk.ApplicationWindow.new(app);
    const win = window.as(gtk.Window);
    win.setTitle("NativeDesktop M1");
    win.setDefaultSize(480, 320);

    const button = gtk.Button.newWithLabel("Click me");
    _ = gtk.Button.signals.clicked.connect(button, ?*anyopaque, &onClicked, null, .{});
    win.setChild(button.as(gtk.Widget));

    win.present();
}

fn onClicked(_: *gtk.Button, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_CLICKED\n", .{});
}

test "toolchain pin matches .zigversion" {
    const builtin = @import("builtin");
    const pinned = std.mem.trim(u8, @embedFile(".zigversion"), " \n\r\t");
    const required = try std.SemanticVersion.parse(pinned);
    try std.testing.expect(builtin.zig_version.order(required) == .eq);
}
```

The exact signal-connect signature (`connect(instance, comptime Data, callback, data, .{})`) and upcast helper (`.as(...)`) follow zig-gobject's own `examples/` directory in the pinned checkout (`~/.cache/zig/p/<pkg>/examples/`); if the pinned commit's example differs (this API stabilized across 2025–2026), match the example — the app's observable behavior (window + button + `ND_CLICKED` on stdout) is the contract, the binding idiom is not.

- [ ] **Step 4: Run it interactively**

Run: `zig build run`
Expected: a 480×320 window titled "NativeDesktop M1" with a "Click me" button; clicking prints `ND_CLICKED`; closing the window exits 0. Run `zig build test` — still passes.

- [ ] **Step 5: Commit**

```bash
git add build.zig build.zig.zon src/main.zig
git commit -m "feat: open gtk4 window with clickable button via zig-gobject"
```

### Task 4: Smoke mode + headless runner script

**Files:**
- Modify: `src/main.zig` (add `--smoke`)
- Create: `scripts/headless-smoke.sh`

**Interfaces:**
- Consumes: Task 3 app, Task 1 weston.
- Produces: `nd-hello --smoke` prints `ND_SMOKE_MAPPED` once the window maps, then exits 0 within 5s. `scripts/headless-smoke.sh` exits 0 iff the marker appeared — CI (Task 5) and future agent tooling call exactly this script.

- [ ] **Step 1: Add smoke mode to `main.zig`**

```zig
var smoke = false;

pub fn main() u8 {
    for (std.os.argv) |arg| {
        if (std.mem.eql(u8, std.mem.span(arg), "--smoke")) smoke = true;
    }
    // ... unchanged Application setup ...
}
```

In `onActivate`, after `win.present()`:

```zig
    if (smoke) {
        _ = gtk.Widget.signals.map.connect(window.as(gtk.Widget), ?*anyopaque, &onMapped, null, .{});
    }
```

And the callback — print the marker, then quit on the next main-loop iteration (never synchronously inside the signal):

```zig
fn onMapped(_: *gtk.Widget, _: ?*anyopaque) callconv(.c) void {
    std.debug.print("ND_SMOKE_MAPPED\n", .{});
    _ = glib.idleAdd(&quitIdle, null);
}

fn quitIdle(_: ?*anyopaque) callconv(.c) c_int {
    if (global_app) |app| gio.Application.quit(app.as(gio.Application));
    return 0; // G_SOURCE_REMOVE
}

var global_app: ?*gtk.Application = null; // set in main() after Application.new
```

- [ ] **Step 2: Write `scripts/headless-smoke.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-0
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland

weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
  sleep 0.1
done

OUT=$(timeout 30 ./zig-out/bin/nd-hello --smoke)
echo "$OUT"
grep -q ND_SMOKE_MAPPED <<<"$OUT"
echo "headless smoke: OK"
```

- [ ] **Step 3: Run it, expect failure before build**

Run: `chmod +x scripts/headless-smoke.sh && rm -rf zig-out && ./scripts/headless-smoke.sh`
Expected: FAIL (`nd-hello: No such file`) — proves the script actually checks something.

- [ ] **Step 4: Build and run it for real**

Run: `zig build && ./scripts/headless-smoke.sh`
Expected: `ND_SMOKE_MAPPED` then `headless smoke: OK`, exit 0 — a GTK4 window mapped with no display attached.

- [ ] **Step 5: Commit**

```bash
git add src/main.zig scripts/headless-smoke.sh
git commit -m "feat: smoke mode and weston headless runner"
```

### Task 5: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: Task 4 script; the flake.
- Produces: a `ci` workflow later milestones extend (protocol tests, conformance suite, kill-9 test all append jobs here).

- [ ] **Step 1: Write the workflow**

```yaml
name: ci
on:
  push: { branches: [main] }
  pull_request:

jobs:
  linux:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: DeterminateSystems/nix-installer-action@v16
      - uses: DeterminateSystems/magic-nix-cache-action@v8
      - name: unit tests
        run: nix develop -c zig build test
      - name: build
        run: nix develop -c zig build
      - name: headless smoke
        run: nix develop -c ./scripts/headless-smoke.sh
```

- [ ] **Step 2: Validate locally (no push)**

Run: `nix develop -c bash -c 'zig build test && zig build && ./scripts/headless-smoke.sh'`
Expected: all three green — the exact command sequence CI runs.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: linux build, unit tests, headless smoke"
```

---

## Self-review notes

- **Spec coverage:** M1 scope from the roadmap — pinned toolchains ✅ (T1/T2), zig-gobject window+button ✅ (T3), headless proof pulled into M1 ✅ (T4/T5). The M1 roadmap line mentions vendoring zig-gobject: the content hash in `build.zig.zon` pins it immutably; physical vendoring (committing the source) is deferred until upstream risk materializes — deliberate deviation, noted for the owner.
- **Known unstable assumptions, called out inline:** zig-gobject module names and signal-connect idiom (T3 step 2/3 include the exact verification commands); `{f}` format specifier for SemanticVersion (T2); `@embedFile` path for `.zigversion` (T2). Each has a bounded, in-task resolution path — none blocks a fresh engineer.
- **Type consistency:** `app_id`, executable `nd-hello`, marker strings `ND_CLICKED`/`ND_SMOKE_MAPPED`, and script path `scripts/headless-smoke.sh` are used identically across tasks.
