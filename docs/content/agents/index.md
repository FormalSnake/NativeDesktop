+++
title = "For agents"
description = "What a coding agent gets from nd new, and how to point one at an nd project."
+++

NativeDesktop is written for agents first. The framework makes the wrong thing fail early: `nd
check` names the rule a change broke, the appearance audit fails a colour that is not a token,
and tier 1 tests run in seconds without a window. These pages are what an agent reads to get it
right the first time.

## What `nd new` writes for agents

`nd new my-app` creates a project with:

- `CLAUDE.md`: the commands, the ten framework rules, the testing tiers, the machine lock and the
  incident workflow, on one screen. Claude Code reads it on start; other agents read it if you
  point them at it.
- `docs/agents/`: the six pages in this section, copied into the project so an agent offline or
  in a sandbox has them next to the code. They are the same files these pages include, so the
  site and every new project never disagree.
- `README.md`: a template with `<!-- nd: ... -->` placeholders for the icon, the one-sentence
  pitch, a measured number, screenshots and install steps. `nd check --release` fails while any
  are left, so an agent fills them in before shipping.
- One test per tier it can run today (`tests/app.rs` for tier 1, `tests/appearance.rs` for the
  audit), a demo mode (`<APP>_DEMO=1`) for tests and screenshots, and a `justfile` with `dev`,
  `check`, `test`, `test-real`, `diagnose` and `web`.

The `CLAUDE.md` template is
[crates/nd-cli/templates/CLAUDE.md.tmpl](../../../crates/nd-cli/templates/CLAUDE.md.tmpl);
`nd new` fills in the app's name and its demo variable.

## Pointing an agent at an nd project

1. Start the agent in the project root, so it picks up `CLAUDE.md`.
2. Tell it to read `docs/agents/donts.md` before its first change; it is short.
3. Give it the loop: edit, `nd check`, `nd test`. Both print `file:line` findings an agent can
   act on without asking.
4. For a bug, make the first command `nd diagnose --last`. The bundle has the inputs, the tree
   and the log tail from when it went wrong.
5. Several agents on one machine can run tier 3 at once; their real-input runs queue on the
   machine lock instead of fighting over the cursor.

For an agent that drives a running app, `nd-mcp` exposes it over MCP: `snapshot` returns the
semantic tree with `[ref=eN]` handles, `execute` runs locator code against it, and `reset`
relaunches it.

## The pages

- [Patterns](patterns.md): system structure, entities, actions, controlled values, overlays, time.
- [Widgets](widgets.md): which element to use, virtual lists, app-defined native widgets.
- [Testing](testing.md): the three tiers, the machine lock, incident bundles.
- [Styling](styling.md): tokens, the appearance audit, override layers, glass.
- [Performance](performance.md): budgets on the slowest machine and how to stay inside them.
- [Don'ts](donts.md): the hard list.
