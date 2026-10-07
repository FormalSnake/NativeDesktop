import { For, createSignal, type Accessor } from "solid-js";
import type { JSX } from "@nativedesktop/react";
import { applyTileDrop, moveTile, placeTile, raiseTile, removeTile, resizeTile, tileDragPayload, updateTile } from "./tiles.ts";
import type { Tile, TileModel, TilePlacement, TileSize } from "./tiles.ts";

/** What renderTile receives. `tile` and `raised` are reactive getters, the way
 * component props are: read them where they are used, never destructure them
 * in the parameter list. */
export interface TileContext<T> {
  readonly id: string;
  readonly tile: Tile<T>;
  /** True for the last tile in layout order, the one drawn on top. */
  readonly raised: boolean;
  /** Drag handle for THIS tile. The tile's own box carries it already once
   * `onChange` and `size` are both set, so spread this only to add a second
   * handle or after turning `dragTiles` off. */
  readonly dragProps: { draggable: true; dragPayload: string };
}

export interface TilesViewProps<T> {
  model: TileModel<T>;
  /** Owns all per-tile chrome, the way PaneTree's renderLeaf does. TilesView
   * supplies the <grid> and one expanding <box> per tile, carrying the
   * gridRow/gridColumn spans the model computed. Runs once per tile mount. */
  renderTile: (ctx: TileContext<T>) => JSX.Element;
  /** Where a dropped tile lands. Omit it (or `size`) and the grid stays
   * display-only: every layout change then comes from a model op the app
   * calls. */
  onChange?: (next: TileModel<T>) => void;
  /** Pixel extent of the grid, which is what turns a drop point into a cell.
   * The grid reports no geometry of its own, so without this a drop has
   * nothing to resolve against and is ignored. */
  size?: TileSize;
  /** Default true: the tile's box is its own drag handle, once `onChange` and
   * `size` make a drop resolvable. Turn it off for tile content that owns its
   * drag gestures and put `dragProps` on chrome instead. */
  dragTiles?: boolean;
  testID?: string;
}

/** Renders a tile layout into the existing <grid> widget. The grid is the drop
 * target for the whole layout: a tile dropped on it takes the cell under the
 * pointer as its top-left, and the tiles it lands on are pushed clear. */
export function TilesView<T>(props: TilesViewProps<T>): JSX.Element {
  // Dragging a tile that has nowhere to land is worse than a tile that does
  // not drag, so both halves turn on together: a drop needs somewhere to send
  // the new model AND the grid extent to resolve the cell against.
  const canDrop = (): boolean => props.onChange !== undefined && props.size !== undefined;
  const dragTiles = (): boolean => (props.dragTiles ?? true) && canDrop();

  // Reads props at event time, so the drop applies against the latest model,
  // never the one current when the grid mounted.
  function onDrop(payload: string, x: number, y: number): void {
    const handler = props.onChange;
    const size = props.size;
    if (!handler || !size) return;
    const current = props.model;
    const next = applyTileDrop(current, payload, size, x, y);
    if (next !== current) handler(next);
  }

  return (
    <grid testID={props.testID} dropTarget={canDrop()} onDropped={(e) => onDrop(e.text, e.data.x, e.data.y)}>
      <For each={props.model.tiles} keyed={(tile) => tile.id}>
        {(tile, index) => {
          const tileId = tile().id;
          const drag = { draggable: true, dragPayload: tileDragPayload(tileId) } as const;
          const ctx: TileContext<T> = {
            id: tileId,
            get tile() {
              return tile();
            },
            get raised() {
              return index() === props.model.tiles.length - 1;
            },
            dragProps: drag,
          };
          return (
            <box
              gridRow={tile().y}
              gridColumn={tile().x}
              gridRowSpan={tile().h}
              gridColumnSpan={tile().w}
              style={{ hexpand: true, vexpand: true }}
              testID={props.testID ? `${props.testID}-tile-${tileId}` : undefined}
              draggable={dragTiles() ? true : undefined}
              dragPayload={dragTiles() ? drag.dragPayload : undefined}
            >
              {props.renderTile(ctx)}
            </box>
          );
        }}
      </For>
    </grid>
  );
}

export interface TilesState<T> {
  model: Accessor<TileModel<T>>;
  /** The model as of the last op, not the last flush: a write is staged until
   * Solid flushes, and `model()` reads the committed value until then. */
  latest: () => TileModel<T>;
  setModel(m: TileModel<T>): void;
  place(placement: TilePlacement<T>): void;
  move(id: string, x: number, y: number): void;
  resize(id: string, w: number, h: number): void;
  raise(id: string): void;
  remove(id: string): void;
  update(id: string, fn: (data: T) => T): void;
}

/** Holds the layout in a signal and applies every op against the latest
 * model, never the committed one, so two ops in one tick (or an op after an
 * await) compose instead of the second reverting the first. An op that
 * changes nothing returns the same reference and writes nothing. */
export function createTiles<T>(initial: TileModel<T> | (() => TileModel<T>)): TilesState<T> {
  let current = typeof initial === "function" ? initial() : initial;
  const [model, setState] = createSignal(current);

  const apply = (next: TileModel<T>): void => {
    if (next === current) return;
    current = next;
    setState(() => next);
  };

  return {
    model,
    latest: () => current,
    setModel: apply,
    place: (placement) => apply(placeTile(current, placement)),
    move: (id, x, y) => apply(moveTile(current, id, x, y)),
    resize: (id, w, h) => apply(resizeTile(current, id, w, h)),
    raise: (id) => apply(raiseTile(current, id)),
    remove: (id) => apply(removeTile(current, id)),
    update: (id, fn) => apply(updateTile(current, id, fn)),
  };
}
