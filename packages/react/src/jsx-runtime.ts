import type { ReactNode } from "react";

export { jsx, jsxs, Fragment } from "react/jsx-runtime";

// The intrinsic elements live in this module's own `JSX` namespace, not a
// `declare global`/`declare module "react"` augmentation: @types/react
// unconditionally declares `label`/`button` (and other HTML tag names) on its
// own `IntrinsicElements`, so reusing those names there is a hard conflict
// (the DOM-shaped prop type always wins). Pointing `jsxImportSource` at this
// package instead of "react" makes the `react-jsx` transform resolve
// `JSX.IntrinsicElements` from here, with no HTML tags to collide with.
export namespace JSX {
  export interface IntrinsicElements {
    window: { title?: string; defaultWidth?: number; defaultHeight?: number; testID?: string; children?: ReactNode };
    box: { orientation?: "vertical" | "horizontal"; spacing?: number; testID?: string; children?: ReactNode };
    label: { text?: string; testID?: string; children?: ReactNode };
    button: { label?: string; onClick?: () => void; testID?: string; children?: ReactNode };
  }
  export type Element = ReactNode;
  export interface ElementChildrenAttribute {
    children: {};
  }
}
