/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";

export interface ButtonGroupItem {
  id: string;
  label: string;
  iconName?: string;
}

export interface ButtonGroupProps {
  items: ButtonGroupItem[];
  onPress: (id: string) => void;
  /** Toggle mode: the matching button renders prominent. Omit for a plain
   * action group with no selection state. */
  selectedId?: string;
  testID?: string;
}

export function ButtonGroup(props: ButtonGroupProps): ReactNode {
  const { items, onPress, selectedId, testID } = props;
  return (
    <box orientation="horizontal" cssClasses={["linked"]} testID={testID}>
      {items.map((item) => (
        <button
          key={item.id}
          label={item.label}
          iconName={item.iconName}
          prominent={item.id === selectedId}
          onClick={() => onPress(item.id)}
          testID={testID ? `${testID}-${item.id}` : undefined}
        />
      ))}
    </box>
  );
}
