/** @jsxImportSource @nativedesktop/react */
// The pragma is load-bearing: see packages/panes/src/PaneTree.tsx for why.

import type { ReactNode } from "react";
import { accordionDragPayload, nextExpandedIds, parseAccordionDrag, reorderedIds } from "./accordion.ts";

export interface AccordionItem {
  id: string;
  label: string;
  content: ReactNode;
}

export interface AccordionProps {
  items: AccordionItem[];
  expandedIds: string[];
  onExpandedChange: (ids: string[]) => void;
  /** Default false: opening one section closes the others. */
  allowMultiple?: boolean;
  /** Supply it and the sections become reorderable by dragging one header
   * onto another: the dragged section takes the target's place. It is handed
   * the full id order, in the same shape `expandedIds` uses, so the app keeps
   * owning `items`. Omit it and no header is a drag source. */
  onReorder?: (ids: string[]) => void;
  testID?: string;
}

export function Accordion(props: AccordionProps): ReactNode {
  const { items, expandedIds, onExpandedChange, allowMultiple = false, onReorder, testID } = props;

  // The <expander> IS the section header, so it is both the drag source and
  // the drop target: there is no separate handle widget to attach either to.
  function reorderProps(id: string): {
    draggable: true;
    dragPayload: string;
    dropTarget: true;
    onDropped: (e: { text: string }) => void;
  } {
    return {
      draggable: true,
      dragPayload: accordionDragPayload(id),
      dropTarget: true,
      onDropped: (e) => {
        const moved = parseAccordionDrag(e.text);
        if (moved === undefined) return;
        const order = items.map((item) => item.id);
        const next = reorderedIds(order, moved, id);
        if (next !== order) onReorder?.(next);
      },
    };
  }

  return (
    <box orientation="vertical" testID={testID}>
      {items.map((item) => (
        <expander
          key={item.id}
          label={item.label}
          expanded={expandedIds.includes(item.id)}
          testID={testID ? `${testID}-item-${item.id}` : undefined}
          onToggled={(e) => onExpandedChange(nextExpandedIds(expandedIds, item.id, e.checked, allowMultiple))}
          {...(onReorder ? reorderProps(item.id) : {})}
        >
          {item.content}
        </expander>
      ))}
    </box>
  );
}
