import type { ReactNode } from "react";

declare global {
  namespace JSX {
    interface IntrinsicElements {
      window: { title?: string; defaultWidth?: number; defaultHeight?: number; children?: ReactNode };
      box: { orientation?: "vertical" | "horizontal"; spacing?: number; children?: ReactNode };
      label: { text?: string; children?: ReactNode };
      button: { label?: string; onClick?: () => void; children?: ReactNode };
    }
  }
}

export {};
