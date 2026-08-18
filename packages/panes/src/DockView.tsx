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
  activateTab,
  activeDockTabIndex,
  addTab,
  applyDockDrop,
  closeTab,
  dockDragPayload,
  dockPanel,
  dockZoneAt,
  moveTab,
  undockTab,
} from "./dock.ts";
import type { DockEdgeZone, DockModel, DockPanel, DockSize, DockTab, DockZone } from "./dock.ts";
import { closePane, focusNeighbor, focusPane, setPaneRatio } from "./model.ts";
import type { PaneNode } from "./model.ts";

/** Spread onto any widget to make it the drag handle for a tab or a panel.
 * `onDragEnded` is part of the bundle because it is what clears the drop
 * indicator when a drag is abandoned outside every panel: there is no
 * drag-leave event on the target side. */
export interface DockDragProps {
  draggable: true;
  dragPayload: string;
  onDragEnded: () => void;
}

/** Classes the hovered panel wears while a drag is over it. Both are in
 * schema/widgets.json's cssClasses list and each is the container card its own
 * backend draws: Adwaita styles `.card` on any widget, AppKit paints a card
 * backing behind a `boxed-list` stack. Neither renders on the other side, so
 * the pair is one highlight per platform, not two stacked. */
const HOVER_CLASSES = ["card", "boxed-list"];

/** The idle class list has to be an empty ARRAY, not an absent prop: the
 * reconciler diffs prop values and an `undefined` one serializes away, so the
 * class set would never be told to clear and the highlight would outlive the
 * drag. Both are module constants so an unrelated render emits no update. */
const NO_CLASSES: string[] = [];

export interface DockTabContext<T> {
  panelId: string;
  tab: DockTab<T>;
  active: boolean;
  /** True when the tab's panel holds the dock's focus, not the tab itself:
   * there is no widget-level focus event to say otherwise. */
  focused: boolean;
  /** Drag handle for THIS tab. DockView already puts it on the tab's own body
   * box, so spread this only to add a second handle (a title row of your own,
   * say) or after turning `dragTabBodies` off. */
  dragProps: DockDragProps;
}

export interface DockPanelContext<T> {
  panelId: string;
  panel: DockPanel<T>;
  focused: boolean;
  solo: boolean;
  /** The panel's <tabview>. Wrap it to add a panel toolbar or a focus ring,
   * and return it as-is to keep the bare tab stack. */
  content: ReactNode;
  /** Drag handle for the WHOLE panel, tab stack included. Nothing carries it
   * by default: a panel's only always-present surface is its content, and that
   * is where the per-tab handle sits. Put it on chrome you draw. */
  dragProps: DockDragProps;
  /** Zone a drag is currently hovering over this panel, or null when no drag
   * is over it. DockView already draws an edge indicator and the card
   * highlight; this is for chrome that wants to react as well. */
  dropZone: DockZone | null;
}

export interface DockViewProps<T> {
  model: DockModel<T>;
  onChange: (next: DockModel<T>) => void;
  /** Renders one tab's body. DockView supplies the <tabview> and one
   * expanding <box> per tab, which is where the tab's label and icon are
   * attached from. */
  renderTab: (ctx: DockTabContext<T>) => ReactNode;
  /** Owns all per-panel chrome, the way PaneTree's renderLeaf does. */
  renderPanel?: (ctx: DockPanelContext<T>) => ReactNode;
  /** Pixel extent of the dock, which is what makes edge zones reachable: drop
   * points arrive in the target panel's own coordinates and nothing on the
   * wire says how big that panel is. Without it every drop is a `center`
   * drop, which still merges and reorders tabs. */
  size?: DockSize;
  /** Fraction of a panel each edge zone claims. Default 0.25. */
  dropEdge?: number;
  /** Default true: a tab's body box is that tab's drag handle, so a drag from
   * anywhere the content does not claim itself moves the tab. Turn it off for
   * content that owns its own drag gestures (a text view, a canvas) and put
   * `dragProps` on chrome instead. */
  dragTabBodies?: boolean;
  testID?: string;
}

interface DockHover {
  panelId: string;
  zone: DockZone;
}

export function DockView<T>(props: DockViewProps<T>): ReactNode {
  // Latest-ref, same reason as PaneTree: the native positionChanged echo
  // lands via a handler captured at an earlier render, and applying it
  // against that render's model would revert everything committed since.
  const modelRef = useRef(props.model);
  modelRef.current = props.model;
  const onChangeRef = useRef(props.onChange);
  onChangeRef.current = props.onChange;
  const [hover, setHover] = useState<DockHover | null>(null);

  const { model, renderTab, renderPanel, size, dropEdge, dragTabBodies = true, testID } = props;
  if (!model.root) return null;
  const solo = model.root.kind === "leaf";

  function commit(next: DockModel<T>): void {
    if (next !== modelRef.current) onChangeRef.current(next);
  }

  function dragProps(kind: "tab" | "panel", id: string): DockDragProps {
    return { draggable: true, dragPayload: dockDragPayload(kind, id), onDragEnded: () => setHover(null) };
  }

  // dragOver fires per pointer motion, so the state update has to be a bail
  // when the zone has not changed: otherwise every mouse move during a drag
  // is a React commit, and the panel under the pointer rebuilds its props at
  // pointer rate.
  function onPanelDragOver(panelId: string, x: number, y: number): void {
    const zone = dockZoneAt(modelRef.current, panelId, x, y, size, dropEdge);
    setHover((current) =>
      current && current.panelId === panelId && current.zone === zone ? current : { panelId, zone },
    );
  }

  function onPanelDrop(panelId: string, payload: string, x: number, y: number): void {
    setHover(null);
    const current = modelRef.current;
    commit(applyDockDrop(current, payload, panelId, dockZoneAt(current, panelId, x, y, size, dropEdge)));
  }

  function renderPanelNode(id: string, panel: DockPanel<T>): ReactNode {
    const focused = id === model.focusedId;
    const zone = hover && hover.panelId === id ? hover.zone : null;
    const tabs = testID ? `${testID}-tabs-${id}` : undefined;
    const content = (
      // selectedIndex is the model's active tab, and the native tab bar's own
      // selectionChanged comes back through activateTab, so clicking a tab
      // natively and activating one from app chrome land in the same place.
      <tabview
        selectedIndex={activeDockTabIndex(panel)}
        style={{ hexpand: true, vexpand: true }}
        testID={tabs}
        onSelectionChanged={(e) => {
          const tab = panel.tabs[e.index];
          if (tab) commit(activateTab(modelRef.current, tab.id));
        }}
      >
        {panel.tabs.map((tab) => (
          <box
            key={tab.id}
            tabLabel={tab.title}
            tabIcon={tab.icon}
            style={{ hexpand: true, vexpand: true }}
            testID={testID ? `${testID}-tab-${tab.id}` : undefined}
            {...(dragTabBodies ? dragProps("tab", tab.id) : {})}
          >
            {renderTab({
              panelId: id,
              tab,
              active: tab.id === panel.activeTabId,
              focused,
              dragProps: dragProps("tab", tab.id),
            })}
          </box>
        ))}
      </tabview>
    );
    const body = renderPanel
      ? renderPanel({ panelId: id, panel, focused, solo, content, dragProps: dragProps("panel", id), dropZone: zone })
      : content;
    const indicator = (edge: DockEdgeZone): ReactNode =>
      zone === edge ? (
        // A native separator IS the platform's insertion line, so the edge a
        // drop would take is drawn with a real widget rather than a hand-sized
        // strip. Vertical edges expand down the panel, horizontal ones across.
        <separator
          orientation={edge === "left" || edge === "right" ? "vertical" : "horizontal"}
          cssClasses={["accent"]}
          style={edge === "left" || edge === "right" ? { vexpand: true } : { hexpand: true }}
          testID={testID ? `${testID}-drop-${edge}-${id}` : undefined}
        />
      ) : null;
    return (
      // The panel box is the drop target for its whole area: both backends
      // keep a drop zone inert until a drag is actually in flight, so this
      // costs the panel nothing the rest of the time.
      <box
        key={id}
        spacing={0}
        style={{ hexpand: true, vexpand: true }}
        cssClasses={zone ? HOVER_CLASSES : NO_CLASSES}
        dropTarget
        onDragOver={(e) => onPanelDragOver(id, e.data.x, e.data.y)}
        onDropped={(e) => onPanelDrop(id, e.text, e.data.x, e.data.y)}
        testID={testID ? `${testID}-panel-${id}` : undefined}
      >
        {indicator("top")}
        {/* spacing 0 on both indicator hosts: the platform default would open
            a gap the moment a drop line appears, and the content under the
            pointer would jump by it. */}
        <box orientation="horizontal" spacing={0} style={{ hexpand: true, vexpand: true }}>
          {indicator("left")}
          {body}
          {indicator("right")}
        </box>
        {indicator("bottom")}
      </box>
    );
  }

  function renderNode(node: PaneNode<DockPanel<T>>): ReactNode {
    if (node.kind === "leaf") return renderPanelNode(node.id, node.data);
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
          // Exact 0/1 (or non-finite) is a zero-size mid-layout artifact from
          // a structural commit racing the backend's debounced echo, not a
          // settled drag. See PaneTree for the full reasoning; the guard has
          // to be identical or the two views desync on the same tree.
          if (!Number.isFinite(e.position) || e.position <= 0 || e.position >= 1) return;
          commit(setPaneRatio(modelRef.current, node.id, e.position));
        }}
      >
        {renderNode(node.children[0])}
        {renderNode(node.children[1])}
      </paned>
    );
  }

  // The bare testID has to land on a real node, not just seed the derived
  // `-panel-`/`-tabs-` ids: automation resolves the dock by its own testID.
  // TilesView puts it on its <grid>; the dock's root is a split or a panel,
  // both of which already carry a derived id, so it needs its own host.
  return (
    <box orientation="vertical" style={{ vexpand: true, hexpand: true }} testID={testID}>
      {renderNode(model.root)}
    </box>
  );
}

export interface UseDock<T> {
  model: DockModel<T>;
  /** The latest-ref invariant, built in: reads the model as of the last op,
   * not the last render. */
  latest: () => DockModel<T>;
  setModel(m: DockModel<T>): void;
  addTab(panelId: string, tab: DockTab<T>, index?: number): void;
  closeTab(tabId: string): void;
  activateTab(tabId: string): void;
  moveTab(tabId: string, targetPanelId: string, index?: number): void;
  undockTab(tabId: string, zone?: DockEdgeZone): void;
  dock(panelId: string, targetPanelId: string, zone: DockZone): void;
  closePanel(panelId: string): void;
  focusPanel(panelId: string): void;
  focusNeighbor(dir: "left" | "right" | "up" | "down"): void;
  setRatio(splitId: string, ratio: number): void;
}

/** Holds the dock in state and applies every op against a ref, never the
 * render-time model, so an await-resuming op can't revert a concurrent
 * divider drag. */
export function useDock<T>(initial: DockModel<T> | (() => DockModel<T>)): UseDock<T> {
  const [model, setState] = useState(initial);
  const ref = useRef(model);

  const apply = (next: DockModel<T>): void => {
    if (next === ref.current) return;
    ref.current = next;
    setState(next);
  };

  return {
    model,
    latest: () => ref.current,
    setModel: apply,
    addTab: (panelId, tab, index) => apply(addTab(ref.current, panelId, tab, index)),
    closeTab: (tabId) => apply(closeTab(ref.current, tabId)),
    activateTab: (tabId) => apply(activateTab(ref.current, tabId)),
    moveTab: (tabId, targetPanelId, index) => apply(moveTab(ref.current, tabId, targetPanelId, index)),
    undockTab: (tabId, zone) => apply(undockTab(ref.current, tabId, zone)),
    dock: (panelId, targetPanelId, zone) => apply(dockPanel(ref.current, panelId, targetPanelId, zone)),
    closePanel: (panelId) => apply(closePane(ref.current, panelId)),
    focusPanel: (panelId) => apply(focusPane(ref.current, panelId)),
    focusNeighbor: (dir) => apply(focusNeighbor(ref.current, dir)),
    setRatio: (splitId, ratio) => apply(setPaneRatio(ref.current, splitId, ratio)),
  };
}
