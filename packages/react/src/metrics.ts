// Platform spacing scale — the numbers each design language is built on
// (GNOME HIG: multiples of 6, 12 as the standard content margin; Apple HIG:
// an 8-based rhythm with 20 pt window margins). This is the fallback for
// bespoke `<box spacing>`/`style.padding` layout; the structural widgets
// (`<settingsgroup>`, `<row>`, `<clamp>`) carry native metrics themselves and
// need none of it. Keyed on the BACKEND (the design language of the widgets
// actually drawing — the GTK backend on macOS still lays out libadwaita), not
// the OS. The backend is only known after render()'s handshake, so Spacing
// resolves per property access and ContentMargin is re-resolved when the
// backend arrives; before that, both fall back to the OS's convention.

import { onBackendKnown, Platform } from "./platform.ts";

export interface SpacingScale {
  xs: number;
  sm: number;
  md: number;
  lg: number;
  xl: number;
}

const APPLE: SpacingScale = { xs: 4, sm: 8, md: 12, lg: 20, xl: 24 };
const GNOME: SpacingScale = { xs: 3, sm: 6, md: 12, lg: 18, xl: 24 };

function appleLanguage(): boolean {
  const backend = Platform.backend;
  if (backend !== "unknown") return backend === "appkit";
  return Platform.os === "macos";
}

export const Spacing: SpacingScale = {
  get xs() {
    return (appleLanguage() ? APPLE : GNOME).xs;
  },
  get sm() {
    return (appleLanguage() ? APPLE : GNOME).sm;
  },
  get md() {
    return (appleLanguage() ? APPLE : GNOME).md;
  },
  get lg() {
    return (appleLanguage() ? APPLE : GNOME).lg;
  },
  get xl() {
    return (appleLanguage() ? APPLE : GNOME).xl;
  },
};

/** The platform's standard margin between window edge and content. A live
 * binding (a number export cannot be a getter): read it at render time, not
 * into a module-scope constant. */
export let ContentMargin: number = appleLanguage() ? 20 : 12;

onBackendKnown(() => {
  ContentMargin = appleLanguage() ? 20 : 12;
});
