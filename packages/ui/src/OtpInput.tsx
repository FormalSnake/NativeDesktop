import { For, createMemo } from "solid-js";
import type { JSX, NdNodeRef } from "@nativedesktop/solid";
import { Spacing, sendCommand } from "@nativedesktop/solid";
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
export function OtpInput(props: OtpInputProps): JSX.Element {
  const length = (): number => props.length ?? 6;
  const chars = createMemo(() => otpChars(props.value, length()));
  const cells: (NdNodeRef<"textinput"> | undefined)[] = [];

  function onCellChanged(i: number, text: string): void {
    const result = otpCellChanged(props.value, length(), i, text);
    props.onChange(result.value);
    if (result.activeIndex !== i) {
      const next = cells[result.activeIndex];
      if (next) sendCommand(next, "focus");
    }
    if (props.onComplete && result.value.length === length()) props.onComplete(result.value);
  }

  // Unkeyed: a cell is its index, so a typed character updates the cell's
  // text instead of replacing the widget that holds the caret.
  return (
    <box orientation="horizontal" spacing={Spacing.xs} testID={props.testID}>
      <For each={chars()} keyed={false}>
        {(ch, i) => (
          <textinput
            ref={(node) => (cells[i] = node)}
            text={ch()}
            cssClasses={["numeric"]}
            testID={props.testID ? `${props.testID}-cell-${i}` : undefined}
            onChanged={(e) => onCellChanged(i, e.text)}
          />
        )}
      </For>
    </box>
  );
}
