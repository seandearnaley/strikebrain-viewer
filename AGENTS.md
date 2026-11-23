# Strikebrain Viewer

A native viewer for text and images, built on GPUI through GPUI Kit.
Design: `docs/superpowers/specs/2026-11-22-strikebrain-viewer-design.md`.

## Rules

- Never invent an API. GPUI and GPUI Kit are pre-1.0 and change weekly. Before using a call, read its current docs or source. Do not translate from memory, React, CSS or older GPUI examples.
- Sources, in order: the vendored source under `~/.cargo/registry/src/*/` (`gpui-pre-0.3.12`, `gpui-base-0.10.0`, `gpui-component-0.10.0`), then `https://gpui-kit.com/llms.txt` (append `.md` to any page).
- `crates/explorer` has no UI dependency. Keep it that way.
- UI code depends on `gpui-kit` only. Never add a GPUI crate directly.
- Versions are pinned exactly. Upgrading `gpui-kit` is its own change.
- No taglines or helper text in the interface. If a screen needs them, fix the layout.
- Prefer the framework's primitive over a custom one. Add a crate only when the standard library and current dependencies do not cover the need.

## Commands

| Task | Command |
| --- | --- |
| Run the app | `cargo run -p viewer` |
| Engine tests | `cargo nextest run -p explorer` |
| All tests | `cargo nextest run --workspace` |
| One UI test | `cargo nextest run -p viewer --test ui <name>` |
| Watch | `bacon` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Lint production build | `cargo clippy -p viewer --lib --bins -- -D warnings` |
| Format | `cargo fmt --all` |
| Dependency policy | `cargo deny check advisories bans sources` |

Iterate on the narrowest check. Engine work needs only the engine tests.

## Build times

Measured on 2026-11-22, Apple silicon, Rust 1.100.0.

| Build | Time |
| --- | --- |
| Cold `cargo build -p viewer` | 39 s |
| Warm rebuild after touching `crates/viewer/src/app.rs` | 0.7 s |
| Engine test cycle after touching `crates/explorer/src/lib.rs` | 0.5 s |

A step that takes much longer than these is a defect. Stop and report it.

## Hot reload

Not available. The trial on 2026-11-22 stopped before a live run:

- `gpui-pre` 0.3.12 has a `hot-reload` feature, but it only applies patches when the app is launched by the Dioxus CLI with `dx serve --hot-patch`. Without `dx` it logs a warning and does nothing. It is compiled in for debug builds only.
- `gpui-kit` 0.10.0 does not re-export that feature. Enabling it needs both the Dioxus CLI and a direct `gpui-pre` dependency, which the Rules forbid.

Neither was added. Rebuild and restart instead.

## Known gaps

None.
