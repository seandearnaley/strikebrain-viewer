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

# The builds take their icons from the pack: macOS its Assets.car and
# .icns, Windows its .ico, Linux its hicolor icons. Needs macOS 26 and
# Xcode 26 or later, resvg, ImageMagick and the Geist font.
# Rebuild brand/Strikebrain-Production-Pack, the icons and identity, from brand/build.sh.
[macos]
brand:
    brand/build.sh

# The Info.plist template lives in crates/viewer/bundle and the icon in the
# brand pack: Assets.car, which macOS 26 and later show with Liquid Glass
# and macOS 11 to 15 flattened, and an .icns beside it. The version comes
# from Cargo. Signed ad hoc, for this Mac only, unless
# CODESIGN_IDENTITY names a Developer ID Application certificate in the
# keychain; either way with the hardened runtime, which notarization
# requires.
# Build the macOS app: target/release/bundle/osx/Strikebrain Viewer.app.
[macos]
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
    cp brand/Strikebrain-Production-Pack/platform/apple/{Assets.car,AppIcon.icns} "$app/Contents/Resources/"
    identity="${CODESIGN_IDENTITY:--}"
    # A secure timestamp needs Apple's server and a real certificate.
    timestamp=$([ "$identity" = "-" ] && echo --timestamp=none || echo --timestamp)
    codesign --force --options runtime "$timestamp" -s "$identity" "$app"
    codesign --verify --strict "$app"
    echo "$app ($version)"

# The image holds the app beside a link to /Applications, and is signed
# when CODESIGN_IDENTITY is set. When NOTARY_KEY also names an App Store
# Connect API key (.p8), with NOTARY_KEY_ID and NOTARY_ISSUER_ID, Apple
# notarizes the app and then the image, and each gets its ticket stapled:
# Gatekeeper then needs no network to open the app copied out of the image.
# Build the macOS disk image: target/release/bundle/Strikebrain-Viewer-<version>-macos-<arch>.dmg.
[macos]
dmg: bundle
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(cargo pkgid -p viewer | sed 's/.*[#@]//')
    app="target/release/bundle/osx/Strikebrain Viewer.app"
    dmg="target/release/bundle/Strikebrain-Viewer-$version-macos-$(uname -m).dmg"
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    # Submits $1 and staples its ticket to $2. `--wait` returns once Apple
    # has an answer, whatever it is.
    notarize() {
        xcrun notarytool submit "$1" --key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" \
            --issuer "$NOTARY_ISSUER_ID" --wait --output-format json > "$work/notary.json"
        cat "$work/notary.json"
        if ! jq -e '.status == "Accepted"' "$work/notary.json" > /dev/null; then
            xcrun notarytool log "$(jq -r .id "$work/notary.json")" --key "$NOTARY_KEY" \
                --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID"
            exit 1
        fi
        xcrun stapler staple "$2"
        xcrun stapler validate "$2"
    }
    if [ -n "${NOTARY_KEY:-}" ]; then
        [ -n "${CODESIGN_IDENTITY:-}" ] || { echo "notarizing needs CODESIGN_IDENTITY" >&2; exit 1; }
        # A zip can be notarized but not stapled; its ticket goes on the app.
        ditto -c -k --keepParent "$app" "$work/app.zip"
        notarize "$work/app.zip" "$app"
    fi
    mkdir "$work/root"
    cp -R "$app" "$work/root/"
    ln -s /Applications "$work/root/Applications"
    hdiutil create -volname "Strikebrain Viewer" -srcfolder "$work/root" -ov -format UDZO "$dmg" > /dev/null
    if [ -n "${CODESIGN_IDENTITY:-}" ]; then
        codesign --force --timestamp -s "$CODESIGN_IDENTITY" "$dmg"
    fi
    hdiutil verify -quiet "$dmg"
    if [ -n "${NOTARY_KEY:-}" ]; then
        notarize "$dmg" "$dmg"
    fi
    echo "$dmg"

# The .tar.gz is laid out as a prefix (bin/, share/), to unpack into
# ~/.local or /usr/local; the .deb's dependencies are read from the binary.
# The icons are the brand pack's hicolor set. Needs dpkg-dev.
# Build the Linux packages, a .tar.gz and a .deb, in target/release/bundle.
[linux]
linux-packages:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked -p viewer
    version=$(cargo pkgid -p viewer | sed 's/.*[#@]//')
    out=target/release/bundle
    id=ai.brainstrike.viewer
    stage() {
        install -Dm755 target/release/strikebrain-viewer "$1/bin/strikebrain-viewer"
        install -Dm644 "crates/viewer/bundle/$id.desktop" "$1/share/applications/$id.desktop"
        mkdir -p "$1/share/icons"
        cp -R brand/Strikebrain-Production-Pack/platform/linux/hicolor "$1/share/icons/"
        install -Dm644 LICENSE-MIT LICENSE-APACHE -t "$1/share/doc/strikebrain-viewer"
    }
    name="strikebrain-viewer-$version-linux-$(uname -m)"
    rm -rf "$out/$name" "$out/deb"
    stage "$out/$name"
    tar -czf "$out/$name.tar.gz" -C "$out" "$name"
    arch=$(dpkg --print-architecture)
    stage "$out/deb/usr"
    mkdir -p "$out/deb/DEBIAN"
    # dpkg-shlibdeps reads the libraries the binary links and names their
    # packages; it wants a source tree's debian/control beside it. The
    # Wayland client library and the Vulkan loader are opened at run time,
    # so they are named by hand.
    depends=$(cd "$out" && mkdir -p debian && touch debian/control \
        && dpkg-shlibdeps -O deb/usr/bin/strikebrain-viewer | sed 's/^shlibs:Depends=//')
    rm -r "$out/debian"
    cat > "$out/deb/DEBIAN/control" <<CONTROL
    Package: strikebrain-viewer
    Version: $version
    Section: utils
    Priority: optional
    Architecture: $arch
    Depends: $depends, libwayland-client0, libvulkan1
    Maintainer: Sean Dearnaley <seandearnaley@users.noreply.github.com>
    Homepage: https://github.com/seandearnaley/strikebrain-viewer
    Description: A viewer for folders of text and images
     A file tree on the left, where you filter and check files, and a stage
     on the right that shows them one at a time or as a gallery.
    CONTROL
    deb="$out/strikebrain-viewer_${version}_$arch.deb"
    dpkg-deb --root-owner-group --build "$out/deb" "$deb" > /dev/null
    rm -r "$out/deb" "$out/$name"
    dpkg-deb --info "$deb"
    dpkg-deb --contents "$deb"
    echo "$out/$name.tar.gz"
    echo "$deb"

# The .zip holds the executable and the licenses; the installer, made with
# Inno Setup 6 from crates/viewer/bundle/windows.iss, puts them in the
# user's Programs folder with a Start menu entry. Unsigned unless SIGNTOOL
# holds a whole signing command, to which the file is added; the release
# workflow sets it to run crates/viewer/bundle/windows-sign.ps1. Inno Setup
# then signs the executable (unless it is signed already), the uninstaller
# and the installer with it, and the .zip, made afterwards, holds the signed
# executable.
# Build the Windows packages, a .zip and an installer, in target\release\bundle.
[windows]
[extension(".ps1")]
windows-packages:
    #!pwsh
    $ErrorActionPreference = 'Stop'
    $PSNativeCommandUseErrorActionPreference = $true
    cargo build --release --locked -p viewer
    $version = (cargo pkgid -p viewer) -replace '.*[#@]'
    $out = Join-Path (Get-Location) 'target\release\bundle'
    $name = "strikebrain-viewer-$version-windows-x64"
    $stage = Join-Path $out $name
    Remove-Item -Recurse -Force $stage -ErrorAction Ignore
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item target\release\strikebrain-viewer.exe, LICENSE-MIT, LICENSE-APACHE $stage
    $sign = @()
    if ($env:SIGNTOOL) {
        # Inno Setup writes a quote in a sign tool's command as $q, and the
        # file to sign as $f.
        $sign = '/DSign', "/Ssign=$($env:SIGNTOOL.Replace('"', '$q')) `$f"
    }
    $iscc = (Get-Command iscc -ErrorAction Ignore)?.Source ?? "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    & $iscc /Qp "/DVersion=$version" "/DStage=$stage" "/O$out" @sign crates\viewer\bundle\windows.iss
    Compress-Archive -Path $stage -DestinationPath "$out\$name.zip" -Force
    Remove-Item -Recurse -Force $stage
    Get-ChildItem "$out\*windows-x64*" | ForEach-Object { $_.FullName }

# Install the pinned tools and the pre-commit hook. Needs cargo-binstall.
setup:
    cargo binstall --no-confirm --locked just@1.59.0 typos-cli@1.52.2 cargo-nextest@0.9.152 cargo-llvm-cov@0.9.2 cargo-deny@0.22.0 cargo-shear@1.14.1 bacon@3.26.0
    git config core.hooksPath .githooks
