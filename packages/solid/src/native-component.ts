import { omit, type Element as SolidElement, type Ref } from "solid-js";
import { sendNativeCommand } from "@nativedesktop/core";
import type { JSX, NdNodeRef } from "./generated/intrinsics.ts";
import { applyRef, createElement, mergeProps, ref, spread, type SolidNode } from "./renderer.ts";

export interface NativeComponentOptions {
  /** Factory key registered by the app's platform-native library. */
  viewKind: string;
}

/// Host-level props (placement, styling, identity) forwarded verbatim to the
/// underlying <nativeview> intrinsic, derived from the generated intrinsic so
/// codegen additions flow through without edits here.
type NativeViewHostProps = Omit<JSX.IntrinsicElements["nativeview"], "viewKind" | "props" | "onNativeEvent" | "ref" | "children">;

export interface NativeComponentProps<Props, Event = unknown, Command = unknown> extends NativeViewHostProps {
  props: Props;
  onNativeEvent?: (event: { name: string; data: Event }) => void;
  ref?: Ref<NativeComponentRef<Command>>;
}

export interface NativeComponentRef<Command = unknown> extends NdNodeRef<"nativeview"> {
  send(command: string, arg?: Command): void;
}

/**
 * Defines a typed Solid component backed by an app-owned GTK/AppKit native view.
 * Props cross the stable NativeView ABI as JSON; events and commands use the
 * generic channel, so app components never require edits to widgets.json.
 */
export function defineNativeComponent<Props extends object, Event = unknown, Command = unknown>(
  options: NativeComponentOptions,
): (props: NativeComponentProps<Props, Event, Command>) => SolidElement {
  // Plain .ts rather than JSX, so an app built ahead of time (build.ts) never
  // loads babel at startup. This is the shape babel-preset-solid emits for a
  // spread intrinsic.
  return function NativeComponent(p) {
    const host: NativeViewHostProps = omit(p, "props", "onNativeEvent", "ref");
    const el = createElement("nativeview");
    ref(
      () => (node: SolidNode) => {
        // applyRef is typed for the renderer's own nodes; it accepts any ref value.
        if (p.ref) (applyRef as (ref: unknown, value: unknown) => void)(p.ref, makeRef<Command>(node));
      },
      el,
    );
    spread(
      el,
      mergeProps(host, {
        get viewKind() {
          return options.viewKind;
        },
        get props() {
          return JSON.stringify(p.props);
        },
        get onNativeEvent() {
          const handler = p.onNativeEvent;
          return handler
            ? (event: { nativeName?: string; data?: unknown }) => handler({ name: event.nativeName ?? "", data: event.data as Event })
            : undefined;
        },
      }) as object,
      false,
    );
    return el as unknown as SolidElement;
  };
}

// The node keeps its identity across a remount but takes a fresh wire id,
// so the wrapper reads the id live rather than copying it.
function makeRef<Command>(node: SolidNode): NativeComponentRef<Command> {
  const wrapper: NativeComponentRef<Command> = {
    get id() {
      return node.id;
    },
    type: "nativeview",
    send(command: string, arg?: Command) {
      sendNativeCommand(wrapper, command, arg);
    },
  };
  return wrapper;
}
