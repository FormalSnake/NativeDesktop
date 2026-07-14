import { forwardRef } from "./dev-react.ts";
import { createElement, type Ref } from "react";
import type { NdNodeRef, StyleProp } from "./generated/intrinsics.ts";
import type { Instance } from "./host-config.ts";
import { sendNativeCommand } from "./renderer.ts";

export interface NativeComponentOptions {
  /** Factory key registered by the app's platform-native library. */
  viewKind: string;
}

export interface NativeComponentProps<Props, Event = unknown> {
  props: Props;
  onNativeEvent?: (event: { name: string; data: Event }) => void;
  testID?: string;
  style?: StyleProp;
  cssClasses?: string[];
  key?: string | number | null;
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
    const setRef = (instance: Instance | null) => {
      if (typeof ref === "function") {
        ref(instance ? makeRef<Command>(instance) : null);
      } else if (ref) {
        (ref as { current: NativeComponentRef<Command> | null }).current = instance ? makeRef<Command>(instance) : null;
      }
    };
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

function makeRef<Command>(instance: Instance): NativeComponentRef<Command> {
  return {
    id: instance.id,
    type: "nativeview",
    send(command: string, arg?: Command) {
      sendNativeCommand(instance, command, arg);
    },
  };
}
