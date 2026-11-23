use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use explorer::{Direction, Explorer, KindFilter, SavedFolder, SavedSelection, Sort, SortKey, Tree};
use gpui_kit::{Bounds, WindowBounds, point, px, size};

use super::{
    APP_ID, FolderSnapshot, Saved, SavedView, SavedWindow, Snapshot, SnapshotView, StateStore,
    decode, encode, fit, plan_launch, resolve, split_app_id,
};
use crate::state::Launch;

/// One folder's part, with open folders and checks.
fn saved_folder(path: PathBuf) -> SavedFolder {
    SavedFolder {
        path,
        open: vec![String::new(), "src".into(), "src/app".into()],
        selection: SavedSelection {
            folders: vec!["docs".into()],
            files: vec!["README.md".into(), "src/main.rs".into()],
        },
    }
}

/// A state with every field away from its default: `root` and a second
/// folder inside it.
fn full(root: &Path) -> Saved {
    Saved {
        view: SavedView {
            folders: vec![
                saved_folder(root.to_owned()),
                SavedFolder {
                    path: root.join("second"),
                    ..SavedFolder::default()
                },
            ],
            show_ignored: true,
            current: Some(root.join("src/main.rs")),
            query: "main -test".into(),
            kind: KindFilter::Text,
            sort: Sort {
                key: SortKey::Size,
                direction: Direction::Descending,
            },
            top_row: Some(root.join("src/app")),
        },
        explorer_width: Some(412.5),
        window: Some(SavedWindow {
            bounds: WindowBounds::Fullscreen(Bounds::new(
                point(px(40.), px(60.)),
                size(px(1200.), px(800.)),
            )),
            display: Some("4A1B".into()),
        }),
    }
}

#[test]
fn a_state_survives_the_trip_through_json() {
    let saved = full(Path::new("/projects/demo"));
    let bytes = encode(&saved).unwrap();
    assert_eq!(decode(&bytes), Some(saved));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\"version\": 2"), "{text}");
    assert!(text.contains("\"kind\": \"text\""), "{text}");
    assert!(
        text.contains("\"root\": \"/projects/demo/second\""),
        "{text}"
    );
    assert!(
        text.contains("\"first_visible_row\": \"/projects/demo/src/app\""),
        "{text}"
    );
}

#[test]
fn an_empty_state_survives_too() {
    let saved = Saved::default();
    assert_eq!(decode(&encode(&saved).unwrap()), Some(saved));
}

#[test]
fn a_file_of_the_old_one_folder_format_loads_as_one_folder() {
    let old = br#"{"version": 1, "root": "/p", "show_ignored": true,
        "expanded": ["src"], "selected": {"folders": ["docs"], "files": ["README.md"]},
        "current": "src/main.rs", "query": "q", "kind": "images",
        "sort": {"key": "type", "direction": "descending"},
        "first_visible_row": "src", "explorer_width": 280.0}"#;
    let saved = decode(old).unwrap();
    assert_eq!(
        saved.view.folders,
        [SavedFolder {
            path: "/p".into(),
            // The folder itself shows open, as a folder added does.
            open: vec![String::new(), "src".into()],
            selection: SavedSelection {
                folders: vec!["docs".into()],
                files: vec!["README.md".into()],
            },
        }]
    );
    assert_eq!(saved.view.current, Some(PathBuf::from("/p/src/main.rs")));
    assert_eq!(saved.view.top_row, Some(PathBuf::from("/p/src")));
    assert!(saved.view.show_ignored);
    assert_eq!(saved.view.kind, KindFilter::Images);
    assert_eq!(saved.view.sort.key, SortKey::Type);
    assert_eq!(saved.explorer_width, Some(280.));
    // With no usable root, nothing is relative to anything.
    let rootless = br#"{"version": 1, "root": "relative", "current": "a.txt"}"#;
    assert_eq!(decode(rootless), Some(Saved::default()));
}

#[test]
fn a_file_that_is_not_a_state_of_a_known_version_is_ignored() {
    let ok = encode(&full(Path::new("/p"))).unwrap();
    let newer = String::from_utf8(ok.clone())
        .unwrap()
        .replace("\"version\": 2", "\"version\": 3");
    for bytes in [
        &b""[..],
        b"{",
        b"[]",
        b"null",
        b"{}",
        b"{\"version\": 2, \"kind\": \"sounds\"}",
        b"{\"version\": 2, \"folders\": 3}",
        b"{\"version\": 1, \"expanded\": 3}",
        b"\xff\xfe",
        newer.as_bytes(),
    ] {
        assert_eq!(decode(bytes), None, "{}", String::from_utf8_lossy(bytes));
    }
    // A file missing fields takes their defaults; values that cannot be
    // used are dropped, not trusted.
    let sparse = br#"{"version": 2, "folders": [{"root": "relative/path"}], "current": "a.txt", "first_visible_row": "b", "explorer_width": -3, "window": {"state": "windowed", "x": 0, "y": 0, "width": 0, "height": 10, "display": null}, "future": true}"#;
    assert_eq!(decode(sparse), Some(Saved::default()));
}

#[test]
fn the_store_writes_atomically_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("nested/state.json"));
    assert_eq!(store.load(), None);
    let saved = full(dir.path());
    let ticket = store.ticket();
    assert!(store.write(ticket, &saved).unwrap());
    assert_eq!(store.load(), Some(saved.clone()));
    // Only the file itself is left: the temporary was renamed over it.
    let left: Vec<_> = fs::read_dir(dir.path().join("nested"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["state.json"]);
    assert_eq!(store.path(), dir.path().join("nested/state.json"));
}

#[test]
fn the_store_skips_an_unchanged_state_and_a_save_overtaken_by_a_later_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("state.json"));
    let first = full(dir.path());
    let (old, new) = (store.ticket(), store.ticket());
    let mut second = first.clone();
    second.view.query = "later".into();
    assert!(store.write(new, &second).unwrap());
    // The earlier save lands after the later one: dropped.
    assert!(!store.write(old, &first).unwrap());
    assert_eq!(store.load().unwrap().view.query, "later");
    // The same state again is not written.
    assert!(!store.write(store.ticket(), &second).unwrap());
    // A clone shares the bookkeeping.
    let clone = store.clone();
    assert!(!clone.write(old, &first).unwrap());
}

#[test]
fn a_store_that_cannot_write_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file"), "").unwrap();
    let store = StateStore::at(dir.path().join("file/state.json"));
    let err = store.write(store.ticket(), &Saved::default()).unwrap_err();
    assert_ne!(err.to_string(), "");
}

#[test]
fn a_write_that_fails_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the state file should be: the rename over it fails
    // after the temporary file is written.
    fs::create_dir_all(dir.path().join("state.json/inside")).unwrap();
    let store = StateStore::at(dir.path().join("state.json"));
    store.write(store.ticket(), &Saved::default()).unwrap_err();
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["state.json"]);
}

#[test]
fn the_state_lives_in_the_config_folder_named_for_the_app() {
    assert_eq!(split_app_id(), ("ai", "brainstrike", "viewer"));
    if std::env::var_os(super::STATE_FILE_VAR).is_none() {
        let store = StateStore::in_config_dir().unwrap();
        assert!(store.path().ends_with("state.json"));
        #[cfg(target_os = "macos")]
        assert!(
            store
                .path()
                .ends_with(format!("Library/Application Support/{APP_ID}/state.json")),
            "{}",
            store.path().display()
        );
    }
}

#[test]
fn the_bundle_names_the_same_identifier() {
    let plist = include_str!("../../bundle/Info.plist");
    assert!(plist.contains(&format!("<string>{APP_ID}</string>")));
}

/// The folders a plan opens.
fn opened(launch: &Launch) -> Vec<PathBuf> {
    match launch {
        Launch::Folders(folders) => folders.clone(),
        Launch::Empty => Vec::new(),
        Launch::Tree(_) => panic!("a plan never opens a tree from memory"),
    }
}

/// The folders a plan's view restores.
fn restored(view: Option<&SavedView>) -> Vec<PathBuf> {
    view.map_or_else(Vec::new, |view| {
        view.folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect()
    })
}

#[test]
fn folder_arguments_open_exactly_those_each_with_its_own_saved_view() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    for folder in ["a", "b", "c"] {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));
    let mut saved = full(&a);
    saved.view.folders[1].path.clone_from(&b);

    // A saved folder by another spelling of its path, and one not saved.
    let plan = plan_launch(vec![root.join("c/../a"), c.clone()], Some(saved.clone()));
    assert_eq!(opened(&plan.launch), [root.join("c/../a"), c.clone()]);
    assert_eq!(restored(plan.view.as_ref()), std::slice::from_ref(&a));
    let view = plan.view.unwrap();
    assert_eq!(view.query, saved.view.query);
    assert_eq!(plan.explorer_width, Some(412.5));
    assert!(plan.window.is_some());

    // Folders never saved: no view, but the window and the width stay.
    let plan = plan_launch(vec![c.clone()], Some(saved.clone()));
    assert_eq!(opened(&plan.launch), [c]);
    assert_eq!(plan.view, None);
    assert_eq!(plan.window, saved.window);

    // A folder that does not exist opens, to show its error.
    let plan = plan_launch(vec![root.join("gone")], Some(saved));
    assert_eq!(opened(&plan.launch), [root.join("gone")]);
    assert_eq!(plan.view, None);
}

#[test]
fn a_path_resolves_as_far_as_it_exists() {
    let dir = tempfile::tempdir().unwrap();
    let real = fs::canonicalize(dir.path()).unwrap();
    // The temporary folder's own path may go through a link (/var on macOS).
    assert_eq!(resolve(dir.path()), real);
    assert_eq!(
        resolve(&dir.path().join("gone/deeper")),
        real.join("gone/deeper")
    );
    assert!(resolve(Path::new("relative-and-missing")).is_absolute());
}

#[test]
fn a_folder_argument_that_is_missing_keeps_its_saved_view() {
    let dir = tempfile::tempdir().unwrap();
    let away = fs::canonicalize(dir.path()).unwrap().join("away");
    let saved = full(&away);
    let plan = plan_launch(vec![dir.path().join("away")], Some(saved));
    assert_eq!(restored(plan.view.as_ref()), std::slice::from_ref(&away));
}

#[test]
fn no_argument_reopens_the_saved_folders_that_still_exist() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(root.join("second")).unwrap();
    let saved = full(&root);

    let plan = plan_launch(Vec::new(), Some(saved.clone()));
    assert_eq!(opened(&plan.launch), [root.clone(), root.join("second")]);
    assert_eq!(plan.view, Some(saved.view.clone()));

    // One that no longer exists is dropped.
    fs::remove_dir(root.join("second")).unwrap();
    let plan = plan_launch(Vec::new(), Some(saved));
    assert_eq!(opened(&plan.launch), std::slice::from_ref(&root));
    assert_eq!(restored(plan.view.as_ref()), std::slice::from_ref(&root));

    let gone = full(&root.join("gone"));
    let plan = plan_launch(Vec::new(), Some(gone));
    assert!(matches!(plan.launch, Launch::Empty));
    assert_eq!(plan.view, None);
    assert_eq!(plan.explorer_width, Some(412.5));

    let plan = plan_launch(Vec::new(), None);
    assert!(matches!(plan.launch, Launch::Empty));
    assert!(plan.window.is_none() && plan.explorer_width.is_none());
}

#[test]
fn a_window_is_moved_and_shrunk_onto_the_display() {
    let display = Bounds::new(point(px(0.), px(25.)), size(px(1440.), px(875.)));
    let inside = Bounds::new(point(px(100.), px(100.)), size(px(800.), px(600.)));
    assert_eq!(fit(inside, display), inside);
    // Off the right and bottom edges, as after unplugging a larger display.
    let off = Bounds::new(point(px(2500.), px(1400.)), size(px(800.), px(600.)));
    assert_eq!(
        fit(off, display),
        Bounds::new(point(px(640.), px(300.)), size(px(800.), px(600.)))
    );
    // Above and left of it, and larger than it.
    let huge = Bounds::new(point(px(-50.), px(-10.)), size(px(3000.), px(2000.)));
    assert_eq!(fit(huge, display), display);
}

#[test]
fn a_snapshot_names_each_folder_in_order() {
    let tree = Arc::new(Tree::from_paths(&["docs/a.md", "docs/b.md", "src/main.rs"]).unwrap());
    let mut explorer = Explorer::new(tree);
    let docs = explorer.tree().find("docs").unwrap();
    explorer.toggle_check(docs);
    explorer.toggle_folder(explorer.tree().find("src").unwrap());
    // A folder still waiting for its scan keeps the view it waits with.
    let waiting = saved_folder(PathBuf::from("/nowhere"));
    let view = SnapshotView {
        live: Some(explorer.snapshot()),
        folders: vec![
            FolderSnapshot::Waiting(waiting.clone()),
            FolderSnapshot::Live(PathBuf::new()),
            FolderSnapshot::Live(PathBuf::from("/not/on/screen")),
        ],
        rest: SavedView {
            current: Some("/x/docs/a.md".into()),
            ..SavedView::default()
        },
    };
    let saved = Snapshot {
        view,
        explorer_width: Some(300.),
        window: None,
    }
    .saved();
    assert_eq!(
        saved.view.folders,
        [
            waiting,
            SavedFolder {
                path: PathBuf::new(),
                open: vec!["src".into()],
                selection: SavedSelection {
                    folders: vec!["docs".into()],
                    files: vec![],
                },
            },
            SavedFolder {
                path: "/not/on/screen".into(),
                ..SavedFolder::default()
            },
        ]
    );
    assert_eq!(saved.view.current, Some(PathBuf::from("/x/docs/a.md")));
    assert_eq!(saved.explorer_width, Some(300.));
}

/// Saving 100,000 checked files. Run in release:
/// `cargo test --release -p viewer --lib -- --ignored --nocapture timing_saving`.
#[test]
#[ignore = "timing probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its timings")]
fn timing_saving_a_hundred_thousand_checked_files() {
    use std::time::Instant;

    // 100 folders of 10 folders of 100 files: 100,000 files with
    // realistic relative paths.
    let file_names: Vec<String> = (0..100)
        .flat_map(|a| {
            (0..10).flat_map(move |b| {
                (0..100).map(move |c| format!("module-{a:03}/part-{b:02}/file-{c:03}.rs"))
            })
        })
        .collect();
    let paths: Vec<&str> = file_names.iter().map(String::as_str).collect();
    let tree = Arc::new(Tree::from_paths(&paths).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("state.json"));

    for (case, skip_every) in [
        ("all checked", None),
        ("every file but one per folder", Some(100)),
    ] {
        let mut explorer = Explorer::new(Arc::clone(&tree));
        explorer.select_all();
        if let Some(every) = skip_every {
            let skipped: Vec<_> = explorer.selection().iter().step_by(every).collect();
            for id in skipped {
                explorer.toggle_check(id);
            }
        }
        let checked = explorer.selection().count();
        let started = Instant::now();
        let snapshot = Snapshot {
            view: SnapshotView {
                live: Some(explorer.snapshot()),
                folders: vec![FolderSnapshot::Live(PathBuf::new())],
                rest: SavedView::default(),
            },
            explorer_width: Some(300.),
            window: None,
        };
        let ui = started.elapsed();
        let saved = snapshot.saved();
        let named = started.elapsed();
        let bytes = encode(&saved).unwrap();
        let encoded = started.elapsed();
        assert!(store.write(store.ticket(), &saved).unwrap());
        let written = started.elapsed();
        let loaded = Instant::now();
        let back = store.load().unwrap();
        let read = loaded.elapsed();
        let mut restored = Explorer::new(Arc::clone(&tree));
        let folder = &saved.view.folders[0];
        restored.restore_saved_selection(&folder.selection);
        assert_eq!(restored.selection(), explorer.selection());
        assert_eq!(back.view.folders.len(), 0, "a tree in memory is not saved");
        eprintln!(
            "{case}: {checked} checked, {} folder and {} file entries, {} bytes; \
             UI thread {ui:.2?}, named {named:.2?}, encoded {encoded:.2?}, written {written:.2?}; \
             read back {read:.2?}",
            folder.selection.folders.len(),
            folder.selection.files.len(),
            bytes.len(),
        );
    }
}
