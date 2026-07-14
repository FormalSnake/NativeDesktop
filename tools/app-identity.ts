// tools/app-identity.ts — stamps an app's `app` config (file associations +
// URL schemes, packages/nd/src/config.ts's AppIdentity) into the packaging
// outputs `nd package` produces: macOS Info.plist and the Linux AppDir's
// .desktop entry + shared-mime-info XML. Pure string transforms so both
// package-mac.ts/package-linux.ts and tests can call them without touching
// the filesystem.
import type { AppIdentity } from "../packages/nd/src/config.ts";

function escapeXml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Inject CFBundleURLTypes / CFBundleDocumentTypes / CFBundleIdentifier into an Info.plist's XML text. */
export function injectInfoPlist(plistXml: string, app: AppIdentity | undefined): string {
  if (!app) return plistXml;
  let out = app.id
    ? plistXml.replace(/(<key>CFBundleIdentifier<\/key><string>)[^<]*(<\/string>)/, `$1${escapeXml(app.id)}$2`)
    : plistXml;

  let extra = "";
  if (app.urlSchemes?.length) {
    extra += "    <key>CFBundleURLTypes</key>\n    <array>\n";
    for (const s of app.urlSchemes) {
      extra += "        <dict>\n"
        + `            <key>CFBundleURLName</key><string>${escapeXml(s.name ?? app.id ?? s.scheme)}</string>\n`
        + "            <key>CFBundleURLSchemes</key>\n"
        + `            <array><string>${escapeXml(s.scheme)}</string></array>\n`
        + "        </dict>\n";
    }
    extra += "    </array>\n";
  }
  if (app.fileAssociations?.length) {
    extra += "    <key>CFBundleDocumentTypes</key>\n    <array>\n";
    for (const fa of app.fileAssociations) {
      const role = fa.role === "viewer" ? "Viewer" : "Editor";
      extra += "        <dict>\n"
        + "            <key>CFBundleTypeExtensions</key>\n"
        + `            <array><string>${escapeXml(fa.ext)}</string></array>\n`
        + `            <key>CFBundleTypeName</key><string>${escapeXml(fa.name ?? fa.ext)}</string>\n`
        + `            <key>CFBundleTypeRole</key><string>${role}</string>\n`
        + "            <key>LSHandlerRank</key><string>Owner</string>\n"
        + "        </dict>\n";
    }
    extra += "    </array>\n";
  }
  if (!extra) return out;
  return out.replace("</dict>\n</plist>", `${extra}</dict>\n</plist>`);
}

/** Extend a .desktop entry's MimeType= line and make sure Exec receives launch args (%U). */
export function injectDesktopFile(desktop: string, app: AppIdentity | undefined): string {
  if (!app) return desktop;
  const mimeTypes = [
    ...(app.fileAssociations ?? []).map((fa) => fa.mimeType).filter((m): m is string => !!m),
    ...(app.urlSchemes ?? []).map((s) => `x-scheme-handler/${s.scheme}`),
  ];
  if (!mimeTypes.length) return desktop;

  const mimeLine = `MimeType=${mimeTypes.join(";")};`;
  let out = /^MimeType=.*$/m.test(desktop)
    ? desktop.replace(/^MimeType=.*$/m, mimeLine)
    : `${desktop.trimEnd()}\n${mimeLine}\n`;

  const execLine = out.match(/^Exec=.*$/m)?.[0];
  if (execLine && !/%[uU]/.test(execLine)) out = out.replace(execLine, `${execLine} %U`);
  return out;
}

/** shared-mime-info XML declaring each mimeType'd file association (usr/share/mime/packages/<app>.xml). Returns null if there's nothing to declare. */
export function buildMimeInfoXml(app: AppIdentity | undefined): string | null {
  const withMime = (app?.fileAssociations ?? []).filter((fa): fa is typeof fa & { mimeType: string } => !!fa.mimeType);
  if (!withMime.length) return null;
  const entries = withMime.map((fa) =>
    `  <mime-type type="${escapeXml(fa.mimeType)}">\n`
    + `    <comment>${escapeXml(fa.name ?? fa.ext)}</comment>\n`
    + `    <glob pattern="*.${escapeXml(fa.ext)}"/>\n`
    + "  </mime-type>"
  ).join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>\n<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">\n${entries}\n</mime-info>\n`;
}
