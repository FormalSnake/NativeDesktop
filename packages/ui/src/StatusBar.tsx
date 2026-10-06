import type { JSX } from "@nativedesktop/solid";
import { Spacing } from "@nativedesktop/solid";

export interface StatusBarProps {
  left?: JSX.Element;
  center?: JSX.Element;
  right?: JSX.Element;
  testID?: string;
}

export function StatusBar(props: StatusBarProps): JSX.Element {
  return (
    <box orientation="horizontal" spacing={Spacing.sm} cssClasses={["toolbar"]} testID={props.testID}>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ halign: "start" }}>
        {props.left}
      </box>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ hexpand: true, halign: "center" }}>
        {props.center}
      </box>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ halign: "end" }}>
        {props.right}
      </box>
    </box>
  );
}
