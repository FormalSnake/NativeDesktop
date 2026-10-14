+++
title = "Testing"
description = "The three test tiers, goldens, real input, the nd-test locators, the MCP bridge and how nd test runs them."
+++

<!-- docs:example hello -->

Every backend builds one semantic tree: role, name, value, test id, bounds, enabled, focused,
checked, selected, expanded. Tests read and act on that tree, never on pixels or the OS
accessibility API, so a locator written in tier 1 reads the same against a real window in tier 3.

| Tier | What runs | Needs |
| --- | --- | --- |
| 1 | Logic on `TestBackend`, no OS | Nothing; also runs in headless Chromium |
| 2 | In-process real backends: audits and goldens | A GPU or the window server for AppKit |
| 3 | The app as its own process, real input | A window server (a headless Hyprland on Linux) |

The rules for writing tests as an agent are in [for agents](../agents/testing.md).

## Tier 1: logic

`#[nd::test]` runs a test on `TestBackend`, which records backend ops and has a deterministic
clock. The same test compiles for `wasm32-unknown-unknown` and runs in headless Chromium
(`just web-test`).

<!-- docs:snippet examples/hello/tests/tier1.rs#tier1 -->

Locators work in tier 1 through `nd_test::TestHarness`, which wraps a `TestApp` and runs requests
through the same handler the automation socket uses (see
[crates/nd-test](../../../crates/nd-test/src/in_process.rs)).

## Tier 2: backends and goldens

Tier 2 renders with a real backend inside the test process. Three kinds of check live here.

The appearance audit is covered [below](#appearance-audit). Goldens compare a screenshot with a
PNG in the example's `tests/goldens/`. A mismatch past the tolerance writes the difference beside
the golden (`web-light.diff.png`, or `<golden>.actual.png`) and fails. To re-record:

- `ND_BLESS=1`: the web golden of `hello` ([web_golden.rs](../../../examples/hello/tests/web_golden.rs)). It runs only with `ND_WEB_GOLDEN=1` and needs Chrome (`ND_CHROME` picks another).
- `ND_UPDATE_GOLDENS=1`: the macOS goldens of the gallery and overrides examples, which capture the composited window including glass ([goldens.rs](../../../examples/gallery/tests/goldens.rs)).

<!-- docs:snippet examples/hello/tests/web_golden.rs#bless -->

The third kind is the AppKit conformance tests: `harness = false` binaries, one process per test,
that open offscreen windows and check each component's native mapping (an `NSMenu`, an
`NSTableView`). Glass only appears in tier 3 captures.

Fluent goldens on Windows can only be recorded on Windows. The Windows backend has not run yet,
so none exist.

## Appearance audit

`nd::audit::Audit` renders every window you name in light, dark and two matugen themes on each
backend the target links. It reads back every text run and fill that was painted and fails when a
colour is not a theme token (or a state derived from one), or when text has less than WCAG
contrast against what it sits on: 4.5:1, or 3:1 for large text. Each backend is its own test, such
as `appearance_audit::appkit`. Matugen themes run on GPUI only.

<!-- docs:snippet examples/hello/tests/appearance.rs#audit -->

The test target is declared with `harness = false` in `Cargo.toml`. `Audit::allow` lists brand
colours the theme names on purpose; contrast is still checked. `nd check` runs the audit for every
package that has `tests/appearance.rs`.

## Tier 3: real input

Tier 3 starts the app as its own process with `NATIVE_AUTOMATION=1` and drives it over the
automation socket. The child prints `ND_AUTOMATION_LISTENING path=<socket>` on stderr and
`launch_app` waits for it. Each launch gets its own socket and `ND_APP_DATA_DIR`, so parallel
launches share no path.

<!-- docs:snippet examples/hello/tests/locators.rs#tier3-locators -->

Test names that start with `real_`, or live in a `real` module, are tier 3. nextest runs them one
at a time (except on Linux), and `nd test --tier real` selects only them. `connect_app` attaches
to a host somebody else started.

### Locators and expect

`app.get_by_role`, `get_by_test_id`, `get_by_text`, `get_by_label`, `get_by_placeholder` and
`locator("role=button[name=\"Save\"] >> nth=1")` return lazy locators. An action re-resolves the
locator against the current tree and waits until the node is visible, enabled, has real geometry
and has stopped moving. `expect(&locator)` has `to_be_visible`, `to_have_text`, `to_contain_text`
and the like, and polls until the session timeout (5 s, change it with `Session::with_timeout`).
The API blocks; there are no futures.

Selector parts chain with `>>`: `testid=`, `role=`, `text=`, `label=`, `type=`, `has-text=`,
`has=(...)`, `nth=`, `first`, `last`. The grammar is in
[selector.rs](../../../crates/nd-test/src/selector.rs).

Tier 3 adds `app.cursor`, `app.capture` and `app.record` for the real cursor, screenshots and
recordings.

### ndshell

[ndshell](../../../crates/ndshell/src/lib.rs) is the test-only helper that tier 3 stands on. Apps
never link it.

- The machine lock. Real HID input, activation and screen capture share one window server, so tests that use them hold an advisory `flock` (`real-input.lock` under `~/Library/Caches/nd/` on macOS, `$XDG_CACHE_HOME/nd/` or `~/.cache/nd/` on Linux, `%LOCALAPPDATA%\nd\` on Windows; `ND_LOCK_DIR` moves it). A second run, in any worktree, queues and prints who holds it. `ndshell lock status` shows the holder and `ndshell lock -- <command>` runs a command under the lock. `nd_test::real_input(purpose)` takes it from a test. `launch_app` takes it by default on macOS.
- Preflight (macOS). `ndshell preflight` checks that no window manager that intercepts synthetic clicks runs (OmniWM, yabai, AeroSpace, Amethyst), that the process may post events, and that no modifier key is stuck.
- Grant (macOS). `ndshell grant <binary>` writes Screen Recording, Accessibility and PostEvent rows into the system TCC database, keyed to the signing identifier so they survive rebuilds. It needs SIP off and root. Grant the process TCC treats as responsible: the terminal, or `sshd-keygen-wrapper` over ssh, not the test binary.
- Hyprland runner (Linux). `ndshell::hyprland::Hyprland::start` boots a private headless Hyprland per test in its own runtime directory, with a virtual pointer and keyboard and output capture, so Linux tier 3 tests run in parallel. It nests on a headless labwc for a render node. `ND_HYPRLAND`, `ND_LABWC` and `ND_RENDER_NODE` override the binaries and the device; `ND_KEEP_HYPRLAND=1` keeps the logs. An example is [real_overlays.rs](../../../crates/nd-os-linux/tests/real_overlays.rs).
- Windows agent. `ndshell agent` is meant to run in the interactive session, since processes started over ssh cannot post input. The Windows backend has not run yet, so this path is untested.

The incident tests of `hello` launch the real binary with `ND_DEBUG_PANIC`, `ND_DEBUG_HANG_MS` and
`ND_DEBUG_CRASH` and assert on the bundle ([incidents.rs](../../../examples/hello/tests/incidents.rs),
[diagnostics](diagnostics.md)).

## MCP bridge

`nd-mcp` lets an agent drive a running app. It has three tools:

- `execute({code})`: runs JavaScript in QuickJS with `app` bound to the locator API.
- `snapshot`: returns the compact semantic tree, one line per node with a `[ref=eN]`.
- `reset`: relaunches the app, or reconnects to the socket.

```sh
nd-mcp --example hello
nd-mcp --socket "$ND_AUTOMATION_SOCKET"
```

`--bin <path>` (env `ND_MCP_BIN`) launches any binary with automation on; arguments after `--` go
to it.

## nd test

`nd test` maps `--tier` to a nextest profile from `.config/nextest.toml` (which `nd new` writes
for your app too) and runs `cargo nextest run`.

| Profile | Runs |
| --- | --- |
| `fast` | Tiers 1 and 2, fail-fast. What `--tier fast` and CI's first step use. |
| `real` | Tier 3 only, one at a time off Linux. |
| `default` | Everything except tier 3. |
| `ci` | The default filter, no retries, JUnit output. |
| `gate` | Everything, including tier 3, for the milestone gates (`just gate m1`). |

Every test is its own process. Tests that hold the machine lock sit in one nextest test group, so
they also queue behind each other inside a run. A test that takes the lock but is missing from the
group wastes a parallel slot; keep the two in step.

```sh
nd test -p gallery forms
nd test --tier all
```

Real-input tests also run from the repository with `cargo nextest run --profile real -p hello`.
