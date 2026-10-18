+++
title = "For agents"
description = "What a coding agent gets from nd new, and how to point one at an nd project."
+++

NativeDesktop is built for coding agents first. The framework makes mistakes fail early: `nd
check` names the rule that a change broke, the appearance audit fails any color that isn't a
token, and tier 1 tests run in seconds without a window. These pages are what an agent reads to
get it right the first time.

The docs are online at https://native.formalsnake.dev. Every page is also plain markdown
at the same path with `.md` added. https://native.formalsnake.dev/llms.txt lists the pages,
and https://native.formalsnake.dev/llms-full.txt has all of them in one file.

## Start a project

Releases are git tags, not crates.io versions. Install the CLI from the tag that you build
against, and then scaffold the app:

```sh
cargo install cargo-nextest --locked
cargo install --git https://github.com/FormalSnake/NativeDesktop --tag v0.6.1 --locked nd-cli
nd new my-app
cd my-app
just dev
```

On Windows, first install the MSVC build tools, rustup, and git from PowerShell with `winget`.
The exact lines are under Install in [getting started](../getting-started.md). On macOS, install
Xcode or the Command Line Tools. `nd new` pins the project to the CLI's own tag and writes nd's
`Cargo.lock`, so the first build uses the versions that nd was released with. If the new
directory sits inside another Cargo workspace, add `--standalone`. To build against a local clone
of nd instead of the tag, add `--nd-path <checkout>/crates/nd`.

## What `nd new` writes for agents

`nd new my-app` creates a project with:

- `CLAUDE.md`: the commands, the ten framework rules, the testing tiers, the machine lock, and the
  incident workflow, on one screen. Claude Code reads it on start. Other agents read it if you
  point them at it.
- `docs/agents/`: the six pages in this section, copied into the project so that an agent that's
  offline or in a sandbox has them next to the code. These pages include the same files, so the
  site and every new project always agree.
- `README.md`: a template with `<!-- nd: ... -->` placeholders for the icon, the one-sentence
  pitch, a measured number, screenshots, and install steps. `nd check --release` fails while any
  placeholder is left, so an agent fills them in before it ships.
- One test for each tier that it can run today (`tests/app.rs` for tier 1, `tests/appearance.rs`
  for the audit), a demo mode (`<APP>_DEMO=1`) for tests and screenshots, and a `justfile` with
  `dev`, `check`, `test`, `test-real`, `diagnose`, and `web`.

The `CLAUDE.md` template is
[crates/nd-cli/templates/CLAUDE.md.tmpl](../../../crates/nd-cli/templates/CLAUDE.md.tmpl).
`nd new` fills in the app's name and its demo variable. The pages that it copies to `docs/agents/`
are in [crates/nd-cli/templates/docs/agents](../../../crates/nd-cli/templates/docs/agents/donts.md).
`nd new` doesn't write an `AGENTS.md`. If your agent reads `AGENTS.md` instead of `CLAUDE.md`
(Codex, for example), copy `CLAUDE.md` to `AGENTS.md`, or tell the agent to read `CLAUDE.md` first.

## Pointing an agent at an nd project

1. Start the agent in the project root, so that it picks up `CLAUDE.md`.
2. Tell it to read `docs/agents/donts.md` before its first change. The page is short.
3. Give it the loop: edit, `nd check`, `nd test`. Both commands print `file:line` findings that an
   agent can act on without asking.
4. For a bug, make the first command `nd diagnose --last`. The bundle has the inputs, the tree,
   and the log tail from when it went wrong.

Several agents on one machine can run tier 3 at the same time. Their real-input runs queue on the
machine lock instead of fighting over the cursor.

Before an agent calls a change done, it runs these checks in this order:

1. `nd check`, with no findings left or each one silenced by `// nd-allow(rule): why`.
2. `nd test`, with a new test in the lowest tier that shows the change (tier 1 for logic, a
   `.window(..)` line in `tests/appearance.rs` for every new window).
3. `nd test --tier real`, only when the change depends on real input, focus, resize, or menus.
4. `nd check --target web`, when the app also ships to the browser.

## Upgrading nd

```sh
nd upgrade
```

This command moves the `nd` dependency and the GPUI `[patch.crates-io]` block in `Cargo.toml` to
the newest release tag, re-resolves `Cargo.lock`, and prints the changelog entries in between.
`--to v0.7.0` picks a tag, and `--path <dir>` points it at an app outside the current directory.
If the new tag doesn't resolve, `nd upgrade` puts `Cargo.toml` back. After an upgrade:

1. Install the matching CLI with the same `cargo install --git ... --tag <tag> --locked nd-cli`
   line.
2. Run `nd check` and `nd test`. The changelog lists renamed APIs and new lint rules.

Never edit the tag or the patch block by hand. A project created with `--nd-path` has no tag to
move, so pull the checkout instead.

## Driving a running app

To let an agent drive a running app, use `nd-mcp`, which exposes the app over MCP. `snapshot`
returns the semantic tree with `[ref=eN]` handles, `execute` runs locator code against it, and
`reset` relaunches the app.

## The pages

- [Patterns](patterns.md): system structure, entities, actions, controlled values, overlays, and time.
- [Widgets](widgets.md): which element to use, virtual lists, and app-defined native widgets.
- [Testing](testing.md): the three tiers, the machine lock, and incident bundles.
- [Styling](styling.md): tokens, the appearance audit, override layers, and glass.
- [Performance](performance.md): budgets on the slowest machine, and how to stay inside them.
- [Don'ts](donts.md): the hard list.
