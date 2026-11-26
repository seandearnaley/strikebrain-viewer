# Strikebrain Viewer design

Status: design approved by Sean on 2026-11-22. Milestones 0 to 5 are built (2026-11-23), with the final review's fixes, milestone 6, several folders, and milestone 7, the gallery (2026-11-23); where the build settled a question the design left open, the section below says so.
Companion page: https://claude.ai/artifact/4PaAcuEDiMKYJKyEFCpkD3

## Purpose

A native desktop viewer for text and image files.

- Left pane: brainstrike's file explorer, ported to Rust, holding any number of folders. Filter the tree, check files.
- Right pane: a stage. Two or more checked files show as a gallery, a grid of tiles; one file opened from it, or from the tree, fills the pane, and arrow keys move through the checked set.
- Space on the stage starts a fullscreen presentation.

Goals, in order: speed at million-file scale, a small codebase, and code Sean can learn Rust and GPUI from.

Platform: macOS on Apple silicon. GPUI also targets Windows and Linux. Nothing here should prevent them; neither is tested in version one.

## Settled decisions

1. **GPUI through GPUI Kit.** `gpui-kit = "=0.10.0"`. The explorer tree is built directly on GPUI's `uniform_list`, not the Kit's `Tree`.
2. **The stage has two views.** The gallery, a grid of the checked files, and the file view, where one file fills the right pane and ← and → move through the checked files. The checks decide which shows; see [Stage](#stage) and [Gallery](#gallery).
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
| `image` 0.25 | | Milestone 7: the gallery's thumbnails, decoded and scaled down to the tile; a GPUI image is made of its frames. GPUI already builds it with the same formats; it was a dev-dependency before. |
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
├─ rust-toolchain.toml, rustfmt.toml, clippy.toml, deny.toml, _typos.toml, bacon.toml
├─ justfile, .config/nextest.toml, .githooks/
├─ AGENTS.md, README.md
├─ docs/superpowers/specs/
├─ brand/               build.sh and the production pack it writes: icons and identity
└─ crates/
   ├─ explorer/           library, no GPUI
   │  ├─ src/lib.rs, content.rs, bits.rs, parallel.rs, remap.rs, rows.rs
   │  ├─ src/tree.rs, tree/{build,join,names,paths,tests}.rs   join.rs: several scanned folders as one tree
   │  ├─ src/scan.rs, scan/{sizes,changes,tests}.rs   changes.rs: which changes on disk a rescan would show
   │  ├─ src/selection.rs, selection/tests.rs
   │  ├─ src/filter.rs, filter/tests.rs, src/sort.rs, sort/tests.rs
   │  ├─ src/explorer.rs, explorer/{filtering,folders,gallery,row,saved,tests}.rs, explorer/tests/{props,folders}.rs
   │  ├─ tests/scale.rs
   │  └─ benches/engine.rs
   └─ viewer/             binary `strikebrain-viewer`
      ├─ src/main.rs, lib.rs, app.rs, launch.rs, menus.rs, keys.rs, theme.rs, assets.rs
      ├─ src/explorer_panel.rs, explorer_panel/{toolbar,filter_bar,header,selection_bar}.rs, row.rs, columns.rs
      ├─ src/stage.rs, stage/{loading,render,load,text,images,cache,gallery}.rs
      ├─ src/stage/gallery/{grid,read,thumbs,tile}.rs   the gallery's layout, reading, cache and tiles
      ├─ src/state.rs, state/{folders,rows,checks,gallery,scan,filter,restore,watch}.rs
      ├─ src/persist.rs, persist/{format,tests}.rs
      ├─ bundle/          Info.plist template, desktop entry and Windows resources
      └─ tests/ui/main.rs and one module per area, tests/shots.rs, tests/fixture/
```

`explorer` never depends on `gpui-kit`. Its tests must compile and run without building the UI stack.

Workspace settings:

- `resolver = "3"`.
- Lints: `unsafe_code = "forbid"`; clippy `all` and `pedantic` at warn.
- Dev profile: `debug = "limited"`, `split-debuginfo = "unpacked"`, and `opt-level = 3` for all dependencies via `[profile.dev.package."*"]`, because the Kit guide's per-crate list omits `gpui-base` and `gpui-pre-macos`.

## Engine: crate `explorer`

### Tree

A scan produces one immutable `Tree`, shared as `Arc<Tree>`. Nodes are numbered in depth-first pre-order. `NodeId` wraps a `u32`. The root folder is id 0.

The explorer shows a joined tree: the scanned folders as its top-level folders, in the order they were added, under a root of its own that is no folder on disk and never a row. Each folder's nodes keep their order and come one folder after another, so ids stay pre-order and a subtree stays one id range; selection, filter, sort and rows work on it unchanged. Joining copies each column in bulk with its ids shifted and sorts nothing again. The tree keeps, for each scanned folder, its top node, its absolute path and its sizes; `abs_path`, `find_abs` and `folder_of` answer by folder. A `Tree::from_paths` fixture is a tree of one folder whose path is empty.

Parallel arrays, one entry per node:

- `names`: a `NameArena`, one `String` buffer plus one offset per node.
- `haystack`: the lowercased names in one buffer, separated by `\0`, for the filter.
- `parent: Vec<NodeId>`
- `end: Vec<u32>`: the subtree of `id` is the id range `id..end[id]`.
- `depth: Vec<u16>`
- `kind: Vec<Kind>`: `Dir`, `File`, `Symlink` or `Unreadable`.
- `size`: a file's byte size; a folder's is the sum of its files. Kept per scanned folder and set once, when that folder's sizes are read, so one folder's sizes can arrive after the others'. A joined folder shares its sizes with the scan it came from, so sizes read into either reach both.
- `ext: Vec<u16>`: index into a small table of interned lowercase extensions.
- `files: FixedBitSet`: 1 where the node is a file or a symlink.
- `children`: each folder's child ids as one slice of a shared `Vec<NodeId>`. This list holds the display order.

Invariants. Each is a property test.

1. `parent[id] < id` for every node except the root.
2. For a file, `end[id] == id + 1`. For a folder, `id..end[id]` contains exactly its descendants and itself.
3. Ids follow canonical order: within a folder, folders first, then natural name order ascending. The one exception is the root of a joined tree, whose children are its folders in the order joined.
4. `size[folder]` equals the sum of `size` over the files in its range.

Ids are stable for the life of one `Tree`. Sorting never changes them. A rescan produces a new `Tree` with new ids; state is carried across by path.

### Scan

- `ignore::WalkBuilder` in parallel mode, off the UI thread.
- Default rules: honour `.gitignore`, `.ignore` and global git excludes; show dotfiles; never follow symlinks; always skip `.git` and `.DS_Store`.
- "Show ignored" rescans with the ignore rules off. `.git` and `.DS_Store` stay skipped.
- File size comes from the walker's metadata call.
- A folder that cannot be read becomes a node of kind `Unreadable`. A symlink is a leaf, whatever it points to.
- Refresh rescans every folder, each on its own, with the current rules.
- Every scan carries a generation number. A result whose generation is not the latest for its folder is dropped.
- No caps on entries per folder, depth or total nodes.

`Tree::from_paths(&[&str])` builds a tree from path strings, so engine tests need no disk. `Tree::unreadable(path)` is a folder that could not be read: one node of kind `Unreadable`, so it keeps its place among the folders.

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
- A re-sort reorders each folder's slice of `children`. Nothing else changes. The top-level folders of a joined tree keep the order they were added in.

### Rows

`rows: Vec<NodeId>` is the flattened visible list. It is rebuilt in full after any change to expansion, visibility or sort, by a depth-first walk over `children` that skips closed folders. The root node itself is not a row; in a joined tree the top-level rows are the folders.

The gallery is every selected file in display order across the whole tree. The filter does not affect it.

### Facade

`Explorer` owns the `Arc<Tree>`, selection, expansion, visibility, sort and rows. It is a plain struct with no async and no UI types. Its methods cover: read rows, read a node's check state, toggle a check, select all, clear, toggle a folder, expand all, collapse all, set sort, apply a filter result, iterate the gallery, replace the tree, and add, rescan or remove one folder. Each of those carries selection and expansion across by path and returns the old-to-new id mapping and where `current` goes.

Several folders (milestone 6): `add_folder` joins a scan as a top-level folder, open, at a given place among the folders; top-level folders inside it leave, their checks and open folders carried into it by path. `replace_folder` puts a new scan of one folder in its place and `remove_folder` takes one out with its checks. The folders that stay move whole: their ids shift and nothing is compared. A rescanned folder, or one taken in, merges with the new scan by name, as a refresh does. A folder whose every file was checked keeps every file checked across its rescan; a folder added after Select all comes in unchecked, since each folder counts on its own. `snapshot().folders()` names each folder's open folders and checks relative to it, and `restore_folder` brings one folder's back without touching the others.

### Content

`content.rs` holds pure functions with no I/O of their own:

- `viewer_for(name, head) -> Viewer` returns `Image`, `Markdown`, `Code` or `Unsupported`. `head` is the first 8 KB of the file.
- A NUL byte in `head` means `Unsupported`, unless the extension is an image extension.
- `md` and `markdown` are `Markdown`. Other text is `Code`.

## App: crate `viewer`

### State

One entity, `AppState`, owns:

- the folders asked for, in the order added: each with its canonical path, its latest scan, its watcher, why it could not be read, and its part of a saved view waiting for its scan
- `explorer: Explorer`, over the folders on screen
- `current: Option<NodeId>`: the file on the stage
- `view`: which of the stage's views shows, the gallery or the file view
- `tile: Option<NodeId>`: the tile with keyboard focus in the gallery
- `focus_row: usize`: the keyboard row in the tree
- `presenting: bool`
- a generation per folder's scan, and the latest task of each scan and of the filter

Two views observe it: `ExplorerPanel` and `Stage`. The root view lays them out in the Kit's horizontal resizable split. The explorer starts 300 px wide with a 220 px minimum.

Rules for `current` and the view:

- Enter or a click on a file row sets it to that file, checked or not, and shows the file view.
- Checking a single file sets it to that file.
- When it is `None` and the gallery becomes non-empty, it becomes the first gallery file.
- Unchecking the current file moves it to the next gallery file, else the previous, else `None`.
- After any change the person makes to the checks (a checkbox, Select all, Clear): with two or more files checked the gallery shows; with one, the file view shows that file; with none, the stage is blank and `current` is `None`. The gallery's focus goes to the tile of `current` (or the checked file nearest it) when the gallery was not showing or the change moved `current`; otherwise it stays where it was.
- A click or Enter on a tile sets `current` to its file and shows the file view. Esc there, or the grid button, shows the gallery again while two or more files are checked, its focus on `current`'s tile (or the checked file nearest it).
- A rescan or a folder removed that leaves fewer than two checked files gives the gallery way to the file view. A focused tile whose file went, unchecked or gone from disk, gives its focus to the file now at its place in the gallery, as the tree's focus row does.
- Next and previous move within the gallery. When `current` is not in the gallery, next goes to the first gallery file after it in display order and previous to the last one before it. With an empty gallery they do nothing.

### Launch

- `strikebrain-viewer <folder>...` opens exactly those folders, in that order, each taking its saved view if it was saved (compared after canonicalizing). The filter, the sort, Show ignored, the file on the stage, the stage's view and the top row come back with any saved folder that opens; folders never saved open fresh.
- With no argument it reopens the saved folders with their views; a saved folder that no longer exists is dropped. With none left, or no usable saved state, it shows the empty state.
- The window's bounds and the explorer's width are restored either way.
- Add Folder, one action, adds folders beside those open and never replaces them: ⌘O, the toolbar's button, File → Add Folder…, the empty state's button, a folder given by the system ("Open With", or dropped on the Dock icon; the bundle declares `public.folder`, and it arrives as a `file://` URL), and every folder on the command line. The system picker allows several folders at once.
- A file opens too: one named on the command line, sent by "Open With" or dropped on the Dock icon. Its folder is added by the rules for a folder, so a folder already open, or one whose tree lists the file, is not added again. Once a tree lists the file it shows in the file view, unchecked, and its row is the focus row, with the folders above it open. Files opened together (several on the command line, or several sent by the system at once) are checked as well, so they show as the gallery beside any files checked before, with the last one focused. A file on the command line stands for its folder: that folder takes its saved view, and the file asked for replaces the saved file on the stage. The bundle declares, beside `public.folder` and always as an alternate viewer, so the app is offered and never made the default: text (`public.text`, and `public.data` for files with no extension, such as a `justfile`), the image formats the stage decodes, by their type identifiers, and by extension the source formats macOS has no text type for (`rs`, `ts`, `go` and the like). macOS does not offer an app for a file on a catch-all claim (`public.data`, or the `*` extension), so the list is explicit; a file of a type not listed still opens from the command line or dropped on the Dock icon. On Windows and Linux "Open With" passes the file as an argument, which the command line covers; on Linux the packages install a desktop entry that offers the app for folders, text and images; the Windows installer adds a Start menu entry and registers no file types.
- A file is never in the tree twice, and that is the whole rule (see Milestone 6, Overlap). A folder holds what its tree on screen lists, nothing more: adding a folder already open, or one an open folder's tree lists, scans nothing; the tree opens to its row and makes it the focus row, scrolled into view, and keyboard focus stays in the pane that has it. A folder inside an open folder that its tree does not list (its ignore rules hide it, or it could not be read) is added as a folder of its own. One added inside a folder whose first scan runs waits for that scan, then is shown or added as above. Whenever a folder's scan lands, readable, it takes in every other open folder it lists, with its checks and open folders, and leaves the rest as they are; a first scan of a folder an open folder already lists is dropped.
- Each folder scans on its own and lands in its place among the folders, whichever scan lands first. A launch settles once every folder is on screen: at the saved top row, or at the first row, with focus there. A folder added while the app runs becomes the focus row, scrolled into view. A rescan, a folder taken in or a folder removed keeps the tree where it was: the row at the top stays there, to the pixel, and a focus row that was on screen stays on screen.
- The window title is the folder's name with one folder open, the application's name otherwise.
- A folder or a file dropped on the window opens as one dropped on the Dock icon does; nothing is dropped while presenting.
- `--version` and `--help` print and exit. A release build on Windows opens no console window, and carries the app's icon, which the window and the taskbar show, and its version details.
- The window is never narrower than the two panes at their narrowest, nor shorter than 240 px. On Linux its application id is the bundle identifier, which a desktop entry is matched by.
- The menu bar: the app menu (About, Services, Hide, Hide Others, Show All, Quit), File (Add Folder…, Reveal in Finder, Copy Path, Close Window), Edit (Undo, Redo, Cut, Copy, Paste, Select All, for the filter input and the code view), View (Present, Show Ignored with a check, Refresh; macOS adds Enter Full Screen itself) and Window (Minimize, Zoom). Reveal in Finder and Copy Path act on the file on the stage while the stage holds focus, and on the focus row, file or folder, otherwise. Close Window quits, as the close button does. Present has its own unbound action, because a menu item shows its action's first key as a shortcut and `Present` is Space, which the menu would then take from the filter input. Add Folder is disabled while presenting, as ⌘O is ignored there.

### Explorer panel

Top to bottom:

1. **Toolbar**, 36 px. Left: Expand all, Collapse all. Right: Add Folder, Show ignored, Refresh, Select all (which becomes Clear when anything is selected). Icon buttons, 24 px, each with a tooltip and an accessible label.
2. **Filter input**, 32 px, with a search icon and a clear button.
3. **Kind control**, three equal segments: All, Images, Text.
4. **Column header**, 28 px, sticky. `Name` always. `Size` when the tree is at least 300 px wide. `Type` when it is at least 420 px wide. Clicking a header sorts by it; clicking again flips direction.
5. **Tree**, one `uniform_list` over `rows`.
6. **Selection bar**, floating at the bottom of the tree when anything is selected: "N selected" and a clear button. It never moves rows; the list gains bottom padding while it shows.

Row, left to right. These are brainstrike's measurements, except the indent.

- Height 28 px plus a 1 px bottom rule.
- Indent 12 px per level. Brainstrike's 20 px left a name five levels down about 50 px of a 300 px tree.
- Chevron in a 24 px target, folders only. Files get a 24 px spacer.
- 6 px gap, 16 px checkbox, 6 px gap, 16 px icon, 6 px gap, name at 14/16 px, truncated. A file's icon says its kind, quietly in the muted colour: a picture, Markdown, other text or code, or anything else (a known binary format), as the Images and Text kinds sort them.
- `Size` cell: monospace 12 px, tabular figures, one decimal, units B, KB, MB, GB.

Each folder is a top-level row, always, even when it is the only one: a folder row like any other (chevron, checkbox, icon, name, size), named by its last path component, open when added, at the left edge; its contents sit one 12 px step in. Its tooltip, also its accessible description, is its full path, which tells two folders of one name apart. While hovered, or focused with the focus ring showing, it shows a remove button in place of its size: a 24 px ghost icon button, muted like the chevrons, at the Size cell's right edge (at the row's right edge when the Size column is hidden), tooltip and accessible label "Remove Folder". ⌘⌫ on a focused folder row does the same; keyboard focus moves to the row that takes its place.

Row behaviour:

- Clicking a checkbox toggles it.
- Clicking a chevron opens or closes the folder. Clicking the rest of a folder row only moves focus.
- Clicking a file row shows it on the stage.
- The row for `current` has the accent background.
- Hover uses the muted background at half strength.
- The keyboard focus ring appears only after a key press and clears on pointer down.
- An `Unreadable` folder shows a warning icon where its chevron would be.

States:

- No folder: one "Add Folder" button centred in the pane. The stage is blank. Removing the last folder comes back here.
- Any folder scanning for more than 200 ms: a thin indeterminate bar under the toolbar.
- No rows after filtering: the tree area is blank. A folder with no match hides like any other folder row.
- A folder that cannot be read: its row, with the warning icon where the chevron would be; its tooltip gives its path and, on a second line, the operating system's error message. It keeps the checks and open folders it had, saved as they were, and a scan that can read it again (Refresh, or the next launch) brings them back.

Not in version one: column resizing, a column menu, context menus, type-ahead, reordering folders.

### Stage

The stage has two views: the gallery (next section), and the file view, which shows `current` filling the pane and is described here. Which shows follows the rules for `current` above. While the gallery shows the file view rests: nothing is read for it, and a read in flight is dropped. Back in the file view, a file other than the one it last showed starts blank until its read lands, so the old file never shows meanwhile.

- `Image`: GPUI's `img()` from the file path, scaled down to fit and never enlarged.
- `Markdown`: the Kit's `TextView`, scrollable, in a centred column 720 px wide at most.
- `Code`: the Kit's editor, read-only, with line numbers and `syntect` highlighting chosen by extension. Plain prose (`.txt`, `.text`, `.log`, and names with no extension such as `LICENSE`, but not `Makefile`, `Dockerfile` and the other build files highlighted by name, nor `CMakeLists.txt`) is soft-wrapped with no line numbers.
- `Unsupported`: the file's name and size, centred and muted.
- When `current` is in the gallery, a position such as "2 / 3" sits at the bottom centre, in a 32 px bar.
- While two or more files are checked, a grid button sits in that bar just left of the position (the position stays centred; it sits there alone when `current` is not checked): a 24 px ghost icon button, muted, tooltip and accessible label "Gallery". It shows the gallery, keyboard focus on the stage.
- A blank stage draws nothing.

The stage reads the first 8 KB on the background executor and asks `viewer_for`. Only regular files are opened: a symlink to a FIFO or a device is `Unsupported`. A picture GPUI cannot decode shows as `Unsupported`. Until a read lands the stage keeps what it showed.

Limits and caches:

- Text files over 10 MB are `Unsupported`. Milestone 3 measured the editor and kept 10 MB: see Risks.
- Markdown files over 256 KB open in the code view, as their source: the Markdown view's first frame grows with the square of the document's size (see Risks).
- Invalid UTF-8 is decoded lossily.
- Decoded images live in a bounded least-recently-used `ImageCache` holding 8 images and 512 MB of decoded pixels, every frame of an animation counted. An image's size is counted once it is decoded; making room evicts the images used longest ago, never the one on the stage, which stays whatever its size. Evicted images release their GPU texture.
- Pictures over 64 MB on disk, and GIFs or animated WebP pictures over 16 MB, are `Unsupported`: GPUI decodes a picture whole and every frame of an animation up front.
- The next and previous gallery images are preloaded through the same cache, when they are at most 8 MB on disk.
- Loaded text is kept in a least-recently-used cache capped at 32 MB.
- Every load carries the id it was started for. A result for a file that is no longer `current` or a neighbour is dropped.
- Both caches remember each file's size and modification time: after a refresh a changed file is read again and an unchanged one keeps its decoded pixels, its text and its scroll position.

### Gallery

The checked files, in gallery order, as a grid of tiles. Built in milestone 7.

- **Layout.** Columns fill the width: tiles at least 180 px wide, 4:3, 16 px apart and 16 px in from the pane's edges, in whole pixels (what is left stays at the right edge). Under each tile its name, 8 px below, on a 16 px line, truncated. A pane narrower than a tile still has one column.
- **Virtualized.** One GPUI `uniform_list` of rows of tiles, inside a `container_query` that hands it the pane's size before the rows are built: the columns, the tile size and the row height follow from the width, so the list builds only the rows on screen, whatever the number of files. Each frame asks the engine for the files from its first row's first tile on (`Explorer::gallery_from`).
- **A tile.** A picture is fitted inside the tile, 8 px in, scaled down and never up. A text, code or Markdown file shows its first lines as plain text, the monospace font at 9 px on 12 px lines, in the foreground colour, clipped to the tile at its last whole line, with no highlighting; tabs are four spaces. A file the file view cannot show (a binary file, one over the size limits, a picture that cannot be decoded) is its kind's icon, as in the tree, 32 px, muted. A tile shows nothing until its content is read: no spinner. The tile of `current` has the accent background and its name in the foreground colour; others are muted, a little darker on hover.
- **Loading.** Tiles are read on the background executor, nearest the viewport first: the rows drawn, then the row below, then the row above. At most four reads run at once: a read keeps its slot until it ends. A read for a tile that leaves those rows, or for any tile while the gallery is hidden, is told to stop, and stops before it reads a picture whole and before it decodes it; a decode already under way runs to its end, in its slot. A read follows the file view's checks (only regular files, opened without waiting, the engine's `viewer_for`, the size limits, checked again on the bytes read before any decode). A picture is decoded whole by the `image` crate, as GPUI's loader decodes it, from one buffer the file's size, scaled down to fit the tile at the display's scale, then turned as its EXIF orientation says (a quarter turn swaps the box first), so a turned photo is never copied at full size; an animation keeps its first frame. An SVG is drawn by GPUI's renderer at its own size, or at the largest size inside the tile when its own is larger; a first drawing at a sixteenth of its size tells which. A text keeps its first 48 lines of at most 160 columns.
- **Cache.** Tiles' content lives in its own least-recently-used cache of 128 MB, apart from the file view's: a picture costs its pixels, a text its bytes, each entry 256 bytes more. An evicted picture's GPU texture is released. Each entry keeps its file's stamp: after a rescan a tile shows what it has while its file is checked again, and is read again only if it changed. A thumbnail made for a smaller tile is read again; one made for a larger tile serves a smaller one.
- **Focus.** The stage holds keyboard focus and declares the `Gallery` key context while the gallery shows. The focused tile is kept by node, as the tree's focus row is, and is always a checked file; it shows the focus ring, 3 px outside the tile, after a key press (a tile key, Tab or shift-Tab into the stage, Esc back from the file view or out of a presentation), until a pointer press, as the tree's does. The gallery keeps the focused tile on screen whenever it moves, and when it comes back from the file view or a presentation.
- **Resizing.** The tiles fill the width, so a resize changes their height, and the list keeps its scroll in pixels. When the tiles' size or the columns change, the scroll moves so the focused tile, if it was on screen, stands where it stood; otherwise the first tile on screen keeps its place, the same share of its row above the top.
- **Opening.** A click on a tile, or Enter on the focused tile, opens its file in the file view, with keyboard focus on the stage.

### Presentation

- Starts from the file view with Space or Enter when the stage shows a file, and from the gallery with Space, at its focused tile (which becomes `current`).
- Sets `presenting`, hides the explorer and the position indicator, and makes the window fullscreen if it is not already.
- Leaving restores the split, and the view the presentation started from, and exits fullscreen only if presenting entered it. Back in the gallery, its focus is on the file presented last, scrolled into view.
- Movement stops at the first and last gallery file. It does not wrap.

Milestone 4 settled the rest:

- The presentation is the same stage entity, laid out alone: its file, caches, loaded text and scroll positions carry across both ways. The root view owns the split's sizes, so the explorer comes back at the width it had. `current` is unchanged by leaving, and by entering from the file view; entering from the gallery makes it the focused tile's file.
- An image is presented on black in both appearances, as a photo viewer does, using the whole window, scaled down to fit and never enlarged; Markdown and code keep the stage surface and look as on the stage. In the split the stage keeps its surface for images too. Nothing else is drawn: no explorer, divider or position. The pointer stays visible.
- A file outside the gallery is presented alone when the gallery is empty; with a gallery, → and ← go to the nearest gallery file as on the stage.
- Keys the Presentation column leaves blank (Enter, ⌘F, ⌘O, Tab) do nothing while presenting: they are bound to `NoAction` there. ⌘O is ignored rather than leaving first, so a stray key never ends a presentation. ⌘Q quits.
- Leaving fullscreen through the window (the green button or the system gesture) keeps presenting, windowed; Esc then leaves without touching fullscreen. A fullscreen the person makes again meanwhile is theirs: Esc leaves it as it is.
- A refresh that removes the presented file moves to the nearest gallery file, as on the stage. If nothing is left to show, the presentation ends as if Esc were pressed.
- The window may be resized while presenting; the stage follows it, and the split comes back at its width once the window has its old size (the split keeps its proportions as on any resize).

### Keys

Bindings are GPUI actions bound per key context: `Explorer`, `Filter`, `Gallery`, `Stage` (the file view), `Presentation`.

| Key | Explorer | Gallery | Stage | Presentation |
| --- | --- | --- | --- | --- |
| ↑ ↓ | Move focus | Focus the tile a row up or down; ↓ onto a shorter last row, its last tile | Scroll text | Scroll text |
| → | Open folder, then first child | Next tile, onto the next row from a row's end | Next file | Next file |
| ← | Close folder, then parent | Previous tile, onto the row above from a row's start | Previous file | Previous file |
| Space | Toggle checkbox | Present from the focused tile | Present | Next file |
| Enter | Show file on stage; nothing on a folder | Open the focused tile | Present | |
| Esc | Clear the filter | Focus the explorer | Back to the gallery with two or more files checked; otherwise focus the explorer | Leave |
| Home, End | Jump | First tile, last tile | Jump within text | Jump within text |
| Page Up, Page Down | Jump | Move focus by a screen of rows | Jump within text | Jump within text |
| ⌘F | Focus the filter | Focus the filter | Focus the filter | |
| ⌘O | Add folders | Add folders | Add folders | |
| ⌘⌫ | Remove the focused top-level folder | | | |
| ⌘R | Refresh | Refresh | Refresh | Refresh |
| ⇧⌘. | Show or hide ignored files | The same | The same | The same |
| ⇧⌘R | Reveal the focus row in Finder | Reveal the file | Reveal the file | Reveal the file |
| ⌥⌘C | Copy the focus row's path | Copy the file's path | Copy the file's path | Copy the file's path |
| ⌃⌘F | Fullscreen, or back | The same | The same | The same |
| ⌘W, ⌘Q | Quit | Quit | Quit | Quit |
| Tab | Next pane | Next pane | Next pane | |

In the filter input, Esc clears a non-empty query and otherwise moves focus to the tree. Tab order is filter, tree, stage.

⌘ is the Mac's; on Windows and Linux the same keys take Ctrl, and fullscreen is F11. ⌘M (Minimize), ⌘H (Hide) and ⌥⌘H (Hide Others) are bound on macOS only, where they are the convention.

Differences from brainstrike: Enter and file clicks show the file on the stage instead of opening a tab; checking a file also shows it; the first arrow press moves focus immediately; Page Up, Page Down, ⌘F, ⌘O and ⌘⌫ are new.

### Theme

Brainstrike's tokens, applied through the Kit's theme system, following the system appearance. Light / dark:

- background `hsl(30 25% 99%)` / `hsl(30 8% 9%)`
- primary `hsl(215 55% 45%)` / `hsl(215 50% 58%)`
- muted `hsl(30 15% 94%)` / `hsl(30 5% 18%)`
- accent `hsl(30 12% 90%)` / `hsl(30 5% 22%)`
- border `hsl(30 10% 90%)` / `hsl(30 5% 20%)`
- info, used for folder icons, `hsl(215 50% 38%)` / `hsl(215 45% 74%)`

Checked and mixed checkboxes use primary with a white check or dash. Unchecked boxes use the background with a primary border. A primary button ("Add Folder") carries white text at 4.5:1 or more: dark mode's primary is too light for that (3.7:1), so dark buttons take light mode's blue, `hsl(215 55% 45%)`, darker on hover and press.

### Accessibility

The tree and its rows carry AccessKit roles, labels and checked state. The gallery is a grid (accessible name "Gallery") of rows of grid cells, each labelled with its file's name, selected when it is `current`, with its row and column; the grid gives its row and column counts as last laid out; a picture tile holds an image of the same name, a text tile's text is the value of a label. Tests and agents find rows and tiles by the same labels a screen reader uses. Every icon button has a label.

### Copy

Only these strings appear in the interface: "Filter" as the input placeholder, "All", "Images", "Text", "Name", "Size", "Type", "N selected", "Add Folder", tooltips on icon buttons ("Remove Folder" on a folder row's), a folder row's path as its tooltip, the application's name as the window title with other than one folder open, and operating system error messages. No taglines, no helper text.

Milestone 5 adds the menu bar's items (named in Launch), the About alert ("Strikebrain Viewer", "Version N", "OK"), and the Refresh tooltip while a folder's changes are not followed: "Refresh. Changes on disk are not followed: " and the error.

The release pass adds the menu items named in Launch ("Reveal in Finder" is "Show in File Manager" off macOS) and what `--help` prints.

Milestone 7 adds file names under the gallery's tiles and "Gallery": the grid button's tooltip and label, and the grid's accessible name.

## Milestones 6 and 7: several folders and the gallery

Asked for on 2026-11-23, after the first run of the finished app: the window had no way to add or change folders as brainstrike does, and several checked files should show together as a gallery until one is opened. Where this section and an earlier one disagree, this section wins; each milestone rewrites the earlier sections it changes, so the document describes one app.

### Milestone 6: several folders

The explorer holds any number of folders, as brainstrike's workspace does.

- **Tree.** Each folder is a top-level row, always, even when it is the only one. Folders keep the order they were added in; sorting applies inside each. A folder's row is a folder row like any other (chevron, checkbox, icon, name, size), named by the folder's last path component, open when added. Its tooltip, also its accessible description, is its full path, which tells two folders of one name apart.
- **Engine.** One `Tree` still: ids stay depth-first pre-order and a subtree stays one contiguous range, so selection, filter, sort and rows keep their shape. A scan still reads one folder; the facade joins the scanned folders into the one tree. Adding, removing or rescanning one folder scans only that folder and carries checks, open folders, `current` and the top row across by absolute path. Live refresh rescans only the folder a change is in.
- **Adding.** One action, Add Folder: a toolbar button left of Show ignored, ⌘O, File → Add Folder…, the button of the empty state, a folder dropped on the Dock icon or sent by "Open With", and every folder named on the command line. The system picker allows several folders at once. Adding never replaces what is open.
- **Overlap.** A file is never in the tree twice, and nothing more: a folder holds what its tree lists. Adding a folder that is already open, or one an open folder's tree lists, opens the tree to its row and focuses it. Adding a folder that lies inside an open folder but is hidden by that folder's ignore rules (Show ignored off) adds it as its own top-level folder, since it is not in the tree; if Show ignored is later turned on and the outer folder now shows it, the separate folder is taken in as any contained folder is. Adding a folder that contains open folders removes only those its own scan shows, carrying their checks and open folders; an open folder its ignore rules hide stays as its own top-level folder.
- **Removing.** A folder's row shows a remove button at its right edge while hovered or focused, over the Size cell, with the tooltip "Remove Folder"; ⌘⌫ on a focused folder row does the same. Removing takes the folder's checks with it. Removing the last folder shows the empty state.
- **Filter.** A folder with no match hides like any other folder row.
- **Launch.** With folder arguments, exactly those folders open, taking the saved view of each one that was saved; a file argument opens its folder and shows the file. With none, the saved folders reopen; one that no longer exists is dropped.
- **Saved state.** The state file holds a list of folders, each with its own checks and open folders. A state file in the old one-folder format still loads, as one folder.
- **Window title.** The folder's name with one folder open, the application's name otherwise.
- **Copy.** "Open Folder" becomes "Add Folder" everywhere; "Remove Folder" is the one new tooltip.
- **Budgets** hold for the joined tree. New: adding a folder of about 6,000 files to a tree of 1,000,000 entries shows its rows within 100 ms of the scan landing.

Milestone 6 is built (2026-11-23). As built, where the build settled what this section left open or had to differ:

- **One joined tree.** The engine keeps one `Tree` whose root is no folder: each scanned folder's columns are copied in after the last, ids shifted, so pre-order ids and contiguous subtrees hold and nothing outside the tree changed shape (see Engine). A folder change rebuilds the joined tree and carries the view across with an id map: folders that stay move whole by their offset, the rescanned or taken-in folder merges by name. Measured (release, M4 Max): adding 6,000 files to 1,000,000 entries takes 7.9 ms with the folders closed and 12.0 ms with 1,000,000 rows open and every file checked; rescanning or removing that folder beside them 7.8 ms (bench group `folders`, after the review's fixes; 7.7 ms before them). The other budgets hold as before: a filter keystroke on 1 M names 2.1 to 4.6 ms, checking or clearing 1 M files 17 µs, expanding or collapsing 1 M rows 1.6 ms, scanning and showing 6,000 files (now through the join) 8.4 ms, and the 1,650,231-entry reference corpus with ignored files shown 1.85 s.
- **The top row stays.** The tree view hands the state a probe of the row at the top of its scroll. After a folder is rescanned, taken in or removed, the tree moves its scroll by as many rows as that row moved, mapped by path, keeping the sub-row offset, so rows that appear or go above it do not move what is shown; a focus row that was on screen is brought back only if its place below the top row changed. A launch settles at the saved top row or the first row; a folder added while the app runs is the focus row, scrolled into view.
- **Sizes per folder.** A folder's sizes are its own and shared with its scan, so a folder added late shows its Size cells blank only until its own sizes are read; the other folders keep theirs, and a Size sort is redone as each folder's arrive. A rescan of a folder whose sizes are on screen reads them before it swaps, as before.
- **A folder that cannot be read** shows as one unreadable row (Explorer panel, States), where the one-folder app showed the message in the tree area; the message is its tooltip's second line and in its accessible description. It holds nothing, so a folder open inside it stays a folder of its own until a rescan can read it and takes it in. Its view is kept as it was and saved so; Refresh, or a later launch, brings it back once the folder can be read, and a folder named on the command line while it is away keeps its saved view (a missing path resolves as far as it exists, so it matches the canonical path it was saved under). One never read saves as open. A rescan that makes it readable shows its rows before its sizes are read. The file on the stage, if it was in that folder, moves to the nearest checked file as for any file that goes.
- **"Focused"** for the remove button means the focus row with its ring showing, after a key press; the focus row alone, which a click also sets, does not show it. A folder's row never shows it at launch.
- **Inside or around.** As Overlap says, decided by what a scan lists, never by path alone: the engine's `add_folder` and `replace_folder` take in exactly the top-level folders the new scan lists as folders, and the app removes those from its list. A folder asked for inside one whose first scan runs is remembered and settled when that scan lands. The sort is kept in the state too, so removing the last folder does not reset it.
- **Saved state** is version 2: a list of folders, each with its canonical path, open folders (its own row as `""`) and checks relative to it; the file on the stage and the top row by their paths on disk. A version 1 file loads as one folder, open. Folders named on the command line replace the saved list when the app next saves; a saved folder not named is not kept.
- **Live refresh** runs one watcher per folder; a change rescans only its folder, judged against that folder's part of the tree.
- **A `Tree::from_paths` fixture** shown with `Launch::Tree` keeps its own top-level entries as rows, with no folder rows: it is a test fixture, never what the app opens.

### Milestone 7: the gallery

The stage has two views: the gallery, a grid of the checked files, and the file view that fills the pane, which is the stage as built.

- **Which shows.** Any change to the checked files shows the gallery when two or more are checked, the one file when one is, and a blank stage when none is. A click on a file row or Enter on it opens that file in the file view, checked or not, as before.
- **Tiles**, in gallery order. An image tile is the image fitted inside the tile. A text, code or Markdown tile is the top of the file as plain text in the monospace font at 9 px, clipped to the tile, with no highlighting. A file the stage cannot show is its kind's icon. Under each tile, the file's name on one line, truncated. The tile of `current` has the accent background; the focused tile has the focus ring.
- **Layout.** Columns fill the width: tiles at least 180 px wide, 4:3, 16 px apart and 16 px in from the pane's edges. The grid draws only the rows of tiles on screen.
- **Loading.** Thumbnails and text heads load on the background executor, nearest the viewport first; work for a tile scrolled away is dropped. A thumbnail is decoded no larger than the tile at the display's scale and held in a cache with a byte budget; the file view's full-size cache is separate. A tile shows nothing until its content is ready: no spinner.
- **Opening.** A click on a tile, or Enter on the focused tile, opens that file in the file view. ← and → then move through the gallery as before.
- **Going back.** In the file view, Esc returns to the gallery when two or more files are checked, focused on the tile of the file that was showing and scrolled so it is on screen; otherwise Esc focuses the explorer as before. A grid button left of the position counter does the same, with the tooltip "Gallery".
- **Presentation.** Space presents from either view: from the gallery it starts at the focused tile. Esc leaves to the view it started from.
- **Saved state.** Which view shows is saved and restored with `current`.

Keys in the gallery, a new `Gallery` key context:

| Key | Gallery |
| --- | --- |
| ← → ↑ ↓ | Move focus by one tile |
| Home, End | First tile, last tile |
| Page Up, Page Down | Move focus by a screen of rows |
| Enter | Open the focused tile |
| Space | Present from the focused tile |
| Esc | Focus the explorer |
| ⌘F, ⌘O, Tab | As on the stage |

Budgets: with 100,000 files checked the gallery draws its first frame within 16 ms of the stage being asked, and a scroll frame stays within the display's frame time; thumbnails never hold more than their byte budget.

Milestone 7 is built (2026-11-23); the sections above describe it (State, Stage, Gallery, Presentation, Keys, Accessibility, Copy, saved state, Budgets). As built, where the build settled what this section left open or had to differ:

- **The grid** is GPUI's `uniform_list` of rows of tiles inside GPUI's `container_query`, which hands it the pane's size during layout, so the columns are known before any row is built. The Kit's `GridLayout` is a dashboard grid that lays out every item, and its `VirtualList` needs the item sizes up front; neither fits. A frame asks the engine for one screen of files from the first row on: 2.5 µs from the middle of a million checked files under the name sort, 65 ns under another (bench `other`). Measured (release, M4 Max, a 1000 x 700 window, 100,000 checked files in memory): Select all to the gallery's first frame 5.6 ms, the tree's redraw included; back from the file view 3.6 ms; a scroll frame 0.28 to 0.33 ms median and 0.39 ms at most, at the top and the end, and 0.31 ms flinging 400 px a frame (`timing_the_gallery_over_a_hundred_thousand_checked_files`; measured again after the review's fixes, unchanged). Only the rows on screen are built (`a_hundred_thousand_checked_files_build_only_the_rows_on_screen`).
- **Where the view lives.** The view and the focused tile are the state's (`state/gallery.rs`), so the rules, rescans (the focused tile is carried by path like the focus row), the presentation and the saved state see them; the gallery's view (`stage/gallery.rs`) keeps only its layout, scroll, focus ring and cache, and redraws only when the gallery's content, `current`, the focused tile or its own state changes, not on every key in the tree.
- **Thumbnails** need the `image` crate as a runtime dependency (GPUI's loader decodes at full size and keeps the result; a GPUI image is built from `image::Frame`s). They are decoded for the tile at the display's scale exactly; one made for a larger tile serves a smaller one, so shrinking the pane reads nothing and growing it reads the tiles on screen again. Their cache holds 128 MB: a 6K display's stage covered in tiles holds some 80 MB at its scale. At 128 MB of text heads it holds some 30,000 entries, so the stage's least-recently-used map keeps its keys in order of use as well: making room is a lookup, not a pass over every entry.
- **Four reads at once, really.** The first build dropped a read's task when its tile left the rows around the screen, but a read already decoding ran on in the background with its slot freed, so a fling could run as many full-size decodes as the background executor has threads (the review's finding). A read now keeps its slot until it ends and is told to stop instead. Measured on real threads, outside the app (the same read: the whole file in one buffer, the `image` crate's decode, a thumbnail; forty-eight grainy 30 MB, 6000 x 4000 JPEGs; release, M4 Max): one at a time peaked at 137 MB of resident memory, about 250 ms a photo; four at a time at 537 MB; sixteen at a time, what the first build could reach, at 2.1 GB. The app itself could not be measured flinging: in the run tried, the screen was locked and the window drew no frames, so no tile was read.
- **The file view rests** while the gallery shows, so a gallery's first frame never waits on the file view's read of `current` and its neighbours.
- **Arrow keys** move in reading order: → from a row's last tile goes to the next row's first, ← back; ↓ from a tile with nothing under it in a shorter last row goes to the last tile. Page Up and Page Down move by the whole rows that fit.
- **After a check changes**, the gallery's focus follows `current` only when the change moved `current` or the gallery was not showing, so checking a file in the tree shows its tile, scrolled into view, while unchecking some other file leaves the focus where it was. (The first build always followed `current`, which threw the focus off screen when an unrelated file was unchecked; the review's finding.)
- **After the review** (2026-11-23), besides the above: a resize keeps the view (see Resizing); Tab into the gallery and leaving a presentation by key show the ring; a focused file that goes gives its focus to the file now at its place; a text tile's text is a label's value, so it reaches assistive technology; the grid gives its row and column counts; a picture is checked against its limit on the bytes read before it is decoded; an SVG is no longer drawn at twice its own size first to learn its proportions; presenting from the gallery brings the file's row into view in the tree. Not changed: an animated WebP's first frame is laid on the file's own background colour where GPUI's loader makes it transparent, and the list builds its first row three times a frame (it measures it twice), which also keeps that row's tiles from being evicted; both are small and left as they are.
- **No files checked** clears `current`, as the rule says, even when it was a file opened from the tree: unchecking the last checked file always leaves a blank stage.
- **Tests that changed with the rules:** where a test checked several files and went on in the file view, it now opens the first tile first (`stage::open_checked`, a presentation test, the flick probe); Esc from the code view with two files checked now returns to the gallery before a second Esc reaches the explorer; the test of unchecking the current file reads `current` while three files show as a gallery.

## Left out on purpose

From brainstrike: the tokens column and token indexing; the "This thread" column, thread presets and checkout header; attach to thread, export and split into parts; workspace tabs and the editor; the selection settings popover and selection cap; context actions; view filter presets, chips, glob lists and regex mode; lazy folder loading and its spinners; the 10,000-entry listing cap; the expand-all confirmation.

From a typical image viewer: zoom and pan, HEIC, AVIF and RAW formats, editing of any kind.

## Testing

Three tiers.

1. **Engine.** Plain `#[test]` and `proptest` in `explorer`. Property tests compare each fast path with a brute-force version: check state against a recursive count, rows against a naive recursive walk, filter visibility against a per-node check, sort against a full comparison sort. Scan tests use `tempfile`.
2. **UI integration.** `#[gpui_kit::test]` in `crates/viewer/tests/ui/`: `main.rs` holds the shared helpers and one module per area (rows, checks, filter, header, keyboard, toolbar, folders, launch, stage, gallery, presentation, refresh, watch, persist, menus, races, timing probes and others). Each opens a headless window on the real root view over a `Tree::from_paths` fixture or a temporary folder, drives it with clicks and key presses, and asserts on labels, values, focus and model state. `crates/viewer/tests/shots.rs` (`just shot`) renders key states to PNG.
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
| Add a folder beside others, its rows on screen once its scan lands | 6,000 files added to 1 M entries | 100 ms (measured 7.9 ms; milestone 6) |
| Open a folder with ignored files shown | 1.65 M entries | 3 s, window responsive throughout |
| Filter keystroke to updated rows | 1 M names | 16 ms |
| Check or clear a folder | 1 M files | 1 ms |
| Expand or collapse, rows rebuilt | 1 M visible rows | 16 ms |
| Scroll | any | every frame at display rate |
| Show the gallery, first frame | 100,000 checked files | 16 ms (measured 5.6 ms from Select all, tree included; 3.6 ms back from the file view; milestone 7) |
| Scroll the gallery | 100,000 checked files | the display's frame time (measured 0.28 to 0.33 ms median, 0.39 ms at most, at the top and the end; milestone 7) |
| Thumbnails held | any | 128 MB, never more (milestone 7) |

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
| M6 Several folders | Folders as top-level rows of one joined tree; Add Folder and Remove Folder; per-folder scans, sizes, live refresh and saved views; the old state file still loads. | Several folders come back in order, each with its own view, and adding 6,000 files to a 1 M-entry tree meets its budget. Done: `tests/ui/persist.rs` (`several_folders_come_back_in_order_each_with_its_own_view`), `tests/ui/folders.rs`, `tests/ui/launch.rs`; bench group `folders`, 7.9 ms. |
| M7 Gallery | A grid of the checked files in front of the stage. | Which view shows follows the checks; tiles open, Esc and the grid button go back; the grid builds only the rows on screen at 100,000 files within budget. Done: `tests/ui/gallery.rs`, `tests/ui/persist.rs` (`the_stage_comes_back_in_the_view_it_showed`), the probe `timing_the_gallery_over_a_hundred_thousand_checked_files`. |

Milestone 0 detail:

- The Kit's agent skills install with `npx skills add longbridge/gpui-kit`. That downloads third-party agent instructions into the repo, so Sean runs it or approves it at the time.

Milestone 5 detail:

- State is a JSON file in the platform config directory: root path, expanded paths, selected paths, query, kind, sort, first visible row, split width, window bounds. Paths are stored relative to the root. (Milestone 6 made it a list of folders: see As built.)
- Live refresh uses a recursive `notify` watcher with a 150 ms debounce. Any batch of events triggers a full background rescan and a tree swap that carries state across by path.

As built:

- **File.** `~/Library/Application Support/ai.brainstrike.viewer/state.json` on macOS (`directories`' config folder for the bundle identifier). `STRIKEBRAIN_VIEWER_STATE=<file>` points a run elsewhere. Fields (version 2, milestone 6): `version` (2), `folders` (in the order added, each `root`, absolute and canonical, `expanded` and `selected` (`folders` and `files`), paths relative to the folder, `""` the folder itself), `show_ignored`, `current` and `first_visible_row` (paths on disk), `view` (`file` or `gallery`, milestone 7; a file without it shows the file view), `query`, `kind`, `sort` (`key`, `direction`), `explorer_width`, `window` (`state` windowed, maximized or fullscreen; `x`, `y` the frame's top-left corner relative to its display and `width`, `height` the content's size, title bar excluded, which is what a window opens with; the display's UUID). A version 1 file (one `root`, its `expanded`, `selected`, `current` and `first_visible_row` relative to it) loads as one folder, open. A missing, unreadable, malformed or other-version file is ignored and the app starts fresh; it is replaced at the next save. Unusable values (a relative root or path, a non-positive size) are dropped.
- **Checks, compactly.** A folder whose every file is checked is one entry under its top-level folder's `folders` (the top-level folder itself is `""`); other checked files are listed under `files`. Select all saves as one entry per top-level folder. Restoring a folder entry checks every file in it at that time, including files added since. A rescan does the same, so a session and a relaunch agree: a folder whose every file is checked (each top-level folder, after Select all) keeps every file checked when files appear in it, by Refresh or live refresh; a file added to a partly checked folder comes in unchecked. Measured (release, M4 Max): 100,000 checked files with no folder fully checked save as 3.96 MB in 22 ms on the background executor (names 12 ms, encoding 5 ms, writing 5 ms) and read back in 4 ms; all 100,000 checked save as 402 bytes. The UI thread copies two bit sets, under a microsecond at this size.
- **First visible row** is stored by its path on disk, not index, so it survives files added above it. Restoring puts that row at the top of the tree and gives it keyboard focus. Under a Size sort it waits for sizes, which reorder the rows.
- **When it saves.** Half a second after a change, at most once per half second (a change while a save waits rides along), and when the app quits. Closing the window quits, so that save runs with the window still there. A save copies the state on the UI thread and names, encodes and writes it on the background executor; saves land in the order they were taken, an unchanged state is not written again, and the file is written to a temporary name, synced to disk and renamed over the old one; a write that fails removes the temporary file. Scrolling alone does not schedule a save; the next save or the quit records it.
- **Nothing on screen to save.** Refresh or Show ignored before a folder's first scan lands scan it again with its saved view still waiting, and that view is what is saved meanwhile; a folder added and still scanning saves as open, as it will show. Since milestone 6, a saved folder that no longer exists at launch is dropped, and folders named on the command line are exactly the folders saved next. A folder that vanishes while open (moved, deleted, unmounted) shows as an unreadable row and keeps its open folders and checks as the ones saved; Refresh once it is back restores them.
- **Restoring** happens in steps: the filter and Show ignored apply before any scan, so the first trees are already right; the sort applies to the first tree; each folder's open folders and checks apply when that folder's first scan lands; `current`, the stage's view and the top row once every folder is on screen. The gallery comes back only while two or more files are checked, focused on `current`'s tile. Window bounds go to the window when it opens, moved and shrunk onto the saved display's visible area, or the primary display's when that display is gone. A fullscreen the presentation made is saved as the window it came from. The explorer's width is fitted to the window: at least 220 px, leaving the stage at least 220 px. A disk change during a first scan does not lose the saved top row: the rescan that follows puts it at the top.
- **Live refresh.** Each folder has its own watcher (`notify`'s FSEvents backend; creations, removals and modifications, no reads), on its own thread, feeding a channel; a change rescans only its folder (milestone 6). A GPUI task waits for a change, gathers more for 150 ms on GPUI's timer, then asks `explorer::changes_matter`, off the UI thread, whether a new scan would show any of them: a path the tree lists matters; otherwise the first part of the path the tree lacks matters only if it still exists and the scan's own walker, with the same ignore rules, would list it. Changes inside `.git`, to `.DS_Store`, under ignored folders such as `target/` and `node_modules/`, and to the app's own state file and its temporary name never rescan. Measured: 400 files written under an ignored `target/` and a log ignored at the top, over a second, caused no rescan; one new file in `src/` caused exactly one. Changes that arrive during a folder's first scan wait for its tree and are judged against it. A change while a folder's scan runs asks for one more scan of it when it lands, never more. A watcher error, or a watcher that lost track with no path to name, rescans its folder once. A watcher is dropped with its folder or the state. One that cannot start (the folder is missing, a watch limit) leaves the folder unfollowed and turns Refresh the warning colour, its tooltip giving the reason; Refresh starts it again. A rescan that finds the file on the stage unchanged (size and modification time) neither reads nor rebuilds its view, so its scroll, selection and keyboard focus stay; a changed file is read again, and focus leaves the view only when the file or its viewer changes. A Markdown link opens only when it is `http`, `https` or `mailto`.
- **Bundle.** `just bundle` builds the release profile (thin LTO, one codegen unit, debug info stripped, symbol names kept, panics unwind) and assembles `target/release/bundle/osx/Strikebrain Viewer.app` by hand, as the Kit's packaging guide does: the Info.plist template in `crates/viewer/bundle` with the version from Cargo, the icon from the brand pack (`brand/Strikebrain-Production-Pack`: an `Assets.car` compiled from an Icon Composer icon, named by `CFBundleIconName`, and an `.icns`), and an ad-hoc signature. Measured on 2026-11-23: binary 26.1 MB (21.4 MB with symbol names stripped as well), bundle 25 MB; launching the bundled binary to its first on-screen window, 0.67 s the first time after a build and 0.20 s after.

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
- **Large photos.** GPUI decodes at full resolution. Measured in milestone 3 (release, M4 Max), an 8000 x 6000 picture decodes in 50 ms as a smooth PNG and 0.59 s as a grainy 44 MB JPEG, on the background executor, and holds 183 MB of pixels, plus as much again as a GPU texture while it is drawn; from the cache it draws in 3 to 5 ms. A cache bounded by count alone held some 1.5 GB of such photos, or about 740 MB of 24-megapixel ones. The cache now also has a byte budget, 512 MB, and pictures have size limits (64 MB on disk, 16 MB for an animation, 8 MB to be decoded ahead). Measured after (release, M4 Max): flicking through eight 6000 x 4000 JPEGs of 22 MB (92 MB of pixels each) at 0.2 s a step, the cache holds five, 458 MB, and the process's memory stops growing at the sixth (801 MB, from 343 MB with one). A single picture under the limit can still be larger than the budget; it stays alone. The real fix is still a loader that decodes at display size, which needs the `image` crate as a runtime dependency and the GIF, WebP and SVG paths GPUI's loader already has. Probes: `timing_a_large_photo`, `timing_flicking_through_large_photos`.
- **Editor on large files.** The Kit's 0.10.0 source documents the editor as handling about 50,000 lines. Measured in milestone 3 (release, M4 Max, 27-byte lines, test build): 0.5 MB reaches the screen 13 ms after the click and scrolls at 0.45 ms a frame; 5.5 MB (200,000 lines) 50 ms and 1.1 ms; 10 MB (360,000 lines) 0.1 s and 2.6 ms; one 4.8 MB line 85 ms and 0.5 ms. Highlighting arrives later from the background executor: 0.85 s, 7 s and 13 s. So the text limit stays at 10 MB. Two costs remain. The Kit's syntect backend parses a document of up to 64 KB at once on the UI thread: the limit is a private constant (`SYNC_PARSE_MAX_BYTES`), the highlighter is not `Send`, and nothing public runs it elsewhere. Until milestone 4 every such file was parsed twice, because setting the editor's text with a language set parses it and the next frame parses it again; the stage now sets the text as plain text and then the language, so it is parsed once, in the first frame, at about 1.2 ms a KB. Measured in milestone 4 (release, M4 Max, syntax set loaded), click to first frame before and after: 8 KB 24 → 15 ms, 27 KB 71 → 37 ms, 62 KB 155 → 80 ms; a 68 KB file still shows in 5 ms and is highlighted 82 ms later in the background. The first file highlighted also loads syntect's syntax set, about 20 ms. So → through files of 30 to 64 KB still hitches by 40 to 80 ms a file. Removing that means a highlighter of our own, given to the editor through its public `set_highlighter_factory` and parsing on the background executor, with `syntect` (already built, through the Kit) as a direct dependency. And the editor copies its whole text into an accessibility value on every frame while an assistive technology is connected (always, in test builds): at 10 MB a test build grows by about 10 MB a frame until the allocator settles near 870 MB, which a release build without a screen reader does not do. Probe: `timing_the_code_view_on_large_files`.
- **Markdown on large files.** The text view lays out only the blocks on screen: 0.25 ms a frame at 4 KB to 345 KB. It measures every block once when a document opens: 28 ms at 4 KB, 41 ms at 86 KB, 0.21 s at 345 KB. Opening a document whose text shares no prefix with the one before costs more, growing with the square of the size (release, M4 Max, click to first frame, two kinds of generated document): 0.07 to 0.12 s at 128 KB, 0.18 to 0.21 s at 256 KB, 0.32 to 0.34 s at 384 KB, 0.47 to 0.53 s at 512 KB, 0.93 to 0.97 s at 768 KB, 1.5 to 1.6 s at 1 MB, 5.1 to 5.3 s at 2 MB. So Markdown over 256 KB, the largest size measured under 0.3 s, opens in the code view: 10 ms at 256 to 512 KB, 17 ms at 1 MB, 29 ms at 2 MB. Probes: `timing_the_markdown_view_on_large_documents`, `timing_markdown_around_its_size_limit`.
- **Thumbnails decode whole pictures.** The `image` crate has no reduced-size JPEG decode, so a tile's picture is decoded at full size, then scaled down and turned; the full-size pixels live only while the tile is read, on the background executor, at most four reads at once (a read keeps its slot until it ends). Measured (release, M4 Max, the test executor reading one tile after another): a grainy 22 MB, 6000 x 4000 JPEG takes about 200 ms a tile; forty of them took the process from 136 MB to 187 MB, then 193 MB after scrolling to the end, holding 13 MB of thumbnails. Before the picture was read into one buffer of its size, a buffer grown by doubling left macOS's allocator holding some 30 MB a photo (116 MB to 488 MB, then 782 MB). On real threads, outside the app, the same read over grainy 30 MB, 6000 x 4000 JPEGs peaked at 137 MB one at a time, 537 MB four at a time and 2.1 GB sixteen at a time: four at once is about half a gigabyte of such photos for a moment, and four 100-megapixel pictures would be well over a gigabyte; the `image` crate's 512 MB allocation limit refuses larger ones, which show their kind's icon. A fling through such photos in the running app has not been measured. The file view reads text the old way, growing its buffer up to 10 MB; the flick probe sees no growth from it, but it has not been measured on large texts. Probe: `timing_the_gallery_over_large_photos`.
- **Small scroll steps far down a very long list.** GPUI keeps a list's scroll offset in an `f32`: `uniform_list`, the Kit's `VirtualList`, and `list()`, whose `ListState` anchors on a row but adds each wheel delta to an absolute `f32` top. At 29 px a row, a delta under half the `f32` step is lost: from row 289,262 (2^23 px, step 1 px) sub-pixel deltas go; from row 578,525 (2^24 px, step 2 px) a 1 px delta goes and 3 px moves 4; from row 1,157,049 (2^25 px, step 4 px) everything under 2 px goes. Measured on the 1,650,231-row reference corpus as one flat list (M2, release, M4 Max): at the middle, 0.5 and 1 px moved nothing and 3 px moved 4; at the end, 0.5 and 1 px moved nothing, 3 px moved 4 and 10 px moved 8. Frame cost is not the problem: a scrolling frame took 0.66 to 0.69 ms median and at most 1.1 ms at the top, the middle and the end (frame build without GPU raster). So past about row 578,000 a slow trackpad scroll sticks while flicks, the wheel, the scrollbar and the keys still move. The fix is scroll state of our own (a row plus an offset inside it, or `f64`) with our own wheel handling and a scrollbar adapter, not a framework list; it is left out of M2. Probe: `cargo test --release -p viewer --test ui -- --ignored --nocapture timing_scrolling`.
- **Cold build time** is unknown until milestone 0.
- **The caret's line number.** GPUI Base 0.10.0 draws the line number of the line holding the caret in the theme's foreground and ignores the theme's `editor.active_line_number`, which the app sets to the muted colour. A read-only view still has a caret, at the top when a file opens, so line 1's number stands out until the person clicks elsewhere. Not fixable through the theme in this version.
- **Window bounds on a real screen.** GPUI reports a window's frame but opens a window at a content size, so the window used to grow by its title bar at each launch (measured on the bundle: 1128, 1160, 1192, 1224 px over four launches); it now saves the content size and stayed at 1096 px over four launches. Those launches ran while the Mac's screen was locked, with no active display, so GPUI placed the window without one; a launch with a display attached, and a window on a second display, have not been seen.
- **Esc inside the fullscreen animation.** GPUI 0.3.12 makes a window fullscreen asynchronously and keeps AppKit's will-enter and did-enter callbacks to itself; nothing public says a transition is under way. Space then Esc within the half second of the animation can leave the window fullscreen with the presentation ended (the toggle that should undo it may land mid-animation, or before the window reports fullscreen). The green button puts it right. Not handled.
- **Live refresh on very large folders.** Each change a scan would show rescans the whole folder it is in, as decided; the other folders are left alone. With Show ignored on, a busy folder the tree then shows (a build's `target/`) is rescanned back to back while it changes. On Linux, `notify` uses inotify, which watches every folder separately; a 1.65 M-entry tree would exceed default watch limits. Neither is handled. A change to `.git/info/exclude` or the global excludes does not rescan.

## References

- GPUI: https://gpui.rs/ and https://github.com/zed-industries/zed/tree/main/crates/gpui
- GPUI Kit docs index: https://gpui-kit.com/llms.txt. Any page is Markdown with `.md` appended.
- GPUI Kit repository and agent skills: https://github.com/longbridge/gpui-kit
- Brainstrike explorer: `packages/file-explorer`, `file-explorer-ui`, `file-explorer-hooks`, `file-tree-worker`, `file-system`, and `packages/ui/src/tree-table` in `/Users/seandearnaley/Documents/GitHub/brainstrike`
