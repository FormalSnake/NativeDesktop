import { For, Show, createSignal, type Accessor } from "solid-js";
import type { JSX } from "@nativedesktop/solid";
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
import type { PaneLeaf, PaneNode, PaneSplit } from "./model.ts";
import { KeyedNode, isSettledRatio } from "./keyed.tsx";

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

/** The idle class list has to be an empty ARRAY, not an absent prop, so the
 * class set is told to clear the moment the drag leaves. */
const NO_CLASSES: string[] = [];

/** What renderTab receives. Every field but `panelId` and `dragProps` is a
 * reactive getter, the way component props are: read it where it is used,
 * never destructure it in the parameter list. */
export interface DockTabContext<T> {
  readonly panelId: string;
  readonly tab: DockTab<T>;
  readonly active: boolean;
  /** True when the tab's panel holds the dock's focus, not the tab itself:
   * there is no widget-level focus event to say otherwise. */
  readonly focused: boolean;
  /** Drag handle for THIS tab. DockView already puts it on the tab's own body
   * box, so spread this only to add a second handle (a title row of your own,
   * say) or after turning `dragTabBodies` off. */
  readonly dragProps: DockDragProps;
}

/** What renderPanel receives, with the same getter rule as DockTabContext. */
export interface DockPanelContext<T> {
  readonly panelId: string;
  readonly panel: DockPanel<T>;
  readonly focused: boolean;
  readonly solo: boolean;
  /** The panel's <tabview>. Wrap it to add a panel toolbar or a focus ring,
   * and return it as-is to keep the bare tab stack. Insert it once. */
  readonly content: JSX.Element;
  /** Drag handle for the WHOLE panel, tab stack included. Nothing carries it
   * by default: a panel's only always-present surface is its content, and that
   * is where the per-tab handle sits. Put it on chrome you draw. */
  readonly dragProps: DockDragProps;
  /** Zone a drag is currently hovering over this panel, or null when no drag
   * is over it. DockView already draws an edge indicator and the card
   * highlight; this is for chrome that wants to react as well. */
  readonly dropZone: DockZone | null;
}

export interface DockViewProps<T> {
  model: DockModel<T>;
  onChange: (next: DockModel<T>) => void;
  /** Renders one tab's body. DockView supplies the <tabview> and one
   * expanding <box> per tab, which is where the tab's label and icon are
   * attached from. Runs once per tab mount. */
  renderTab: (ctx: DockTabContext<T>) => JSX.Element;
  /** Owns all per-panel chrome, the way PaneTree's renderLeaf does. Runs once
   * per panel mount. */
  renderPanel?: (ctx: DockPanelContext<T>) => JSX.Element;
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

export function DockView<T>(props: DockViewProps<T>): JSX.Element {
  const [hover, setHover] = createSignal<DockHover | null>(null);
  const solo = (): boolean => props.model.root?.kind === "leaf";
  const id = (suffix: string): string | undefined => (props.testID ? `${props.testID}-${suffix}` : undefined);

  // Handlers read props at event time, so an echo or a drop applies against
  // the latest model, never the one current when its widget mounted.
  function commit(next: DockModel<T>): void {
    if (next !== props.model) props.onChange(next);
  }

  function dragProps(kind: "tab" | "panel", dragId: string): DockDragProps {
    return { draggable: true, dragPayload: dockDragPayload(kind, dragId), onDragEnded: () => setHover(null) };
  }

  // dragOver fires per pointer motion, so the write has to hand back the
  // current value when the zone has not changed: the signal's equality check
  // then drops it, and the panel under the pointer does not update its props
  // at pointer rate.
  function onPanelDragOver(panelId: string, x: number, y: number): void {
    const zone = dockZoneAt(props.model, panelId, x, y, props.size, props.dropEdge);
    setHover((current) => (current && current.panelId === panelId && current.zone === zone ? current : { panelId, zone }));
  }

  function onPanelDrop(panelId: string, payload: string, x: number, y: number): void {
    setHover(null);
    const current = props.model;
    commit(applyDockDrop(current, payload, panelId, dockZoneAt(current, panelId, x, y, props.size, props.dropEdge)));
  }

  function renderPanelNode(leaf: Accessor<PaneLeaf<DockPanel<T>>>): JSX.Element {
    const panelId = leaf().id;
    const panel = (): DockPanel<T> => leaf().data;
    const focused = (): boolean => panelId === props.model.focusedId;
    const zone = (): DockZone | null => {
      const h = hover();
      return h && h.panelId === panelId ? h.zone : null;
    };
    const dragBodies = (): boolean => props.dragTabBodies ?? true;

    const content = (
      // selectedIndex is the model's active tab, and the native tab bar's own
      // selectionChanged comes back through activateTab, so clicking a tab
      // natively and activating one from app chrome land in the same place.
      <tabview
        selectedIndex={activeDockTabIndex(panel())}
        style={{ hexpand: true, vexpand: true }}
        testID={id(`tabs-${panelId}`)}
        onSelectionChanged={(e) => {
          const tab = panel().tabs[e.index];
          if (tab) commit(activateTab(props.model, tab.id));
        }}
      >
        <For each={panel().tabs} keyed={(tab) => tab.id}>
          {(tab) => {
            const tabId = tab().id;
            const drag = dragProps("tab", tabId);
            const ctx: DockTabContext<T> = {
              panelId,
              get tab() {
                return tab();
              },
              get active() {
                return tabId === panel().activeTabId;
              },
              get focused() {
                return focused();
              },
              dragProps: drag,
            };
            return (
              <box
                tabLabel={tab().title}
                tabIcon={tab().icon}
                style={{ hexpand: true, vexpand: true }}
                testID={id(`tab-${tabId}`)}
                draggable={dragBodies() ? true : undefined}
                dragPayload={dragBodies() ? drag.dragPayload : undefined}
                onDragEnded={dragBodies() ? drag.onDragEnded : undefined}
              >
                {props.renderTab(ctx)}
              </box>
            );
          }}
        </For>
      </tabview>
    );

    const renderPanel = props.renderPanel;
    const body = renderPanel
      ? renderPanel({
          panelId,
          get panel() {
            return panel();
          },
          get focused() {
            return focused();
          },
          get solo() {
            return solo();
          },
          content,
          dragProps: dragProps("panel", panelId),
          get dropZone() {
            return zone();
          },
        })
      : content;

    // A native separator IS the platform's insertion line, so the edge a drop
    // would take is drawn with a real widget rather than a hand-sized strip.
    // Vertical edges expand down the panel, horizontal ones across.
    const indicator = (edge: DockEdgeZone): JSX.Element => (
      <Show when={zone() === edge}>
        <separator
          orientation={edge === "left" || edge === "right" ? "vertical" : "horizontal"}
          cssClasses={["accent"]}
          style={edge === "left" || edge === "right" ? { vexpand: true } : { hexpand: true }}
          testID={id(`drop-${edge}-${panelId}`)}
        />
      </Show>
    );

    return (
      // The panel box is the drop target for its whole area: both backends
      // keep a drop zone inert until a drag is actually in flight, so this
      // costs the panel nothing the rest of the time.
      <box
        spacing={0}
        style={{ hexpand: true, vexpand: true }}
        cssClasses={zone() ? HOVER_CLASSES : NO_CLASSES}
        dropTarget
        onDragOver={(e) => onPanelDragOver(panelId, e.data.x, e.data.y)}
        onDropped={(e) => onPanelDrop(panelId, e.text, e.data.x, e.data.y)}
        testID={id(`panel-${panelId}`)}
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

  function renderSplit(split: Accessor<PaneSplit<DockPanel<T>>>): JSX.Element {
    const splitId = split().id;
    return (
      <paned
        orientation={split().orientation}
        position={split().ratio}
        testID={id(`split-${splitId}`)}
        onPositionChanged={(e) => {
          if (!isSettledRatio(e.position)) return;
          commit(setPaneRatio(props.model, splitId, e.position));
        }}
      >
        {renderNode(() => split().children[0])}
        {renderNode(() => split().children[1])}
      </paned>
    );
  }

  function renderNode(node: () => PaneNode<DockPanel<T>> | undefined): JSX.Element {
    return (
      <KeyedNode node={node()}>
        {(n) =>
          n().kind === "leaf"
            ? renderPanelNode(n as Accessor<PaneLeaf<DockPanel<T>>>)
            : renderSplit(n as Accessor<PaneSplit<DockPanel<T>>>)
        }
      </KeyedNode>
    );
  }

  // The bare testID has to land on a real node, not just seed the derived
  // `-panel-`/`-tabs-` ids: automation resolves the dock by its own testID.
  // TilesView puts it on its <grid>; the dock's root is a split or a panel,
  // both of which already carry a derived id, so it needs its own host.
  return (
    <Show when={props.model.root}>
      <box orientation="vertical" style={{ vexpand: true, hexpand: true }} testID={props.testID}>
        {renderNode(() => props.model.root)}
      </box>
    </Show>
  );
}

export interface DockState<T> {
  model: Accessor<DockModel<T>>;
  /** The model as of the last op, not the last flush: a write is staged until
   * Solid flushes, and `model()` reads the committed value until then. */
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

/** Holds the dock in a signal and applies every op against the latest model,
 * never the committed one, so two ops in one tick (or an op after an await)
 * compose instead of the second reverting the first. An op that changes
 * nothing returns the same reference and writes nothing. */
export function createDock<T>(initial: DockModel<T> | (() => DockModel<T>)): DockState<T> {
  let current = typeof initial === "function" ? initial() : initial;
  const [model, setState] = createSignal(current);

  const apply = (next: DockModel<T>): void => {
    if (next === current) return;
    current = next;
    setState(() => next);
  };

  return {
    model,
    latest: () => current,
    setModel: apply,
    addTab: (panelId, tab, index) => apply(addTab(current, panelId, tab, index)),
    closeTab: (tabId) => apply(closeTab(current, tabId)),
    activateTab: (tabId) => apply(activateTab(current, tabId)),
    moveTab: (tabId, targetPanelId, index) => apply(moveTab(current, tabId, targetPanelId, index)),
    undockTab: (tabId, zone) => apply(undockTab(current, tabId, zone)),
    dock: (panelId, targetPanelId, zone) => apply(dockPanel(current, panelId, targetPanelId, zone)),
    closePanel: (panelId) => apply(closePane(current, panelId)),
    focusPanel: (panelId) => apply(focusPane(current, panelId)),
    focusNeighbor: (dir) => apply(focusNeighbor(current, dir)),
    setRatio: (splitId, ratio) => apply(setPaneRatio(current, splitId, ratio)),
  };
}
