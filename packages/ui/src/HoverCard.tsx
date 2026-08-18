/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
import { useMountEffect, useRef, useState } from "@nativedesktop/react";

export interface HoverCardProps {
  content: ReactNode;
  children: ReactNode;
  /** Ms of continuous hover before the card opens. */
  openDelay?: number;
  /** Ms after the pointer leaves before the card closes. */
  closeDelay?: number;
  testID?: string;
}

/** The anchor's `hoverChanged` is the only signal available (no separate
 * enter/leave events), so open/close both key off that one boolean, each on
 * its own timer so a quick pass-through never flashes the card open. */
export function HoverCard(props: HoverCardProps): ReactNode {
  const { content, children, openDelay = 400, closeDelay = 200, testID } = props;
  const [open, setOpen] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  function clearPending(): void {
    if (timerRef.current !== undefined) {
      clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
  }

  useMountEffect(() => clearPending);

  function handleHoverChanged(hovering: boolean): void {
    clearPending();
    timerRef.current = setTimeout(() => setOpen(hovering), hovering ? openDelay : closeDelay);
  }

  return (
    <box orientation="vertical" onHoverChanged={(e) => handleHoverChanged(e.checked)} testID={testID}>
      {children}
      <popover open={open} onClosed={() => setOpen(false)} testID={testID ? `${testID}-popover` : undefined}>
        {content}
      </popover>
    </box>
  );
}
