import { For, type Accessor } from "solid-js";
import type { JSX } from "@nativedesktop/react";

/** Renders one tree node keyed on its kind and id. A model op copies every
 * node on the path it touched, so the same pane arrives as a new object:
 * keying on identity would recreate its native widget on every op, while
 * this keeps it and feeds the new object through the accessor. A different
 * node at the same position (a structural collapse) remounts, which also
 * matters because a paned's orientation is create-only on both backends. */
export function KeyedNode<N extends { kind: string; id: string }>(props: {
  node: N | undefined;
  children: (node: Accessor<N>) => JSX.Element;
}): JSX.Element {
  return (
    <For each={props.node ? [props.node] : []} keyed={(n) => `${n.kind}:${n.id}`}>
      {(n) => props.children(n)}
    </For>
  );
}

/** Exact 0/1 (or non-finite) is dropped: structural commits racing the
 * backend's debounced echo report a zero-size mid-layout artifact at exactly
 * those values, and feeding one to the model would collapse the pane on the
 * next render. Anything inside (0, 1) is a settled drag and flows through:
 * AppKit pins a 120pt minimum pane extent, but GTK only floors at the CHILD's
 * own minimum size, so a min-size-zero child can rest out past the clamp
 * bounds; setPaneRatio's clamp then updates the model and the position prop
 * write snaps the native divider back to the bound instead of desyncing the
 * two. PaneTree and DockView share it so the two views never desync on the
 * same tree. */
export function isSettledRatio(position: number): boolean {
  return Number.isFinite(position) && position > 0 && position < 1;
}
