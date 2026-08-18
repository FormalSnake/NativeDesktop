/** @jsxImportSource @nativedesktop/react */
// The pragma is load-bearing: this is a raw-src package, and Bun transpiles
// node_modules TSX with the ROOT tsconfig's JSX settings, so the per-file
// pragma is what guarantees the ND jsx-runtime regardless of the consumer.
//
// Hooks come from @nativedesktop/react, never react directly (dev-react.ts's
// pinned-dispatcher contract for `nd dev` hot re-eval).

import { useRef, useState } from "@nativedesktop/react";
import type { ReactNode } from "react";
import { applyTileDrop, moveTile, placeTile, raiseTile, removeTile, resizeTile, tileDragPayload, updateTile } from "./tiles.ts";
import type { Tile, TileModel, TilePlacement, TileSize } from "./tiles.ts";

export interface TileContext<T> {
  id: string;
  tile: Tile<T>;
  /** True for the last tile in layout order, the one drawn on top. */
  raised: boolean;
  /** Drag handle for THIS tile. The tile's own box carries it already once
   * `onChange` and `size` are both set, so spread this only to add a second
   * handle or after turning `dragTiles` off. */
  dragProps: { draggable: true; dragPayload: string };
}

export interface TilesViewProps<T> {
  model: TileModel<T>;
  /** Owns all per-tile chrome, the way PaneTree's renderLeaf does. TilesView
   * supplies the <grid> and one expanding <box> per tile, carrying the
   * gridRow/gridColumn spans the model computed. */
  renderTile: (ctx: TileContext<T>) => ReactNode;
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
export function TilesView<T>(props: TilesViewProps<T>): ReactNode {
  const { model, renderTile, onChange, size, dragTiles = true, testID } = props;
  // Latest-ref, same reason as DockView: the drop handler that fires was
  // captured at an earlier render, and applying it against that render's
  // model would revert everything committed since.
  const modelRef = useRef(model);
  modelRef.current = model;
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const last = model.tiles.length - 1;
  // Dragging a tile that has nowhere to land is worse than a tile that does
  // not drag, so both halves turn on together: a drop needs somewhere to send
  // the new model AND the grid extent to resolve the cell against.
  const canDrop = onChange !== undefined && size !== undefined;

  function onDrop(payload: string, x: number, y: number): void {
    const handler = onChangeRef.current;
    if (!handler || !size) return;
    const current = modelRef.current;
    const next = applyTileDrop(current, payload, size, x, y);
    if (next !== current) handler(next);
  }

  return (
    <grid testID={testID} dropTarget={canDrop} onDropped={(e) => onDrop(e.text, e.data.x, e.data.y)}>
      {model.tiles.map((tile, index) => {
        const drag = { draggable: true, dragPayload: tileDragPayload(tile.id) } as const;
        return (
          <box
            key={tile.id}
            gridRow={tile.y}
            gridColumn={tile.x}
            gridRowSpan={tile.h}
            gridColumnSpan={tile.w}
            style={{ hexpand: true, vexpand: true }}
            testID={testID ? `${testID}-tile-${tile.id}` : undefined}
            {...(dragTiles && canDrop ? drag : {})}
          >
            {renderTile({ id: tile.id, tile, raised: index === last, dragProps: drag })}
          </box>
        );
      })}
    </grid>
  );
}

export interface UseTiles<T> {
  model: TileModel<T>;
  /** The latest-ref invariant, built in: reads the model as of the last op,
   * not the last render. */
  latest: () => TileModel<T>;
  setModel(m: TileModel<T>): void;
  place(placement: TilePlacement<T>): void;
  move(id: string, x: number, y: number): void;
  resize(id: string, w: number, h: number): void;
  raise(id: string): void;
  remove(id: string): void;
  update(id: string, fn: (data: T) => T): void;
}

/** Holds the layout in state and applies every op against a ref, never the
 * render-time model, so an await-resuming op can't revert a move committed
 * while it was suspended. */
export function useTiles<T>(initial: TileModel<T> | (() => TileModel<T>)): UseTiles<T> {
  const [model, setState] = useState(initial);
  const ref = useRef(model);

  const apply = (next: TileModel<T>): void => {
    if (next === ref.current) return;
    ref.current = next;
    setState(next);
  };

  return {
    model,
    latest: () => ref.current,
    setModel: apply,
    place: (placement) => apply(placeTile(ref.current, placement)),
    move: (id, x, y) => apply(moveTile(ref.current, id, x, y)),
    resize: (id, w, h) => apply(resizeTile(ref.current, id, w, h)),
    raise: (id) => apply(raiseTile(ref.current, id)),
    remove: (id) => apply(removeTile(ref.current, id)),
    update: (id, fn) => apply(updateTile(ref.current, id, fn)),
  };
}
