// Pure client-side filtering for SearchableList.

export interface SearchableListItem {
  id: string;
  label: string;
}

export type SearchableListFilter = (item: SearchableListItem, query: string) => boolean;

export function defaultFilter(item: SearchableListItem, query: string): boolean {
  return item.label.toLowerCase().includes(query.toLowerCase());
}

/** Empty query is a same-reference no-op: every item passes, and skipping
 * the filter call keeps an unrelated re-render from allocating a new array. */
export function filterItems(
  items: readonly SearchableListItem[],
  query: string,
  filter: SearchableListFilter = defaultFilter,
): SearchableListItem[] {
  if (query.length === 0) return items as SearchableListItem[];
  return items.filter((item) => filter(item, query));
}
