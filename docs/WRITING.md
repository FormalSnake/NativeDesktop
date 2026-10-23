# Writing the docs

The docs are pages in `docs/content`. `nd docs build` turns them into the docs site (static
HTML, with the web examples running live from `docs/run`) plus raw markdown and `llms.txt`; the
docs app (`docs/app`) shows the same pages natively on the desktop. `nd docs check` fails on
anything below that is broken.

## Pages

- One file per page: `docs/content/<section>/<name>.md`, id `<section>/<name>`. Every page is
  listed in `docs/content/nav.toml`, which sets the sidebar order.
- Front matter between `+++` lines: `title` and a one-sentence `description`. Without a title the
  first `# Heading` is used.
- Link pages by relative path (`[theming](../concepts/theming.md#tokens)`) and repository files the
  same way (`[lib.rs](../../../examples/hello/src/lib.rs)`). Both work on GitHub; the check
  resolves every one and every `#anchor`.

## Code comes from the examples

Code on a page is cut from a full example app in `examples/`, which CI compiles for every
target. Never paste code that does not compile; a short signature or a shell command is fine
inline.

Mark the cut in the example with anchor comments, then name it on the page:

```rust
// docs:begin counter
pub struct Counter {
    count: u32,
}
// docs:end counter
```

```markdown
<!-- docs:snippet examples/hello/src/lib.rs#counter -->
```

Without `#anchor` the whole file is shown. Anchor lines never show up in a snippet and do not
change the example's capture hash.

Other directives, each on a line of its own:

- `<!-- docs:example hello -->`: the four-tab embed (Web live, macOS, Linux, Windows captures).
- `<!-- docs:gallery -->`: every example with its embed.
- `<!-- docs:components -->`: the generated component reference.
- `<!-- docs:include crates/nd-cli/templates/docs/agents/donts.md -->`: inline another markdown
  file. The agent docs live once, in the `nd new` templates, and the docs pages include them.

## Examples and captures

Every directory under `examples/` is in `docs/examples.toml` (or under `[skip]` with a reason).
`nd docs capture` runs each one through the test harness on the current OS, light and dark, and
records its source hash in `docs/app/assets/captures/manifest.toml`. When an example changes,
its captures go stale and the embed says "not captured yet" until someone captures again on
that OS. A stale screenshot is never shown.

## Prose

Short, factual, specific. Write like an engineer leaving notes for the next one.

- No em dashes, and no spaced en dashes. Use a comma, a colon, parentheses or two sentences.
- No selling words: `comprehensive`, `robust`, `seamless`, `powerful`, `leverage`, `delve`,
  `elevate`, `streamline`, `significantly`, `effortless`. Say what the thing does.
- No empty setup (`it's worth noting`, `in summary`, `let's dive in`), no rule-of-three lists
  for rhythm, no "it isn't X, it's Y", no generic upbeat closers.
- Sentence-case headings. Plain "is" and "has" over "serves as" and "boasts".
- Name the check by what it does ("check it boots"), never `smoke test`.

`nd docs check` runs these rules over every page and every template `nd new` writes.
