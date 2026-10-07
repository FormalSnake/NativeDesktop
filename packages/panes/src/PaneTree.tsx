import { createSignal, type Accessor } from "solid-js";
import type { JSX } from "@nativedesktop/react";
import {
  closePane,
  focusNeighbor,
  focusPane,
  focusPaneAt,
  setPaneRatio,
  splitPane,
  updatePane,
} from "./model.ts";
import type { PaneLeaf, PaneModel, PaneNode, PaneSplit, SplitOrientation } from "./model.ts";
import { KeyedNode, isSettledRatio } from "./keyed.tsx";

/** What renderLeaf receives. Every field but `id` is a reactive getter, the
 * way component props are: read it where it is used, never destructure it in
 * the parameter list, or the leaf stops following the model. */
export interface PaneLeafContext<T> {
  readonly id: string;
  readonly data: T;
  readonly focused: boolean;
  readonly solo: boolean;
}

export interface PaneTreeProps<T> {
  model: PaneModel<T>;
  onChange: (next: PaneModel<T>) => void;
  /** Owns all per-pane chrome (focus ring, toolbar); PaneTree supplies only
   * the flags and one expanding <box> wrapper per leaf. Runs once per leaf
   * mount, not once per model change. */
  renderLeaf: (ctx: PaneLeafContext<T>) => JSX.Element;
  testID?: string;
}

export function PaneTree<T>(props: PaneTreeProps<T>): JSX.Element {
  const solo = (): boolean => props.model.root?.kind === "leaf";
  const id = (suffix: string): string | undefined => (props.testID ? `${props.testID}-${suffix}` : undefined);

  function renderLeaf(leaf: Accessor<PaneLeaf<T>>): JSX.Element {
    const leafId = leaf().id;
    const ctx: PaneLeafContext<T> = {
      id: leafId,
      get data() {
        return leaf().data;
      },
      get focused() {
        return leafId === props.model.focusedId;
      },
      get solo() {
        return solo();
      },
    };
    return (
      <box style={{ hexpand: true, vexpand: true }} testID={id(`leaf-${leafId}`)}>
        {props.renderLeaf(ctx)}
      </box>
    );
  }

  function renderSplit(split: Accessor<PaneSplit<T>>): JSX.Element {
    const splitId = split().id;
    return (
      <paned
        orientation={split().orientation}
        position={split().ratio}
        testID={id(`split-${splitId}`)}
        onPositionChanged={(e) => {
          if (!isSettledRatio(e.position)) return;
          // Handlers read props at event time, so the echo applies against the
          // latest model, never the one current when this paned mounted.
          // setPaneRatio returns the same reference when the clamped ratio is
          // unchanged; skipping onChange there is what stops the programmatic
          // write -> echo -> render -> write loop.
          const current = props.model;
          const next = setPaneRatio(current, splitId, e.position);
          if (next !== current) props.onChange(next);
        }}
      >
        {renderNode(() => split().children[0])}
        {renderNode(() => split().children[1])}
      </paned>
    );
  }

  function renderNode(node: () => PaneNode<T> | undefined): JSX.Element {
    return (
      <KeyedNode node={node()}>
        {(n) =>
          n().kind === "leaf" ? renderLeaf(n as Accessor<PaneLeaf<T>>) : renderSplit(n as Accessor<PaneSplit<T>>)
        }
      </KeyedNode>
    );
  }

  return renderNode(() => props.model.root);
}

export interface PaneTreeState<T> {
  model: Accessor<PaneModel<T>>;
  /** The model as of the last op, not the last flush: a write is staged until
   * Solid flushes, and `model()` reads the committed value until then. */
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

/** Holds the model in a signal and applies every op against the latest
 * model, never the committed one, so two ops in one tick (or an op after an
 * await) compose instead of the second reverting the first. An op that
 * changes nothing returns the same reference and writes nothing. */
export function createPaneTree<T>(initial: PaneModel<T> | (() => PaneModel<T>)): PaneTreeState<T> {
  let current = typeof initial === "function" ? initial() : initial;
  const [model, setState] = createSignal(current);

  const apply = (next: PaneModel<T>): void => {
    if (next === current) return;
    current = next;
    setState(() => next);
  };

  return {
    model,
    latest: () => current,
    setModel: apply,
    split: (paneId, o, data) => apply(splitPane(current, paneId, o, data)),
    close: (paneId) => apply(closePane(current, paneId)),
    focus: (paneId) => apply(focusPane(current, paneId)),
    focusAt: (index) => apply(focusPaneAt(current, index)),
    focusNeighbor: (dir) => apply(focusNeighbor(current, dir)),
    setRatio: (splitId, ratio) => apply(setPaneRatio(current, splitId, ratio)),
    update: (paneId, fn) => apply(updatePane(current, paneId, fn)),
  };
}
