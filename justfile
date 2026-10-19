# Targets follow private/docs/architecture/ARCHITECTURE.md §7. devnet-up, e2e, adv,
# priv, demo and status arrive with the tickets that build them.

# Once per clone: turn on the commit hooks.
setup:
    git config core.hooksPath .githooks

# Run before every commit. CI runs exactly this.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo deny check
    scripts/secret-scan.sh
