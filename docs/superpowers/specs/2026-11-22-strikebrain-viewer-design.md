# Strikebrain Viewer design

Status: design approved by Sean on 2026-11-22. Nothing is built yet.
Companion page: https://claude.ai/artifact/4PaAcuEDiMKYJKyEFCpkD3

## Purpose

A native desktop viewer for text and image files.

- Left pane: brainstrike's file explorer, ported to Rust. Filter the tree, check files.
- Right pane: a stage showing one file at a time from the checked set. Arrow keys move through the set.
- Space on the stage starts a fullscreen presentation.

Goals, in order: speed at million-file scale, a small codebase, and code Sean can learn Rust and GPUI from.

Platform: macOS on Apple silicon. GPUI also targets Windows and Linux. Nothing here should prevent them; neither is tested in version one.

## Settled decisions

1. **GPUI through GPUI Kit.** `gpui-kit = "=0.10.0"`. The explorer tree is built directly on GPUI's `uniform_list`, not the Kit's `Tree`.
2. **The gallery is a stage.** One file fills the right pane. No tile grid in version one.
3. **Space depends on focus.** It toggles a checkbox in the tree and starts the presentation on the stage.
4. **Filter terms are ANDed.** A file must contain every include term.
5. **The filter block is small.** One text input and three kinds: All, Images, Text. No presets, chips, glob lists or regex mode.

## Stack

| Piece | Version | Notes |
| --- | --- | --- |
| Rust | 1.100.0, edition 2024 | Pinned in `rust-toolchain.toml` with `rustfmt` and `clippy`. |
| `gpui-kit` | `=0.10.0` | Feature `syntect`. Dev-dependency adds `test-support` at the same version. Re-exports GPUI (`gpui-pre 0.3.12`). |
| `ignore` | 0.4 | Parallel directory walk with `.gitignore` rules. |
| `memchr` | latest | Substring search for the filter. |
| `fixedbitset` | 0.5 | Selection, expansion and visibility sets. |
| `icu_collator` | 2 | Natural, case-insensitive name order. Matches brainstrike's `Intl.Collator(undefined, { numeric: true, sensitivity: "base" })`. |
| `notify` 9, `directories` 6, `serde` 1, `serde_json` latest | | Milestone 5 only. |
| `criterion` 0.8, `proptest` latest, `tempfile` latest | | Dev-dependencies of the engine. |

Rules for every dependency:

- "Latest" means the newest stable release on crates.io on the day the crate is added. `Cargo.lock` is committed.
- Check the current docs and source before using an API. GPUI and the Kit are pre-1.0 and release weekly. Never translate an API from memory, from React, or from an older GPUI example.
- Application code depends on `gpui-kit` alone for UI. It never lists a GPUI crate directly.
- Add a crate only when the standard library or an existing dependency does not cover the need.

## Repository layout

```
strikebrain-viewer/
├─ Cargo.toml             workspace: pinned versions, lints, dev profile
├─ rust-toolchain.toml
├─ AGENTS.md
├─ docs/superpowers/specs/
└─ crates/
   ├─ explorer/           library, no GPUI
   │  ├─ src/{tree,scan,selection,filter,sort,rows,content}.rs
   │  └─ benches/
   └─ viewer/             binary `strikebrain-viewer`
      ├─ src/{main,app,explorer_panel,stage,keys,theme}.rs
      └─ tests/ui.rs
```

`explorer` never depends on `gpui-kit`. Its tests must compile and run without building the UI stack.

Workspace settings:

- `resolver = "3"`.
- Lints: `unsafe_code = "forbid"`; clippy `all` and `pedantic` at warn.
- Dev profile, taken from the Kit's installation guide: `debug = "limited"`, `split-debuginfo = "unpacked"`, and `opt-level = 3` for `gpui-pre`, `gpui-pre-platform`, `gpui-kit`, `gpui-component`, `gpui-kit-assets`, `taffy`, `rustybuzz`, `ttf-parser`.

## Engine: crate `explorer`

### Tree

A scan produces one immutable `Tree`, shared as `Arc<Tree>`. Nodes are numbered in depth-first pre-order. `NodeId` wraps a `u32`. The root folder is id 0.

Parallel arrays, one entry per node:

- `names`: a `NameArena`, one `String` buffer plus one offset per node.
- `haystack`: the lowercased names in one buffer, separated by `\0`, for the filter.
- `parent: Vec<NodeId>`
- `end: Vec<u32>`: the subtree of `id` is the id range `id..end[id]`.
- `depth: Vec<u16>`
- `kind: Vec<Kind>`: `Dir`, `File`, `Symlink` or `Unreadable`.
- `size: Vec<u64>`: a file's byte size; a folder's is the sum of its files.
- `ext: Vec<u16>`: index into a small table of interned lowercase extensions.
- `files: FixedBitSet`: 1 where the node is a file or a symlink.
- `children`: each folder's child ids as one slice of a shared `Vec<NodeId>`. This list holds the display order.

Invariants. Each is a property test.

1. `parent[id] < id` for every node except the root.
2. For a file, `end[id] == id + 1`. For a folder, `id..end[id]` contains exactly its descendants and itself.
3. Ids follow canonical order: within a folder, folders first, then natural name order ascending.
4. `size[folder]` equals the sum of `size` over the files in its range.

Ids are stable for the life of one `Tree`. Sorting never changes them. A rescan produces a new `Tree` with new ids; state is carried across by path.

### Scan

- `ignore::WalkBuilder` in parallel mode, off the UI thread.
- Default rules: honour `.gitignore`, `.ignore` and global git excludes; show dotfiles; never follow symlinks; always skip `.git` and `.DS_Store`.
- "Show ignored" rescans with the ignore rules off. `.git` and `.DS_Store` stay skipped.
- File size comes from the walker's metadata call.
- A folder that cannot be read becomes a node of kind `Unreadable`. A symlink is a leaf, whatever it points to.
- Refresh rescans with the current rules.
- Every scan carries a generation number. A result whose generation is not the latest is dropped.
- No caps on entries per folder, depth or total nodes.

`Tree::from_paths(&[&str])` builds a tree from path strings, so engine tests need no disk.

### Selection

One `FixedBitSet` over node ids. Only file bits are ever set.

The universe of a folder is the set of files in its range that pass the current filter. With no filter it is every file in the range.

- A file is `On` or `Off`.
- A folder is `Off` when no file in its universe is selected, `On` when its universe is non-empty and fully selected, `Mixed` otherwise. An empty universe is `Off`.
- Clicking an `On` folder clears every file in its range, including files the filter hides.
- Clicking an `Off` or `Mixed` folder selects every file in its universe.
- Select all selects every file that passes the filter. Clear empties the set.
- There is no selection cap.

A folder's state is a population count over its range. It is computed only for rows on screen.

### Filter

Input: the query string and a kind (`All`, `Images`, `Text`).

Query grammar, as in brainstrike:

- Split on whitespace. A term starting with `-` and longer than one character is an exclude term. Every other term is an include term.
- Terms match the node's name, never its path. Matching is a lowercase substring test.

Rules:

- With an empty query and kind `All` the filter is inactive and every node is visible, empty folders included.
- A file passes the query when its name contains every include term and no exclude term.
- A folder whose name contains an exclude term is removed with its whole subtree.
- Kind `Images` passes files whose extension is one of `png jpg jpeg webp gif bmp tif tiff ico svg`. Kind `Text` passes files that are not images and whose extension is not in the known-binary list in `content.rs`. Kind `All` passes every file.
- A file is visible when it passes both the query and the kind.
- A folder is visible when it is not removed and either has a visible descendant or its own name contains every include term of a non-empty include list.

Output: a `Visibility` bitset, built by one pass over `haystack` followed by one backwards pass that marks ancestors.

While a query or a kind other than `All` is active:

- Every folder with a visible descendant is shown open. The stored expansion set is not modified and returns when the filter clears.
- Collapsing a folder is allowed. Those collapses live in a temporary set that resets whenever the query text or kind changes.

The filter is a pure function `(&Tree, &Query, KindFilter) -> Visibility`. The app runs it on a background task and applies the result with a generation check. Replacing the stored task cancels the previous run.

### Sort

- Keys: `Name`, `Type`, `Size`. Direction: ascending or descending. Default `Name` ascending.
- Folders always come before files, in both directions.
- `Name`: natural order, case-insensitive. `Type`: extension, then name. `Size`: bytes, then name. Direction applies to the primary key only; the name tiebreak is always ascending.
- A re-sort reorders each folder's slice of `children`. Nothing else changes.

### Rows

`rows: Vec<NodeId>` is the flattened visible list. It is rebuilt in full after any change to expansion, visibility or sort, by a depth-first walk over `children` that skips closed folders. The root node itself is not a row.

The gallery is every selected file in display order across the whole tree. The filter does not affect it.

### Facade

`Explorer` owns the `Arc<Tree>`, selection, expansion, visibility, sort and rows. It is a plain struct with no async and no UI types. Its methods cover: read rows, read a node's check state, toggle a check, select all, clear, toggle a folder, expand all, collapse all, set sort, apply a filter result, iterate the gallery, and replace the tree. Replacing the tree carries selection and expansion across by path and returns the old-to-new id mapping, so the app can carry `current`.

### Content

`content.rs` holds pure functions with no I/O of their own:

- `viewer_for(name, head) -> Viewer` returns `Image`, `Markdown`, `Code` or `Unsupported`. `head` is the first 8 KB of the file.
- A NUL byte in `head` means `Unsupported`, unless the extension is an image extension.
- `md` and `markdown` are `Markdown`. Other text is `Code`.

## App: crate `viewer`

### State

One entity, `AppState`, owns:

- `explorer: Explorer`
- `current: Option<NodeId>`: the file on the stage
- `focus_row: usize`: the keyboard row in the tree
- `presenting: bool`
- generation counters for scan and filter, and the latest task of each

Two views observe it: `ExplorerPanel` and `Stage`. The root view lays them out in the Kit's horizontal resizable split. The explorer starts 300 px wide with a 220 px minimum.

Rules for `current`:

- Enter or a click on a file row sets it to that file, checked or not.
- Checking a single file sets it to that file.
- When it is `None` and the gallery becomes non-empty, it becomes the first gallery file.
- Unchecking the current file moves it to the next gallery file, else the previous, else `None`.
- Next and previous move within the gallery. When `current` is not in the gallery, next goes to the first gallery file after it in display order and previous to the last one before it. With an empty gallery they do nothing.

### Launch

- `strikebrain-viewer <folder>` opens that folder.
- With no argument it reopens the last folder (milestone 5) or shows the empty state.
- ⌘O opens the system folder picker.
- The window title is the folder's name.

### Explorer panel

Top to bottom:

1. **Toolbar**, 36 px. Left: Expand all, Collapse all. Right: Show ignored, Refresh, Select all (which becomes Clear when anything is selected). Icon buttons, 24 px, each with a tooltip and an accessible label.
2. **Filter input**, 32 px, with a search icon and a clear button.
3. **Kind control**, three equal segments: All, Images, Text.
4. **Column header**, 28 px, sticky. `Name` always. `Size` when the tree is at least 300 px wide. `Type` when it is at least 420 px wide. Clicking a header sorts by it; clicking again flips direction.
5. **Tree**, one `uniform_list` over `rows`.
6. **Selection bar**, floating at the bottom of the tree when anything is selected: "N selected" and a clear button. It never moves rows; the list gains bottom padding while it shows.

Row, left to right. These are brainstrike's measurements.

- Height 28 px plus a 1 px bottom rule.
- Indent 20 px per level.
- Chevron in a 24 px target, folders only. Files get a 24 px spacer.
- 6 px gap, 16 px checkbox, 6 px gap, 16 px icon, 6 px gap, name at 14/16 px, truncated.
- `Size` cell: monospace 12 px, tabular figures, one decimal, units B, KB, MB, GB.

Row behaviour:

- Clicking a checkbox toggles it.
- Clicking a chevron opens or closes the folder. Clicking the rest of a folder row only moves focus.
- Clicking a file row shows it on the stage.
- The row for `current` has the accent background.
- Hover uses the muted background at half strength.
- The keyboard focus ring appears only after a key press and clears on pointer down.
- An `Unreadable` folder shows a warning icon where its chevron would be.

States:

- No folder: one "Open Folder" button centred in the pane. The stage is blank.
- Scanning for more than 200 ms: a thin indeterminate bar under the toolbar.
- No rows after filtering: the tree area is blank.
- The folder itself cannot be read: the operating system's error message, one line, in the tree area.

Not in version one: column resizing, a column menu, context menus, multiple root folders, type-ahead.

### Stage

- `Image`: GPUI's `img()` from the file path, scaled down to fit and never enlarged.
- `Markdown`: the Kit's `TextView`, scrollable.
- `Code`: the Kit's editor, read-only, with line numbers and `syntect` highlighting chosen by extension.
- `Unsupported`: the file's name and size, centred and muted.
- When `current` is in the gallery, a position such as "2 / 3" sits at the bottom centre.
- A blank stage draws nothing.

Limits and caches:

- Text files over 10 MB are `Unsupported`. Milestone 3 measures the editor and adjusts this number.
- Invalid UTF-8 is decoded lossily.
- Decoded images live in a bounded least-recently-used `ImageCache` holding 8 images. Evicted images release their GPU texture.
- The next and previous gallery images are preloaded through the same cache.
- Loaded text is kept in a least-recently-used cache capped at 32 MB.
- Every load carries the id it was started for. A result for a file that is no longer `current` or a neighbour is dropped.

### Presentation

- Starts from the stage with Space or Enter when the stage shows a file.
- Sets `presenting`, hides the explorer and the position indicator, and makes the window fullscreen if it is not already.
- Leaving restores the split and exits fullscreen only if presenting entered it.
- Movement stops at the first and last gallery file. It does not wrap.

### Keys

Bindings are GPUI actions bound per key context: `Explorer`, `Filter`, `Stage`, `Presentation`.

| Key | Explorer | Stage | Presentation |
| --- | --- | --- | --- |
| ↑ ↓ | Move focus | Scroll text | Scroll text |
| → | Open folder, then first child | Next file | Next file |
| ← | Close folder, then parent | Previous file | Previous file |
| Space | Toggle checkbox | Present | Next file |
| Enter | Show file on stage; nothing on a folder | Present | |
| Esc | Clear the filter | Focus the explorer | Leave |
| Home, End, Page Up, Page Down | Jump | Jump within text | Jump within text |
| ⌘F | Focus the filter | Focus the filter | |
| ⌘O | Open folder | Open folder | |
| Tab | Next pane | Next pane | |

In the filter input, Esc clears a non-empty query and otherwise moves focus to the tree. Tab order is filter, tree, stage.

Differences from brainstrike: Enter and file clicks show the file on the stage instead of opening a tab; checking a file also shows it; the first arrow press moves focus immediately; Page Up, Page Down, ⌘F and ⌘O are new.

### Theme

Brainstrike's tokens, applied through the Kit's theme system, following the system appearance. Light / dark:

- background `hsl(30 25% 99%)` / `hsl(30 8% 9%)`
- primary `hsl(215 55% 45%)` / `hsl(215 50% 58%)`
- muted `hsl(30 15% 94%)` / `hsl(30 5% 18%)`
- accent `hsl(30 12% 90%)` / `hsl(30 5% 22%)`
- border `hsl(30 10% 90%)` / `hsl(30 5% 20%)`
- info, used for folder icons, `hsl(215 50% 38%)` / `hsl(215 45% 74%)`

Checked and mixed checkboxes use primary with a white check or dash. Unchecked boxes use the background with a primary border.

### Accessibility

The tree and its rows carry AccessKit roles, labels and checked state. Tests and agents find rows by the same labels a screen reader uses. Every icon button has a label.

### Copy

Only these strings appear in the interface: "Filter" as the input placeholder, "All", "Images", "Text", "Name", "Size", "Type", "N selected", "Open Folder", tooltips on icon buttons, and operating system error messages. No taglines, no helper text.

## Left out on purpose

From brainstrike: the tokens column and token indexing; the "This thread" column, thread presets and checkout header; attach to thread, export and split into parts; workspace tabs and the editor; the selection settings popover and selection cap; context actions; multiple roots; view filter presets, chips, glob lists and regex mode; lazy folder loading and its spinners; the 10,000-entry listing cap; the expand-all confirmation.

From a typical image viewer: zoom and pan, a tile grid, HEIC, AVIF and RAW formats, editing of any kind.

## Testing

Three tiers.

1. **Engine.** Plain `#[test]` and `proptest` in `explorer`. Property tests compare each fast path with a brute-force version: check state against a recursive count, rows against a naive recursive walk, filter visibility against a per-node check, sort against a full comparison sort. Scan tests use `tempfile`.
2. **UI integration.** `#[gpui_kit::test]` in `crates/viewer/tests/ui.rs`. Each opens a headless window on the real root view over a `Tree::from_paths` fixture, drives it with clicks and key presses, and asserts on labels, values, focus and model state.
3. **Benchmarks.** `criterion` in `explorer/benches`, over a synthetic million-node tree and a real scan.

Behaviour tests ported from brainstrike's suite, by name:

- a mixed folder completes on click
- a fully checked folder clears on click
- clicking a folder row does not open it; the chevron does
- Space toggles the checkbox
- select all under a filter selects only matches
- unchecking a folder under a filter clears hidden files too
- folders sort before files in both directions
- `file2` sorts before `file10`

New behaviour tests:

- checking a file shows it on the stage
- unchecking the current file moves the stage to its neighbour
- the filter finds a file inside a closed folder and shows its ancestors open
- every include term must match
- Space on the stage enters the presentation and Esc restores the split

Commands: `cargo nextest run` for tests, `bacon` for watch mode, `cargo clippy --workspace --all-targets`, `cargo deny check`.

## Budgets

Targets, not measurements. Milestone 1 ends when the benchmarks meet them or explain why not. Reference corpus: the brainstrike checkout, 5,962 tracked files and 1,650,231 entries on disk.

| Operation | Scale | Budget |
| --- | --- | --- |
| Open a folder, tree on screen | 6,000 files | 100 ms |
| Open a folder with ignored files shown | 1.65 M entries | 3 s, window responsive throughout |
| Filter keystroke to updated rows | 1 M names | 16 ms |
| Check or clear a folder | 1 M files | 1 ms |
| Expand or collapse, rows rebuilt | 1 M visible rows | 16 ms |
| Scroll | any | every frame at display rate |

## Milestones

Each ends with something that runs and tests that hold it. Each gets its own implementation plan, written when the one before it is done.

| | Scope | Done when |
| --- | --- | --- |
| M0 Skeleton | Toolchain pin, workspace, lints, dev profile, `AGENTS.md`, Kit agent skills, a window with two resizable panes, a one-hour trial of GPUI's `hot-reload` feature. | The window opens, one headless UI test passes, and cold and warm build times are recorded in `AGENTS.md`. |
| M1 Engine | Tree, scan, selection, filter, sort, rows, facade, `content`. | Property tests pass and benchmarks meet the budgets. |
| M2 Explorer panel | Rows, checkboxes, chevrons, keyboard, filter input, kind control, toolbar, sortable header, selection bar, theme, accessibility roles. | The ported behaviour tests pass and 1.65 M rows scroll at display rate. |
| M3 Stage | Image, Markdown and code viewers, left and right, caches, preloading. | Flicking through 1,000 mixed files stays under a fixed memory ceiling recorded in the test. |
| M4 Presentation | Fullscreen, keys, leave. | Entering and leaving keeps the current file and the window's prior fullscreen state. |
| M5 Finish | Saved state, live refresh, a packaged `.app`. | Quitting and reopening returns the same view. |

Milestone 0 detail:

- The Kit's agent skills install with `npx skills add longbridge/gpui-kit`. That downloads third-party agent instructions into the repo, so Sean runs it or approves it at the time.

Milestone 5 detail:

- State is a JSON file in the platform config directory: root path, expanded paths, selected paths, query, kind, sort, first visible row, split width, window bounds. Paths are stored relative to the root.
- Live refresh uses a recursive `notify` watcher with a 150 ms debounce. Any batch of events triggers a full background rescan and a tree swap that carries state across by path.

## Functions Sean writes

Each is set up with its signature, doc comment and failing tests. Sean writes the body.

| Function | File | Milestone |
| --- | --- | --- |
| `Selection::check` | `crates/explorer/src/selection.rs` | M1 |
| `sibling_order` | `crates/explorer/src/sort.rs` | M1 |
| `Query::parse` | `crates/explorer/src/filter.rs` | M1 |
| `viewer_for` | `crates/explorer/src/content.rs` | M1 |
| `bindings` | `crates/viewer/src/keys.rs` | M2 |

## Risks

- **Nothing has been compiled.** Every API named here was checked against current docs and upstream source. Milestone 0 is the first contact with a compiler.
- **Pre-1.0 dependencies.** The Kit went from 0.7 to 0.10 in six weeks. Versions are pinned exactly and upgraded on purpose.
- **`gpui-pre` is a republish.** It is Zed's source, published by the Kit's maintainer and not by Zed. `Cargo.lock` and `cargo deny` guard against drift. A git dependency on `zed-industries/zed` is the fallback.
- **Large photos.** GPUI decodes at full resolution; a 48-megapixel image is about 190 MB decoded. If milestone 3 measures a problem, the fix is a custom loader that decodes at display size.
- **Editor on large files.** The Kit documents its editor as stable at 200,000 lines.
- **Cold build time** is unknown until milestone 0.

## References

- GPUI: https://gpui.rs/ and https://github.com/zed-industries/zed/tree/main/crates/gpui
- GPUI Kit docs index: https://gpui-kit.com/llms.txt. Any page is Markdown with `.md` appended.
- GPUI Kit repository and agent skills: https://github.com/longbridge/gpui-kit
- Brainstrike explorer: `packages/file-explorer`, `file-explorer-ui`, `file-explorer-hooks`, `file-tree-worker`, `file-system`, and `packages/ui/src/tree-table` in `/Users/seandearnaley/Documents/GitHub/brainstrike`
