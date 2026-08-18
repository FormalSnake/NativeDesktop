# NativeDesktop docs site

The documentation site at the root of this repo's `docs-site/`, built with Astro and
[Starlight](https://starlight.astro.build).

```bash
bun install
bun dev        # http://localhost:4321
```

| Command | What it does |
|---|---|
| `bun dev` | Local dev server with hot reload |
| `bun build` | Production build into `./dist/` |
| `bun preview` | Serve the production build locally |
| `bun astro check` | Type-check content and config |

## Writing pages

Pages live in `src/content/docs/`. Each `.md` or `.mdx` file becomes a route from its path, so
`src/content/docs/components/webview.md` serves at `/components/webview/`. Every page needs
frontmatter with a `title` and a `description`.

Adding a page also means adding it to the `sidebar` array in `astro.config.mjs`. Starlight will not
pick it up on its own.

Images go in `src/assets/` and are embedded with a relative link. Favicons and other static files go
in `public/`.

Mermaid diagrams work in fenced ` ```mermaid ` blocks, through `astro-mermaid`, which follows the
site's light and dark toggle.

## One generated page, one port

`components/widget-reference.md` is written by `tools/codegen.ts` from `schema/widgets.json`, the
same run that writes `docs/widgets.md`. Never hand-edit it: change the schema (or the page's
per-widget notes in codegen's `SITE_DOC_NOTES`) and run `bun tools/codegen.ts`.

The styling tables are still a port of `docs/styling.md`, also generated from the schema. The schema
wins any disagreement: change it there, run codegen, then update the port to match.
