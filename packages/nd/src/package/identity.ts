// App identity resolution + the pure string transforms `nd package` stamps
// into packaging outputs: macOS Info.plist, the Linux AppDir's .desktop entry,
// shared-mime-info XML, and the AppRun launcher. No filesystem writes here so
// tests can cover everything without touching disk.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { AppIcon, AppIdentity, LinuxPackageConfig, MacPackageConfig, NativeDesktopConfig } from "../config.ts";
import { appRunTemplate, desktopEntryTemplate, infoPlistSkeleton, type AppRunSpec } from "./templates.ts";

function escapeXml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/**
 * Replaces a plist key's <string> value, tolerating any whitespace between
 * the tags (Xcode/plutil put the <string> on its own line). Throws when the
 * key is absent so a custom plist can't ship silently unstamped.
 */
function stampPlistString(xml: string, key: string, value: string): string {
  const re = new RegExp(`(<key>${key}</key>\\s*<string>)[^<]*(</string>)`);
  if (!re.test(xml)) throw new Error(`nd: Info.plist has no <key>${key}</key> entry to stamp (add it to the mac.infoPlist file)`);
  return xml.replace(re, `$1${escapeXml(value)}$2`);
}

/** Inject CFBundleURLTypes / CFBundleDocumentTypes / CFBundleIdentifier into an Info.plist's XML text. */
export function injectInfoPlist(plistXml: string, app: AppIdentity | undefined): string {
  if (!app) return plistXml;
  let out = app.id ? stampPlistString(plistXml, "CFBundleIdentifier", app.id) : plistXml;

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

export interface ResolvedIdentity {
  id?: string;
  name: string;
  displayName: string;
  /** Filesystem-safe lowercase name: usr/bin/<slug>, icon names, archive basenames. */
  slug: string;
  version: string;
  icon?: AppIcon;
  categories: string[];
  fileAssociations?: AppIdentity["fileAssociations"];
  urlSchemes?: AppIdentity["urlSchemes"];
}

export function slugify(name: string): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  if (!slug) throw new Error(`nd: app name "${name}" produces an empty slug`);
  return slug;
}

/** Name/version/id precedence: overrides.version > ND_APP_VERSION > config > package.json > "0.0.0". */
export function resolveIdentity(
  config: NativeDesktopConfig,
  appDir: string,
  overrides: { version?: string } = {},
): ResolvedIdentity {
  let pkg: { name?: string; version?: string } = {};
  try {
    pkg = JSON.parse(readFileSync(resolve(appDir, "package.json"), "utf8"));
  } catch {
    // No package.json: config must carry the name.
  }
  const app = config.app ?? {};
  const name = app.name ?? pkg.name;
  if (!name) throw new Error(`nd: no app name (set app.name in nativedesktop.config.ts or "name" in ${appDir}/package.json)`);
  const icon = typeof app.icon === "string" ? { source: app.icon } : app.icon;
  return {
    id: app.id,
    name,
    displayName: app.displayName ?? name,
    slug: slugify(name),
    version: overrides.version ?? process.env.ND_APP_VERSION ?? app.version ?? pkg.version ?? "0.0.0",
    icon,
    categories: app.categories ?? ["Utility"],
    fileAssociations: app.fileAssociations,
    urlSchemes: app.urlSchemes,
  };
}

/**
 * Info.plist XML for the bundle. `customXml` (mac.infoPlist file contents)
 * replaces the generated skeleton but still gets the identity stamped in:
 * CFBundleIdentifier / document types / URL schemes via injectInfoPlist, plus
 * CFBundleExecutable and the version keys so the plist always matches the
 * binary `nd package` actually installs.
 */
export function buildInfoPlist(
  identity: ResolvedIdentity,
  mac: MacPackageConfig | undefined,
  customXml?: string,
  iconFile?: string,
): string {
  let xml = customXml ?? infoPlistSkeleton({
    name: identity.name,
    displayName: identity.displayName,
    id: identity.id ?? `dev.nativedesktop.${identity.slug}`,
    executable: identity.name,
    version: identity.version,
    minimumSystemVersion: mac?.minimumSystemVersion ?? "26.0",
    category: mac?.category,
    iconFile,
  });
  if (customXml) {
    xml = stampPlistString(xml, "CFBundleExecutable", identity.name);
    xml = stampPlistString(xml, "CFBundleShortVersionString", identity.version);
    xml = stampPlistString(xml, "CFBundleVersion", identity.version);
  }
  xml = injectInfoPlist(xml, {
    id: identity.id,
    fileAssociations: identity.fileAssociations,
    urlSchemes: identity.urlSchemes,
  });
  const extra = Object.entries(mac?.extraPlist ?? {});
  if (extra.length) {
    const lines = extra.map(([key, value]) => {
      const v = typeof value === "boolean" ? (value ? "<true/>" : "<false/>") : typeof value === "number"
        ? `<integer>${value}</integer>`
        : `<string>${escapeXml(value)}</string>`;
      return `    <key>${escapeXml(key)}</key>${v}\n`;
    }).join("");
    xml = xml.replace("</dict>\n</plist>", `${lines}</dict>\n</plist>`);
  }
  return xml;
}

/** .desktop entry text for the AppDir, identity-stamped (MimeType/%U/StartupWMClass). */
export function buildDesktopEntry(identity: ResolvedIdentity, linux: LinuxPackageConfig | undefined): string {
  const desktop = desktopEntryTemplate({
    displayName: identity.displayName,
    slug: identity.slug,
    categories: identity.categories,
    appId: identity.id,
    extra: linux?.desktopEntry,
  });
  return injectDesktopFile(desktop, {
    id: identity.id,
    fileAssociations: identity.fileAssociations,
    urlSchemes: identity.urlSchemes,
  });
}

export function buildAppRun(spec: AppRunSpec): string {
  return appRunTemplate(spec);
}
