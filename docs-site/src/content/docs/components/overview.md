---
title: Overview
description: "How widgets are described in the schema: intrinsics, container models, attached props, and automation roles."
---

Every widget NativeDesktop knows about is declared once in `schema/widgets.json` and rendered as a
lowercase JSX intrinsic (`<window>`, `<box>`, `<button>`, `<splitview>`, …), never as a capitalized
component you import. `tools/codegen.ts` generates the Zig, TypeScript, and Swift bindings from that
one schema, plus the [Widget Reference](/components/widget-reference/) page itself, so the three
backends and this documentation cannot drift from each other.

## What each widget declaration carries

**Props**, each with a type and an `appliesTo` of `create` (set once), `createAndUpdate` (live), or
`meta` (framework bookkeeping like `testID`, never rendered).

**Events**, each mapped to a React handler prop name (`clicked` becomes `onClick`) and, where
relevant, a payload shape.

**Container model**: `null` for a leaf widget, `single` for one child (`<window>`, `<scrollview>`),
or `multi` for many (`<box>`, `<splitview>`).

**Attached props**, which a container reads off its children rather than off itself: `slot` on a
`<splitview>`'s or `<headerbar>`'s children, `gridRow` and `gridColumn` on a `<grid>`'s children,
`tabLabel` on a `<tabview>`'s children. They apply at attach time only, so changing one after mount
is a no-op.

**Automation role and text source.** Every widget declares an automation `role` (`button`,
`textbox`, `list`, and so on) and, where applicable, which prop `getTree` reports as its `text`.
That is what makes the tree an agent reads meaningful instead of a bag of opaque refs.

**Platform availability.** An optional `platforms` list restricts a widget to specific operating
systems, which is how `<trayitem>` and `<sharebutton>` end up macOS-only. Elsewhere the widget mounts
as an invisible no-op and `nd dev` logs a one-time console warning telling you to gate it with
`Platform.os`. See [Platform Support](/native-platform/platform-support/#platform-only-widgets).

## Styling applies uniformly

Every widget accepts the same two styling props, described in
[Styling & Design Language](/core-concepts/styling-design-language/): `style` (theme-neutral
geometry) and `cssClasses` (named design-language classes). Neither is declared per widget in the
schema; both are validated against one shared allowlist regardless of which widget they're set on.

## Provenance

The [Widget Reference](/components/widget-reference/) is a faithful port of the generated
`docs/widgets.md`, which is itself generated from `schema/widgets.json`. If a widget's props ever
look wrong here, check the schema first, not this page.
