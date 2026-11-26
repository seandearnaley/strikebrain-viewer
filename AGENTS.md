# Strikebrain Viewer

A native viewer for text and images, built on GPUI through GPUI Kit.
Design: `docs/superpowers/specs/2026-11-22-strikebrain-viewer-design.md`.

## Rules

- Never invent an API. GPUI and GPUI Kit are pre-1.0 and change weekly. Before using a call, read its current docs or source. Do not translate from memory, React, CSS or older GPUI examples.
- Sources, in order: the vendored source under `~/.cargo/registry/src/*/` (`gpui-kit-0.10.0`, which holds `open_window`, `init` and the `test` module; `gpui-pre-0.3.12`; `gpui-base-0.10.0`; `gpui-component-0.10.0`), then `https://gpui-kit.com/llms.txt` (append `.md` to any page).
- Where the installed skills under `.claude/skills` or gpui-kit.com disagree with these Rules or with the vendored 0.10.0 source, the Rules and the source win. The website describes the Kit's main branch, not 0.10.0.
- `crates/explorer` has no UI dependency. Keep it that way.
- UI code depends on `gpui-kit` only. Never add a GPUI crate directly.
- The toolchain and gpui-kit are pinned exactly; upgrading either is its own change. Other crates use caret requirements, with Cargo.lock committed.
- No taglines or helper text in the interface. If a screen needs them, fix the layout.
- Prefer the framework's primitive over a custom one. Add a crate only when the standard library and current dependencies do not cover the need.
- `just check` must pass before every commit. The pre-commit hook runs `just lint`; never bypass it with `--no-verify`.
- Never weaken a lint, a `deny.toml` policy, a test timeout or the coverage threshold to make a check pass. Fix the code.
- Silence a lint only with `#[expect(lint, reason = "...")]` on the narrowest item. Clippy rejects `#[allow]` and a missing reason, and an `expect` that no longer fires is an error.

## Commands

| Task | Command |
| --- | --- |
| Every gate; run before each commit | `just check` |
| Format, spelling and Clippy (the pre-commit hook) | `just lint` |
| Apply rustfmt and Clippy fixes | `just fix` |
| Engine coverage; fails under 90% of lines | `just cov` |
| Engine benchmarks against the budgets; not part of `check` (`EXPLORER_SCAN_ROOT=<large folder>` adds real scans) | `just bench [group]` |
| Run the app | `just run` |
| See the app without a window: key states as PNG images in `target/shots/` | `just shot` |
| Release build wrapped as `target/release/bundle/osx/Strikebrain Viewer.app`, signed ad hoc | `just bundle` |
| That app in a disk image, `target/release/bundle/*.dmg` | `just dmg` |
| On Linux, a `.tar.gz` and a `.deb` in `target/release/bundle` | `just linux-packages` |
| On Windows, a `.zip` and an Inno Setup installer in `target\release\bundle` (needs Inno Setup 6) | `just windows-packages` |
| Rebuild the icons and identity in `brand/Strikebrain-Production-Pack` from `brand/build.sh` (macOS 26+, Xcode 26+, resvg, ImageMagick, Geist) | `just brand` |
| Engine tests | `cargo nextest run -p explorer` |
| All tests | `cargo nextest run --workspace` |
| One UI test | `cargo nextest run -p viewer --test ui <name>` |
| Watch: Clippy on save; `n` nextest, ctrl-j other jobs | `bacon` |
| Tools and git hook in a fresh clone | `just setup` |

CI (`.github/workflows/ci.yml`) runs `just check` on macOS for every pull request and push to `main`, and builds and tests on Linux and Windows.

## Releasing

`.github/workflows/release.yml` builds the macOS disk image, the Linux packages and the Windows packages with the recipes above, signs them, checks them as a person would install them, and publishes them. To release:

1. Set `version` in the root `Cargo.toml`, run `cargo check` so `Cargo.lock` follows, and merge that.
2. Run `gh workflow run release.yml --ref main` (or Actions > Release > Run workflow). Add `-f dry_run=true` to do everything but publish, for any version, released or not. Follow it with `gh run watch`.
3. When every package passes, the workflow tags the commit `v<version>`, publishes the release with `SHA256SUMS` and a build attestation, then downloads and verifies the published files. A failure anywhere publishes nothing; a version already tagged or released stops the run before it builds.

A pull request that touches the workflow, the `justfile`, `crates/viewer/bundle` or `crates/viewer/build.rs` runs it as a rehearsal: unsigned, unpublished, its files kept as artifacts.

Signing runs in the repository's `release` environment, which only `main` can use; its secrets and variables are named in the workflow's header. macOS imports the Developer ID Application certificate into a keychain made for the job, signs with the hardened runtime, and `just dmg` notarizes the app with an App Store Connect API key (`strikebrain-viewer-notarize`) and staples its ticket to it, then builds, notarizes and staples the disk image. The app needs its own stapled ticket: the image's ticket stays with the image, so without it Gatekeeper fetches the app's ticket online on its first launch, and refuses an offline one. The workflow checks both tickets and Gatekeeper. Locally, `just bundle` and `just dmg` sign ad hoc, or with `CODESIGN_IDENTITY` when it is set, and notarize when `NOTARY_KEY`, `NOTARY_KEY_ID` and `NOTARY_ISSUER_ID` are set.

Windows signs with Azure Artifact Signing (account `seandsigning`, profile `public-trust`). The Entra app `strikebrain-viewer-release` trusts only this repository's `release` environment through GitHub's OIDC token, so no Azure password is stored. The workflow signs the executable with Microsoft's action, then sets `SIGNTOOL` to run `crates/viewer/bundle/windows-sign.ps1`, which `just windows-packages` hands to Inno Setup to sign the uninstaller and the installer. The check requires every file to carry a valid, timestamped signature from one publisher. `crates/viewer/build.rs` compiles `bundle/windows.rc` into the executable: icon 1, which GPUI loads for the window, and the version details.

The icons come from `brand/Strikebrain-Production-Pack` (its README describes each file): macOS's `Assets.car` and `AppIcon.icns`, compiled by `actool` from the Icon Composer icon `AppIcon.icon`, which the bundle names with `CFBundleIconName`; Windows's `Strikebrain.ico`; Linux's `hicolor` tree. `actool` needs macOS 26 and Xcode 26, which the runners lack, so `just brand` runs on a Mac and its output is committed; the builds only copy it.

To renew: a new Developer ID certificate means a new `MACOS_APPLICATION_P12`, `MACOS_P12_PASSWORD` and `MACOS_APPLICATION_IDENTITY` (`gh secret set <name> --env release`); a revoked notary key means new `NOTARY_API_*` secrets. Azure rotates its short-lived certificates itself; keep the identity validation current.

Iterate on the narrowest check. Engine work needs only the engine tests.

`just check` stops at the first failure. In order: `cargo fmt --check`, `typos`, Clippy on all targets, Clippy on the production feature set (`-p viewer --lib --bins`, without test-support), nextest with the `ci` profile, doc-tests (nextest skips them), `cargo doc` (broken doc links), engine coverage, `cargo shear` (unused dependencies and source files no module includes), `cargo deny check` (advisories, licenses, bans, sources; fetches the RustSec database). It sets `CARGO_BUILD_WARNINGS=deny`, so any warning fails it; plain `cargo` commands only warn, so you can iterate.

Lints live in the root `Cargo.toml` (`[workspace.lints]`, one comment per non-obvious choice) and `clippy.toml`. Tests may `unwrap` and `panic`; elsewhere return errors or use `expect("why it cannot fail")`. `std::thread::sleep` is banned: in tests, drive time with the GPUI executor's timer and clock.

nextest reports a test as slow after 10 s and kills it after 30 s (`.config/nextest.toml`). A test that needs longer is a defect.

## Saved state and live refresh

- The app saves its state to `~/Library/Application Support/ai.brainstrike.viewer/state.json` (the platform config folder, through `directories`; `crates/viewer/src/persist.rs`). `STRIKEBRAIN_VIEWER_STATE=<file>` points a run elsewhere, for example `STRIKEBRAIN_VIEWER_STATE=/tmp/s.json just run`, so trying a build never touches your own state.
- Tests never touch it: `AppView::new` saves nothing and follows nothing. Tests that save pass `Session { store: Some(StateStore::at(<temp>/state.json)), .. }` to `AppView::with_session`, or a store to `open_main_window`. The bundle identifier lives in `persist::APP_ID` and in `crates/viewer/bundle/Info.plist`; a unit test keeps them equal.
- Saves are timed on GPUI's clock: in a test, `cx.executor().advance_clock(SAVE_DELAY)` then `run_until_parked` writes a pending save; `cx.quit()` runs the quit save.
- The file holds a list of folders (`"version": 2`, `persist/format.rs`) and `explorer_hidden` beside `explorer_width`; a version 1 file, from before several folders, still loads as one folder. Files opened on their own are never saved. Read it back in a test with `StateStore::at(path).load()` rather than matching its text.
- Every folder is a top-level row, even alone, and so is a file opened on its own that no folder lists. UI tests open real folders and files with `Launch::Folders(vec![...])`; `tests/ui/main.rs` has `node` (a path inside the first folder), `node_at` (a path on disk), `under(dir, rows)` (the rows of one folder, its own row first), `checked` (checks by path inside their folder), `open_app_with_session` (a window over a `Session`: a saved view, a state store, the explorer hidden) and `dispatch` (an action, as a key, a menu item or a button sends it). `Launch::Tree` shows a `Tree::from_paths` fixture as it is, with no folder rows.
- Tests run the bar's and the More menu's commands by dispatching their actions from `viewer::keys` (public for this), not by clicking inside the More menu; only the menu's own tests in `tests/ui/bar.rs` click its items. Tests click the bar's buttons but never press and drag on the bar: the test platform does not implement moving a window.
- Live refresh (`state/watch.rs`) is off unless `Session::watch` is set. Tests with a real watcher (`tests/ui/watch.rs`) call `cx.executor().allow_parking()`, since its thread wakes the app, and wait in real time with `until`. In such a test every executor timer costs up to 100 ms of real time (the test scheduler parks in 100 ms slices), so wait in few, long steps.

## Tools

Pinned like the toolchain; upgrading one is its own change. `just setup` installs these with `cargo-binstall` (1.28.0; `brew install cargo-binstall`, then `cargo binstall just@1.59.0`) and points `core.hooksPath` at `.githooks`.

| Tool | Version | Config |
| --- | --- | --- |
| Rust with rustfmt, Clippy, llvm-tools | 1.100.0 | `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `[workspace.lints]` |
| just | 1.59.0 | `justfile` |
| typos-cli | 1.52.2 | `_typos.toml` |
| cargo-nextest | 0.9.152 | `.config/nextest.toml` |
| cargo-llvm-cov | 0.9.2 | `justfile` (`cov`) |
| cargo-shear | 1.14.1 | none |
| cargo-deny | 0.22.0 | `deny.toml` |
| bacon | 3.26.0 | `bacon.toml` |

## Build times

Measured on 2026-11-23, Apple M4 Max (16 cores), Rust 1.100.0.

| Build | Time |
| --- | --- |
| Cold `cargo build -p viewer` | 64 s |
| Warm rebuild after touching `crates/viewer/src/app.rs` | 0.7 s |
| Engine test cycle after touching `crates/explorer/src/lib.rs` | 0.5 s |
| `just check`, warm, nothing changed (after the final fixes; the live-refresh tests wait on real events) | 14 s |
| `just lint` (the pre-commit hook), warm | 0.5 s |
| `just bundle` after a debug build (thin LTO, one codegen unit) | 73 s |
| `just bundle` again after a small change | 25 s |

On this machine, a step that takes several times longer than these is a defect. Stop and report it.

## Hot reload

Not available. The trial on 2026-11-22 stopped before a live run:

- `gpui-pre` 0.3.12 has a `hot-reload` feature, but it only applies patches when the app is launched by the Dioxus CLI with `dx serve --hot-patch`. Without `dx` it logs a warning and does nothing. It is compiled in for debug builds only.
- `gpui-kit` 0.10.0 does not re-export that feature. Enabling it needs the Dioxus CLI and a direct `gpui-pre` dependency. The Rules forbid the direct dependency.

Neither was added. Rebuild and restart instead.

## Known gaps

- A command's key takes ⌘ on macOS and Ctrl elsewhere: `secondary!` in `crates/viewer/src/keys.rs` writes it for the platform the app is built for, and a test presses it as `secondary-f`. Back, Forward and the sidebar are `cmd-[`, `cmd-]` and `ctrl-cmd-s` on macOS, `alt-left`, `alt-right` and `ctrl-b` elsewhere. GPUI draws no menu bar on Windows and Linux, so the commands there are their keys and the bar's buttons. Tests that make symlinks or FIFOs are `#[cfg(unix)]`.
- The window's bar is the Kit's `TitleBar` on every platform. On Linux, where the desktop draws its own title bar, the window has two bars, the system's and the app's. The Windows bar, with the Kit's caption buttons, is checked only on GitHub's runner; nobody has looked at it on a Windows desktop.
- The Go menu's Back and Forward stay enabled where there is nowhere to go, and do nothing there: GPUI greys a menu item only when nothing handles its action. The bar's Back and Forward are disabled there.
- Paths compared with a tree's go through `explorer::canonical`, which on Windows drops the `\\?\` prefix of the system's canonical form where the path means the same without it (`dunce`); Clippy rejects `std::fs::canonicalize`. A path that needs the prefix (a reserved name such as `con`, more than 260 characters) keeps it, and shows with it.
- A local bundle is signed ad hoc (`codesign -s -`), for this Mac and for tools that find the app by bundle identifier. Only the release workflow signs with the Developer ID certificate and notarizes.
- Releases cover macOS on Apple silicon, and Linux and Windows on x86-64. There is no Intel Mac build (none can be run here to test), no Linux Arm build and no Windows Arm build.
- The Windows packages are checked only on GitHub's runner: the installer runs silently there, the installed app opens its window, and the uninstaller removes it. Nobody has watched the installer's pages or the app's icon on a real Windows desktop from them.
- Live refresh rescans the whole folder a change is in, on any change a scan would show, as the design says; the other folders are left alone. With Show ignored on, a busy folder the tree then shows (a build's `target/`) is rescanned back to back while it changes.
- Folders are told apart by their canonical paths, and `realpath` does not join macOS firmlinks: `/System/Volumes/Data/Users/x` and `/Users/x` stay different, so the same folder added under both is listed twice. The picker and Finder give `/Users/...`.
- A saved folder that no longer exists at launch is dropped from the saved state, as the design asks, even when it is only on a volume not mounted that day.
- A change to `.git/info/exclude` or the global git excludes does not trigger a rescan; a `.gitignore` change does.
- A file opened on its own is watched alone. On Linux, inotify drops that watch after an editor's atomic save (a new file renamed over it), so later changes are not followed: Refresh reads the file again but does not watch it again, since it restarts only a watcher that failed to start. Removing the row and opening the file again follows it again.
- "Open With" lists the app only for the types `crates/viewer/bundle/Info.plist` names: macOS ignores a catch-all claim there. A source format macOS has no text type for needs its extension in the Source Code entry. To see what macOS offers after a change, `just bundle`, then ask `NSWorkspace.shared.urlsForApplications(toOpen:)` for a sample file. Windows and Linux hand the file over as an argument, which works; the Linux desktop entry registers the app, and the Windows installer registers nothing, so there it is Open With > Choose another app.
- `just shot` draws offscreen. To see the real window, `just bundle`, launch the app, find its window id with `CGWindowListCopyWindowInfo` and run `screencapture -x -o -l <id> out.png`. While the screen is locked `screencapture` captures nothing.
- Run the bundled binary with `STRIKEBRAIN_VIEWER_STATE=<temp file>` when trying it; without it the run writes your own saved state.
- GPUI Base 0.10.0 draws the code view's caret line number in the foreground colour whatever the theme says; line 1 stands out when a file opens.
- Space then Esc within the fullscreen animation can leave the window fullscreen: GPUI exposes no transition state.
- The live-refresh tests wait on real file-system events, about 2 to 5 s each; they are the slowest in the suite.
- The gallery decodes each tile's picture whole, then scales it down: about 200 ms for a 24-megapixel JPEG. At most four reads run at once, each keeping its slot until it ends; four grainy 24-megapixel JPEGs at once peak near half a gigabyte. The `image` crate has no reduced-size decode. See the spec's Risks.
