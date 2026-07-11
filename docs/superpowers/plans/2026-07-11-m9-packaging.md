# M9 — Packaging + auto-updates: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Parent context.** This is the M9 milestone plan (`§14`: *"Signed/notarized/auto-updating gallery app on all three platforms from one CI workflow"*). **M1–M8 are landed and green** (see `CLAUDE-activeContext.md`). The packaged artifact is the **gallery app** (`examples/gallery`). The Linux host binary is `zig-out/bin/nd-hello` (GTK embedder over the `libnd` C ABI); the macOS shell is `swift/.build/release/NDShell` (M6b, linking `zig-out/lib/libnd.a`).
>
> **Scope (binding architect decisions — do not widen):**
> - **Windows is OUT of scope.** M7 (Windows backend) was deliberately deferred and does not exist. M9 covers **Linux + macOS only**. Every doc that lists platforms adds a one-line *"Windows packaging lands with M7"* note. **Do not write any Windows task, NSIS manifest, winget manifest, or Azure signing step.**
> - **No Apple Developer ID / notarization credentials exist in this environment.** The `notarytool` + `staple` path is *implemented but gated* on a CI-secrets presence check (`APPLE_ID`/`APPLE_TEAM_ID`/`APPLE_APP_PASSWORD`/`APPLE_SIGN_IDENTITY` env vars). The **ad-hoc codesign path (`codesign -s -`) is what the Mac verification legs actually run** and must work end-to-end locally over `ssh macbook`.
> - **No GitHub push access from this box.** The CI workflow is authored + committed, but its first live run is deferred — follow the `mac.yml` non-blocking precedent (`continue-on-error: true`).
> - **Updates layer = signature-verified full-archive updates (shipping scope). bsdiff+zstd deltas are DEFERRED** (documented, not built) — see the M9 decision record M9-D3. The smallest honest scope that satisfies "auto-updating" is: an Ed25519/minisign-verified update manifest (verification is non-disableable), a full-archive download, and an atomic swap. The update flow is exercised **headlessly in CI against a local Bun HTTP manifest server** — no network dependence in any test.

---

## Goal (M9)

Produce, from committed tooling and one CI workflow, a **packaged, signed, auto-updating gallery app** on Linux (AppImage) and macOS (`.app`), where "auto-updating" means the Zig core can fetch an update manifest, **verify its Ed25519/minisign signature (non-disableable)**, download the referenced full archive, verify *its* signature, and stage an atomic replace — all provable headlessly in CI with zero network access.

Four deliverables:

1. **Update-verification core in Zig** (`src/core/update.zig` + tests): minisign-compatible Ed25519 manifest + archive signature verification, a version-chained update model (`from`→`to`), and a full-archive fallback. Verification is a pure function over bytes — no network, no filesystem coupling — so it is unit-testable under plain `zig build test` with vendored key/signature fixtures.
2. **`nd package` orchestration in TypeScript/Bun** (`tools/package.ts`, invoked as `nd package <platform>`): builds the platform bundle (AppImage / `.app`), signs it (minisign for the update archive; ad-hoc or Developer-ID `codesign` for the `.app`), and emits a signed update manifest. Platform tools are called out to; Zig stays the verification core only.
3. **Update-fetch driver + local manifest server** (`tools/update-server.ts`, `scripts/m9-drive.ts`): a Bun HTTP server that serves a signed manifest + archive over `http://127.0.0.1:<port>` and a driver that exercises the full verify→download→stage flow against it, headless.
4. **CI wiring + docs**: extend `.github/workflows/` with a Linux packaging job and a (non-blocking) macOS packaging job; `docs/packaging.md`; `CLAUDE-activeContext.md` M9 entry.

**Non-goals (M9):** Windows anything; real bsdiff/zstd deltas (deferred, M9-D3); a live production update server (the CI server is a test fixture); a running notarization against real Apple credentials (gated on secrets that don't exist here); any schema / React / NDP-protocol / `include/nd.h` change.

---

## Global Constraints

Copied verbatim from the spec + `CLAUDE-activeContext.md`. **Every task's requirements implicitly include this section.**

- **Zig 0.16.0 exactly** (`build.zig` panics otherwise). **Bun 1.3.13**, React 19.2.7 — all pinned.
- **`nd` is a documented convention, not a binary yet.** `nd codegen` ≡ `bun tools/codegen.ts` (see `template/README.md`, `docs/agents/README.md`). M9 adds `nd package` ≡ `bun tools/package.ts` by the *same* convention — a Bun entrypoint under `tools/`, documented, not a shipped CLI wrapper. Do NOT invent a `bin/nd` dispatcher.
- **`include/nd.h` is FROZEN** (M6a-D1). The update core is a new module reachable from `src/core/root.zig`; it adds **no** vtable field and **no** ABI export unless a task explicitly says so (T2 adds exactly one export, `nd_verify_update_manifest`, gated behind a decision).
- **Flakes only see git-tracked files.** `git add` every new file (including `flake.nix` additions and new tools) **before** any `nix develop`/`zig build`/build step, or the build won't see it.
- **The full existing gate must stay green** before and after every task:

```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md \
  && zig build test \
  && zig build \
  && bun install --frozen-lockfile \
  && ./scripts/headless-smoke.sh \
  && ./scripts/headless-m2.sh \
  && ./scripts/kill9-test.sh \
  && ./scripts/headless-m3.sh \
  && ./scripts/headless-m4.sh \
  && ./scripts/headless-m5b.sh \
  && ./scripts/headless-m5c.sh \
  && ./scripts/headless-m8.sh'
```

- **Markers print to stderr; scripts capture `2>&1`.** M9 markers: `ND_UPDATE_*` (core), `M9_*` (drivers/scripts), `MAC_M9_*` (Mac legs). Follow the `ND_*`/`M<n>_*` precedent exactly.
- **Each headless script uses a UNIQUE weston socket name** to avoid CI collisions (`nd-headless-m9`, `nd-headless-m9-update`, …). Never reuse another script's socket.
- **Commit style:** short imperative lowercase subject, conventional prefix (`feat(update):`, `feat(package):`, `feat(mac):`, `ci(package):`, `docs(packaging):`). No co-author trailers. `git add` **explicit paths per task** — never `git add -A`. `node_modules`/caches/`CLAUDE*.md` are never staged.

### Hard facts for implementers (carried from `CLAUDE-activeContext.md` — do NOT rediscover)

**Zig 0.16 API drift (blogs and training data are wrong):**
- **No `std.heap.GeneralPurposeAllocator`.** Aux allocators come from the juicy-main `init.gpa`. For a pure unit-tested module use `std.testing.allocator` in tests.
- **No `std.posix.getenv`/`unlink`.** Sockets live in `std.Io.net`. `std.Io.Mutex` needs `.init` (a `self.* = undefined` skips field defaults and bricks it).
- **`std.time.milliTimestamp` and `std.Thread.sleep` are GONE.** Use `std.Io.sleep(io, .fromMilliseconds(n), .awake)` and poll-count bounds instead of wall-clock.
- **File reads moved under `std.Io` (writergate).** `readFileAlloc` lives on `std.Io.Dir` and takes an `io` param + a `.limited(N)` size cap — `build.zig` uses `b.build_root.handle.readFileAlloc(b.graph.io, "schema/widgets.json", b.allocator, .limited(1 << 20))`. The update core does **not** read files itself (it verifies byte slices handed in); file I/O stays in the Bun/test-harness layer. Keep it that way — it's what makes the core unit-testable with no `io`.
- **`zig build test` does NOT collect tests transitively through `@import`** — every test-bearing file needs its own `addTest` root in `build.zig`, or its tests are silently skipped (this bit `style.zig`). The new `src/core/update.zig` needs its own `addTest` root wired into `test_step`.
- **`@embedFile` cannot cross a module's package-path boundary** (the directory of its `root_source_file`). Test fixtures for `update.zig` that live under `src/core/` (a sibling of the module root) embed fine; fixtures outside it must be handed in as build options (the `schema_json` pattern in `build.zig:159`) — prefer keeping fixtures under `src/core/testdata/`.

**Ed25519 / minisign (verified against the vendored std this session):**
- `std.crypto.sign.Ed25519` exists. `Ed25519.PublicKey.fromBytes([32]u8)`, `Ed25519.Signature.fromBytes([64]u8)`, and `signature.verify(msg, public_key) VerifyError!void` (or `verifier(pk)` for streaming) are the exact call shapes (`std/crypto/25519/ed25519.zig:114,254,275`). `PublicKey.encoded_length == 32`, `Signature.encoded_length == 64`.
- `std.crypto.hash.blake2.Blake2b512` exists (`Blake2b512.hash(bytes, &out, .{})`, `digest_length == 64`) — this is minisign's **prehash** (`ED` algorithm tag) digest.
- `std.base64.standard.Decoder` decodes minisign's base64 lines.
- **Minisign format truth (state this, don't hand-wave):** a minisign `.minisig` file is two base64 lines each preceded by an untrusted/trusted comment line. The first base64 blob decodes to `signature_algorithm[2] ‖ key_id[8] ‖ signature[64]`. Algorithm `Ed` (0x45 0x64) = sign the raw message; `ED` (0x45 0x44) = sign **Blake2b-512(message)** (prehashed — required for large files). M9 uses the **prehashed `ED`** form for archives (archives are large) and can use either for the small manifest; the core verifies whichever the 2-byte tag declares. The public key file decodes to `signature_algorithm[2] ‖ key_id[8] ‖ public_key[32]`; the `key_id` must match the signature's `key_id`.

**Mac dev loop (M6b facts — reproduce, do NOT re-probe):**
- **Mac login shell is FISH.** All remote commands go through `ssh macbook 'bash -euo pipefail -s' <<'REMOTE' … REMOTE` heredocs (never rely on `&&` chains in fish). ssh prints harmless port-forward bind noise — filter it, never treat it as an error.
- **Toolchain is the profile PATH, NOT nix.** `zig 0.16.0` + `bun 1.3.13` at `/etc/profiles/per-user/kyandesutter/bin`; every Mac script `export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"`. `swiftc`/`swift`/`xcrun`/`codesign`/`xcodebuild` come from Xcode. **`nix` is NOT usable non-interactively over ssh.**
- **GUI session uid is 502** (not 501). Use `$(id -u)`, never a hard-coded uid.
- **`ar`/`libtool` repack recipe (if a Mac leg relinks `libnd.a`):** zig's archiver emits `.a` members Apple's `ld` rejects ("not 8-byte aligned", 0-permission on extract). Repack before any swiftc link: `ar x libnd.a && chmod 644 *.o && libtool -static -o libnd.a *.o` (feeding the archive to `libtool` directly silently drops members). `build.zig` already sets `libnd.bundle_compiler_rt = true` (required — swiftc's link lacks Zig's implicit compiler-rt). **M9 does not modify the ABI or relink logic; it packages the already-built `NDShell` binary.**
- **`mac-sync.sh`** rsyncs the tree to `~/nd` on the Mac via a bare repo + `mac` remote and re-sets `remote.origin.fetch` every sync. All Mac M9 legs start with `"$(dirname "$0")/mac-sync.sh"`.

**Automation driver reuse:** `scripts/m6-drive.ts` and `packages/mcp/src/socket.ts`'s `AutomationClient` (`AutomationClient.connect()` reads `ND_AUTOMATION_SOCKET`; `.call(method, params)`) are the pattern for any post-launch smoke of a packaged app.

---

## M9 decision record (owner-facing judgment calls, locked by this plan)

- **M9-D1 — Update verification lives in Zig; download + staging live in Bun/TS.** The **non-disableable signature check** (spec §11) is a pure Zig function (`verifyMinisign(pubkey, message, sig_blob) -> bool`) that never touches the network or filesystem — it verifies byte slices. The Bun layer (`tools/package.ts` producer, `tools/update-server.ts` + `scripts/m9-drive.ts` consumer) does the HTTP fetch and the atomic file swap, then calls the Zig verifier (in CI, via a tiny Zig verify CLI, `zig build update-verify`, whose whole job is `read bytes → verify → exit 0/1`). Rationale: keeps the security-critical primitive in the audited core with zero I/O surface, keeps churny network/FS code in the disposable scripting layer, and makes the verifier unit-testable with fixtures under `zig build test`.
- **M9-D2 — Full-archive updates ship; deltas are deferred.** The shipping update payload is a **full compressed archive** (`.tar.zst` on Linux via the `zstd` CLI from the devshell; `.tar.gz` on macOS via system `tar`), signed with minisign. The manifest declares a single `full` artifact per version plus an optional (unused-in-v1) `delta` array. **zig-bsdiff + zstd deltas chaining from the previous version are DEFERRED** and documented as such in `docs/packaging.md` — the manifest schema reserves the `delta` field so adding deltas later is additive, not a flag day. This satisfies "auto-updating" honestly (verify-download-swap works end-to-end) without pulling a bsdiff dependency into scope. **State this deferral explicitly in the docs task.**
- **M9-D3 — macOS `.app`: ad-hoc codesign is the working path; notarization is gated on secrets.** `tools/package.ts mac` deep-signs the nested binaries (the `NDShell` executable, the bundled `libnd`-derived Mach-O, the Bun runtime, any `.dylib`) inside-out then the `.app` last, with the hardened runtime + a `com.apple.security.cs.allow-jit` entitlement plist (JSC-under-Bun needs it on Apple Silicon, spec §11). **Signing identity resolution:** if `APPLE_SIGN_IDENTITY` is set → use it (`codesign -s "$APPLE_SIGN_IDENTITY" --options runtime --entitlements …`); else → **ad-hoc (`codesign -s - --options runtime --entitlements …`)**. **Truth about ad-hoc + hardened runtime (state it in docs):** `--options runtime` (hardened runtime) IS accepted with an ad-hoc (`-s -`) signature by `codesign`, and the entitlements ARE embedded, but the OS only *enforces* hardened-runtime protections for signatures backed by a valid Team ID — an ad-hoc-signed app still launches locally (and over ssh in-session) and `codesign --verify` passes, which is all the Mac leg asserts. Notarization (`xcrun notarytool submit` + `xcrun stapler staple`) runs **only** when all of `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` are present; otherwise it's skipped with an `ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials` marker. No credentials exist in this environment, so the skip path is what runs.
- **M9-D4 — Linux: AppImage is built + smoke-launched headlessly; the Flatpak manifest is committed + lint-validated but its full build is CI-only-on-a-real-runner.** `flatpak-builder` inside a nix sandbox / stock CI is notoriously fragile (portal + runtime + bubblewrap requirements). So M9 Linux packaging = **(a)** a working **AppImage** assembled by `tools/package.ts linux` (AppDir + `appimagetool`/squashfs from the devshell) and **launched headlessly under weston in CI** to prove it runs, plus **(b)** a committed, **`flatpak-builder --show-manifest`-lint-validated** `packaging/flatpak/<app-id>.yml` (GNOME 50 runtime; automation documented as in-process-first since portals gate everything else) whose *full* `flatpak-builder` build is documented as *"CI-only on a real GNOME runner"* and NOT run in the M9 gate. **State this scoping explicitly** in `docs/packaging.md`.
- **M9-D5 — Update flow is tested headlessly against a local Bun HTTP server; zero network.** `tools/update-server.ts` serves the manifest + archive over `http://127.0.0.1:<ephemeral-port>`; `scripts/m9-drive.ts` fetches, runs the Zig verifier (`zig build update-verify`), asserts a **tampered** archive/manifest is REJECTED and a **valid** one is ACCEPTED, and asserts the atomic-swap staging produces the expected files. **No task may depend on `github.com`, `ziglang.org`, or any remote host at test time** — CI runners fetch nothing beyond the pinned toolchains the existing jobs already install.

---

## File structure (decomposition)

**New files, by owner (no two tasks in the same wave share a file):**

| Path | Owner | Responsibility |
|---|---|---|
| `src/core/update.zig` | T2 | minisign/Ed25519 verify + manifest parse (pure, no I/O) |
| `src/core/testdata/*.pub`,`*.minisig`,`*.manifest.json` | T2 | signature fixtures (generated by `minisign`, committed) |
| `src/core/update_verify_main.zig` | T2 | tiny `zig build update-verify` CLI (bytes-in → 0/1 out) |
| `build.zig` | **T1 (sole owner)** | wire `update.zig` test root + `update-verify` exe/step |
| `flake.nix` | **T1 (sole owner)** | add `minisign`, `zstd`, `squashfsTools`, `flatpak-builder` |
| `tools/package.ts` | T3 (Linux) + T4 (mac) — **split by function, see waves** | `nd package <platform>` |
| `tools/manifest.ts` | T3 | manifest builder + minisign signing shared helper |
| `packaging/flatpak/<app-id>.yml` | T3 | committed Flatpak manifest (lint-validated) |
| `packaging/AppDir.template/` | T3 | AppImage AppDir skeleton (`.desktop`, `AppRun`, icon) |
| `packaging/macos/Info.plist`,`entitlements.plist` | T4 | `.app` bundle metadata + allow-jit entitlement |
| `tools/update-server.ts` | T5 | local Bun manifest/archive HTTP server (test fixture) |
| `scripts/m9-drive.ts` | T5 | update verify/download/stage driver |
| `scripts/headless-m9.sh` | T6 (sole) | Linux headless M9 gate (AppImage launch + update flow) |
| `scripts/mac/mac-m9.sh` | T6 (sole) | Mac `.app` package + ad-hoc sign + launch + update-verify |
| `docs/packaging.md` | T7 | packaging + updates doc (scoping, deferrals, Windows note) |
| `.github/workflows/package.yml` | T7 (sole) | one packaging workflow (Linux job + non-blocking mac job) |
| `CLAUDE-activeContext.md` | T7 | M9 entry (NOT committed — memory-bank exclusion) |

**`tools/package.ts` contention:** the Linux packer (T3) and the mac packer (T4) both want `tools/package.ts`. To keep them on **disjoint files within a wave**, `tools/package.ts` is a **thin dispatcher created by T3** (`switch(argv) { linux, mac }`) that imports `tools/package-linux.ts` (T3) and `tools/package-mac.ts` (T4). T3 owns `tools/package.ts` + `tools/package-linux.ts` + `tools/manifest.ts`; T4 owns `tools/package-mac.ts`. They never touch the same file. (The dispatcher lands in T3's wave with a stub `mac` arm that T4 fills — T4 only edits `tools/package-mac.ts` and re-exports; see T4.)

---

## WAVE STRUCTURE

- **WAVE 0 (serial, spine):** **T1** — build-graph + devshell wiring. Sole owner of `build.zig` and `flake.nix` (the two high-contention files). Everything else depends on the `update-verify` step and the devshell tools existing. **Must land and be green before Wave 1 starts.**
- **WAVE 1 (parallel — disjoint files):** **T2** (`src/core/update.zig` + testdata + `update_verify_main.zig`), **T3** (`tools/package.ts` + `tools/package-linux.ts` + `tools/manifest.ts` + `packaging/flatpak/*` + `packaging/AppDir.template/*`), **T4** (`tools/package-mac.ts` + `packaging/macos/*`). These three share **no** files. (T3 creates the `tools/package.ts` dispatcher with a stub `mac` arm; T4 fills `tools/package-mac.ts` which the dispatcher imports — the dispatcher file itself is T3's and is written mac-ready, so T4 never edits it.)
- **WAVE 2 (serial, join):** **T5** — update-flow driver + local server. Depends on T2 (`zig build update-verify`) and T3 (`tools/manifest.ts` to produce a signed manifest fixture). Owns `tools/update-server.ts` + `scripts/m9-drive.ts`.
- **WAVE 3 (parallel — disjoint files):** **T6** — the two acceptance scripts (`scripts/headless-m9.sh`, `scripts/mac/mac-m9.sh`). Depends on T3/T4 (packers) + T5 (driver). Both scripts are disjoint files and can be written by one subagent sequentially or two in parallel.
- **WAVE 4 (serial, END):** **T7** — CI workflow + docs + activeContext. Depends on everything green. Owns `.github/workflows/package.yml`, `docs/packaging.md`, `CLAUDE-activeContext.md`.

---

## TASK 1 — Build-graph + devshell wiring (WAVE 0, sole owner of `build.zig` + `flake.nix`)

**Files:** Modify `build.zig` (add the `update.zig` test root + the `update-verify` exe/step — placeholders that compile against T2's module surface), Modify `flake.nix` (add packaging tools). **This task owns both high-contention files so no later task edits them.**

**Interfaces produced:**
- `zig build update-verify` — an installed exe `zig-out/bin/nd-update-verify` (rooted at `src/core/update_verify_main.zig`, which T2 fills). T1 wires the build step referencing that path; the file is created by T2. **To keep T1 green independently, T1 creates a MINIMAL committed stub of `src/core/update_verify_main.zig` and `src/core/update.zig` that compiles** (T2 replaces the bodies). The stub declares the exact public surface T2 must keep:
  - `pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool`
  - `pub const Manifest = struct { app_id: []const u8, version: []const u8, from: ?[]const u8, full_url: []const u8, full_sig_b64: []const u8 };`
  - `pub fn parseManifest(gpa: std.mem.Allocator, json: []const u8) !Manifest`
- devshell gains `minisign`, `zstd`, `squashfsTools`, `flatpak-builder` (all confirmed present in nixpkgs this session).

- [ ] **Step 1: Create the compile-only stub `src/core/update.zig`** (T2 replaces every body; the *signatures* are the contract):

```zig
const std = @import("std");

pub const Manifest = struct {
    app_id: []const u8,
    version: []const u8,
    from: ?[]const u8 = null,
    full_url: []const u8,
    full_sig_b64: []const u8,
};

/// Verify a minisign signature blob over `message` with a minisign public-key blob.
/// STUB (T1): always false. T2 implements Ed25519 + Blake2b512-prehash verification.
pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool {
    _ = pubkey_blob;
    _ = message;
    _ = sig_blob;
    return false;
}

/// Parse the update manifest JSON. STUB (T1): T2 implements with std.json.
pub fn parseManifest(gpa: std.mem.Allocator, json: []const u8) !Manifest {
    _ = gpa;
    _ = json;
    return error.NotImplemented;
}

test "update stub compiles" {
    try std.testing.expect(!verifyMinisign("", "", ""));
}
```

- [ ] **Step 2: Create the compile-only stub `src/core/update_verify_main.zig`** — the CLI T2 fills. Reads `--pubkey <file> --message <file> --sig <file>`, calls `verifyMinisign`, exits 0 (valid) / 1 (invalid). STUB body prints usage + exits 2 so T1's build is green without asserting behavior:

```zig
const std = @import("std");
const update = @import("update.zig");

pub fn main() !u8 {
    // STUB (T1): T2 implements arg parsing + file reads (std.Io.Dir.readFileAlloc)
    // + verifyMinisign + exit-code contract (0 valid / 1 invalid / 2 usage).
    std.debug.print("ND_UPDATE_VERIFY stub\n", .{});
    return 2;
}
```

- [ ] **Step 3: Wire the test root + the exe/step in `build.zig`.** Add, next to the other `addTest` roots (after `abi_tests`, before the `libnd` block), a dedicated test root for `update.zig` (Zig 0.16: tests are NOT collected transitively — this is mandatory) and the `update-verify` exe:

```zig
    // Update-verification core (M9): its own addTest root — Zig 0.16 does not
    // collect `test {}` blocks transitively through @import, so without this
    // update.zig's minisign tests are silently skipped.
    const update_tests = b.addTest(.{
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/core/update.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    test_step.dependOn(&b.addRunArtifact(update_tests).step);

    // `nd-update-verify` (M9): bytes-in → exit 0/1 CLI wrapping verifyMinisign,
    // used by scripts/m9-drive.ts to run the non-disableable signature check.
    const update_verify_mod = b.createModule(.{
        .root_source_file = b.path("src/core/update_verify_main.zig"),
        .target = target,
        .optimize = optimize,
    });
    const update_verify_exe = b.addExecutable(.{ .name = "nd-update-verify", .root_module = update_verify_mod });
    const update_verify_step = b.step("update-verify", "Build nd-update-verify (minisign check CLI)");
    update_verify_step.dependOn(&b.addInstallArtifact(update_verify_exe, .{}).step);
    b.installArtifact(update_verify_exe);
```

(Note: `update_verify_main.zig` uses `@import("update.zig")` — a same-directory relative import inside `src/core/`, which is legal because both files share the `src/core/` root. No named-module gymnastics needed.)

- [ ] **Step 4: Add packaging tools to `flake.nix`.** In the Linux-only `optionals` list add `squashfsTools` and `flatpak-builder`; add `minisign` and `zstd` to the **common** `packages` list (both platforms need `minisign` to sign, `zstd` for the Linux archive — but `minisign` is cross-platform and used in tests on Linux). The AppImage tool: nixpkgs exposes `appimagekit`/`appimage-run`; if `appimagetool` is not directly attr-named, the implementer builds the AppImage via `squashfsTools` (`mksquashfs`) + a runtime stub, OR resolves the correct attr with `nix search nixpkgs appimage` — **wire whichever resolves; do not block.** Edit:

```nix
            packages = with pkgs; [
              zig
              zls
              bun
              pkg-config
              minisign     # M9: sign/verify update manifests + archives
              zstd         # M9: .tar.zst full-archive updates (Linux)
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              gtk4
              libadwaita
              glib
              gobject-introspection
              libxslt
              weston
              squashfsTools    # M9: AppImage assembly (mksquashfs)
              flatpak-builder  # M9: Flatpak manifest lint (--show-manifest)
            ];
```

- [ ] **Step 5: `git add` the new + changed files, then verify the build graph is green.** Flakes only see tracked files:

```bash
git add build.zig flake.nix src/core/update.zig src/core/update_verify_main.zig
nix develop -c bash -c 'zig build test 2>&1 | tail -5 && zig build update-verify 2>&1 | tail -3 && ls -la zig-out/bin/nd-update-verify && zig build 2>&1 | tail -3'
```

Expected: `zig build test` passes (the stub test "update stub compiles" runs — proving the new test root is wired), `zig build update-verify` produces `zig-out/bin/nd-update-verify`, and the full `zig build` still succeeds. The devshell now has `minisign`/`zstd` on PATH:

```bash
nix develop -c bash -c 'command -v minisign && command -v zstd && command -v flatpak-builder'
```

Expected: all three resolve.

- [ ] **Step 6: Commit.**

```bash
git commit -m "build(update): wire update.zig test root + nd-update-verify + packaging devshell tools"
```

---

## TASK 2 — Minisign/Ed25519 update-verification core (WAVE 1)

**Files:** Replace bodies in `src/core/update.zig` (T1 stub → real), Replace `src/core/update_verify_main.zig` (T1 stub → real CLI), Create `src/core/testdata/` fixtures (`test.pub`, `msg-valid.txt`, `msg-valid.txt.minisig`, `msg-tampered.txt`, `sample.manifest.json`). **Depends on: T1 (build wiring + stub signatures). Owns only `src/core/*` — disjoint from T3/T4.**

Implement the **non-disableable** minisign-compatible verifier (spec §11). Pure functions over byte slices — no network, no filesystem inside `update.zig` (the CLI does the file reads). Tests run under plain `zig build test` (T1 wired the root) with committed fixtures generated by the real `minisign` CLI (now on the devshell PATH from T1).

**Interfaces produced (keep these signatures EXACTLY — T1's stub declared them, T5's driver consumes them via the CLI):**
- `pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool` — `pubkey_blob` = the raw bytes of the second (base64) line of a minisign `.pub`, already base64-decoded to `algo[2]‖keyid[8]‖pk[32]`; `sig_blob` = the base64-decoded first signature line `algo[2]‖keyid[8]‖sig[64]`. Returns true iff the key_ids match and the Ed25519 signature verifies over (message | Blake2b512(message)) per the algo tag.
- `pub fn parseManifest(gpa, json) !Manifest`
- `nd-update-verify --pubkey <f> --message <f> --sig <f>` → exit `0` valid / `1` invalid / `2` usage.

- [ ] **Step 1: Generate + commit the test fixtures with the real `minisign` CLI.** (Do this first — the tests embed them.) Deterministic, committed, no password:

```bash
mkdir -p src/core/testdata
nix develop -c bash -c '
  cd src/core/testdata
  # -W = no password (test key only; NEVER a real signing key).
  minisign -G -W -p test.pub -s test.sec
  printf "native-desktop update payload v1\n" > msg-valid.txt
  cp msg-valid.txt msg-tampered.txt && printf "X" >> msg-tampered.txt
  # -S signs; default (no -H) is the prehashed ED form on recent minisign — but
  # pin it explicitly: sign the message; the .minisig records the algo tag.
  minisign -S -s test.sec -m msg-valid.txt
  rm test.sec   # do NOT commit the secret key
  ls -la
'
```

Expected files committed: `test.pub`, `msg-valid.txt`, `msg-valid.txt.minisig`, `msg-tampered.txt`. **`test.sec` is deleted, never committed.** (State in a comment in `update.zig` that these are throwaway test keys.)

- [ ] **Step 2: Write the failing tests in `src/core/update.zig`** (embed the fixtures; `@embedFile` works because they're under `src/core/`, the module root's directory):

```zig
const std = @import("std");
const Ed25519 = std.crypto.sign.Ed25519;
const Blake2b512 = std.crypto.hash.blake2.Blake2b512;

const test_pub_file = @embedFile("testdata/test.pub");
const valid_msg = @embedFile("testdata/msg-valid.txt");
const valid_sig_file = @embedFile("testdata/msg-valid.txt.minisig");
const tampered_msg = @embedFile("testdata/msg-tampered.txt");

// Decode the second (base64) line of a minisign .pub / .minisig file.
fn decodeMinisignBlob(alloc: std.mem.Allocator, file: []const u8) ![]u8 {
    var it = std.mem.splitScalar(u8, file, '\n');
    _ = it.next(); // untrusted/trusted comment line
    const b64 = std.mem.trim(u8, it.next() orelse return error.BadFormat, " \r\t");
    const dec = std.base64.standard.Decoder;
    const n = try dec.calcSizeForSlice(b64);
    const out = try alloc.alloc(u8, n);
    try dec.decode(out, b64);
    return out;
}

test "valid minisign signature verifies" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file); defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file); defer a.free(sig);
    try std.testing.expect(verifyMinisign(pk, valid_msg, sig));
}

test "tampered message is rejected" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file); defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file); defer a.free(sig);
    try std.testing.expect(!verifyMinisign(pk, tampered_msg, sig));
}

test "key_id mismatch is rejected" {
    const a = std.testing.allocator;
    const pk = try decodeMinisignBlob(a, test_pub_file); defer a.free(pk);
    const sig = try decodeMinisignBlob(a, valid_sig_file); defer a.free(sig);
    var bad_pk = try a.dupe(u8, pk); defer a.free(bad_pk);
    bad_pk[2] ^= 0xFF; // corrupt the key_id
    try std.testing.expect(!verifyMinisign(bad_pk, valid_msg, sig));
}

test "parseManifest reads app_id/version/full" {
    const a = std.testing.allocator;
    const json =
        \\{"app_id":"com.nativedesktop.gallery","version":"1.2.0","from":"1.1.0",
        \\ "full_url":"http://127.0.0.1:9/full.tar.zst","full_sig_b64":"AAAA"}
    ;
    const m = try parseManifest(a, json);
    try std.testing.expectEqualStrings("com.nativedesktop.gallery", m.app_id);
    try std.testing.expectEqualStrings("1.2.0", m.version);
    try std.testing.expectEqualStrings("1.1.0", m.from.?);
}
```

- [ ] **Step 3: Run the tests to confirm they FAIL against the stub.**

```bash
nix develop -c zig build test 2>&1 | tail -15
```

Expected: FAIL — the stub `verifyMinisign` returns false, so "valid minisign signature verifies" fails; `parseManifest` returns `error.NotImplemented`.

- [ ] **Step 4: Implement `verifyMinisign` + `parseManifest`** (replace the T1 stub bodies). Blobs are already base64-decoded by the caller (the tests decode; the CLI decodes):

```zig
const KEY_ID_LEN = 8;
const ALGO_LEN = 2;

pub fn verifyMinisign(pubkey_blob: []const u8, message: []const u8, sig_blob: []const u8) bool {
    // pubkey_blob: algo[2] ‖ key_id[8] ‖ pk[32]  (42 bytes)
    // sig_blob:    algo[2] ‖ key_id[8] ‖ sig[64]  (74 bytes)
    if (pubkey_blob.len != ALGO_LEN + KEY_ID_LEN + Ed25519.PublicKey.encoded_length) return false;
    if (sig_blob.len != ALGO_LEN + KEY_ID_LEN + Ed25519.Signature.encoded_length) return false;

    const pk_keyid = pubkey_blob[ALGO_LEN .. ALGO_LEN + KEY_ID_LEN];
    const sig_algo = sig_blob[0..ALGO_LEN];
    const sig_keyid = sig_blob[ALGO_LEN .. ALGO_LEN + KEY_ID_LEN];
    if (!std.mem.eql(u8, pk_keyid, sig_keyid)) return false; // wrong signer

    var pk_bytes: [Ed25519.PublicKey.encoded_length]u8 = undefined;
    @memcpy(&pk_bytes, pubkey_blob[ALGO_LEN + KEY_ID_LEN ..]);
    var sig_bytes: [Ed25519.Signature.encoded_length]u8 = undefined;
    @memcpy(&sig_bytes, sig_blob[ALGO_LEN + KEY_ID_LEN ..]);

    const pk = Ed25519.PublicKey.fromBytes(pk_bytes) catch return false;
    const sig = Ed25519.Signature.fromBytes(sig_bytes);

    // Algo tag: "Ed" (0x45,0x64) signs the raw message; "ED" (0x45,0x44) signs
    // Blake2b-512(message) (minisign's prehashed form for large files).
    if (sig_algo[0] == 'E' and sig_algo[1] == 'd') {
        sig.verify(message, pk) catch return false;
        return true;
    } else if (sig_algo[0] == 'E' and sig_algo[1] == 'D') {
        var digest: [Blake2b512.digest_length]u8 = undefined;
        Blake2b512.hash(message, &digest, .{});
        sig.verify(&digest, pk) catch return false;
        return true;
    }
    return false; // unknown algorithm tag
}

pub fn parseManifest(gpa: std.mem.Allocator, json: []const u8) !Manifest {
    const Parsed = struct {
        app_id: []const u8,
        version: []const u8,
        from: ?[]const u8 = null,
        full_url: []const u8,
        full_sig_b64: []const u8,
    };
    const parsed = try std.json.parseFromSlice(Parsed, gpa, json, .{ .ignore_unknown_fields = true });
    // Caller owns `parsed`; but Manifest fields point into parsed.value's arena.
    // For the test/CLI single-shot use, leak-free is not required (arena dies at
    // process exit); if a caller needs persistence it must dupe. Return copies to
    // be safe:
    return Manifest{
        .app_id = try gpa.dupe(u8, parsed.value.app_id),
        .version = try gpa.dupe(u8, parsed.value.version),
        .from = if (parsed.value.from) |f| try gpa.dupe(u8, f) else null,
        .full_url = try gpa.dupe(u8, parsed.value.full_url),
        .full_sig_b64 = try gpa.dupe(u8, parsed.value.full_sig_b64),
    };
}
```

**If any of the `Ed25519`/`Blake2b512`/`base64` call shapes above is rejected by the compiler, verify against the vendored std FIRST** (`std/crypto/25519/ed25519.zig`, `std/crypto/blake2.zig`, `std/base64.zig`) — the shapes were read from that std this session (`PublicKey.fromBytes`, `Signature.fromBytes`, `sig.verify(msg, pk)`, `Blake2b512.hash(b, &out, .{})`) but a point release may drift; the fix is a signature adjustment, never a hallucinated API.

- [ ] **Step 5: Run the tests to confirm they PASS.**

```bash
nix develop -c zig build test 2>&1 | tail -15
```

Expected: PASS — all four update tests green (valid verifies, tampered rejected, key-id mismatch rejected, manifest parses).

- [ ] **Step 6: Implement the `update-verify` CLI** (`src/core/update_verify_main.zig`) — reads three files, base64-decodes the pubkey/sig blobs (minisign second lines), calls `verifyMinisign`, exits per the contract. Use `std.Io.Dir.readFileAlloc` (0.16 writergate — file reads take `io` + `.limited(N)`):

```zig
const std = @import("std");
const update = @import("update.zig");

fn readAll(io: std.Io, path: []const u8, gpa: std.mem.Allocator) ![]u8 {
    return std.Io.Dir.cwd().readFileAlloc(io, path, gpa, .limited(64 << 20));
}

fn decodeMinisignSecondLine(gpa: std.mem.Allocator, file: []const u8) ![]u8 {
    var it = std.mem.splitScalar(u8, file, '\n');
    _ = it.next();
    const b64 = std.mem.trim(u8, it.next() orelse return error.BadFormat, " \r\t");
    const dec = std.base64.standard.Decoder;
    const n = try dec.calcSizeForSlice(b64);
    const out = try gpa.alloc(u8, n);
    try dec.decode(out, b64);
    return out;
}

pub fn main() !u8 {
    // juicy-main gives us init.gpa + init.io in Zig 0.16 — pull them the way the
    // rest of the codebase does (mirror src/gtk/main.zig / src/abi.zig's main
    // startup for the exact `init` shape; do NOT hand-roll GeneralPurposeAllocator,
    // it's gone in 0.16). Parse `--pubkey <f> --message <f> --sig <f>`.
    // ... arg parse ...
    const pk_file = try readAll(io, pubkey_path, gpa);
    const msg = try readAll(io, message_path, gpa);
    const sig_file = try readAll(io, sig_path, gpa);
    const pk = try decodeMinisignSecondLine(gpa, pk_file);
    const sig = try decodeMinisignSecondLine(gpa, sig_file);
    if (update.verifyMinisign(pk, msg, sig)) {
        std.debug.print("ND_UPDATE_VERIFY_OK\n", .{});
        return 0;
    }
    std.debug.print("ND_UPDATE_VERIFY_FAIL\n", .{});
    return 1;
}
```

**The exact `init.gpa`/`init.io` acquisition must mirror an existing `main`** (`src/gtk/main.zig` or wherever the codebase pulls the juicy-main handles) — read that file's opening before writing this, don't guess the `init` struct shape.

- [ ] **Step 7: Verify the CLI end-to-end against the committed fixtures.**

```bash
nix develop -c bash -c '
  zig build update-verify 2>&1 | tail -2
  ./zig-out/bin/nd-update-verify --pubkey src/core/testdata/test.pub --message src/core/testdata/msg-valid.txt --sig src/core/testdata/msg-valid.txt.minisig; echo "valid exit=$?"
  ./zig-out/bin/nd-update-verify --pubkey src/core/testdata/test.pub --message src/core/testdata/msg-tampered.txt --sig src/core/testdata/msg-valid.txt.minisig; echo "tampered exit=$?"
'
```

Expected: valid → `ND_UPDATE_VERIFY_OK` + `valid exit=0`; tampered → `ND_UPDATE_VERIFY_FAIL` + `tampered exit=1`.

- [ ] **Step 8: Commit.**

```bash
git add src/core/update.zig src/core/update_verify_main.zig src/core/testdata/test.pub src/core/testdata/msg-valid.txt src/core/testdata/msg-valid.txt.minisig src/core/testdata/msg-tampered.txt
git commit -m "feat(update): minisign/ed25519 verifier + parseManifest + verify CLI"
```

---

## TASK 3 — Linux packaging: `nd package linux` (AppImage) + Flatpak manifest + manifest signer (WAVE 1)

**Files:** Create `tools/package.ts` (the `nd package <platform>` dispatcher, mac-ready), Create `tools/package-linux.ts`, Create `tools/manifest.ts` (shared minisign-signing manifest builder), Create `packaging/AppDir.template/{AppRun,gallery.desktop,gallery.png}`, Create `packaging/flatpak/com.nativedesktop.gallery.yml`. **Depends on: T1 (devshell tools). Owns `tools/package.ts`, `tools/package-linux.ts`, `tools/manifest.ts`, `packaging/AppDir.template/*`, `packaging/flatpak/*` — disjoint from T2 (`src/core/*`) and T4 (`tools/package-mac.ts`, `packaging/macos/*`).**

`nd package linux` (≡ `bun tools/package.ts linux`) assembles an AppImage of the gallery app and emits a **minisign-signed update manifest + full archive**. Per M9-D4, the Flatpak manifest is committed + lint-validated but its full build is out of the M9 gate.

**Interfaces produced (T5 consumes `tools/manifest.ts`):**
- `bun tools/package.ts linux` → writes `dist/linux/Gallery-<version>.AppImage`, `dist/update/gallery-<version>-linux.tar.zst`, `dist/update/gallery-<version>-linux.tar.zst.minisig`, `dist/update/manifest-linux.json` (+ `.minisig`).
- `tools/manifest.ts`: `export async function buildAndSignManifest(opts: { appId, version, from, archivePath, url, secKey, pubKey }): Promise<{ manifestPath: string, manifestSigPath: string }>` — hashes/signs the archive with `minisign`, writes the manifest JSON matching `update.zig`'s `Manifest` (`app_id`,`version`,`from`,`full_url`,`full_sig_b64`), and signs the manifest itself. `full_sig_b64` = the base64 of the archive's `.minisig` second line (the raw `algo‖keyid‖sig` blob) so the Zig verifier can decode it directly.

- [ ] **Step 1: Create the `tools/package.ts` dispatcher** (mac-ready so T4 never edits it):

```typescript
#!/usr/bin/env bun
// nd package <platform>  (nd convention: nd package ≡ bun tools/package.ts).
// Linux → AppImage + signed update archive/manifest (M9-D4).
// mac   → .app + deep codesign (ad-hoc or Developer-ID) + gated notarize (M9-D3).
import { packageLinux } from "./package-linux.ts";
import { packageMac } from "./package-mac.ts";

const platform = process.argv[2];
if (platform === "linux") {
  await packageLinux();
} else if (platform === "mac") {
  await packageMac();
} else {
  console.error("usage: bun tools/package.ts <linux|mac>  (Windows lands with M7)");
  process.exit(2);
}
```

- [ ] **Step 2: Create `tools/manifest.ts`** — the shared minisign-signing manifest builder. Shells out to the `minisign` CLI (on PATH from T1). Generate a throwaway signing keypair per package run (or read `ND_MINISIGN_SEC`/`ND_MINISIGN_PUB` if set) — the CI/test flow uses an ephemeral key it also hands to the verifier:

```typescript
import { $ } from "bun";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

export interface ManifestOpts {
  appId: string; version: string; from: string | null;
  archivePath: string; url: string; secKey: string; pubKey: string;
}
// Returns { manifestPath, manifestSigPath }.
export async function buildAndSignManifest(o: ManifestOpts) {
  // Sign the archive (minisign -S writes <archive>.minisig). -W = no password.
  await $`minisign -S -s ${o.secKey} -m ${o.archivePath}`.quiet();
  const archiveSig = `${o.archivePath}.minisig`;
  // The second line of the .minisig is the base64 algo‖keyid‖sig blob the Zig
  // verifier decodes directly — pass it through as full_sig_b64.
  const sigB64 = readFileSync(archiveSig, "utf8").split("\n")[1]!.trim();
  const manifest = {
    app_id: o.appId, version: o.version, from: o.from,
    full_url: o.url, full_sig_b64: sigB64,
  };
  const manifestPath = `${o.archivePath.replace(/[^/]+$/, "")}manifest-${o.version}.json`;
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
  await $`minisign -S -s ${o.secKey} -m ${manifestPath}`.quiet();
  return { manifestPath, manifestSigPath: `${manifestPath}.minisig` };
}

// Ephemeral key helper for CI/tests (no real signing identity in this env).
export async function ensureEphemeralKey(dir: string) {
  const sec = `${dir}/nd-sign.sec`, pub = `${dir}/nd-sign.pub`;
  if (!existsSync(sec)) await $`minisign -G -W -p ${pub} -s ${sec}`.quiet();
  return { sec, pub };
}
```

- [ ] **Step 3: Create the AppDir skeleton.** `packaging/AppDir.template/gallery.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=NativeDesktop Gallery
Exec=AppRun
Icon=gallery
Categories=Development;
```

`packaging/AppDir.template/AppRun` (executable; launches the host binary with the gallery script — the AppImage bundles `nd-hello` + the gallery example + the Bun runtime):

```bash
#!/usr/bin/env bash
HERE="$(dirname "$(readlink -f "$0")")"
export ND_SCRIPT="$HERE/app/examples/gallery/main.tsx"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/nd-hello" "$@"
```

(`gallery.png` — a placeholder 1x1 or the project icon; if the repo has an icon set, use it per the CLAUDE.md "never hardcode SVG, use the project's icon set" rule — otherwise a minimal committed PNG placeholder is acceptable for the AppImage to assemble.)

- [ ] **Step 4: Write `tools/package-linux.ts`** — assemble the AppDir (copy `zig-out/bin/nd-hello`, the Bun binary, `examples/gallery`, `packages/*` deps needed at runtime, the AppDir template), run `appimagetool` (or `mksquashfs` + a runtime stub if `appimagetool` isn't attr-resolved — mirror whatever T1 wired), then produce the `.tar.zst` full archive and sign it via `tools/manifest.ts`:

```typescript
import { $ } from "bun";
import { mkdirSync, cpSync, chmodSync } from "node:fs";
import { buildAndSignManifest, ensureEphemeralKey } from "./manifest.ts";

const VERSION = process.env.ND_APP_VERSION ?? "0.9.0";
const APP_ID = "com.nativedesktop.gallery";

export async function packageLinux() {
  const dist = "dist/linux", updDir = "dist/update";
  mkdirSync(dist, { recursive: true });
  mkdirSync(updDir, { recursive: true });
  const appdir = `${dist}/AppDir`;
  cpSync("packaging/AppDir.template", appdir, { recursive: true });
  mkdirSync(`${appdir}/usr/bin`, { recursive: true });
  cpSync("zig-out/bin/nd-hello", `${appdir}/usr/bin/nd-hello`);
  chmodSync(`${appdir}/usr/bin/nd-hello`, 0o755);
  chmodSync(`${appdir}/AppRun`, 0o755);
  // Bundle the app source (the gallery is script-driven, not compiled).
  mkdirSync(`${appdir}/app`, { recursive: true });
  cpSync("examples", `${appdir}/app/examples`, { recursive: true });
  cpSync("packages", `${appdir}/app/packages`, { recursive: true });
  // Bundle the Bun runtime the host spawns.
  const bunPath = (await $`command -v bun`.text()).trim();
  cpSync(bunPath, `${appdir}/usr/bin/bun`);
  chmodSync(`${appdir}/usr/bin/bun`, 0o755);

  // AppImage assembly. appimagetool if available; else mksquashfs.
  const appImage = `${dist}/Gallery-${VERSION}.AppImage`;
  await $`appimagetool ${appdir} ${appImage}`.quiet()
    .catch(async () => { await $`mksquashfs ${appdir} ${appImage} -root-owned -noappend`.quiet(); });
  console.log(`ND_PACKAGE_APPIMAGE ${appImage}`);

  // Full-archive update payload (.tar.zst), signed (M9-D2).
  const archive = `${updDir}/gallery-${VERSION}-linux.tar.zst`;
  await $`tar -C ${dist} -cf - AppDir | zstd -q -o ${archive}`.quiet();
  const { sec, pub } = await ensureEphemeralKey(updDir);
  const { manifestPath } = await buildAndSignManifest({
    appId: APP_ID, version: VERSION, from: null,
    archivePath: archive, url: `http://127.0.0.1:0/${archive.split("/").pop()}`,
    secKey: sec, pubKey: pub,
  });
  console.log(`ND_PACKAGE_MANIFEST ${manifestPath} pub=${pub}`);
}
```

- [ ] **Step 5: Create the committed Flatpak manifest** `packaging/flatpak/com.nativedesktop.gallery.yml` (GNOME 50 runtime; automation in-process-first per §11). This is committed + lint-validated only (M9-D4):

```yaml
app-id: com.nativedesktop.gallery
runtime: org.gnome.Platform
runtime-version: "50"
sdk: org.gnome.Sdk
command: nd-hello
finish-args:
  - --socket=wayland
  - --socket=fallback-x11
  - --device=dri
# Automation is in-process-first (spec §11): NO portal permissions requested —
# the in-process getTree/screenshot path needs none. The optional OS-input mode
# (libei/portals) is out of scope; document that it would add finish-args here.
modules:
  - name: nd-gallery
    buildsystem: simple
    build-commands:
      - install -Dm755 nd-hello /app/bin/nd-hello
    sources:
      - type: dir
        path: ../../zig-out/bin
# NOTE (M9-D4): the FULL flatpak-builder build is CI-only on a real GNOME
# runner. The M9 gate only lint-validates this manifest (flatpak-builder
# --show-manifest). See docs/packaging.md.
```

- [ ] **Step 6: Verify (AppImage assembles + manifest signs + Flatpak manifest lints).** `git add` first (flakes):

```bash
git add tools/package.ts tools/package-linux.ts tools/manifest.ts packaging/AppDir.template packaging/flatpak
nix develop -c bash -c '
  zig build 2>&1 | tail -2
  ND_APP_VERSION=0.9.0 bun tools/package.ts linux 2>&1 | tail -8
  ls -la dist/linux dist/update
  # Flatpak manifest lints (does NOT build):
  flatpak-builder --show-manifest packaging/flatpak/com.nativedesktop.gallery.yml >/dev/null && echo "FLATPAK_MANIFEST_OK"
'
```

Expected: `ND_PACKAGE_APPIMAGE …`, `ND_PACKAGE_MANIFEST …`, a `.AppImage` + `.tar.zst` + `.minisig` + `manifest-0.9.0.json` under `dist/`, and `FLATPAK_MANIFEST_OK`. (If `--show-manifest` isn't the exact lint subcommand in this flatpak-builder version, use `flatpak-builder --show-manifest` or `flatpak run org.flatpak.Builder --show-manifest` — resolve the working invocation; the assertion is that the YAML parses.)

- [ ] **Step 7: Commit.**

```bash
git add tools/package.ts tools/package-linux.ts tools/manifest.ts packaging/AppDir.template packaging/flatpak
git commit -m "feat(package): nd package linux (AppImage + signed update archive) + flatpak manifest"
```

**Note:** `dist/` is build output — add it to `.gitignore` if not already ignored (check first; do not commit `dist/`).

---

## TASK 4 — macOS packaging: `nd package mac` (.app + deep codesign + gated notarize) (WAVE 1)

**Files:** Create `tools/package-mac.ts`, Create `packaging/macos/Info.plist`, Create `packaging/macos/entitlements.plist`. **Depends on: T1 (devshell) + the T3 dispatcher contract (`packageMac` export). Owns `tools/package-mac.ts`, `packaging/macos/*` — disjoint from T2 and T3.** (T3's `tools/package.ts` already imports `./package-mac.ts` — T4 creates that file; T4 never edits the dispatcher.)

`nd package mac` builds a `.app` bundle around `swift/.build/release/NDShell` + the Zig core + Bun + the gallery, deep-signs the nested binaries inside-out (ad-hoc `codesign -s -` by default; Developer-ID if `APPLE_SIGN_IDENTITY` is set) with the hardened runtime + `com.apple.security.cs.allow-jit`, and runs `notarytool`+`stapler` **only** when Apple credentials are present (M9-D3). This runs **on the Mac over ssh** (codesign/xcrun are Mac-only) — the smoke of it is T6's `mac-m9.sh`. T4's own verify runs the packer on the Mac.

**Interfaces produced (T6's `mac-m9.sh` consumes):**
- `packageMac()` (exported from `tools/package-mac.ts`) → writes `dist/mac/Gallery.app` (signed), plus `dist/update/gallery-<version>-mac.tar.gz` (+ `.minisig`, `manifest-mac.json`) via `tools/manifest.ts`.

- [ ] **Step 1: Create `packaging/macos/entitlements.plist`** — the allow-jit entitlement (JSC-under-Bun crashes on Apple Silicon without it, spec §11):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.allow-jit</key>
    <true/>
    <key>com.apple.security.cs.allow-unsigned-executable-memory</key>
    <true/>
</dict>
</plist>
```

- [ ] **Step 2: Create `packaging/macos/Info.plist`** — the bundle metadata:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Gallery</string>
    <key>CFBundleDisplayName</key><string>NativeDesktop Gallery</string>
    <key>CFBundleIdentifier</key><string>com.nativedesktop.gallery</string>
    <key>CFBundleExecutable</key><string>NDShell</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.9.0</string>
    <key>CFBundleVersion</key><string>0.9.0</string>
    <key>LSMinimumSystemVersion</key><string>26.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
```

- [ ] **Step 3: Write `tools/package-mac.ts`.** Assemble `Gallery.app/Contents/{MacOS,Resources,Frameworks}`, copy `NDShell` + the Bun runtime + the gallery app + `Info.plist`, then **deep-sign inside-out** (nested Mach-O first, `.app` last) with hardened runtime + entitlements, resolving the identity per M9-D3, and gate notarization on credential presence:

```typescript
import { $ } from "bun";
import { mkdirSync, cpSync, chmodSync } from "node:fs";
import { buildAndSignManifest, ensureEphemeralKey } from "./manifest.ts";

const VERSION = process.env.ND_APP_VERSION ?? "0.9.0";
const APP_ID = "com.nativedesktop.gallery";

export async function packageMac() {
  const dist = "dist/mac", updDir = "dist/update";
  const app = `${dist}/Gallery.app`, c = `${app}/Contents`;
  mkdirSync(`${c}/MacOS`, { recursive: true });
  mkdirSync(`${c}/Resources`, { recursive: true });
  mkdirSync(`${c}/Frameworks`, { recursive: true });
  mkdirSync(updDir, { recursive: true });

  cpSync("packaging/macos/Info.plist", `${c}/Info.plist`);
  cpSync("swift/.build/release/NDShell", `${c}/MacOS/NDShell`);
  chmodSync(`${c}/MacOS/NDShell`, 0o755);
  const bunPath = (await $`command -v bun`.text()).trim();
  cpSync(bunPath, `${c}/MacOS/bun`); chmodSync(`${c}/MacOS/bun`, 0o755);
  cpSync("examples", `${c}/Resources/examples`, { recursive: true });
  cpSync("packages", `${c}/Resources/packages`, { recursive: true });

  // Signing identity (M9-D3): Developer-ID if set, else ad-hoc.
  const identity = process.env.APPLE_SIGN_IDENTITY ?? "-";
  const ent = "packaging/macos/entitlements.plist";
  // Deep-sign inside-out: nested Mach-O first, the .app last.
  for (const nested of [`${c}/MacOS/bun`, `${c}/MacOS/NDShell`]) {
    await $`codesign --force --options runtime --entitlements ${ent} --sign ${identity} ${nested}`;
  }
  await $`codesign --force --deep --options runtime --entitlements ${ent} --sign ${identity} ${app}`;
  await $`codesign --verify --strict ${app}`;
  console.log(`ND_PACKAGE_APP_SIGNED ${app} identity=${identity === "-" ? "adhoc" : "developer-id"}`);

  // Notarization gated on credentials (M9-D3).
  const { APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD } = process.env;
  if (APPLE_ID && APPLE_TEAM_ID && APPLE_APP_PASSWORD) {
    const zip = `${dist}/Gallery.zip`;
    await $`ditto -c -k --keepParent ${app} ${zip}`;
    await $`xcrun notarytool submit ${zip} --apple-id ${APPLE_ID} --team-id ${APPLE_TEAM_ID} --password ${APPLE_APP_PASSWORD} --wait`;
    await $`xcrun stapler staple ${app}`;
    console.log(`ND_PACKAGE_NOTARIZE_OK ${app}`);
  } else {
    console.log(`ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`);
  }

  // Full-archive update payload (.tar.gz on mac) + signed manifest (M9-D2).
  const archive = `${updDir}/gallery-${VERSION}-mac.tar.gz`;
  await $`tar -C ${dist} -czf ${archive} Gallery.app`;
  const { sec, pub } = await ensureEphemeralKey(updDir);
  const { manifestPath } = await buildAndSignManifest({
    appId: APP_ID, version: VERSION, from: null,
    archivePath: archive, url: `http://127.0.0.1:0/${archive.split("/").pop()}`,
    secKey: sec, pubKey: pub,
  });
  console.log(`ND_PACKAGE_MANIFEST ${manifestPath} pub=${pub}`);
}
```

- [ ] **Step 4: Verify (on the Mac over ssh — codesign/xcrun are Mac-only).** Sync, build `libnd` + the Swift shell, run the packer, assert an ad-hoc-signed `.app` that `codesign --verify` accepts + the notarize-skip marker:

```bash
./scripts/mac/mac-sync.sh
ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"
cd ~/nd
zig build libnd -Dbackend=abi >/dev/null 2>&1
# repack libnd.a for Apple ld (ar/libtool recipe — zig's archiver members are rejected):
workdir="$(mktemp -d)"; ( cd "$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o ); rm -rf "$workdir"
(cd swift && swift build -c release >/dev/null 2>&1)
bun install --frozen-lockfile >/dev/null 2>&1
ND_APP_VERSION=0.9.0 bun tools/package.ts mac 2>&1 | tail -8
codesign --verify --strict --verbose=2 dist/mac/Gallery.app && echo "CODESIGN_VERIFY_OK"
codesign -d --entitlements - dist/mac/Gallery.app 2>&1 | grep -q "allow-jit" && echo "ALLOW_JIT_PRESENT"
REMOTE
```

Expected: `ND_PACKAGE_APP_SIGNED … identity=adhoc`, `ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`, `CODESIGN_VERIFY_OK`, `ALLOW_JIT_PRESENT`, `ND_PACKAGE_MANIFEST …`. (If `codesign --verify --strict` rejects an ad-hoc + hardened-runtime combo on this OS, drop `--strict` for the ad-hoc path and record the exact message in `docs/packaging.md` — this is the one carried-forward macOS truth per M9-D3; verify empirically, don't assume.)

- [ ] **Step 5: Commit.**

```bash
git add tools/package-mac.ts packaging/macos/Info.plist packaging/macos/entitlements.plist
git commit -m "feat(mac): nd package mac (.app deep-sign + allow-jit + gated notarize)"
```

---

## TASK 5 — Update-flow driver + local manifest server (WAVE 2, join)

**Files:** Create `tools/update-server.ts` (Bun HTTP server serving a signed manifest + archive over `127.0.0.1`), Create `scripts/m9-drive.ts` (verify→download→stage driver). **Depends on: T2 (`zig build update-verify`) + T3 (`tools/manifest.ts`). Owns `tools/update-server.ts`, `scripts/m9-drive.ts` — disjoint from all others.**

Exercise the whole update path **headlessly with zero network** (M9-D5): serve a signed manifest + archive on an ephemeral localhost port, fetch, run the non-disableable Zig verifier, assert tamper-rejection + valid-acceptance, and stage an atomic swap.

**Interfaces produced (T6's `headless-m9.sh` runs `m9-drive.ts`):**
- `bun tools/update-server.ts <dir> <port>` — serves files from `<dir>` over `http://127.0.0.1:<port>`; prints `ND_UPDATE_SERVER_LISTENING port=<port>`.
- `bun scripts/m9-drive.ts <serverUrl> <pubKey> <stageDir>` — fetches `manifest-<version>.json` + the archive, runs `nd-update-verify`, asserts valid + rejects tampered, atomically swaps; prints `M9_UPDATE_OK` / exits non-zero on any verification failure.

- [ ] **Step 1: Write `tools/update-server.ts`** — a minimal Bun HTTP file server (test fixture only):

```typescript
#!/usr/bin/env bun
// Local update-manifest server (TEST FIXTURE — M9-D5). Serves a dir over
// 127.0.0.1 only. No network egress; the update flow test never leaves loopback.
import { join, normalize } from "node:path";
const dir = process.argv[2]!, port = Number(process.argv[3] ?? 0);
const server = Bun.serve({
  hostname: "127.0.0.1", port,
  fetch(req) {
    const path = normalize(new URL(req.url).pathname).replace(/^(\.\.[/\\])+/, "");
    const file = Bun.file(join(dir, path));
    return new Response(file);
  },
});
console.log(`ND_UPDATE_SERVER_LISTENING port=${server.port}`);
```

- [ ] **Step 2: Write `scripts/m9-drive.ts`** — the verify→download→stage driver. It downloads the archive + manifest, writes the pubkey and the archive-signature to temp files, runs `nd-update-verify`, and asserts the exit-code contract for both a valid and a tampered archive:

```typescript
#!/usr/bin/env bun
// Update-flow driver (M9-D5): fetch a signed manifest+archive from the local
// server, run the NON-DISABLEABLE Zig verifier (zig-out/bin/nd-update-verify),
// assert valid=accept + tampered=reject, then atomic-swap-stage. Zero network.
import { $ } from "bun";
import { writeFileSync, mkdirSync, renameSync, readFileSync } from "node:fs";

const [serverUrl, pubKey, stageDir] = process.argv.slice(2);
const version = process.env.ND_APP_VERSION ?? "0.9.0";
const tmp = `${stageDir}/.staging`; mkdirSync(tmp, { recursive: true });

// 1. Fetch the manifest + archive over loopback.
const manifestJson = await (await fetch(`${serverUrl}/manifest-${version}.json`)).text();
const manifest = JSON.parse(manifestJson);
const archiveName = manifest.full_url.split("/").pop()!;
const archiveBytes = new Uint8Array(await (await fetch(`${serverUrl}/${archiveName}`)).arrayBuffer());
const archivePath = `${tmp}/${archiveName}`;
writeFileSync(archivePath, archiveBytes);

// 2. Reconstruct a minisign .minisig file from manifest.full_sig_b64 so the Zig
//    CLI (which reads a .minisig second line) can verify. Line 1 = comment.
const sigPath = `${archivePath}.minisig`;
writeFileSync(sigPath, `untrusted comment: nd update\n${manifest.full_sig_b64}\n`);

// 3. Run the NON-DISABLEABLE verifier on the VALID archive → must exit 0.
const verify = "zig-out/bin/nd-update-verify";
const okProc = await $`${verify} --pubkey ${pubKey} --message ${archivePath} --sig ${sigPath}`.nothrow();
if (okProc.exitCode !== 0) { console.error("M9_UPDATE_FAIL valid archive rejected"); process.exit(1); }

// 4. Tamper the archive → verifier MUST reject (exit 1).
const tamperedPath = `${tmp}/tampered-${archiveName}`;
const tampered = new Uint8Array(archiveBytes); tampered[tampered.length - 1] ^= 0xFF;
writeFileSync(tamperedPath, tampered);
const badProc = await $`${verify} --pubkey ${pubKey} --message ${tamperedPath} --sig ${sigPath}`.nothrow();
if (badProc.exitCode === 0) { console.error("M9_UPDATE_FAIL tampered archive ACCEPTED — security hole"); process.exit(1); }

// 5. Atomic-swap staging: only a verified archive is promoted.
const staged = `${stageDir}/${archiveName}`;
renameSync(archivePath, staged);
console.log(`M9_UPDATE_OK verified+staged version=${version} staged=${staged}`);
```

- [ ] **Step 3: Verify the whole flow headlessly (produce a signed manifest via the Linux packer, serve it, drive it).** `git add` first:

```bash
git add tools/update-server.ts scripts/m9-drive.ts
nix develop -c bash -c '
  set -e
  zig build && zig build update-verify
  ND_APP_VERSION=0.9.0 bun tools/package.ts linux >/tmp/pkg.log 2>&1 || { cat /tmp/pkg.log; exit 1; }
  PUB=$(grep -m1 ND_PACKAGE_MANIFEST /tmp/pkg.log | sed "s/.*pub=//")
  # Serve dist/update on an ephemeral port.
  bun tools/update-server.ts dist/update 0 >/tmp/srv.log 2>&1 &
  SRV=$!; for _ in $(seq 1 50); do grep -q ND_UPDATE_SERVER_LISTENING /tmp/srv.log && break; sleep 0.1; done
  PORT=$(grep -m1 ND_UPDATE_SERVER_LISTENING /tmp/srv.log | sed "s/.*port=//")
  mkdir -p /tmp/m9-stage
  ND_APP_VERSION=0.9.0 bun scripts/m9-drive.ts "http://127.0.0.1:$PORT" "$PUB" /tmp/m9-stage 2>&1 | tail -5
  kill "$SRV" 2>/dev/null || true
'
```

Expected: `M9_UPDATE_OK verified+staged version=0.9.0 …` — the valid archive verified (exit 0), the tampered archive was rejected, and the archive was staged. No network access occurred (all `127.0.0.1`).

- [ ] **Step 4: Commit.**

```bash
git add tools/update-server.ts scripts/m9-drive.ts
git commit -m "feat(update): local manifest server + verify/download/stage driver (headless, zero-network)"
```

---

## TASK 6 — Acceptance scripts: `scripts/headless-m9.sh` + `scripts/mac/mac-m9.sh` (WAVE 3)

**Files:** Create `scripts/headless-m9.sh` (Linux headless M9 gate), Create `scripts/mac/mac-m9.sh` (Mac `.app` package + sign + launch + update-verify). **Depends on: T3, T4, T5. Both files disjoint.**

`headless-m9.sh` is the M9 analog of `headless-m4.sh`: it packages the AppImage, launches the packaged host headlessly under weston to prove the bundle runs, then runs the full update-flow (server + driver). `mac-m9.sh` mirrors `mac-m6.sh`: sync → build → `nd package mac` → assert the signed `.app` + the update flow on the Mac.

- [ ] **Step 1: Write `scripts/headless-m9.sh`.** Unique weston socket `nd-headless-m9` (never reuse another script's socket). Assert the AppImage assembles, the packaged host launches + presents a commit under weston, and `M9_UPDATE_OK`:

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
export WAYLAND_DISPLAY=nd-headless-m9
export GSK_RENDERER=cairo
export GDK_BACKEND=wayland
export ND_APP_VERSION=0.9.0

zig build >/dev/null 2>&1
zig build update-verify >/dev/null 2>&1

# 1. Package the Linux AppImage + signed update artifacts.
bun tools/package.ts linux >"$XDG_RUNTIME_DIR/pkg.log" 2>&1 || { echo "FAIL: package linux"; cat "$XDG_RUNTIME_DIR/pkg.log"; exit 1; }
grep -q ND_PACKAGE_APPIMAGE "$XDG_RUNTIME_DIR/pkg.log" || { echo "FAIL: no AppImage"; cat "$XDG_RUNTIME_DIR/pkg.log"; exit 1; }
PUB=$(grep -m1 ND_PACKAGE_MANIFEST "$XDG_RUNTIME_DIR/pkg.log" | sed 's/.*pub=//')

# 2. Launch the PACKAGED host headlessly (prove the bundle runs). We run the
#    AppDir's binary directly under weston (AppImage FUSE-mount is unavailable in
#    CI sandboxes; the assembled AppDir is the equivalent runnable tree).
weston --backend=headless --socket="$WAYLAND_DISPLAY" --idle-time=0 &
WESTON_PID=$!
trap 'kill "$WESTON_PID" 2>/dev/null || true; kill "${HOST_PID:-0}" 2>/dev/null || true; kill "${SRV_PID:-0}" 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break; sleep 0.1; done

LOG=$(mktemp)
ND_SCRIPT=dist/linux/AppDir/app/examples/gallery/main.tsx \
  ./dist/linux/AppDir/usr/bin/nd-hello >"$LOG" 2>&1 &
HOST_PID=$!
for _ in $(seq 1 80); do grep -q ND_COMMIT_APPLIED "$LOG" && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED "$LOG" || { echo "FAIL: packaged host did not present a commit"; cat "$LOG"; exit 1; }
kill -TERM "$HOST_PID" 2>/dev/null || true; wait "$HOST_PID" 2>/dev/null || true
echo "M9_PACKAGED_LAUNCH_OK"

# 3. Full update flow (verify/download/stage, zero-network — M9-D5).
bun tools/update-server.ts dist/update 0 >"$XDG_RUNTIME_DIR/srv.log" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 50); do grep -q ND_UPDATE_SERVER_LISTENING "$XDG_RUNTIME_DIR/srv.log" && break; sleep 0.1; done
PORT=$(grep -m1 ND_UPDATE_SERVER_LISTENING "$XDG_RUNTIME_DIR/srv.log" | sed 's/.*port=//')
mkdir -p "$XDG_RUNTIME_DIR/m9-stage"
bun scripts/m9-drive.ts "http://127.0.0.1:$PORT" "$PUB" "$XDG_RUNTIME_DIR/m9-stage" >"$XDG_RUNTIME_DIR/drive.log" 2>&1 \
  || { echo "FAIL: update driver"; cat "$XDG_RUNTIME_DIR/drive.log"; exit 1; }
cat "$XDG_RUNTIME_DIR/drive.log"
grep -q M9_UPDATE_OK "$XDG_RUNTIME_DIR/drive.log" || { echo "FAIL: update flow"; exit 1; }
kill "$SRV_PID" 2>/dev/null || true

echo "headless m9: OK (AppImage assembled, packaged host launched, update verify/stage green)"
```

- [ ] **Step 2: `chmod +x scripts/headless-m9.sh` and verify it end-to-end.**

```bash
chmod +x scripts/headless-m9.sh
git add scripts/headless-m9.sh
nix develop -c ./scripts/headless-m9.sh 2>&1 | tail -12
```

Expected: `M9_PACKAGED_LAUNCH_OK`, `M9_UPDATE_OK …`, then `headless m9: OK …`.

- [ ] **Step 3: Write `scripts/mac/mac-m9.sh`** — mirrors `mac-m6.sh` (sync → build → drive on the Mac). Packages the `.app`, asserts the ad-hoc-signed bundle + allow-jit, launches it headful-in-session to prove the packaged app runs, and runs the update-verify flow on the Mac (its own `nd-update-verify` built there):

```bash
#!/usr/bin/env bash
set -euo pipefail
"$(dirname "$0")/mac-sync.sh"

ssh macbook 'bash -euo pipefail -s' <<'REMOTE'
export PATH="/etc/profiles/per-user/kyandesutter/bin:$PATH"
cd ~/nd
zig build libnd -Dbackend=abi >/dev/null 2>&1
# Repack libnd.a for Apple ld (ar/libtool recipe — zig's members are rejected).
workdir="$(mktemp -d)"; ( cd "$workdir" && ar x ~/nd/zig-out/lib/libnd.a && chmod 644 *.o && libtool -static -o ~/nd/zig-out/lib/libnd.a *.o ); rm -rf "$workdir"
(cd swift && swift build -c release >/dev/null 2>&1)
zig build update-verify >/dev/null 2>&1
bun install --frozen-lockfile >/dev/null 2>&1

# 1. Package + deep-sign the .app (ad-hoc; notarize skipped — no creds here).
ND_APP_VERSION=0.9.0 bun tools/package.ts mac >/tmp/mac-pkg.log 2>&1 || { echo "FAIL package"; cat /tmp/mac-pkg.log; exit 1; }
cat /tmp/mac-pkg.log
grep -q ND_PACKAGE_APP_SIGNED /tmp/mac-pkg.log || { echo "FAIL not signed"; exit 1; }
grep -q ND_PACKAGE_NOTARIZE_SKIPPED /tmp/mac-pkg.log || { echo "FAIL notarize gate marker missing"; exit 1; }
codesign --verify dist/mac/Gallery.app && echo "MAC_M9_CODESIGN_OK"
codesign -d --entitlements - dist/mac/Gallery.app 2>&1 | grep -q allow-jit && echo "MAC_M9_ALLOW_JIT_OK"
PUB=$(grep -m1 ND_PACKAGE_MANIFEST /tmp/mac-pkg.log | sed 's/.*pub=//')

# 2. Launch the packaged .app headful-in-session; assert it presents a commit.
ND_SCRIPT=examples/gallery/main.tsx dist/mac/Gallery.app/Contents/MacOS/NDShell >/tmp/mac-app.log 2>&1 &
APP_PID=$!
for _ in $(seq 1 100); do grep -q ND_COMMIT_APPLIED /tmp/mac-app.log && break; sleep 0.1; done
grep -q ND_COMMIT_APPLIED /tmp/mac-app.log || { echo "FAIL packaged .app did not present"; cat /tmp/mac-app.log; kill "$APP_PID" 2>/dev/null; exit 1; }
kill -TERM "$APP_PID" 2>/dev/null || true; wait "$APP_PID" 2>/dev/null || true
echo "MAC_M9_LAUNCH_OK"

# 3. Update flow on the Mac (zero-network, loopback server).
bun tools/update-server.ts dist/update 0 >/tmp/mac-srv.log 2>&1 &
SRV=$!; for _ in $(seq 1 50); do grep -q ND_UPDATE_SERVER_LISTENING /tmp/mac-srv.log && break; sleep 0.1; done
PORT=$(grep -m1 ND_UPDATE_SERVER_LISTENING /tmp/mac-srv.log | sed 's/.*port=//')
mkdir -p /tmp/mac-m9-stage
ND_APP_VERSION=0.9.0 bun scripts/m9-drive.ts "http://127.0.0.1:$PORT" "$PUB" /tmp/mac-m9-stage 2>&1 | tail -4
kill "$SRV" 2>/dev/null || true
echo "MAC_M9_OK"
REMOTE
echo "MAC_M9_DONE"
```

- [ ] **Step 4: `chmod +x scripts/mac/mac-m9.sh` and verify on the Mac.**

```bash
chmod +x scripts/mac/mac-m9.sh
git add scripts/mac/mac-m9.sh
./scripts/mac/mac-m9.sh 2>&1 | tail -25
```

Expected: `ND_PACKAGE_APP_SIGNED … identity=adhoc`, `ND_PACKAGE_NOTARIZE_SKIPPED reason=no-credentials`, `MAC_M9_CODESIGN_OK`, `MAC_M9_ALLOW_JIT_OK`, `MAC_M9_LAUNCH_OK`, `M9_UPDATE_OK …`, `MAC_M9_OK`, `MAC_M9_DONE`. (If launching the packaged `.app`'s inner `NDShell` over a non-GUI ssh session fails on `NSApplication` — the M6b GUI-session caveat — run it in the uid-$(id -u) GUI session exactly as `mac-m6.sh` does, or if it still churns, mark the launch leg documented-and-deferred like M6b-T7 and keep the sign + update-verify legs as the Mac gate.)

- [ ] **Step 5: Commit.**

```bash
git add scripts/headless-m9.sh scripts/mac/mac-m9.sh
git commit -m "feat(package): headless-m9 (AppImage + update flow) + mac-m9 (.app sign + launch + verify)"
```

---

## TASK 7 — CI workflow + packaging docs + activeContext (WAVE 4, END)

**Files:** Create `.github/workflows/package.yml` (one packaging workflow: Linux job + non-blocking mac job), Create `docs/packaging.md`, Modify `CLAUDE-activeContext.md` (M9 entry — **NOT committed**, memory-bank exclusion). **Depends on: everything green.**

- [ ] **Step 1: Run the FULL existing gate + the M9 headless gate to confirm zero regression.**

```bash
nix develop -c bash -c 'bun tools/codegen.ts \
  && git diff --exit-code -- schema packages/react/src/generated src/generated docs/widgets.md docs/styling.md \
  && zig build test && zig build && bun install --frozen-lockfile \
  && ./scripts/headless-smoke.sh && ./scripts/headless-m2.sh && ./scripts/kill9-test.sh \
  && ./scripts/headless-m3.sh && ./scripts/headless-m4.sh && ./scripts/headless-m5b.sh \
  && ./scripts/headless-m5c.sh && ./scripts/headless-m8.sh && ./scripts/headless-m9.sh'
```

Expected: every leg green through `headless m9: OK`.

- [ ] **Step 2: Write `.github/workflows/package.yml`.** One workflow with a Linux packaging job (blocking, extends the `ci.yml` pattern — nix devshell, `headless-m9.sh`) and a **non-blocking** macOS packaging job (`continue-on-error: true`, mirrors `mac.yml`'s Zig+Bun+repack setup, runs `nd package mac` + `codesign --verify` on a stock runner). First live run deferred (no push access) — the `mac.yml` precedent:

```yaml
name: package
on:
  push: { branches: [main] }
  pull_request:

jobs:
  linux-package:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: DeterminateSystems/nix-installer-action@v16
      - uses: DeterminateSystems/magic-nix-cache-action@v8
      - name: build + install workspace
        run: |
          nix develop -c zig build
          nix develop -c zig build update-verify
          nix develop -c bun install --frozen-lockfile
      - name: package + update-flow gate (AppImage + verify/stage, zero-network)
        run: nix develop -c ./scripts/headless-m9.sh
      - name: flatpak manifest lint (build is real-runner-only, M9-D4)
        run: nix develop -c flatpak-builder --show-manifest packaging/flatpak/com.nativedesktop.gallery.yml >/dev/null

  # STRETCH, non-blocking (M9-D3): a red mac job must never block a merge.
  # First live run deferred (no GitHub push access from the authoring session).
  macos-package:
    runs-on: macos-latest
    continue-on-error: true
    steps:
      - uses: actions/checkout@v4
      - name: Install Zig 0.16.0
        run: |
          curl -sL https://ziglang.org/download/0.16.0/zig-aarch64-macos-0.16.0.tar.xz | tar xJ
          echo "$PWD/zig-aarch64-macos-0.16.0" >> "$GITHUB_PATH"
      - uses: oven-sh/setup-bun@v2
        with: { bun-version: 1.3.13 }
      - name: minisign (for update signing)
        run: brew install minisign
      - name: Build libnd.a (+ repack for Apple ld) + Swift shell
        run: |
          zig build libnd -Dbackend=abi
          workdir="$(mktemp -d)"
          ( cd "$workdir" && ar x "$GITHUB_WORKSPACE/zig-out/lib/libnd.a" && chmod 644 *.o && libtool -static -o "$GITHUB_WORKSPACE/zig-out/lib/libnd.a" *.o )
          rm -rf "$workdir"
          (cd swift && swift build -c release)
          zig build update-verify
      - name: bun install (workspace)
        run: bun install --frozen-lockfile
      - name: nd package mac (ad-hoc sign; notarize skipped — no creds)
        run: |
          ND_APP_VERSION=0.9.0 bun tools/package.ts mac
          codesign --verify dist/mac/Gallery.app && echo "CI_MAC_CODESIGN_OK"
      - uses: actions/upload-artifact@v4
        if: always()
        with: { name: mac-gallery-app, path: dist/mac/Gallery.app }
```

- [ ] **Step 3: Write `docs/packaging.md`** — the packaging + updates doc. **MUST state every deferral/scoping call explicitly** (the M9 decision record in prose): the Windows note, the delta-deferral (M9-D2/D3), the Flatpak-build-is-real-runner-only note (M9-D4), the ad-hoc-vs-notarized truth + the hardened-runtime-with-ad-hoc reality (M9-D3), and the zero-network test design (M9-D5). Include the `nd package linux` / `nd package mac` commands, the update-manifest schema (`app_id`/`version`/`from`/`full_url`/`full_sig_b64` + the reserved-for-later `delta` field), and how signature verification is non-disableable (verify runs unconditionally in the core; there is no flag to skip it). Cross-reference `scripts/headless-m9.sh` / `scripts/mac/mac-m9.sh`. One explicit sentence: **"Windows packaging (signed NSIS installer + winget manifest) lands with M7, which is not yet implemented."**

- [ ] **Step 4: Verify the workflow YAML parses + docs exist.**

```bash
nix develop -c bash -c '
  bun -e "import {parse} from \"yaml\"; parse(require(\"fs\").readFileSync(\".github/workflows/package.yml\",\"utf8\")); console.log(\"YAML_OK\")" 2>/dev/null \
    || python3 -c "import yaml,sys; yaml.safe_load(open(\".github/workflows/package.yml\")); print(\"YAML_OK\")"
  test -f docs/packaging.md && grep -q "lands with M7" docs/packaging.md && echo "DOCS_OK"
'
```

Expected: `YAML_OK` + `DOCS_OK`.

- [ ] **Step 5: Update `CLAUDE-activeContext.md`** — add an `**M9 done & green:**` bullet after the M6b entry summarizing what landed (minisign/Ed25519 verifier in `src/core/update.zig` + `nd-update-verify`; `nd package linux`=AppImage + signed `.tar.zst` update, `nd package mac`=`.app` deep-sign ad-hoc + allow-jit + gated notarize; committed lint-validated Flatpak manifest; zero-network update flow via `tools/update-server.ts` + `scripts/m9-drive.ts`; `scripts/headless-m9.sh` + `scripts/mac/mac-m9.sh`; `.github/workflows/package.yml` non-blocking mac job) and the M9 hard-won facts + explicit deferrals (deltas deferred, Windows deferred to M7, notarization gated on absent creds, Flatpak full build real-runner-only). Add `./scripts/headless-m9.sh` to the documented full-gate command. **Do NOT `git add`/commit `CLAUDE-activeContext.md`** (memory-bank exclusion rule).

- [ ] **Step 6: Commit (workflow + docs only — NOT activeContext).**

```bash
git add .github/workflows/package.yml docs/packaging.md
git commit -m "ci(package): one packaging workflow (linux gate + non-blocking mac) + packaging docs"
```

---

## Parallelism note (M9)

M9 is a **short spine with two fan-outs**:

- **WAVE 0 = T1 alone** (owns `build.zig` + `flake.nix`, the high-contention files — serialized deliberately). It ships compile-only stubs so the build graph is green before anything depends on it.
- **WAVE 1 = T2 ‖ T3 ‖ T4** — three disjoint islands: T2 owns `src/core/*`, T3 owns `tools/package.ts`+`tools/package-linux.ts`+`tools/manifest.ts`+`packaging/{AppDir.template,flatpak}/*`, T4 owns `tools/package-mac.ts`+`packaging/macos/*`. The only cross-reference is the `tools/package.ts` dispatcher (T3's file) importing `./package-mac.ts` (T4's file) — written mac-ready by T3 so T4 never edits it. Dispatch all three concurrently.
- **WAVE 2 = T5 alone** (join): needs T2's `nd-update-verify` + T3's `tools/manifest.ts`.
- **WAVE 3 = T6** (two disjoint scripts): needs T3/T4/T5.
- **WAVE 4 = T7 alone** (END): CI + docs + activeContext, after full-gate green.

**Linux vs Mac:** T1/T2/T3/T5/T6-Linux/T7 are authored + verified on **this Linux box**. T4 and T6-Mac build + verify on the Mac over `ssh macbook` (`mac-sync.sh` → `swift build` → `nd package mac`); this box orchestrates. The whole Linux gate stays green throughout (M9 adds files, changes no existing generated bytes, touches no schema/renderer).

---

## Self-review (plan-level)

- **Spec §11 coverage:** macOS `.app` + hardened runtime + `com.apple.security.cs.allow-jit` + deep-signing nested binaries + notarytool/staple → **T4** (notarize gated on absent creds per the binding scope, M9-D3). Linux Flatpak manifest (GNOME 50, in-process-first automation) + AppImage → **T3/T6** (M9-D4). Updates: minisign/Ed25519-verified manifests, non-disableable → **T2** (verification is a core function with no skip flag); zig-bsdiff+zstd deltas chaining from previous version → **DEFERRED, documented** (M9-D2, T7 docs); full-download fallback always hosted → **T3/T4** (the `full` archive is the shipping payload). Windows NSIS/winget/Azure → **explicitly OUT (M7)**, noted in docs.
- **§14 M9 definition ("one CI workflow"):** `.github/workflows/package.yml` with both the Linux gate and the non-blocking mac job → **T7**.
- **Zero-network test requirement:** `tools/update-server.ts` binds `127.0.0.1` only; `scripts/m9-drive.ts` fetches loopback; no task touches a remote host at test time → **T5/T6** (M9-D5).
- **Type consistency:** `Manifest` fields (`app_id`,`version`,`from`,`full_url`,`full_sig_b64`) are declared once in T1's stub, implemented in T2, produced by `tools/manifest.ts` (T3), and consumed by `scripts/m9-drive.ts` (T5) — same names throughout. `verifyMinisign(pubkey_blob, message, sig_blob) -> bool` is stable T1→T2, and `nd-update-verify --pubkey --message --sig` is the CLI T5 calls.
- **Disjoint-file waves:** no two tasks in any wave share a file; `build.zig`/`flake.nix`/`.github/workflows/package.yml` each have a single owning task (T1, T1, T7). The `tools/package.ts` contention is resolved by the dispatcher-owned-by-T3 / `package-mac.ts`-owned-by-T4 split.
- **No placeholders:** every code step shows real code; the two genuinely-unverifiable-from-Linux specifics (the exact `appimagetool` nixpkgs attr, and whether `codesign --verify --strict` accepts ad-hoc+hardened-runtime on this macOS) are flagged as *"resolve empirically, don't block; record the truth in docs"* — not left as silent TODOs.
- **Grounding:** every Zig API used (`Ed25519.PublicKey.fromBytes`, `Signature.fromBytes`/`.verify`, `Blake2b512.hash`, `std.base64.standard.Decoder`, `std.Io.Dir.readFileAlloc` with `io`+`.limited`) was read from the vendored 0.16 std this session; every devshell tool (`minisign`, `zstd`, `flatpak-builder`, `squashfsTools`) was confirmed present in nixpkgs; the Mac facts are the M6b-landed reproductions, not fresh probes.

---

## Landed-code cross-references (authoritative, `file:line`)

- Build graph to extend: `build.zig:178` (`abi_tests` — the pattern for the new `update_tests` root), `build.zig:193` (`libnd` — the exe/step pattern for `nd-update-verify`), `build.zig:159` (the `readFileAlloc(b.graph.io, …, .limited(…))` shape).
- Core module wiring: `src/core/root.zig` (how `libnd` re-exports + the comptime export-retention block — the model if T2's `nd_verify_update_manifest` ever needs ABI exposure; v1 does not).
- Zig std APIs (vendored, read this session): `std/crypto/25519/ed25519.zig:114` (`PublicKey.fromBytes`), `:254` (`Signature.fromBytes`), `:275` (`sig.verify(msg, pk)`); `std/crypto/blake2.zig:455` (`Blake2b512`), `:527` (`.hash`); `std/base64.zig:37` (`standard`).
- Headless script pattern to mirror: `scripts/headless-m4.sh` (weston socket, launch/wait-for-marker/drive/kill, unique socket name).
- Mac dev-loop pattern to mirror: `scripts/mac/mac-m6.sh` (sync → build-on-Mac → drive-headful-in-session → scp), `scripts/mac/mac-sync.sh` (bare-repo sync + refspec fix), `scripts/mac/mac-build.sh` (profile PATH, no nix, ar/libtool repack).
- CI precedent: `.github/workflows/ci.yml` (nix devshell Linux gate), `.github/workflows/mac.yml` (Zig+Bun install, `continue-on-error`, ar/libtool repack, first-run-deferred note).
- Automation reuse for smoke: `packages/mcp/src/socket.ts` `AutomationClient`, `scripts/m6-drive.ts`.
- Convention: `template/README.md:32` + `docs/agents/README.md:39` (`nd dev` documented convention; `nd package` follows the same `bun tools/*.ts` shape).
```
