// App-facing hooks built on the pinned first-eval react instance
// (dev-react.ts), so they keep working across `nd dev` hot re-evals.

import { useEffect } from "./dev-react.ts";

/** One-time external sync on mount; the returned cleanup runs on unmount.
 * The named wrapper marks the one useEffect shape that has no derived-state
 * replacement, so call sites don't carry a raw `useEffect(fn, [])`. */
export function useMountEffect(effect: () => void | (() => void)): void {
  useEffect(effect, []);
}
