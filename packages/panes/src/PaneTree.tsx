/** @jsxImportSource @nativedesktop/react */
// The pragma is load-bearing: this is a raw-src package, and Bun transpiles
// node_modules TSX with the ROOT tsconfig's JSX settings, so the per-file
// pragma is what guarantees the ND jsx-runtime regardless of the consumer.
//
// Hooks come from @nativedesktop/react, never react directly (dev-react.ts's
// pinned-dispatcher contract for `nd dev` hot re-eval).

import { useRef, useState } from "@nativedesktop/react";
import type { ReactNode } from "react";
import {
  closePane,
  focusNeighbor,
  focusPane,
  focusPaneAt,
  setPaneRatio,
  splitPane,
  updatePane,
} from "./model.ts";
import type { PaneModel, PaneNode, SplitOrientation } from "./model.ts";

export interface PaneTreeProps<T> {
  model: PaneModel<T>;
  onChange: (next: PaneModel<T>) => void;
  /** Owns all per-pane chrome (focus ring, toolbar); PaneTree supplies only
   * the flags and one expanding <box> wrapper per leaf. */
  renderLeaf: (ctx: { id: string; data: T; focused: boolean; solo: boolean }) => ReactNode;
  testID?: string;
}

export function PaneTree<T>(props: PaneTreeProps<T>): ReactNode {
  // Latest-ref: the native positionChanged echo lands via a handler captured
  // at an earlier render; applying it against that render's model would
  // revert everything committed since (a concurrent split, another divider's
  // drag). Both refs are refreshed every render, so handlers always see the
  // current model and onChange.
  const modelRef = useRef(props.model);
  modelRef.current = props.model;
  const onChangeRef = useRef(props.onChange);
  onChangeRef.current = props.onChange;

  const { model, renderLeaf, testID } = props;
  if (!model.root) return null;
  const solo = model.root.kind === "leaf";

  function renderNode(node: PaneNode<T>): ReactNode {
    if (node.kind === "leaf") {
      return (
        <box
          key={node.id}
          style={{ hexpand: true, vexpand: true }}
          testID={testID ? `${testID}-leaf-${node.id}` : undefined}
        >
          {renderLeaf({ id: node.id, data: node.data, focused: node.id === model.focusedId, solo })}
        </box>
      );
    }
    return (
      // Keyed on the split node id: orientation is create-only on both
      // backends, so a structural collapse landing a DIFFERENT split at this
      // position must remount rather than mutate.
      <paned
        key={node.id}
        orientation={node.orientation}
        position={node.ratio}
        testID={testID ? `${testID}-split-${node.id}` : undefined}
        onPositionChanged={(e) => {
          // Both backends enforce a native minimum pane extent, so a settled
          // drag can never rest at or beyond the clamp bounds; an echo out
          // there is a zero-size mid-layout artifact (structural commits
          // racing the backend's debounced echo), not a drag. Feeding it to
          // the model would collapse the pane on the next render.
          if (e.position <= 0.05 || e.position >= 0.95) return;
          const current = modelRef.current;
          // setPaneRatio returns the same reference when the clamped ratio is
          // unchanged; skipping onChange there is what stops the programmatic
          // write -> echo -> render -> write loop.
          const next = setPaneRatio(current, node.id, e.position);
          if (next !== current) onChangeRef.current(next);
        }}
      >
        {renderNode(node.children[0])}
        {renderNode(node.children[1])}
      </paned>
    );
  }

  return renderNode(model.root);
}

export interface UsePaneTree<T> {
  model: PaneModel<T>;
  /** The latest-ref invariant, built in: reads the model as of the last op,
   * not the last render. */
  latest: () => PaneModel<T>;
  setModel(m: PaneModel<T>): void;
  split(paneId: string, o: SplitOrientation, data: T): void;
  close(paneId: string): void;
  focus(paneId: string): void;
  focusAt(index: number): void;
  focusNeighbor(dir: "left" | "right" | "up" | "down"): void;
  setRatio(splitId: string, ratio: number): void;
  update(paneId: string, fn: (d: T) => T): void;
}

/** Holds the model in state and applies every op against a ref, never the
 * render-time model, so an await-resuming op can't revert a concurrent
 * divider drag. */
export function usePaneTree<T>(initial: PaneModel<T> | (() => PaneModel<T>)): UsePaneTree<T> {
  const [model, setState] = useState(initial);
  const ref = useRef(model);

  const apply = (next: PaneModel<T>): void => {
    if (next === ref.current) return;
    ref.current = next;
    setState(next);
  };

  return {
    model,
    latest: () => ref.current,
    setModel: apply,
    split: (paneId, o, data) => apply(splitPane(ref.current, paneId, o, data)),
    close: (paneId) => apply(closePane(ref.current, paneId)),
    focus: (paneId) => apply(focusPane(ref.current, paneId)),
    focusAt: (index) => apply(focusPaneAt(ref.current, index)),
    focusNeighbor: (dir) => apply(focusNeighbor(ref.current, dir)),
    setRatio: (splitId, ratio) => apply(setPaneRatio(ref.current, splitId, ratio)),
    update: (paneId, fn) => apply(updatePane(ref.current, paneId, fn)),
  };
}
