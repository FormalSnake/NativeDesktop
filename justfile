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
