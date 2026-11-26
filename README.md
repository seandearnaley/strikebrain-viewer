# Strikebrain Viewer

A native viewer for folders of text and images, for macOS, Linux and Windows: a file tree on the left, holding as many folders as you add, where you filter and check files, and a stage on the right. Check two or more files and the stage shows them as a gallery, a grid of thumbnails and the tops of text files; open one and it fills the stage, where ← and → move through the rest. It is written in Rust on GPUI through GPUI Kit.

## Install

The [releases page](https://github.com/seandearnaley/strikebrain-viewer/releases) has:

- **macOS, Apple silicon:** `Strikebrain-Viewer-<version>-macos-arm64.dmg`. Open it and drag the app to Applications.
- **Debian and Ubuntu:** `strikebrain-viewer_<version>_amd64.deb`. Install it with `sudo apt install ./strikebrain-viewer_<version>_amd64.deb`.
- **Other Linux, x86-64:** `strikebrain-viewer-<version>-linux-x86_64.tar.gz`, laid out as `bin/` and `share/`. Unpack it into `~/.local` with `tar -xzf <file> -C ~/.local --strip-components=1`. It needs the xcb, xkbcommon and Wayland client libraries and a Vulkan driver, which a desktop already has.
- **Windows, x64:** `Strikebrain-Viewer-<version>-windows-x64-setup.exe` installs it for you alone, with a Start menu entry and no administrator prompt. `strikebrain-viewer-<version>-windows-x64.zip` holds the same program to unpack and run anywhere.

The macOS image is signed with a Developer ID and notarized by Apple; the Windows installer and program are signed through Azure Artifact Signing. `SHA256SUMS` lists each file's checksum, and `gh attestation verify <file> -R seandearnaley/strikebrain-viewer` checks that GitHub built it from this repository.

## Run from source

Needs the toolchain in `rust-toolchain.toml` (rustup installs it). It is developed on macOS on Apple silicon; CI builds and tests it on Linux and Windows. On Linux, install the libraries `.github/workflows/ci.yml` lists.

```sh
cargo run -p viewer -- ~/some/folder ~/another   # open these folders
cargo run -p viewer -- ~/Downloads/photo.jpg     # reopen the last folders, and show this file
cargo run -p viewer                              # reopen the last folders as you left them
```

Add Folder (⌘O, the bar's folder button, or File > Add Folder…) adds more, as does a folder dropped on the window; each folder is a top-level row, removed with the button that shows on its row when you point at it, or ⌘⌫. A file opened on its own (named on the command line, dropped on the window, or sent by Open With) shows in an open folder that holds it, or else as a row of its own, without reading its folder, until the app quits. The explorer hides with ⌃⌘S, the bar's sidebar button or View > Hide Sidebar, and the stage takes the window. Back and Forward (⌘[ and ⌘], the bar's arrows, the Go menu or the mouse's side buttons) go through what the stage showed. The app remembers the folders, their open folders and checks, the filter, sort, scroll, the file on the stage and whether the gallery showed, the explorer's width and whether it was hidden, and the window, in `~/Library/Application Support/ai.brainstrike.viewer/state.json`. It follows each folder and each file row on disk and refreshes it when it changes.

## Keys

| Key | Tree | Gallery | Stage | Presentation |
| --- | --- | --- | --- | --- |
| ↑ ↓ | Move focus | Move focus up or down a row | Scroll text | Scroll text |
| → | Open folder, then first child | Next tile | Next file | Next file |
| ← | Close folder, then parent | Previous tile | Previous file | Previous file |
| Space | Check or uncheck | Present from the focused tile | Present | Next file |
| Enter | Show the file | Open the focused tile | Present | |
| Esc | Clear the filter | Back to the tree | Back to the gallery, with two or more checked; else to the tree | Leave, to the gallery or the file |
| Home, End | Jump | First or last tile | Jump within text | Jump within text |
| Page Up, Page Down | Jump | A screen of rows | Jump within text | Jump within text |
| ⌘F | Filter | Filter | Filter | |
| ⌘O | Add folders | Add folders | Add folders | |
| ⌘⌫ | Remove the focused folder or file row | | | |
| ⌘[ ⌘] | Back, Forward | Back, Forward | Back, Forward | |
| ⌃⌘S | Hide the explorer | Hide or show the explorer | Hide or show the explorer | |
| ⌘R | Refresh | Refresh | Refresh | Refresh |
| ⇧⌘. | Show ignored files, or hide them | The same | The same | The same |
| ⇧⌘R | Reveal the focused row in Finder | Reveal the file | Reveal the file | Reveal the file |
| ⌥⌘C | Copy the focused row's path | Copy the file's path | Copy the file's path | Copy the file's path |
| ⌃⌘F | Fullscreen, or back | The same | The same | The same |
| Tab | Next pane | Next pane | Next pane | |

On Windows and Linux, Ctrl stands for ⌘, F11 is fullscreen, Alt+← and Alt+→ go back and forward, and Ctrl+B hides or shows the explorer.

A click on a tile opens it too; under the file, the grid button left of its place ("2 / 3") goes back to the gallery. In the filter, Esc clears the text, then returns to the tree.

## Build the packages

```sh
just bundle           # macOS: target/release/bundle/osx/Strikebrain Viewer.app
just dmg              # macOS: the app in a disk image
just linux-packages   # Linux: a .tar.gz and a .deb
just windows-packages # Windows: a .zip and an installer (needs Inno Setup 6)
```

On macOS this builds the release binary and wraps it in an `.app`, signed ad hoc for this Mac. You can then drop a folder or a file on its Dock icon, or choose it in Finder's Open With: a folder is added, and a file shows in an open folder that holds it, or as a row of its own.

## Develop

`just` lists the tasks. `just check` runs every gate and must pass before each commit; `just shot` renders key screens to `target/shots/` without a window. `AGENTS.md` has the rules, the commands and how the tests work.

## Layout

```
crates/explorer   the engine: scan, tree, filter, sort, checks, rows; no UI
crates/viewer     the app: GPUI views, state, saved state, live refresh, menus
  bundle/         Info.plist template, icon source and Linux desktop entry for the packages
  tests/ui/       headless UI tests on the real views
docs/superpowers/specs/2026-11-22-strikebrain-viewer-design.md   the design
```

The design spec is the source of truth for behaviour; read it before changing any.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option. Unless you state otherwise, any contribution you submit for inclusion in this work, as defined in the Apache-2.0 license, is dual licensed as above, without any additional terms or conditions.
