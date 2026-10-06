import { For } from "solid-js";
import type { JSX } from "@nativedesktop/solid";

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

export function ButtonGroup(props: ButtonGroupProps): JSX.Element {
  return (
    <box orientation="horizontal" cssClasses={["linked"]} testID={props.testID}>
      <For each={props.items} keyed={(item) => item.id}>
        {(item) => (
          <button
            label={item().label}
            iconName={item().iconName}
            prominent={item().id === props.selectedId}
            onClick={() => props.onPress(item().id)}
            testID={props.testID ? `${props.testID}-${item().id}` : undefined}
          />
        )}
      </For>
    </box>
  );
}
