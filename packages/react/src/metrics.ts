// Platform spacing scale — the numbers each OS's design language is built on
// (GNOME HIG: multiples of 6, 12 as the standard content margin; Apple HIG:
// an 8-based rhythm with 20 pt window margins). This is the fallback for
// bespoke `<box spacing>`/`style.padding` layout; the structural widgets
// (`<settingsgroup>`, `<row>`, `<clamp>`) carry native metrics themselves and
// need none of it. Resolved once from `Platform.os` — the process never
// changes OS mid-run.

import { Platform } from "./platform.ts";

export interface SpacingScale {
  xs: number;
  sm: number;
  md: number;
  lg: number;
  xl: number;
}

export const Spacing: SpacingScale =
  Platform.os === "macos"
    ? { xs: 4, sm: 8, md: 12, lg: 20, xl: 24 }
    : { xs: 3, sm: 6, md: 12, lg: 18, xl: 24 };

/** The platform's standard margin between window edge and content. */
export const ContentMargin: number = Platform.os === "macos" ? 20 : 12;
