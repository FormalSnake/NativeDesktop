# WASM Plugin Tier — Deferral Rationale + `nd_plugin_v1` Slot-In Design

**Date:** 2026-07-11
**Status:** Deferred, owner-visible descope. Not implemented in M10. No wasmtime/Extism dependency is added to the nix devshell, `build.zig.zon`, or any `package.json` by this document.
**References:** Design spec `docs/superpowers/specs/2026-07-09-nativedesktop-design.md` §9 ("Extensibility & security", decision D12) and §14 (M10 scope: "wasmtime/Extism tier"); plan `docs/superpowers/plans/2026-07-11-m10-plugins-hardening-binary.md` (Task 13, and the "owner-visible descopes" note against Task 8); landed code `include/nd_plugin.h` (committed at `5bddb99`, the frozen native-plugin ABI), `src/plugin.zig` (the native loader/dispatcher), `src/abi.zig` (`nd_load_plugin`), `src/runtime.zig`'s `handlePluginCommand` (the `pluginCommand`/`pluginResult` NDP frame path).

## 1. Status

The WASM/untrusted plugin tier described in design spec §9 point 3 ("Untrusted plugins: WASM components under wasmtime via Extism... deliberately minimal host-function surface") is **not built this milestone**. This is an explicit, owner-visible descope, not a silent gap: M10 ships the native-plugin tier (`nd_plugin_v1`, `dlopen`-loaded, first-party/audited) and the capability ACL that both tiers will share, but no wasmtime runtime, no Extism PDK, and no `.wasm` loading path exist in the tree after M10. This document exists so the eventual WASM tier has a concrete slot to land in rather than requiring a redesign of the plugin surface, and so the deferral itself is reviewable.

## 2. Why deferred

**Dependency weight vs. milestone scope.** M10's actual deliverables — the binary NDP encoding, the capability ACL at NDP dispatch, and the native `nd_plugin_v1` ABI with a demo plugin — are additive changes to an already-frozen core (`src/tree.zig`/`src/runtime.zig`/`src/automation.zig`/`src/protocol.zig` stay untouched; see the M10 plan's architecture note). wasmtime is not that kind of addition: it is a large Rust dependency (the wasmtime crate tree, plus Cranelift or Winch as the codegen backend) with no existing footprint in this repo's toolchain. The nix devshell pins Zig 0.16.0 and Bun 1.3.13 (design spec §14, M1) and nothing Rust-shaped; adding wasmtime means either vendoring a prebuilt `libwasmtime` C API archive or bringing a Rust toolchain into the flake, plus an Extism host SDK on top to get the plugin-manifest/PDK ergonomics design spec §9 assumes. Either path is its own milestone of toolchain and packaging work (nix flake surface, `nd package`'s per-platform bundling in design spec §11, CI images across all three backends) — it does not fit inside a wave already carrying the binary encoding and the ACL.

**The 2026 wasmtime CVE lesson.** Design spec §9 cites "2026 wasmtime CVEs show the host API is the real sandbox boundary" as the reason the host-function surface must be deliberately minimal, not merely "sandboxed because it's WASM." That lesson cuts against rushing an integration: a wasmtime embedding is only as safe as the host functions it exposes to guest code, and getting that surface right (least-privilege, no ambient authority, single narrow entry point) is a design exercise in its own right, not a checkbox next to "add wasmtime as a dependency." Landing it hastily under M10 time pressure — with a host-function surface designed to make the demo work rather than designed to be minimal — would reproduce exactly the class of mistake the CVE lesson is warning about. Doing this properly means: reading which CVEs classes have recurred (host-function-mediated escapes, not sandbox-escape-from-guest-bytecode issues, which wasmtime's compiler-level isolation already handles well), and shaping the host-function surface around that lesson before writing the loader. That is real design work that deserves its own milestone and its own review, not a rider on M10.

**Scope fit.** M10's own charter (design spec §14: "Plugins + hardening + binary fast path") already bundles three substantial, independent pieces of work. The native tier alone (ABI freeze, `dlopen` loader, ACL wiring, demo plugin, capability-denial test) exercises the entire capability-ACL mechanism end to end without needing a second, heavier loader to prove the design. Shipping the native tier first and validating the ACL against it is the correct order: the WASM tier's job (per §3 below) is to be a second loader behind the *same* proven surface, so proving that surface once, cheaply, before adding the expensive loader is the right sequencing, not a stall.

## 3. How it slots in: a second loader behind the same `nd_plugin_v1` surface

The design constraint that makes this deferral safe is that the WASM tier is not a parallel plugin system — it is **a second loader implementing the identical `nd_plugin_v1` contract**, so that everything downstream of "a plugin is loaded" (capability declarations, ACL checks, command dispatch, the `pluginCommand`/`pluginResult` NDP frame) is unchanged. Concretely, from `include/nd_plugin.h`:

```c
typedef struct nd_plugin_v1 {
  uint32_t abi_version;            /* must == ND_PLUGIN_ABI_VERSION */
  const char* name;                /* plugin identity, e.g. "hello" */
  const char* const* capabilities; /* NULL-terminated permission strings */
  int32_t (*init)(nd_plugin_registry*);
  void (*deinit)(void);
} nd_plugin_v1;
```

Today (M10), the only way to obtain an `nd_plugin_v1*` is the native loader in `src/plugin.zig`: `dlopen()` a `.so`/`.dylib`/`.dll`, resolve the `nd_plugin_entry` symbol, and call it to get a pointer to a statically-lived `nd_plugin_v1`. `src/abi.zig`'s `nd_load_plugin` is the one entry point the embedder calls to opt a plugin in (per `nd_plugin.h`'s header comment: "nothing loads a plugin unless the embedder calls `nd_load_plugin`").

The WASM tier's job is to add a **second implementation of that same acquisition step**: instead of `dlopen` + symbol resolution, a WASM loader instantiates an Extism plugin from a `.wasm` module and **adapts its exported functions to the same `nd_plugin_registry.register_command` surface**:

```c
struct nd_plugin_registry {
  void* host;
  void (*register_command)(nd_plugin_registry*, const char* command, nd_command_fn);
};
```

Concretely, a WASM loader (`src/plugin_wasm.zig`, not written by this document) would, at `init()` time, read the plugin's declared exports from the Extism manifest, and for each one call `registry->register_command(registry, name, adapter_fn)`, where `adapter_fn` is a small trampoline with the exact `nd_command_fn` signature (`int32_t (*)(const char* arg_json, char** result_out)`) that marshals the call into an Extism plugin invocation (arg JSON in, result JSON out) instead of a direct native function pointer call. From `src/runtime.zig`'s `handlePluginCommand` downward — ACL permission-string construction (`plugin:<plugin>.<command>`), the `isAllowed` gate, the `pluginCommand`/`pluginResult` frame shapes — **nothing changes**. A WASM plugin declares `capabilities` in its manifest exactly the way a native plugin declares them in its `nd_plugin_v1.capabilities` array, and is denied by the identical ACL check in `handlePluginCommand` (`src/runtime.zig:405-436`) if the permission isn't granted for the window. The dispatch code has no branch for "is this plugin native or WASM" below the loader boundary — that distinction is fully absorbed by which loader produced the `nd_plugin_v1*` in the first place.

This is why the deferral does not fork the plugin ABI: `nd_plugin_v1` and `nd_plugin_registry` are frozen now (M10, `include/nd_plugin.h`), and the WASM tier's entire job when it lands is to be a second producer of that same struct, never a second consumer-side contract.

## 4. The minimal host-function surface principle

Per §2's CVE lesson, the WASM loader's design must start from an explicit minimal host-function surface, not from "expose whatever's convenient and lock it down later." The principle: **the only host function a WASM guest plugin can call is the equivalent of `register_command`** (during `init`) and, transitively, whatever a registered command handler needs to return its result (writing the result bytes back to the host — the WASM analogue of `nd_command_fn` writing to `*result_out`). No ambient filesystem access, no ambient network access, no ambient access to other plugins' registries, no host clock/environment/process APIs unless a future capability explicitly grants and audits one. This mirrors the native tier's own shape — a native plugin only receives the `nd_plugin_registry*` handed to it at `init`, and only ever gets called back through `nd_command_fn` — so the WASM tier doesn't need a *wider* host surface than native plugins already have; if anything it should be narrower, since a `.wasm` module is by definition less trusted than an first-party/audited native `.so`. Concretely, this ADR-level rule: the Extism host-function table the loader registers with the wasmtime `Store`/`Linker` contains one function group (register/respond) and nothing else — no `extism_pdk` conveniences that reach outside that group get wired up. Any future ask for a plugin to touch the filesystem or network becomes a new, explicitly named, explicitly ACL-gated capability string (`plugin:<name>.<cap>`), never an ambient host import.

## 5. Manifest-feeds-codegen follow-up (not built in M10)

Design spec §9 also promises "a metadata manifest feeds codegen so TS types appear automatically" for native plugins; the M10 plan's Task 8 explicitly scopes that down to "minimum viable: the plugin registers a command reachable from JS and denied without its capability" and defers the manifest/codegen half as a documented follow-up — this document is that follow-up note. Neither native nor WASM plugins get a `tools/codegen.ts`-fed TS type for their commands in M10; the demo plugin's `greet` command (and any future WASM-plugin command) is reachable today only via the untyped `pluginCommand` NDP frame (`{"type":"pluginCommand","plugin":"hello","command":"greet","arg":...}`) from the runtime side, with the result arriving as an untyped `pluginResult` frame. When the manifest-feeds-codegen work lands (a later milestone, unscheduled), it should cover both loaders identically: a plugin manifest (native: sidecar JSON or a query API on `nd_plugin_v1`; WASM: the Extism manifest already carries export signatures) becomes codegen input the same way `widgets.schema.json` feeds widget-type codegen today (design spec §6), so `pluginCommand("hello", "greet", {...})` becomes a typed call instead of a raw frame construction.

## 6. Acceptance test when the WASM tier lands

When a future milestone implements the WASM loader, the bar for "done" is a test that exercises the loader-swap property directly, not just "a wasm plugin runs":

1. **Same demo, second loader.** The existing `plugins/hello` demo plugin (or an equivalent minimal plugin) is compiled to a `.wasm` module exposing the same `greet` command and the same `plugin:hello.greet` capability declaration used by the native version. A test loads it via the WASM loader instead of `dlopen`, sends the same `pluginCommand` NDP frame the native-plugin test sends, and asserts byte-identical `pluginResult` output — proving the loader swap is invisible above `nd_plugin_v1`.
2. **Capability denial parity.** The same ACL-denial test that exists for the native tier (a `pluginCommand` sent without the window being granted `plugin:hello.greet`, asserting `ND_ACL_DENY` + the `error` frame per `src/runtime.zig`'s `handlePluginCommand`) is re-run against the WASM-loaded plugin and must deny identically — same permission string, same error frame shape, same log line.
3. **Host-surface negative test.** A deliberately adversarial `.wasm` test plugin that attempts to import a host function outside the minimal surface (§4) — e.g. a fabricated `extism_pdk` filesystem or network import — fails to instantiate (import resolution error at load time), not merely "fails to have an effect at runtime." This is the concrete, automatable form of the CVE lesson: the sandbox boundary is enforced at link/instantiate time by the narrowness of the `Linker`'s import table, and the test proves the table stays narrow.
4. **ABI version parity.** The WASM loader reads/validates `abi_version == ND_PLUGIN_ABI_VERSION` exactly as `src/plugin.zig` does for native plugins (see the existing `abi_version` mismatch test in `src/plugin.zig`), so an ABI bump is caught identically regardless of loader.

Until that milestone, this document — plus the frozen `nd_plugin_v1`/`nd_plugin_registry` structs in `include/nd_plugin.h` — is the complete specification of where the WASM tier attaches.
