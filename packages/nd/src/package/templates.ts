// In-code packaging templates: default Info.plist, entitlements, AppRun, and
// .desktop entry. Strings instead of template files so packages/nd's
// `files: ["src"]` ships everything `nd package` needs.

export interface InfoPlistFields {
  name: string;
  displayName: string;
  id: string;
  executable: string;
  version: string;
  minimumSystemVersion: string;
  category?: string;
  iconFile?: string;
}

export function infoPlistSkeleton(f: InfoPlistFields): string {
  let body =
    `    <key>CFBundleName</key><string>${f.name}</string>\n` +
    `    <key>CFBundleDisplayName</key><string>${f.displayName}</string>\n` +
    `    <key>CFBundleIdentifier</key><string>${f.id}</string>\n` +
    `    <key>CFBundleExecutable</key><string>${f.executable}</string>\n` +
    "    <key>CFBundlePackageType</key><string>APPL</string>\n" +
    `    <key>CFBundleShortVersionString</key><string>${f.version}</string>\n` +
    `    <key>CFBundleVersion</key><string>${f.version}</string>\n` +
    `    <key>LSMinimumSystemVersion</key><string>${f.minimumSystemVersion}</string>\n` +
    "    <key>NSHighResolutionCapable</key><true/>\n";
  if (f.category) body += `    <key>LSApplicationCategoryType</key><string>${f.category}</string>\n`;
  if (f.iconFile) {
    body += `    <key>CFBundleIconFile</key><string>${f.iconFile}</string>\n`;
    body += `    <key>CFBundleIconName</key><string>${f.iconFile}</string>\n`;
  }
  return (
    '<?xml version="1.0" encoding="UTF-8"?>\n' +
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n' +
    '<plist version="1.0">\n<dict>\n' +
    body +
    "</dict>\n</plist>\n"
  );
}

/** Hardened-runtime entitlements. JSC-under-Bun needs allow-jit on Apple Silicon. */
export const ENTITLEMENTS_PLIST =
  '<?xml version="1.0" encoding="UTF-8"?>\n' +
  '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n' +
  '<plist version="1.0">\n<dict>\n' +
  "    <key>com.apple.security.cs.allow-jit</key>\n    <true/>\n" +
  "    <key>com.apple.security.cs.allow-unsigned-executable-memory</key>\n    <true/>\n" +
  "</dict>\n</plist>\n";

export interface AppRunSpec {
  /** App-root-relative script the host boots. */
  entry: string;
  /** App-root-relative working directory. */
  cwd: string;
  /** usr/bin/<slug> executable name. */
  slug: string;
  appId?: string;
  /** App-root-relative native plugin paths. */
  pluginPaths: string[];
}

export function appRunTemplate(s: AppRunSpec): string {
  let script =
    "#!/usr/bin/env bash\n" +
    'HERE="$(dirname "$(readlink -f "$0")")"\n' +
    `export ND_SCRIPT="$HERE/app/${s.entry}"\n` +
    'export PATH="$HERE/usr/bin:$PATH"\n';
  if (s.appId) script += `export ND_APP_ID="${s.appId}"\n`;
  if (s.pluginPaths.length) {
    script += 'export ND_PLUGINS="1"\n';
    script += `export ND_PLUGIN_PATHS="${s.pluginPaths.map((p) => `$HERE/app/${p}`).join(":")}"\n`;
  }
  script += `cd "$HERE/app/${s.cwd}"\n`;
  script += `exec "$HERE/usr/bin/${s.slug}" "$@"\n`;
  return script;
}

export interface DesktopEntrySpec {
  displayName: string;
  slug: string;
  categories: string[];
  appId?: string;
  /** Extra/overriding lines from LinuxPackageConfig.desktopEntry. */
  extra?: Record<string, string>;
}

export function desktopEntryTemplate(s: DesktopEntrySpec): string {
  const lines = new Map<string, string>([
    ["Type", "Application"],
    ["Name", s.displayName],
    ["Exec", "AppRun"],
    ["Icon", s.slug],
    ["Categories", `${s.categories.join(";")};`],
  ]);
  if (s.appId) lines.set("StartupWMClass", s.appId);
  for (const [k, v] of Object.entries(s.extra ?? {})) lines.set(k, v);
  return `[Desktop Entry]\n${[...lines].map(([k, v]) => `${k}=${v}`).join("\n")}\n`;
}

/** 1x1 RGBA PNG, the no-icon-configured placeholder (appimagetool requires a root icon). */
export const PLACEHOLDER_PNG: Uint8Array = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==",
  "base64",
);
