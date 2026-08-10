# Releasing

Versions are lockstep: every publishable package carries the same version, and
a `v<version>` tag publishes them all via `.github/workflows/release.yml`.

## One-time owner bootstrap (before the first release)

1. `npm login` as FormalSnake (or sign in at npmjs.com).
2. Create the `nativedesktop` org: npmjs.com/org/create. The Free tier covers
   unlimited public packages.
3. Create a granular access token at npmjs.com/settings/~/tokens: type
   **Automation** (bypasses 2FA-on-publish, which otherwise fails
   non-interactively in CI), read and write, scoped to the `@nativedesktop`
   packages and `babel-plugin-nativedesktop`, 1-year expiry.
4. `gh secret set NPM_TOKEN --repo FormalSnake/NativeDesktop` and paste the
   token.
5. After the first publish: `npm access ls-packages @nativedesktop` to confirm
   every package is public.

## Cutting a release

1. Bump every publishable package.json to the new version (the list lives in
   `scripts/release/check-versions.ts`), run `bun install` so the lockfile
   absorbs it, and commit.
2. Dry-run the pipeline: `gh workflow run release.yml` (the `dry_run` input
   defaults to true, which builds both hosts and runs `bun publish --dry-run`).
3. `git tag v<version> && git push origin v<version>`.

The tag run builds `nd-shell` on macos-15 and `nd-hello` in an ubuntu:24.04
container (never the Nix devshell: that bakes /nix/store RPATHs into the
binary), stages both into the `packages/host-*` platform packages, publishes
everything in dependency order, and attaches the raw binaries plus the
`nd-hello.ldd.txt` soname record to a GitHub release for Nix and non-npm
consumers.

Two ordering constraints the workflow already encodes, for anyone publishing
by hand:

- `bun install` must run in the checkout before `bun publish`: the
  `workspace:*` rewrite reads resolved versions from the lockfile and crashes
  without it.
- Platform binary packages publish before `@nativedesktop/host`, and
  `@nativedesktop/cli` publishes last (see `scripts/release/publish-all.sh`).
