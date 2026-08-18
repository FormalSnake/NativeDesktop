/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
import type { NdNodeRef } from "@nativedesktop/react";
import { Spacing, sendCommand, useRef } from "@nativedesktop/react";
import { otpCellChanged, otpChars } from "./otp.ts";

export interface OtpInputProps {
  length?: number;
  value: string;
  onChange: (value: string) => void;
  onComplete?: (value: string) => void;
  testID?: string;
}

/** N single-character <textinput>s. otp.ts computes the cell focus should move
 * to, and the `focus` command moves the caret there, so typing, backspace and
 * paste all advance without the user clicking between boxes. */
export function OtpInput(props: OtpInputProps): ReactNode {
  const { length = 6, value, onChange, onComplete, testID } = props;
  const chars = otpChars(value, length);
  const cells = useRef<(NdNodeRef<"textinput"> | null)[]>([]);

  return (
    <box orientation="horizontal" spacing={Spacing.xs} testID={testID}>
      {chars.map((ch, i) => (
        <textinput
          key={i}
          ref={(node) => { cells.current[i] = node; }}
          text={ch}
          cssClasses={["numeric"]}
          testID={testID ? `${testID}-cell-${i}` : undefined}
          onChanged={(e) => {
            const result = otpCellChanged(value, length, i, e.text);
            onChange(result.value);
            if (result.activeIndex !== i) {
              const next = cells.current[result.activeIndex];
              if (next) sendCommand(next, "focus");
            }
            if (onComplete && result.value.length === length) onComplete(result.value);
          }}
        />
      ))}
    </box>
  );
}
