# Strikebrain Viewer

A native macOS viewer for folders of text and images: a file tree on the left, holding as many folders as you add, where you filter and check files, and a stage on the right that shows one checked file at a time. It is written in Rust on GPUI through GPUI Kit.

## Run

Needs the toolchain in `rust-toolchain.toml` (rustup installs it) and macOS on Apple silicon.

```sh
cargo run -p viewer -- ~/some/folder ~/another   # open these folders
cargo run -p viewer                              # reopen the last folders as you left them
```

Add Folder (⌘O, the toolbar's folder button, or File > Add Folder…) adds more; each folder is a top-level row, removed with the button that shows on its row when you point at it, or ⌘⌫. The app remembers the folders, their open folders and checks, the filter, sort, scroll, the file on the stage, the explorer's width and the window, in `~/Library/Application Support/ai.brainstrike.viewer/state.json`. It follows each folder on disk and refreshes that folder when its files change.

## Keys

| Key | Tree | Stage | Presentation |
| --- | --- | --- | --- |
| ↑ ↓ | Move focus | Scroll text | Scroll text |
| → | Open folder, then first child | Next file | Next file |
| ← | Close folder, then parent | Previous file | Previous file |
| Space | Check or uncheck | Present | Next file |
| Enter | Show the file | Present | |
| Esc | Clear the filter | Back to the tree | Leave |
| Home, End, Page Up, Page Down | Jump | Jump within text | Jump within text |
| ⌘F | Filter | Filter | |
| ⌘O | Add folders | Add folders | |
| ⌘⌫ | Remove the focused folder | | |
| Tab | Next pane | Next pane | |

In the filter, Esc clears the text, then returns to the tree.

## Build the app bundle

```sh
just bundle
open "target/release/bundle/osx/Strikebrain Viewer.app"
```

This builds the release binary and wraps it in a signed (ad hoc) `.app`. You can then drop a folder on its Dock icon, or choose it in Finder's Open With, to add it.

## Develop

`just` lists the tasks. `just check` runs every gate and must pass before each commit; `just shot` renders key screens to `target/shots/` without a window. `AGENTS.md` has the rules, the commands and how the tests work.

## Layout

```
crates/explorer   the engine: scan, tree, filter, sort, checks, rows; no UI
crates/viewer     the app: GPUI views, state, saved state, live refresh, menus
  bundle/         Info.plist template and icon source for `just bundle`
  tests/ui/       headless UI tests on the real views
docs/superpowers/specs/2026-11-22-strikebrain-viewer-design.md   the design
```

The design spec is the source of truth for behaviour; read it before changing any.
