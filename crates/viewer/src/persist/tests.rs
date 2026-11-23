use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use explorer::{Direction, Explorer, KindFilter, SavedSelection, Sort, SortKey, Tree};
use gpui_kit::{Bounds, WindowBounds, point, px, size};

use super::{
    APP_ID, FolderView, Saved, SavedWindow, Snapshot, SnapshotView, StateStore, decode, encode,
    fit, plan_launch, split_app_id,
};
use crate::state::Launch;

/// A state with every field away from its default.
fn full(root: PathBuf) -> Saved {
    Saved {
        root: Some(root),
        view: FolderView {
            show_ignored: true,
            expanded: vec!["src".into(), "src/app".into()],
            selected: SavedSelection {
                folders: vec!["docs".into()],
                files: vec!["README.md".into(), "src/main.rs".into()],
            },
            current: Some("src/main.rs".into()),
            query: "main -test".into(),
            kind: KindFilter::Text,
            sort: Sort {
                key: SortKey::Size,
                direction: Direction::Descending,
            },
            top_row: Some("src/app".into()),
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
    let saved = full(PathBuf::from("/projects/demo"));
    let bytes = encode(&saved).unwrap();
    assert_eq!(decode(&bytes), Some(saved));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\"version\": 1"), "{text}");
    assert!(text.contains("\"kind\": \"text\""), "{text}");
    assert!(
        text.contains("\"first_visible_row\": \"src/app\""),
        "{text}"
    );
}

#[test]
fn an_empty_state_survives_too() {
    let saved = Saved::default();
    assert_eq!(decode(&encode(&saved).unwrap()), Some(saved));
}

#[test]
fn a_file_that_is_not_a_state_of_this_version_is_ignored() {
    let ok = encode(&full(PathBuf::from("/p"))).unwrap();
    let newer = String::from_utf8(ok.clone())
        .unwrap()
        .replace("\"version\": 1", "\"version\": 2");
    for bytes in [
        &b""[..],
        b"{",
        b"[]",
        b"null",
        b"{}",
        b"{\"version\": 1, \"kind\": \"sounds\"}",
        b"{\"version\": 1, \"expanded\": 3}",
        b"\xff\xfe",
        newer.as_bytes(),
    ] {
        assert_eq!(decode(bytes), None, "{}", String::from_utf8_lossy(bytes));
    }
    // A version 1 file missing fields takes their defaults; values that
    // cannot be used are dropped, not trusted.
    let sparse =
        br#"{"version": 1, "root": "relative/path", "explorer_width": -3, "window": {"state": "windowed", "x": 0, "y": 0, "width": 0, "height": 10, "display": null}, "future": true}"#;
    assert_eq!(decode(sparse), Some(Saved::default()));
}

#[test]
fn the_store_writes_atomically_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("nested/state.json"));
    assert_eq!(store.load(), None);
    let saved = full(dir.path().to_owned());
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
    let first = full(dir.path().to_owned());
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

#[test]
fn a_folder_argument_opens_and_takes_the_view_only_if_it_was_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let saved = full(fs::canonicalize(&a).unwrap());

    // The same folder, by another spelling of its path.
    let plan = plan_launch(Some(dir.path().join("b/../a")), Some(saved.clone()));
    assert!(matches!(&plan.launch, Launch::Folder(path) if path.ends_with("b/../a")));
    assert_eq!(plan.view, Some(saved.view.clone()));
    assert_eq!(plan.explorer_width, Some(412.5));
    assert!(plan.window.is_some());

    // Another folder: no view, but the window and the width stay.
    let plan = plan_launch(Some(b.clone()), Some(saved.clone()));
    assert!(matches!(&plan.launch, Launch::Folder(path) if *path == b));
    assert_eq!(plan.view, None);
    assert_eq!(plan.window, saved.window);
    // The saved folder stays saved until another one opens.
    let kept = plan.kept.expect("the saved folder is kept");
    assert_eq!(
        (kept.root, kept.view),
        (saved.root.clone().unwrap(), saved.view.clone())
    );

    // A folder that does not exist opens, to show its error.
    let plan = plan_launch(Some(dir.path().join("gone")), Some(saved));
    assert!(matches!(plan.launch, Launch::Folder(_)));
    assert_eq!(plan.view, None);
}

#[test]
fn no_argument_reopens_the_saved_folder_while_it_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let saved = full(root.clone());

    let plan = plan_launch(None, Some(saved.clone()));
    assert!(matches!(&plan.launch, Launch::Folder(path) if *path == root));
    assert_eq!(plan.view, Some(saved.view.clone()));
    assert_eq!(plan.kept, None);

    let gone = full(root.join("gone"));
    let plan = plan_launch(None, Some(gone.clone()));
    assert!(matches!(plan.launch, Launch::Empty));
    assert_eq!(plan.view, None);
    assert_eq!(plan.explorer_width, Some(412.5));
    // Kept, to be saved again: an unmounted volume loses nothing.
    let kept = plan.kept.expect("the saved folder is kept");
    assert_eq!((Some(kept.root), kept.view), (gone.root, gone.view));

    let plan = plan_launch(None, None);
    assert!(matches!(plan.launch, Launch::Empty));
    assert!(plan.window.is_none() && plan.explorer_width.is_none());
    assert!(plan.kept.is_none());
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
fn a_snapshot_names_its_paths_and_makes_the_root_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let tree = Arc::new(Tree::from_paths(&["docs/a.md", "docs/b.md", "src/main.rs"]).unwrap());
    let mut explorer = Explorer::new(tree);
    let docs = explorer.tree().find("docs").unwrap();
    explorer.toggle_check(docs);
    explorer.toggle_folder(explorer.tree().find("src").unwrap());
    let rest = FolderView {
        current: Some("docs/a.md".into()),
        ..FolderView::default()
    };
    let snapshot = Snapshot {
        root: Some(dir.path().join(".")),
        view: Some(SnapshotView::Live(explorer.snapshot(), rest)),
        explorer_width: Some(300.),
        window: None,
    };
    let saved = snapshot.saved();
    assert_eq!(saved.root, Some(fs::canonicalize(dir.path()).unwrap()));
    assert_eq!(saved.view.expanded, ["src"]);
    assert_eq!(saved.view.selected.folders, ["docs"]);
    assert_eq!(saved.view.current.as_deref(), Some("docs/a.md"));

    // Before the first scan lands, the view waiting to be restored is kept.
    let waiting = full(PathBuf::from("/nowhere")).view;
    let snapshot = Snapshot {
        root: Some(PathBuf::from("/nowhere/at/all")),
        view: Some(SnapshotView::Waiting(waiting.clone())),
        explorer_width: None,
        window: None,
    };
    let saved = snapshot.saved();
    assert_eq!(saved.view, waiting);
    assert_eq!(saved.root, Some(PathBuf::from("/nowhere/at/all")));
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
            root: Some(dir.path().to_owned()),
            view: Some(SnapshotView::Live(
                explorer.snapshot(),
                FolderView::default(),
            )),
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
        restored.restore_saved_selection(&back.view.selected);
        assert_eq!(restored.selection(), explorer.selection());
        eprintln!(
            "{case}: {checked} checked, {} folder and {} file entries, {} bytes; \
             UI thread {ui:.2?}, named {named:.2?}, encoded {encoded:.2?}, written {written:.2?}; \
             read back {read:.2?}",
            saved.view.selected.folders.len(),
            saved.view.selected.files.len(),
            bytes.len(),
        );
    }
}
