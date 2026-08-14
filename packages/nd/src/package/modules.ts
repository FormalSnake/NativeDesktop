// Runtime node_modules flattening: BFS over the real module graph, copying
// every package the app can reach at runtime into one flat node_modules that
// an ordinary upward walk resolves. Subsumes Bun's isolated store
// (node_modules/.bun/<pkg>/node_modules/<pkg> siblings), workspace symlinks,
// and file: deps; replaces `bun install` inside the bundle.
import { cpSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import { basename, dirname, join, sep } from "node:path";

interface QueueEntry {
  spec: string;
  /** Directory the spec resolves from (the requiring package's real root). */
  anchor: string;
  /** Second anchor to try, used for peers whose consumer does not supply one. */
  fallbackAnchor?: string;
  /** Dest-relative dir of the requiring package, for version-conflict nesting. */
  requirer?: string;
  optional: boolean;
  /** A peerDependency: the CONSUMER's copy is the correct one, so an existing
   * flat claim wins outright instead of nesting a second copy. */
  peer?: boolean;
}

const skipNodeModules = (src: string) => basename(src) !== "node_modules";

function packageRoot(spec: string, anchor: string): string {
  // dereference to the real store/workspace directory so two symlinked routes
  // to the same package compare equal.
  return realpathSync(dirname(Bun.resolveSync(`${spec}/package.json`, anchor)));
}

export interface FlattenOptions {
  /** The app directory whose package.json dependencies seed the walk. */
  appDir: string;
  /** Extra seed specifiers (PackageConfig.runtimeDependencies), resolved from appDir. */
  extraRoots?: string[];
  /** The flat node_modules directory to populate. */
  dest: string;
}

/** Copies the app's runtime dependency closure into `dest`. Returns the flat package names. */
export function flattenRuntimeModules({ appDir, extraRoots = [], dest }: FlattenOptions): string[] {
  const appPkg = JSON.parse(readFileSync(join(appDir, "package.json"), "utf8")) as {
    dependencies?: Record<string, string>;
  };
  const anchor = realpathSync(appDir);
  const queue: QueueEntry[] = [
    ...Object.keys(appPkg.dependencies ?? {}).map((spec) => ({ spec, anchor, optional: false })),
    ...extraRoots.map((spec) => ({ spec, anchor, optional: false })),
  ];
  // First claim of a name wins the flat slot; a later claim with a different
  // realpath nests under the requiring package's own node_modules.
  const claimed = new Map<string, string>();
  // Conflict copies already made, keyed by dest-relative slot. Consulted up
  // the requirer chain (Node resolution order) so a dependency cycle whose
  // version is already reachable stops instead of nesting forever.
  const nested = new Map<string, string>();
  const resolvedInDest = (spec: string, requirer: string): string | undefined => {
    for (let dir = requirer; ; ) {
      const hit = nested.get(join(dir, "node_modules", spec));
      if (hit) return hit;
      const cut = dir.lastIndexOf(`${sep}node_modules${sep}`);
      if (cut === -1) break;
      dir = dir.slice(0, cut);
    }
    return claimed.get(spec);
  };
  mkdirSync(dest, { recursive: true });

  while (queue.length) {
    const entry = queue.shift()!;
    // A peer the app already provides is settled: taking the dependency's own
    // copy instead is what puts two reacts in one bundle.
    if (entry.peer && claimed.has(entry.spec)) continue;
    let root: string;
    try {
      root = packageRoot(entry.spec, entry.anchor);
    } catch (err) {
      if (entry.fallbackAnchor) {
        try {
          root = packageRoot(entry.spec, entry.fallbackAnchor);
        } catch {
          if (entry.optional) continue;
          throw new Error(`nd: cannot resolve runtime dependency "${entry.spec}" from ${entry.anchor}: ${err}`);
        }
      } else {
        if (entry.optional) continue;
        throw new Error(`nd: cannot resolve runtime dependency "${entry.spec}" from ${entry.anchor}: ${err}`);
      }
    }

    const existing = claimed.get(entry.spec);
    let destDir: string;
    if (existing === undefined) {
      destDir = entry.spec;
      claimed.set(entry.spec, root);
    } else if (existing === root) {
      continue;
    } else {
      if (!entry.requirer) throw new Error(`nd: conflicting versions of "${entry.spec}" among the app's direct dependencies`);
      if (resolvedInDest(entry.spec, entry.requirer) === root) continue;
      destDir = join(entry.requirer, "node_modules", entry.spec);
      nested.set(destDir, root);
    }
    cpSync(root, join(dest, destDir), { recursive: true, dereference: true, filter: skipNodeModules });

    const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as {
      dependencies?: Record<string, string>;
      optionalDependencies?: Record<string, string>;
      peerDependencies?: Record<string, string>;
    };
    for (const spec of Object.keys(pkg.dependencies ?? {})) {
      queue.push({ spec, anchor: root, requirer: destDir, optional: false });
    }
    for (const spec of Object.keys(pkg.optionalDependencies ?? {})) {
      queue.push({ spec, anchor: root, requirer: destDir, optional: true });
    }
    // Peers resolve from the APP, not from the package that declares them. A
    // `file:` linked dependency living outside the app tree resolves its own
    // peer to its own copy, and a second react in the bundle is the classic
    // "Invalid hook call" at launch. The package's root is only the fallback,
    // for a peer the app does not provide at all.
    for (const spec of Object.keys(pkg.peerDependencies ?? {})) {
      queue.push({ spec, anchor, fallbackAnchor: root, requirer: destDir, optional: true, peer: true });
    }
  }

  return [...claimed.keys()];
}

/**
 * Whether `exports` offers the package's ROOT specifier. A string is one; an
 * object is one only through a `"."` key or the conditions shorthand (keys that
 * are not subpaths). A subpath-only map (`{"./include/nd.h": …}`, which is what
 * `@nativedesktop/native` publishes) declares no root entry, so failing to
 * resolve the bare name is correct rather than a missing build.
 */
function declaresRootEntry(exports: unknown): boolean {
  if (typeof exports === "string") return true;
  if (exports === null || typeof exports !== "object") return false;
  return Object.keys(exports).some((key) => key === "." || !key.startsWith("."));
}

/**
 * Asserts each flat package's main/exports entry resolves inside the bundle.
 * Catches a copied-but-unbuilt package (e.g. @nativedesktop/react without its
 * dist/) before it ships as a broken bundle.
 */
export function assertResolvableEntries(appRoot: string, names: string[]): void {
  for (const name of names) {
    try {
      Bun.resolveSync(name, appRoot);
    } catch {
      // A package that declares no entry point at all (binary/asset carriers
      // like the platform host packages) is legitimately unresolvable.
      const pkg = JSON.parse(readFileSync(join(appRoot, "node_modules", name, "package.json"), "utf8")) as {
        main?: string;
        module?: string;
        exports?: unknown;
      };
      if (pkg.main === undefined && pkg.module === undefined && !declaresRootEntry(pkg.exports)) continue;
      throw new Error(
        `nd: bundled package "${name}" has no resolvable entry (its build output is missing). ` +
        `Run \`bun run build\` in ${name} and package again.`,
      );
    }
  }
}
