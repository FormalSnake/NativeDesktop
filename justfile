# Milestone gate (ARCHITECTURE.md 16): `just gate m0`, `just gate m1-automation`. Tier 3 gate
# tests launch the example binaries, so those are built first. A lane gate's hyphen becomes the
# underscore its test module name needs.
gate m:
    cargo build --workspace --bins
    cargo nextest run --profile gate -E 'test(/^gate_{{replace(m, "-", "_")}}::/)'

# Tier 1 of the web examples in headless Chrome, then hello's web goldens (ARCHITECTURE.md 12).
web-test:
    scripts/web-test hello overrides gallery scroll-bench
    ND_WEB_GOLDEN=1 cargo test -p hello --test web_golden

# Each lane worktree's build dir on the cache drive reaches 30 to 50 GB. Run after merging a lane:
# cleans every sibling worktree whose branch main already contains and that has no local changes.
clean-merged:
    #!/usr/bin/env bash
    set -euo pipefail
    git worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r wt; do
        [ "$wt" = "$PWD" ] && continue
        [ -d "$wt/.git" ] || [ -f "$wt/.git" ] || continue
        branch=$(git -C "$wt" branch --show-current 2>/dev/null) || continue
        [ -n "$branch" ] || continue
        git merge-base --is-ancestor "$branch" main || continue
        [ "$(git rev-parse "$branch")" != "$(git rev-parse main)" ] || continue
        [ -z "$(git -C "$wt" status --porcelain)" ] || continue
        echo "clean $wt ($branch)"
        (cd "$wt" && cargo clean -q)
    done
    df -h "/Volumes/Kyan Drive" | tail -1

# The docs (ARCHITECTURE.md 13): pages, links, prose and the capture report; `docs-capture` takes
# this OS's missing or stale captures, `docs-serve` builds the web site and serves it on :8080.
docs-check:
    cargo run -p nd-cli -- docs check

docs-capture:
    cargo run -p nd-cli -- docs capture --stale

docs-serve:
    cargo run -p nd-cli -- docs serve
