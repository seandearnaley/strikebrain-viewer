//! Listing only the directories the person opens during a slow initial scan.
//! The full scan owns search, saved views and overlap resolution. Both use
//! the ordinary explorer tree and its path-based handoff.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{DirectoryOutline, NodeId, ScanOptions, Tree};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::{AppContext as _, Context, Result, Task};

use super::AppState;

/// One serial reader per slow initial scan. Dropping it also cancels a
/// listing that is already executing on a worker thread.
pub(super) struct Browse {
    requests: UnboundedSender<PathBuf>,
    cancel: Arc<AtomicBool>,
    _task: Task<Result<()>>,
}

impl Drop for Browse {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl AppState {
    pub(super) fn start_browsing(&mut self, key: u64, generation: u64, cx: &mut Context<Self>) {
        let Some(folder) = self.folder(key) else {
            return;
        };
        let root = folder.path.clone();
        // A full tree already on screen stays intact on Refresh. A saved
        // whole-folder check needs all its descendants to restore.
        if folder.file
            || folder.shown
            || self.folder_node(&root).is_some()
            || folder
                .waiting
                .as_ref()
                .is_some_and(|saved| !saved.selection.folders.is_empty())
            || self
                .folders
                .iter()
                .any(|other| other.key != key && other.path.starts_with(&root))
        {
            return;
        }
        let options = ScanOptions {
            show_ignored: self.show_ignored,
        };
        let (requests, mut paths) = unbounded::<PathBuf>();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let saved = folder.waiting.clone();
        let task = cx.spawn(async move |this, cx| {
            let initial_cancel = Arc::clone(&stop);
            let (mut outline, first) = match cx
                .background_spawn(async move {
                    DirectoryOutline::open(&root, options, saved.as_ref(), &initial_cancel)
                })
                .await
            {
                Ok(outline) => outline,
                Err(err) => {
                    return this.update(cx, |this, cx| {
                        this.outline_error(key, generation, &err.to_string(), cx);
                    });
                }
            };
            this.update(cx, |this, cx| {
                this.show_outline(key, generation, &first, cx);
            })?;
            drop(first);
            while let Some(path) = paths.next().await {
                let stop = stop.clone();
                let (updated, tree) = cx
                    .background_spawn(async move {
                        let tree = outline.read(&path, &stop);
                        (outline, tree)
                    })
                    .await;
                outline = updated;
                match tree {
                    Ok(None) => {}
                    Ok(Some(tree)) => {
                        this.update(cx, |this, cx| this.show_outline(key, generation, &tree, cx))?;
                    }
                    Err(err) => {
                        return this.update(cx, |this, cx| {
                            this.outline_error(key, generation, &err.to_string(), cx);
                        });
                    }
                }
            }
            Ok(())
        });
        if let Some(folder) = self.folder_mut(key) {
            folder.browse = Some(Browse {
                requests,
                cancel,
                _task: task,
            });
        }
    }

    fn outline_error(&mut self, key: u64, generation: u64, message: &str, cx: &mut Context<Self>) {
        if let Some(folder) = self
            .folder_mut(key)
            .filter(|folder| folder.generation == generation && folder.browse.is_some())
        {
            folder.error = Some(message.to_owned().into());
            folder.browse = None;
            cx.notify();
        }
    }

    fn show_outline(&mut self, key: u64, generation: u64, tree: &Tree, cx: &mut Context<Self>) {
        let Some(folder) = self.folder(key) else {
            return;
        };
        let root = tree.root_path();
        if folder.generation != generation
            || folder.browse.is_none()
            || self
                .folders
                .iter()
                .any(|other| other.key != key && tree.find_abs(&other.path).is_some())
        {
            return;
        }
        let first = self.folder_node(root).is_none();
        let before = self.before();
        match self.install_folder(key, root, tree, false) {
            Ok(map) => {
                if first {
                    if let Some(saved) = self
                        .folder_mut(key)
                        .and_then(|folder| folder.waiting.take())
                    {
                        self.restore_folder(&saved);
                    }
                    if let Some(top) = self.folder_node(root) {
                        self.reveal(top, cx);
                    }
                } else {
                    self.landed_keeping(before, &map, cx);
                }
                cx.notify();
            }
            Err(err) => self.outline_error(key, generation, &err.to_string(), cx),
        }
    }

    /// A person navigating the provisional tree has chosen their place.
    /// Finishing a launch must not take them back to its saved row or file.
    pub(super) fn use_browsed_row(&mut self, id: NodeId) {
        if self.restore.is_some()
            && self
                .explorer
                .as_ref()
                .is_some_and(|e| e.tree().is_partial(id))
        {
            self.restore = None;
            self.restore_top = None;
        }
    }

    /// The normal disclosure opens now; its children arrive from this reader
    /// or the full index, whichever gets there first.
    pub(super) fn read_open_directory(&self, id: NodeId) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        let tree = explorer.tree();
        if !tree.is_partial(id) || !explorer.is_open(id) || !tree.children(id).is_empty() {
            return;
        }
        let Some(top) = tree.folder_of(id) else {
            return;
        };
        let root = tree.abs_path(top);
        if let Some(browse) = self
            .folders
            .iter()
            .find(|folder| folder.path == root)
            .and_then(|folder| folder.browse.as_ref())
        {
            browse
                .requests
                .unbounded_send(tree.abs_path(id))
                .expect("a live directory reader owns its receiver");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use explorer::{ScanOptions, scan};
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::state::{Launch, Motion};

    /// Hold back the recursive result, then drive the ordinary slow-scan
    /// signal. The directory reader runs against the real fixture.
    fn browsing(
        cx: &mut TestAppContext,
        root: &std::path::Path,
    ) -> (gpui_kit::Entity<AppState>, u64) {
        let state = cx.new(|cx| AppState::new(Launch::Folders(vec![root.to_owned()]), cx));
        let generation = state.update(cx, |state, _| {
            state.folders[0].scan_task = None;
            state.folders[0].generation
        });
        cx.executor().advance_clock(super::super::scan::SLOW_SCAN);
        cx.run_until_parked();
        (state, generation)
    }

    #[gpui_kit::test]
    fn a_slow_folder_can_be_browsed_and_checked_before_its_full_index_lands(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("art/more")).unwrap();
        fs::write(dir.path().join("art/demo.txt"), "demo").unwrap();
        fs::write(dir.path().join("art/more/later.txt"), "").unwrap();
        let path = explorer::canonical(dir.path()).unwrap();
        let (state, generation) = browsing(cx, &path);
        state.update(cx, |state, cx| {
            let explorer = state.explorer().unwrap();
            assert_eq!(explorer.rows().len(), 2);
            assert!(
                explorer
                    .tree()
                    .find_abs(&path.join("art/demo.txt"))
                    .is_none()
            );
            state.move_focus(Motion::Down(1), cx);
            let directory = state.focus().unwrap();
            assert!(!state.explorer().unwrap().can_check(directory));
            state.toggle_focus_check(cx);
            assert_eq!(state.selected_count(), 0);
            state.expand(cx);
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            state.move_focus(Motion::Last, cx);
            state.toggle_focus_check(cx);
            let file = path.join("art/demo.txt");
            let current = |state: &AppState| {
                state
                    .explorer()
                    .unwrap()
                    .tree()
                    .abs_path(state.current().unwrap())
            };
            assert_eq!(current(state), file);
            let saved = state.take_snapshot().named();
            assert_eq!(saved.folders[0].selection.files, ["art/demo.txt"]);
            let complete = scan(&path, ScanOptions::default(), &AtomicBool::new(false)).unwrap();
            state.finish_scan(0, generation, Ok(Arc::new(complete)), cx);
            assert_eq!(current(state), file);
            assert_eq!(state.selected_count(), 1);
            assert_eq!(state.focus(), state.current());
            assert!(
                state
                    .explorer()
                    .unwrap()
                    .tree()
                    .find_abs(&path.join("art/more/later.txt"))
                    .is_some()
            );
        });
    }

    #[gpui_kit::test]
    fn removing_a_browsable_folder_rejects_both_its_late_results(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        let root = explorer::canonical(dir.path()).unwrap();
        let (state, generation) = browsing(cx, &root);
        state.update(cx, |state, cx| {
            let top = state.folder_node(&root).unwrap();
            state.remove_folder(top, cx);
            let tree = scan(&root, ScanOptions::default(), &AtomicBool::new(false)).unwrap();
            state.show_outline(0, generation, &tree, cx);
            state.finish_scan(0, generation, Ok(Arc::new(tree)), cx);
            assert!(state.is_empty());
        });
    }
    #[gpui_kit::test]
    fn right_arrow_keeps_the_chosen_directory_when_a_launch_finishes(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("art")).unwrap();
        let root = explorer::canonical(dir.path()).unwrap();
        let (state, generation) = browsing(cx, &root);
        state.update(cx, |state, cx| {
            state.expand(cx); // Right from the open root focuses its first child.
            let tree = scan(&root, ScanOptions::default(), &AtomicBool::new(false)).unwrap();
            state.finish_scan(0, generation, Ok(Arc::new(tree)), cx);
            assert_eq!(
                state
                    .explorer()
                    .unwrap()
                    .tree()
                    .abs_path(state.focus().unwrap()),
                root.join("art")
            );
        });
    }
}
