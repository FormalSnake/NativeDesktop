import { forwardRef, useCallback } from "./dev-react.ts";
import { createElement, type Ref } from "react";
import type { JSX, NdNodeRef } from "./generated/intrinsics.ts";
import type { Instance } from "./host-config.ts";
import { sendNativeCommand } from "./renderer.ts";

export interface NativeComponentOptions {
  /** Factory key registered by the app's platform-native library. */
  viewKind: string;
}

/// Host-level props (placement, styling, identity) forwarded verbatim to the
/// underlying <nativeview> intrinsic. Derived from the generated intrinsic —
/// minus the props defineNativeComponent manages itself — so codegen
/// additions (new slots, placement props) flow through without edits here.
type NativeViewHostProps = Omit<
  JSX.IntrinsicElements["nativeview"],
  "viewKind" | "props" | "onNativeEvent" | "ref" | "children"
>;

export interface NativeComponentProps<Props, Event = unknown> extends NativeViewHostProps {
  props: Props;
  onNativeEvent?: (event: { name: string; data: Event }) => void;
}

export interface NativeComponentRef<Command = unknown> extends NdNodeRef<"nativeview"> {
  send(command: string, arg?: Command): void;
}

/**
 * Defines a typed React component backed by an app-owned GTK/AppKit native view.
 * Props cross the stable NativeView ABI as JSON; events and commands use the
 * generic channel, so app components never require edits to widgets.json.
 */
export function defineNativeComponent<Props extends object, Event = unknown, Command = unknown>(
  options: NativeComponentOptions,
) {
  return forwardRef<NativeComponentRef<Command>, NativeComponentProps<Props, Event>>(function NativeComponent(componentProps, ref) {
    const { props, onNativeEvent, ...hostProps } = componentProps;
    // Stable across re-renders: a fresh callback ref would make the
    // reconciler detach (ref(null)) and reattach on every committed update.
    const setRef = useCallback((instance: Instance | null) => {
      if (typeof ref === "function") {
        ref(instance ? makeRef<Command>(instance) : null);
      } else if (ref) {
        (ref as { current: NativeComponentRef<Command> | null }).current = instance ? makeRef<Command>(instance) : null;
      }
    }, [ref]);
    return createElement("nativeview", {
      ...hostProps,
      viewKind: options.viewKind,
      props: JSON.stringify(props),
      onNativeEvent: onNativeEvent
        ? (event: { nativeName?: string; data?: Event }) => onNativeEvent({ name: event.nativeName ?? "", data: event.data as Event })
        : undefined,
      ref: setRef as Ref<NdNodeRef<"nativeview">>,
    });
  });
}

// One wrapper per instance, so ref consumers see a stable object identity
// for the lifetime of the underlying native view.
const refCache = new WeakMap<Instance, NativeComponentRef<unknown>>();

function makeRef<Command>(instance: Instance): NativeComponentRef<Command> {
  let wrapper = refCache.get(instance);
  if (!wrapper) {
    wrapper = {
      id: instance.id,
      type: "nativeview",
      send(command: string, arg?: unknown) {
        sendNativeCommand({ id: instance.id, type: "nativeview" }, command, arg);
      },
    };
    refCache.set(instance, wrapper);
  }
  return wrapper as NativeComponentRef<Command>;
}
