/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
import { Spacing } from "@nativedesktop/react";

export interface StatusBarProps {
  left?: ReactNode;
  center?: ReactNode;
  right?: ReactNode;
  testID?: string;
}

export function StatusBar(props: StatusBarProps): ReactNode {
  const { left, center, right, testID } = props;
  return (
    <box orientation="horizontal" spacing={Spacing.sm} cssClasses={["toolbar"]} testID={testID}>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ halign: "start" }}>
        {left}
      </box>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ hexpand: true, halign: "center" }}>
        {center}
      </box>
      <box orientation="horizontal" spacing={Spacing.xs} style={{ halign: "end" }}>
        {right}
      </box>
    </box>
  );
}
