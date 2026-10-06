import { For } from "solid-js";
import type { JSX } from "@nativedesktop/solid";
import { accordionDragPayload, nextExpandedIds, parseAccordionDrag, reorderedIds } from "./accordion.ts";

export interface AccordionItem {
  id: string;
  label: string;
  content: JSX.Element;
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

export function Accordion(props: AccordionProps): JSX.Element {
  function onDropped(targetId: string, text: string): void {
    const moved = parseAccordionDrag(text);
    if (moved === undefined) return;
    const order = props.items.map((item) => item.id);
    const next = reorderedIds(order, moved, targetId);
    if (next !== order) props.onReorder?.(next);
  }

  // The <expander> IS the section header, so it is both the drag source and
  // the drop target: there is no separate handle widget to attach either to.
  return (
    <box orientation="vertical" testID={props.testID}>
      <For each={props.items} keyed={(item) => item.id}>
        {(item) => (
          <expander
            label={item().label}
            expanded={props.expandedIds.includes(item().id)}
            testID={props.testID ? `${props.testID}-item-${item().id}` : undefined}
            onToggled={(e) =>
              props.onExpandedChange(nextExpandedIds(props.expandedIds, item().id, e.checked, props.allowMultiple ?? false))
            }
            draggable={props.onReorder ? true : undefined}
            dragPayload={props.onReorder ? accordionDragPayload(item().id) : undefined}
            dropTarget={props.onReorder ? true : undefined}
            onDropped={props.onReorder ? (e) => onDropped(item().id, e.text) : undefined}
          >
            {item().content}
          </expander>
        )}
      </For>
    </box>
  );
}
