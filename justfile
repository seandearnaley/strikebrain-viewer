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

# Engine benchmarks against the design's budgets; not part of `check`. Pass a
# filter such as `filter` to run one group. Set EXPLORER_SCAN_ROOT to a large
# folder (a brainstrike checkout) to bench real scans.
bench *args:
    cargo bench -p explorer --bench engine -- {{args}}

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

# Render key states of the app offscreen (GPUI's Metal renderer, no window)
# to target/shots/*.png.
shot:
    cargo test -p viewer --test shots

# Draw the app icon into crates/viewer/bundle/icon.png, the source the
# bundle's icon is made from. Run after changing tests/icon.rs.
icon:
    cargo test -p viewer --test icon

# Build the release binary and wrap it in a macOS app, signed ad hoc:
# target/release/bundle/osx/Strikebrain Viewer.app. The Info.plist template
# and the icon live in crates/viewer/bundle; the version comes from Cargo.
bundle:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked -p viewer
    version=$(cargo pkgid -p viewer | sed 's/.*[#@]//')
    app="target/release/bundle/osx/Strikebrain Viewer.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp target/release/strikebrain-viewer "$app/Contents/MacOS/"
    sed "s/@VERSION@/$version/g" crates/viewer/bundle/Info.plist > "$app/Contents/Info.plist"
    plutil -lint -s "$app/Contents/Info.plist"
    iconset="$(mktemp -d)/AppIcon.iconset"
    mkdir "$iconset"
    for side in 16 32 128 256 512; do
        sips -z "$side" "$side" crates/viewer/bundle/icon.png --out "$iconset/icon_${side}x${side}.png" > /dev/null
        sips -z "$((side * 2))" "$((side * 2))" crates/viewer/bundle/icon.png --out "$iconset/icon_${side}x${side}@2x.png" > /dev/null
    done
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
    rm -r "$iconset"
    codesign --force --deep -s - "$app"
    codesign --verify --strict "$app"
    echo "$app ($version)"

# Install the pinned tools and the pre-commit hook. Needs cargo-binstall.
setup:
    cargo binstall --no-confirm --locked just@1.59.0 typos-cli@1.52.2 cargo-nextest@0.9.152 cargo-llvm-cov@0.9.2 cargo-deny@0.22.0 cargo-shear@1.14.1 bacon@3.26.0
    git config core.hooksPath .githooks
