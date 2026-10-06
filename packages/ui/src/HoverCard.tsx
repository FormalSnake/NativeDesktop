import { createSignal, onCleanup } from "solid-js";
import type { JSX } from "@nativedesktop/solid";

export interface HoverCardProps {
  content: JSX.Element;
  children: JSX.Element;
  /** Ms of continuous hover before the card opens. */
  openDelay?: number;
  /** Ms after the pointer leaves before the card closes. */
  closeDelay?: number;
  testID?: string;
}

/** The anchor's `hoverChanged` is the only signal available (no separate
 * enter/leave events), so open/close both key off that one boolean, each on
 * its own timer so a quick pass-through never flashes the card open. */
export function HoverCard(props: HoverCardProps): JSX.Element {
  const [open, setOpen] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function clearPending(): void {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  }

  onCleanup(clearPending);

  function handleHoverChanged(hovering: boolean): void {
    clearPending();
    timer = setTimeout(() => setOpen(hovering), hovering ? (props.openDelay ?? 400) : (props.closeDelay ?? 200));
  }

  return (
    <box orientation="vertical" onHoverChanged={(e) => handleHoverChanged(e.checked)} testID={props.testID}>
      {props.children}
      <popover open={open()} onClosed={() => setOpen(false)} testID={props.testID ? `${props.testID}-popover` : undefined}>
        {props.content}
      </popover>
    </box>
  );
}
