# Milestone gate (ARCHITECTURE.md 16): `just gate m0`. Tier 3 gate tests launch the example
# binaries, so those are built first.
gate m:
    cargo build --workspace --bins
    cargo nextest run --profile gate -E 'test(/^gate_{{m}}::/)'
