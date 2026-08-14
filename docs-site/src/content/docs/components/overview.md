---
title: Overview
description: "How widgets are described in the schema: intrinsics, container models, attached props, and automation roles."
---

Every widget is declared once in `schema/widgets.json` and rendered as a lowercase JSX intrinsic
(`<window>`, `<box>`, `<button>`, `<splitview>`), never as a capitalized component you import.
`tools/codegen.ts` generates the Zig, TypeScript, and Swift bindings from that schema, plus the
[Widget Reference](/components/widget-reference/) page, so the three backends and these docs cannot
drift apart.

## What a widget declaration carries

**Props**, each with a type and an `appliesTo` of `create` (set once), `createAndUpdate` (live), or
`meta` (framework bookkeeping like `testID`, never rendered).

**Events**, each mapped to a React handler prop name (`clicked` becomes `onClick`) and, where
relevant, a payload shape.

**Container model**: `null` for a leaf widget, `single` for one child (`<window>`, `<scrollview>`),
`multi` for many (`<box>`, `<splitview>`).

**Attached props**, which a container reads off its children rather than off itself: `slot` on a
`<splitview>`'s or `<headerbar>`'s children, `gridRow` and `gridColumn` on a `<grid>`'s children,
`tabLabel` on a `<tabview>`'s children. They apply at attach time only, so changing one after mount
is a no-op.

**Automation role and text source**: an automation `role` (`button`, `textbox`, `list`, and so on)
and, where applicable, the prop `getTree` reports as the node's `text`. This is what makes the tree
an agent reads meaningful rather than a bag of opaque refs.

**Platform availability**: an optional `platforms` list restricts a widget to specific operating
systems, which is how `<trayitem>` and `<sharebutton>` end up macOS-only. Elsewhere the widget
mounts as an invisible no-op and `nd dev` logs a one-time console warning telling you to gate it
with `Platform.os`. See [Platform Support](/native-platform/platform-support/#platform-only-widgets).

## Styling applies uniformly

Every widget accepts the same two styling props: `style` (theme-neutral geometry) and `cssClasses`
(named design-language classes). Neither is declared per widget in the schema; both validate
against one shared allowlist. See
[Styling & Design Language](/core-concepts/styling-design-language/).

The [Widget Reference](/components/widget-reference/) is generated from `schema/widgets.json`. If a
widget's props look wrong there, check the schema first.
