# Strikebrain Viewer tasks: https://just.systems/man/en/
# `just check` must pass before every commit. Bare `just` lists the recipes.

# List the recipes.
[default]
help:
    @just --list --unsorted

# Every gate, cheapest first; stops at the first failure.
[env("CARGO_BUILD_WARNINGS", "deny")]
check: lint && cov deps
    cargo clippy -p viewer --lib --bins
    cargo nextest run --workspace --profile ci
    cargo test --workspace --doc
    cargo doc --workspace --no-deps

# Format, spelling and lints. The pre-commit hook runs this.
[env("CARGO_BUILD_WARNINGS", "deny")]
lint:
    cargo fmt --all --check
    typos
    cargo clippy --workspace --all-targets

# Explorer line coverage with uncovered lines listed; fails under 90%.
[env("CARGO_BUILD_WARNINGS", "deny")]
cov:
    cargo llvm-cov nextest -p explorer --fail-under-lines 90 --show-missing-lines

# Unused dependencies or source files, then advisories, licenses, bans, sources.
deps:
    cargo shear --deny-warnings
    cargo deny check

# Apply rustfmt and Clippy's machine-applicable fixes.
fix:
    cargo clippy --fix --allow-dirty --allow-staged --workspace --all-targets
    cargo fmt --all

# Run the app.
run:
    cargo run -p viewer

# Install the pinned tools and the pre-commit hook. Needs cargo-binstall.
setup:
    cargo binstall --no-confirm --locked just@1.59.0 typos-cli@1.52.2 cargo-nextest@0.9.152 cargo-llvm-cov@0.9.2 cargo-deny@0.22.0 cargo-shear@1.14.1 bacon@3.26.0
    git config core.hooksPath .githooks
