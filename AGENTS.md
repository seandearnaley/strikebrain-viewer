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
| Engine tests | `cargo nextest run -p explorer` |
| All tests | `cargo nextest run --workspace` |
| One UI test | `cargo nextest run -p viewer --test ui <name>` |
| Watch: Clippy on save; `n` nextest, ctrl-j other jobs | `bacon` |
| Tools and git hook in a fresh clone | `just setup` |

Iterate on the narrowest check. Engine work needs only the engine tests.

`just check` stops at the first failure. In order: `cargo fmt --check`, `typos`, Clippy on all targets, Clippy on the production feature set (`-p viewer --lib --bins`, without test-support), nextest with the `ci` profile, doc-tests (nextest skips them), `cargo doc` (broken doc links), engine coverage, `cargo shear` (unused dependencies and source files no module includes), `cargo deny check` (advisories, licenses, bans, sources; fetches the RustSec database). It sets `CARGO_BUILD_WARNINGS=deny`, so any warning fails it; plain `cargo` commands only warn, so you can iterate.

Lints live in the root `Cargo.toml` (`[workspace.lints]`, one comment per non-obvious choice) and `clippy.toml`. Tests may `unwrap` and `panic`; elsewhere return errors or use `expect("why it cannot fail")`. `std::thread::sleep` is banned: in tests, drive time with the GPUI executor's timer and clock.

nextest reports a test as slow after 10 s and kills it after 30 s (`.config/nextest.toml`). A test that needs longer is a defect.

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
| `just check`, warm, nothing changed | 3.4 s |
| `just check` after touching both crates | 5.3 s |
| `just lint` (the pre-commit hook), warm | 0.4 s |

On this machine, a step that takes several times longer than these is a defect. Stop and report it.

## Hot reload

Not available. The trial on 2026-11-22 stopped before a live run:

- `gpui-pre` 0.3.12 has a `hot-reload` feature, but it only applies patches when the app is launched by the Dioxus CLI with `dx serve --hot-patch`. Without `dx` it logs a warning and does nothing. It is compiled in for debug builds only.
- `gpui-kit` 0.10.0 does not re-export that feature. Enabling it needs the Dioxus CLI and a direct `gpui-pre` dependency. The Rules forbid the direct dependency.

Neither was added. Rebuild and restart instead.

## Known gaps

- Closing the window does not quit the app, so `cargo run -p viewer` does not return; stop it with Ctrl-C. Quit handling arrives with key bindings in milestone 2.
