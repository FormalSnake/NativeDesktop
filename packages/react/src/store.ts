import { createSignal, onCleanup, type Accessor } from "solid-js";
import type { Store } from "@nativedesktop/react/core";

/** The store's value (or a selection of it) as a signal, subscribed for the
 *  lifetime of the calling owner. `select` must be pure. */
export function useStoreValue<T, S = T>(store: Store<T>, select?: (value: T) => S): Accessor<S> {
  const pick = (value: T): S => (select ? select(value) : (value as unknown as S));
  const [value, setValue] = createSignal<S>(pick(store.get()) as Exclude<S, Function>);
  onCleanup(store.subscribe((next) => setValue(() => pick(next))));
  return value;
}
