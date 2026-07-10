# Styling — pointer

GTK styling is **not web CSS**. The authoritative, always-current key list is generated at
[`docs/styling.md`](../styling.md) — read that, not this file, for the actual `style` prop schema.

Do not hallucinate `flex`, `grid`, `position`, `display`, or `justifyContent` — none of these exist
on the `style` prop. Layout comes from container widgets (`<box>`/`<grid>`), never from `style`.
Unknown or web-only keys are rejected at the React renderer with a Levenshtein fix-it message
(`validateStyle`) and defensively rejected host-side too, so a bad key fails loudly at commit time
rather than silently doing nothing.

This file is intentionally short and will not be kept in sync with schema changes by hand — treat
`docs/styling.md` as the source of truth and this file only as the "start here" pointer to it.
