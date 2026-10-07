import { For } from "solid-js";
import type { JSX } from "@nativedesktop/react";

export interface DescriptionListItem {
  label: string;
  value: string;
}

export interface DescriptionListProps {
  items: DescriptionListItem[];
  title?: string;
  testID?: string;
}

export function DescriptionList(props: DescriptionListProps): JSX.Element {
  return (
    <settingsgroup title={props.title} testID={props.testID}>
      <For each={props.items} keyed={false}>
        {(item, i) => (
          <row
            title={item().label}
            subtitle={item().value}
            cssClasses={["property"]}
            testID={props.testID ? `${props.testID}-row-${i}` : undefined}
          />
        )}
      </For>
    </settingsgroup>
  );
}
