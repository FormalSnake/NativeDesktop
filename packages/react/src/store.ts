// React binding for @nativedesktop/core's settings store.

import type { Store } from "@nativedesktop/core";
import { useMemo, useSyncExternalStore } from "./dev-react.ts";

/** Subscribes a component to the store (or a selection of it). The selected
 * snapshot is cached per store value, so `select` may return a fresh object
 * without re-render loops, but it must be pure. */
export function useStoreValue<T, S = T>(store: Store<T>, select?: (value: T) => S): S {
  const getSnapshot = useMemo(() => {
    let lastValue: T | undefined;
    let lastSelected: S;
    let primed = false;
    return (): S => {
      const value = store.get();
      if (!primed || !Object.is(value, lastValue)) {
        lastValue = value;
        lastSelected = select ? select(value) : (value as unknown as S);
        primed = true;
      }
      return lastSelected;
    };
  }, [store, select]);
  const subscribe = useMemo(() => {
    return (onChange: () => void) => store.subscribe(() => onChange());
  }, [store]);
  return useSyncExternalStore(subscribe, getSnapshot);
}
