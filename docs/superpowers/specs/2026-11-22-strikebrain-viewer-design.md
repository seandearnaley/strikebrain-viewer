# Strikebrain Viewer design

Status: design approved by Sean on 2026-11-22. Milestones 0 to 5 are built (2026-11-23); where the build settled a question the design left open, the section below says so.
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
| `notify` 9, `directories` 6, `serde` 1, `serde_json` latest | | Milestone 5. No debouncer crate: the 150 ms window runs on GPUI's timer, which tests drive. |
| `futures` 0.3 | | Milestone 5: the channel from the file watcher's thread into a GPUI task. GPUI exposes none and already builds this crate. |
| `criterion` 0.8, `proptest` latest, `tempfile` latest | | Dev-dependencies of the engine. |

Rules for every dependency:

- "Latest" means the newest stable release on crates.io on the day the crate is added. `Cargo.lock` is committed.
- Check the current docs and source before using an API. GPUI and the Kit are pre-1.0 and release weekly. Never translate an API from memory, from React, or from an older GPUI example.
- Application code depends on `gpui-kit` alone for UI. It never lists a GPUI crate directly.
- Add a crate only when the standard library or an existing dependency does not cover the need.

## Repository layout

```
strikebrain-viewer/
├─ Cargo.toml             workspace: pinned versions, lints, dev and release profiles
├─ rust-toolchain.toml
├─ AGENTS.md, README.md
├─ docs/superpowers/specs/
└─ crates/
   ├─ explorer/           library, no GPUI
   │  ├─ src/{tree,scan,selection,filter,sort,rows,content}.rs
   │  ├─ src/scan/changes.rs   which changes on disk a rescan would show
   │  └─ benches/
   └─ viewer/             binary `strikebrain-viewer`
      ├─ src/{main,app,explorer_panel,stage,keys,theme}.rs
      ├─ src/{persist,launch,menus}.rs, src/state/{scan,filter,restore,watch}.rs
      ├─ bundle/          Info.plist template and icon source for `just bundle`
      └─ tests/ui/*.rs, tests/shots.rs, tests/icon.rs
```

`explorer` never depends on `gpui-kit`. Its tests must compile and run without building the UI stack.

Workspace settings:

- `resolver = "3"`.
- Lints: `unsafe_code = "forbid"`; clippy `all` and `pedantic` at warn.
- Dev profile: `debug = "limited"`, `split-debuginfo = "unpacked"`, and `opt-level = 3` for all dependencies via `[profile.dev.package."*"]`, because the Kit guide's per-crate list omits `gpui-base` and `gpui-pre-macos`.

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

- `strikebrain-viewer <folder>` opens that folder. It takes the saved view only if it is the folder saved (compared after canonicalizing).
- With no argument it reopens the last folder with its saved view; if that folder no longer exists, or there is no usable saved state, it shows the empty state.
- The window's bounds and the explorer's width are restored either way.
- ⌘O opens the system folder picker. A folder given by the system ("Open With", or dropped on the Dock icon; the bundle declares `public.folder`) arrives as a `file://` URL and opens in place of the folder on screen.
- The window title is the folder's name.
- The menu bar: the app menu (About, Quit), File (Open Folder…), Edit (Undo, Redo, Cut, Copy, Paste, Select All, for the filter input and the code view), View (Present, Show Ignored with a check, Refresh) and Window (Minimize, Zoom). Present has its own unbound action, because a menu item shows its action's first key as a shortcut and `Present` is Space, which the menu would then take from the filter input. Open Folder is disabled while presenting, as ⌘O is ignored there.

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
- `Markdown`: the Kit's `TextView`, scrollable, in a centred column 720 px wide at most.
- `Code`: the Kit's editor, read-only, with line numbers and `syntect` highlighting chosen by extension.
- `Unsupported`: the file's name and size, centred and muted.
- When `current` is in the gallery, a position such as "2 / 3" sits at the bottom centre.
- A blank stage draws nothing.

The stage reads the first 8 KB on the background executor and asks `viewer_for`. Only regular files are opened: a symlink to a FIFO or a device is `Unsupported`. A picture GPUI cannot decode shows as `Unsupported`. Until a read lands the stage keeps what it showed.

Limits and caches:

- Text files over 10 MB are `Unsupported`. Milestone 3 measured the editor and kept 10 MB: see Risks.
- Markdown files over 256 KB open in the code view, as their source: the Markdown view's first frame grows with the square of the document's size (see Risks).
- Invalid UTF-8 is decoded lossily.
- Decoded images live in a bounded least-recently-used `ImageCache` holding 8 images. Evicted images release their GPU texture.
- The next and previous gallery images are preloaded through the same cache.
- Loaded text is kept in a least-recently-used cache capped at 32 MB.
- Every load carries the id it was started for. A result for a file that is no longer `current` or a neighbour is dropped.
- Both caches remember each file's size and modification time: after a refresh a changed file is read again and an unchanged one keeps its decoded pixels, its text and its scroll position.

### Presentation

- Starts from the stage with Space or Enter when the stage shows a file.
- Sets `presenting`, hides the explorer and the position indicator, and makes the window fullscreen if it is not already.
- Leaving restores the split and exits fullscreen only if presenting entered it.
- Movement stops at the first and last gallery file. It does not wrap.

Milestone 4 settled the rest:

- The presentation is the same stage entity, laid out alone: its file, caches, loaded text and scroll positions carry across both ways. The root view owns the split's sizes, so the explorer comes back at the width it had. `current` is unchanged by entering or leaving.
- Background is the stage surface. An image uses the whole window, scaled down to fit and never enlarged; Markdown and code look as on the stage. Nothing else is drawn: no explorer, divider or position. The pointer stays visible.
- A file outside the gallery is presented alone when the gallery is empty; with a gallery, → and ← go to the nearest gallery file as on the stage.
- Keys the Presentation column leaves blank (Enter, ⌘F, ⌘O, Tab) do nothing while presenting: they are bound to `NoAction` there. ⌘O is ignored rather than leaving first, so a stray key never ends a presentation. ⌘Q quits.
- Leaving fullscreen through the window (the green button or the system gesture) keeps presenting, windowed; Esc then leaves without touching fullscreen.
- A refresh that removes the presented file moves to the nearest gallery file, as on the stage. If nothing is left to show, the presentation ends as if Esc were pressed.
- The window may be resized while presenting; the stage follows it, and the split comes back at its width once the window has its old size (the split keeps its proportions as on any resize).

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

Milestone 5 adds the menu bar's items (named in Launch), the About alert ("Strikebrain Viewer", "Version N", "OK"), and the Refresh tooltip while the folder's changes are not followed: "Refresh. Changes on disk are not followed: " and the error.

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
| M3 Stage | Image, Markdown and code viewers, left and right, caches, preloading. | Flicking through 1,000 mixed files stays under a fixed memory ceiling recorded in the test. Done: the ceiling is 64 MB of growth over 989 steps; measured 5 to 6 MB (`tests/ui/flick.rs`). |
| M4 Presentation | Fullscreen, keys, leave. | Entering and leaving keeps the current file and the window's prior fullscreen state. Done: `tests/ui/presentation.rs`. |
| M5 Finish | Saved state, live refresh, a packaged `.app`. | Quitting and reopening returns the same view. Done: `tests/ui/persist.rs` saves a real folder's view as the app runs and quits, launches a second app over the same state file, and compares every piece. |

Milestone 0 detail:

- The Kit's agent skills install with `npx skills add longbridge/gpui-kit`. That downloads third-party agent instructions into the repo, so Sean runs it or approves it at the time.

Milestone 5 detail:

- State is a JSON file in the platform config directory: root path, expanded paths, selected paths, query, kind, sort, first visible row, split width, window bounds. Paths are stored relative to the root.
- Live refresh uses a recursive `notify` watcher with a 150 ms debounce. Any batch of events triggers a full background rescan and a tree swap that carries state across by path.

As built:

- **File.** `~/Library/Application Support/ai.brainstrike.viewer/state.json` on macOS (`directories`' config folder for the bundle identifier). `STRIKEBRAIN_VIEWER_STATE=<file>` points a run elsewhere. Fields: `version` (1), `root` (absolute, canonical), `show_ignored`, `expanded`, `selected` (`folders` and `files`), `current`, `query`, `kind`, `sort` (`key`, `direction`), `first_visible_row`, `explorer_width`, `window` (`state` windowed, maximized or fullscreen; `x`, `y`, `width`, `height` relative to its display; the display's UUID). A missing, unreadable, malformed or other-version file is ignored and the app starts fresh; it is replaced at the next save. Unusable values (a relative root, a non-positive size) are dropped.
- **Checks, compactly.** A folder whose every file is checked is one entry under `folders` (the root is `""`); other checked files are listed under `files`. Select all over any tree saves as one entry. Restoring a folder entry checks every file in it at that time, including files added since. Measured (release, M4 Max): 100,000 checked files with no folder fully checked save as 3.96 MB in 22 ms on the background executor (names 12 ms, encoding 5 ms, writing 5 ms) and read back in 4 ms; all 100,000 checked save as 402 bytes. The UI thread copies two bit sets, under a microsecond at this size.
- **First visible row** is stored by path, not index, so it survives files added above it. Restoring puts that row at the top of the tree and gives it keyboard focus. Under a Size sort it waits for sizes, which reorder the rows.
- **When it saves.** Half a second after a change, at most once per half second (a change while a save waits rides along), and when the app quits. Closing the window quits, so that save runs with the window still there. A save copies the state on the UI thread and names, encodes and writes it on the background executor; saves land in the order they were taken, an unchanged state is not written again, and the file is written to a temporary name and renamed over the old one. Scrolling alone does not schedule a save; the next save or the quit records it.
- **Restoring** happens in two steps: the filter and Show ignored apply before the first scan, so the first tree is already right; sort, open folders, checks, `current` and the top row apply when the scan lands. Window bounds go to the window when it opens, moved and shrunk onto the saved display's visible area, or the primary display's when that display is gone. A fullscreen the presentation made is saved as the window it came from.
- **Live refresh.** The watcher (`notify`'s FSEvents backend; creations, removals and modifications, no reads) runs on its own thread and feeds a channel. A GPUI task waits for a change, gathers more for 150 ms on GPUI's timer, then asks `explorer::changes_matter`, off the UI thread, whether a new scan would show any of them: a path the tree lists matters; otherwise the first part of the path the tree lacks matters only if it still exists and the scan's own walker, with the same ignore rules, would list it. Changes inside `.git`, to `.DS_Store`, under ignored folders such as `target/` and `node_modules/`, and under the app's own state folder never rescan. Measured: 400 files written under an ignored `target/` and a log ignored at the top, over a second, caused no rescan; one new file in `src/` caused exactly one. A change while a scan runs asks for one more scan when it lands, never more. A watcher error, or a watcher that lost track with no path to name, rescans once. The watcher is replaced when the folder changes and dropped with the state. One that cannot start (the folder is missing, a watch limit) leaves the folder unfollowed and turns Refresh the warning colour, its tooltip giving the reason.
- **Bundle.** `just bundle` builds the release profile (thin LTO, one codegen unit, debug info stripped, symbol names kept, panics unwind) and assembles `target/release/bundle/osx/Strikebrain Viewer.app` by hand, as the Kit's packaging guide does: the Info.plist template in `crates/viewer/bundle` with the version from Cargo, an `.icns` made with `sips` and `iconutil` from the 1024 px `icon.png` that `just icon` draws, and an ad-hoc signature. Measured on 2026-11-23: binary 26.1 MB (21.4 MB with symbol names stripped as well), bundle 25 MB; launching the bundled binary to its first on-screen window, 0.67 s the first time after a build and 0.20 s after.

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
- **Large photos.** GPUI decodes at full resolution. Measured in milestone 3 (release, M4 Max), an 8000 x 6000 picture decodes in 50 ms as a smooth PNG and 0.59 s as a grainy 44 MB JPEG, on the background executor, and holds 183 MB of pixels, plus as much again as a GPU texture while it is drawn; from the cache it draws in 3 to 5 ms. The 8-image cache can therefore hold some 1.5 GB of such photos, or about 740 MB of 24-megapixel ones. Not fixed in milestone 3. The fix is still a loader that decodes at display size, which needs the `image` crate as a runtime dependency and the GIF, WebP and SVG paths GPUI's loader already has; a byte budget on the cache next to its count is the smaller step. Probe: `timing_a_large_photo`.
- **Editor on large files.** The Kit's 0.10.0 source documents the editor as handling about 50,000 lines. Measured in milestone 3 (release, M4 Max, 27-byte lines, test build): 0.5 MB reaches the screen 13 ms after the click and scrolls at 0.45 ms a frame; 5.5 MB (200,000 lines) 50 ms and 1.1 ms; 10 MB (360,000 lines) 0.1 s and 2.6 ms; one 4.8 MB line 85 ms and 0.5 ms. Highlighting arrives later from the background executor: 0.85 s, 7 s and 13 s. So the text limit stays at 10 MB. Two costs remain. The Kit's syntect backend parses a document of up to 64 KB at once on the UI thread: the limit is a private constant (`SYNC_PARSE_MAX_BYTES`), the highlighter is not `Send`, and nothing public runs it elsewhere. Until milestone 4 every such file was parsed twice, because setting the editor's text with a language set parses it and the next frame parses it again; the stage now sets the text as plain text and then the language, so it is parsed once, in the first frame, at about 1.2 ms a KB. Measured in milestone 4 (release, M4 Max, syntax set loaded), click to first frame before and after: 8 KB 24 → 15 ms, 27 KB 71 → 37 ms, 62 KB 155 → 80 ms; a 68 KB file still shows in 5 ms and is highlighted 82 ms later in the background. The first file highlighted also loads syntect's syntax set, about 20 ms. So → through files of 30 to 64 KB still hitches by 40 to 80 ms a file. Removing that means a highlighter of our own, given to the editor through its public `set_highlighter_factory` and parsing on the background executor, with `syntect` (already built, through the Kit) as a direct dependency. And the editor copies its whole text into an accessibility value on every frame while an assistive technology is connected (always, in test builds): at 10 MB a test build grows by about 10 MB a frame until the allocator settles near 870 MB, which a release build without a screen reader does not do. Probe: `timing_the_code_view_on_large_files`.
- **Markdown on large files.** The text view lays out only the blocks on screen: 0.25 ms a frame at 4 KB to 345 KB. It measures every block once when a document opens: 28 ms at 4 KB, 41 ms at 86 KB, 0.21 s at 345 KB. Opening a document whose text shares no prefix with the one before costs more, growing with the square of the size (release, M4 Max, click to first frame, two kinds of generated document): 0.07 to 0.12 s at 128 KB, 0.18 to 0.21 s at 256 KB, 0.32 to 0.34 s at 384 KB, 0.47 to 0.53 s at 512 KB, 0.93 to 0.97 s at 768 KB, 1.5 to 1.6 s at 1 MB, 5.1 to 5.3 s at 2 MB. So Markdown over 256 KB, the largest size measured under 0.3 s, opens in the code view: 10 ms at 256 to 512 KB, 17 ms at 1 MB, 29 ms at 2 MB. Probes: `timing_the_markdown_view_on_large_documents`, `timing_markdown_around_its_size_limit`.
- **Small scroll steps far down a very long list.** GPUI keeps a list's scroll offset in an `f32`: `uniform_list`, the Kit's `VirtualList`, and `list()`, whose `ListState` anchors on a row but adds each wheel delta to an absolute `f32` top. At 29 px a row, a delta under half the `f32` step is lost: from row 289,262 (2^23 px, step 1 px) sub-pixel deltas go; from row 578,525 (2^24 px, step 2 px) a 1 px delta goes and 3 px moves 4; from row 1,157,049 (2^25 px, step 4 px) everything under 2 px goes. Measured on the 1,650,231-row reference corpus as one flat list (M2, release, M4 Max): at the middle, 0.5 and 1 px moved nothing and 3 px moved 4; at the end, 0.5 and 1 px moved nothing, 3 px moved 4 and 10 px moved 8. Frame cost is not the problem: a scrolling frame took 0.66 to 0.69 ms median and at most 1.1 ms at the top, the middle and the end (frame build without GPU raster). So past about row 578,000 a slow trackpad scroll sticks while flicks, the wheel, the scrollbar and the keys still move. The fix is scroll state of our own (a row plus an offset inside it, or `f64`) with our own wheel handling and a scrollbar adapter, not a framework list; it is left out of M2. Probe: `cargo test --release -p viewer --test ui -- --ignored --nocapture timing_scrolling`.
- **Cold build time** is unknown until milestone 0.
- **Live refresh on very large folders.** Each change a scan would show rescans the whole folder, as decided. With Show ignored on, a busy folder the tree then shows (a build's `target/`) is rescanned back to back while it changes. On Linux, `notify` uses inotify, which watches every folder separately; a 1.65 M-entry tree would exceed default watch limits. Neither is handled. A change to `.git/info/exclude` or the global excludes does not rescan.

## References

- GPUI: https://gpui.rs/ and https://github.com/zed-industries/zed/tree/main/crates/gpui
- GPUI Kit docs index: https://gpui-kit.com/llms.txt. Any page is Markdown with `.md` appended.
- GPUI Kit repository and agent skills: https://github.com/longbridge/gpui-kit
- Brainstrike explorer: `packages/file-explorer`, `file-explorer-ui`, `file-explorer-hooks`, `file-tree-worker`, `file-system`, and `packages/ui/src/tree-table` in `/Users/seandearnaley/Documents/GitHub/brainstrike`
