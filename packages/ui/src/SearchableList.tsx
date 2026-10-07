import { createMemo, createSignal } from "solid-js";
import type { JSX } from "@nativedesktop/react";
import { Spacing } from "@nativedesktop/react";
import { filterItems } from "./searchable-list.ts";
import type { SearchableListFilter, SearchableListItem } from "./searchable-list.ts";

export interface SearchableListProps {
  items: SearchableListItem[];
  onActivate: (item: SearchableListItem) => void;
  filter?: SearchableListFilter;
  placeholder?: string;
  emptyIconName?: string;
  emptyTitle?: string;
  emptyDescription?: string;
  testID?: string;
}

export function SearchableList(props: SearchableListProps): JSX.Element {
  const [query, setQuery] = createSignal("");
  const filtered = createMemo(() => filterItems(props.items, query(), props.filter));

  return (
    <box orientation="vertical" spacing={Spacing.sm} testID={props.testID}>
      <searchinput
        text={query()}
        placeholder={props.placeholder}
        onChanged={(e) => setQuery(e.text)}
        testID={props.testID ? `${props.testID}-search` : undefined}
      />
      <listview
        items={filtered().map((item) => item.label)}
        emptyIconName={props.emptyIconName}
        emptyTitle={props.emptyTitle}
        emptyDescription={props.emptyDescription}
        style={{ vexpand: true }}
        onRowActivated={(e) => {
          const item = filtered()[e.index];
          if (item) props.onActivate(item);
        }}
        testID={props.testID ? `${props.testID}-list` : undefined}
      />
    </box>
  );
}
