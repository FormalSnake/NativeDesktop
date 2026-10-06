// Plain solid-js with no JSX and nothing NativeDesktop-specific, so the same
// file works in a web or any other Solid codebase.
import { createSignal, type Accessor } from "solid-js";

export function useToggle(initial = false): [Accessor<boolean>, () => void] {
  const [on, setOn] = createSignal(initial);
  return [on, () => setOn((v) => !v)];
}
