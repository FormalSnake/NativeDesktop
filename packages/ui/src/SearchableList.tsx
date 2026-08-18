/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
import { Spacing, useMemo, useState } from "@nativedesktop/react";
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

export function SearchableList(props: SearchableListProps): ReactNode {
  const { items, onActivate, filter, placeholder, emptyIconName, emptyTitle, emptyDescription, testID } = props;
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => filterItems(items, query, filter), [items, query, filter]);

  return (
    <box orientation="vertical" spacing={Spacing.sm} testID={testID}>
      <searchinput
        text={query}
        placeholder={placeholder}
        onChanged={(e) => setQuery(e.text)}
        testID={testID ? `${testID}-search` : undefined}
      />
      <listview
        items={filtered.map((item) => item.label)}
        emptyIconName={emptyIconName}
        emptyTitle={emptyTitle}
        emptyDescription={emptyDescription}
        style={{ vexpand: true }}
        onRowActivated={(e) => {
          const item = filtered[e.index];
          if (item) onActivate(item);
        }}
        testID={testID ? `${testID}-list` : undefined}
      />
    </box>
  );
}
