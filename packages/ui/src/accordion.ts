// Pure expansion-state transition for Accordion. Each <expander> reports its
// own `checked` on toggle; this decides what the FULL expandedIds list
// becomes, including closing siblings in single mode. Returns the same
// reference when the toggle is a no-op (e.g. an echo of the already-applied
// state), matching the panes model's echo-guard convention.

export function nextExpandedIds(
  expandedIds: readonly string[],
  id: string,
  checked: boolean,
  allowMultiple: boolean,
): string[] {
  if (allowMultiple) {
    const has = expandedIds.includes(id);
    if (checked === has) return expandedIds as string[];
    return checked ? [...expandedIds, id] : expandedIds.filter((x) => x !== id);
  }
  const isOnlyOpen = expandedIds.length === 1 && expandedIds[0] === id;
  if (checked) return isOnlyOpen ? (expandedIds as string[]) : [id];
  return isOnlyOpen ? [] : (expandedIds as string[]);
}

const DRAG_PREFIX = "nd-accordion:item:";

/** The `dragPayload` a reorderable section header carries. Namespaced because
 * the drop side also receives plain text dragged in from other applications,
 * and that must not be read as a section id. */
export function accordionDragPayload(id: string): string {
  return `${DRAG_PREFIX}${id}`;
}

export function parseAccordionDrag(payload: string): string | undefined {
  if (!payload.startsWith(DRAG_PREFIX)) return undefined;
  const id = payload.slice(DRAG_PREFIX.length);
  return id.length > 0 ? id : undefined;
}

/** Order after dropping section `movedId` on section `targetId`: the dragged
 * section takes the target's place and the ones between it shift up. Returns
 * the same reference when the drop changes nothing (a section dropped on
 * itself, or an id from another accordion), so a drag that ends where it
 * started cannot loop a render+persist cycle. */
export function reorderedIds(ids: readonly string[], movedId: string, targetId: string): string[] {
  const from = ids.indexOf(movedId);
  const to = ids.indexOf(targetId);
  if (from < 0 || to < 0 || from === to) return ids as string[];
  const next = ids.slice();
  next.splice(from, 1);
  next.splice(to, 0, movedId);
  return next;
}
