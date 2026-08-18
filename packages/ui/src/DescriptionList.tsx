/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";

export interface DescriptionListItem {
  label: string;
  value: string;
}

export interface DescriptionListProps {
  items: DescriptionListItem[];
  title?: string;
  testID?: string;
}

export function DescriptionList(props: DescriptionListProps): ReactNode {
  const { items, title, testID } = props;
  return (
    <settingsgroup title={title} testID={testID}>
      {items.map((item, i) => (
        <row
          key={`${item.label}-${i}`}
          title={item.label}
          subtitle={item.value}
          cssClasses={["property"]}
          testID={testID ? `${testID}-row-${i}` : undefined}
        />
      ))}
    </settingsgroup>
  );
}
