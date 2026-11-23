//! Property tests: the explorer after random steps, against models that
//! share none of its state.

use std::collections::BTreeSet;
use std::sync::Arc;

use proptest::prelude::*;

use crate::sort::DisplayOrder;
use crate::tree::tests::{path_sets, tree_of};
use crate::{
    Direction, Explorer, FilterJob, Kind, KindFilter, NodeId, Query, Sort, SortKey, Tree,
    Visibility, filter,
};

/// One step a user can take. Indices pick a node modulo the node count.
#[derive(Clone, Debug)]
enum Step {
    Folder(usize),
    SetOpen(usize, bool),
    Check(usize),
    Filter(&'static str, KindFilter),
    Sort(SortKey, Direction),
    ExpandAll,
    CollapseAll,
    SelectAll,
    Reveal(usize),
    /// Asks the gallery questions the stage asks on every render, so any
    /// cached answer is made mid-sequence and must not go stale.
    AskGallery(usize),
}

const KINDS: [KindFilter; 3] = [KindFilter::All, KindFilter::Images, KindFilter::Text];
const TEXTS: [&str; 9] = ["", "src", "a", ".md", "-src", "e b", "-x.d", "2", "k"];

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let kind = prop::sample::select(&KINDS[..]);
    let key = prop::sample::select(&[SortKey::Name, SortKey::Type, SortKey::Size][..]);
    let direction = prop::sample::select(&[Direction::Ascending, Direction::Descending][..]);
    let text = prop::sample::select(&TEXTS[..]);
    let step = prop_oneof![
        3 => (0_usize..80).prop_map(Step::Folder),
        2 => (0_usize..80, any::<bool>()).prop_map(|(i, open)| Step::SetOpen(i, open)),
        3 => (0_usize..80).prop_map(Step::Check),
        2 => (text, kind).prop_map(|(t, k)| Step::Filter(t, k)),
        1 => (key, direction).prop_map(|(k, d)| Step::Sort(k, d)),
        1 => Just(Step::ExpandAll),
        1 => Just(Step::CollapseAll),
        1 => Just(Step::SelectAll),
        1 => (0_usize..80).prop_map(Step::Reveal),
        2 => (0_usize..80).prop_map(Step::AskGallery),
    ];
    prop::collection::vec(step, 0..12)
}

fn node(tree: &Tree, index: usize) -> NodeId {
    NodeId::new(u32::try_from(index % tree.node_count()).unwrap())
}

/// The open state as the design words it, kept apart from the explorer:
/// the folders the user opened, the folders closed by hand under the
/// current filter, and the filter's result computed afresh.
struct Model {
    tree: Arc<Tree>,
    /// Folders the user opened; a filter leaves them alone.
    stored: BTreeSet<NodeId>,
    /// Folders closed by hand under the filter shown.
    collapsed: BTreeSet<NodeId>,
    /// The query text and kind shown.
    applied: (&'static str, KindFilter),
    visibility: Visibility,
}

impl Model {
    fn new(tree: Arc<Tree>) -> Self {
        Self {
            tree,
            stored: BTreeSet::new(),
            collapsed: BTreeSet::new(),
            applied: ("", KindFilter::All),
            visibility: Visibility::all(),
        }
    }

    fn filtered(&self) -> bool {
        self.visibility.is_active()
    }

    /// A readable folder; under a filter, one with something visible below.
    fn can_open(&self, id: NodeId) -> bool {
        if id == NodeId::ROOT {
            false
        } else if self.filtered() {
            self.visibility.opens(id)
        } else {
            self.tree.kind(id) == Kind::Dir
        }
    }

    /// The root always shows its children: they are the top rows.
    fn is_open(&self, id: NodeId) -> bool {
        if id == NodeId::ROOT {
            true
        } else if self.filtered() {
            self.visibility.opens(id) && !self.collapsed.contains(&id)
        } else {
            self.tree.kind(id) == Kind::Dir && self.stored.contains(&id)
        }
    }

    fn set_open(&mut self, id: NodeId, open: bool) {
        if !self.can_open(id) {
            return;
        }
        let (set, add) = if self.filtered() {
            (&mut self.collapsed, !open)
        } else {
            (&mut self.stored, open)
        };
        if add {
            set.insert(id);
        } else {
            set.remove(&id);
        }
    }

    fn step(&mut self, step: &Step) {
        let tree = Arc::clone(&self.tree);
        let at = |i| node(&tree, i);
        match *step {
            Step::Folder(i) => {
                let open = self.is_open(at(i));
                self.set_open(at(i), !open);
            }
            Step::SetOpen(i, open) => self.set_open(at(i), open),
            Step::Filter(text, kind) => {
                self.visibility = filter(&tree, &Query::parse(text), kind);
                if (text, kind) != self.applied {
                    self.collapsed.clear();
                    self.applied = (text, kind);
                }
            }
            Step::ExpandAll if self.filtered() => self.collapsed.clear(),
            Step::ExpandAll => {
                self.stored = tree
                    .ids()
                    .filter(|&id| id != NodeId::ROOT && tree.kind(id) == Kind::Dir)
                    .collect();
            }
            Step::CollapseAll if self.filtered() => {
                self.collapsed = tree.ids().filter(|&id| tree.kind(id).is_folder()).collect();
            }
            Step::CollapseAll => self.stored.clear(),
            Step::Reveal(i) => {
                let target = at(i);
                if target != NodeId::ROOT && self.visibility.is_visible(target) {
                    let mut above = tree.parent(target);
                    while let Some(folder) = above.filter(|&p| p != NodeId::ROOT) {
                        if self.filtered() {
                            self.collapsed.remove(&folder);
                        } else {
                            self.stored.insert(folder);
                        }
                        above = tree.parent(folder);
                    }
                }
            }
            Step::Check(_) | Step::Sort(..) | Step::SelectAll | Step::AskGallery(_) => {}
        }
    }

    /// The rows by recursion: a child shows when visible, and its children
    /// follow when the model says it is open.
    fn rows(&self, order: &DisplayOrder, folder: NodeId, out: &mut Vec<NodeId>) {
        for &child in order.children(&self.tree, folder) {
            if self.visibility.is_visible(child) {
                out.push(child);
                if self.is_open(child) {
                    self.rows(order, child, out);
                }
            }
        }
    }
}

/// Plays `steps` on the explorer and on a model of its open state.
fn play(explorer: &mut Explorer, steps: &[Step]) -> Model {
    let tree = Arc::clone(explorer.tree());
    let mut model = Model::new(Arc::clone(&tree));
    for step in steps {
        let at = |i| node(&tree, i);
        match *step {
            Step::Folder(i) => explorer.toggle_folder(at(i)),
            Step::SetOpen(i, open) => explorer.set_open(at(i), open),
            Step::Check(i) => explorer.toggle_check(at(i)),
            Step::Filter(text, kind) => explorer.set_filter(text, kind),
            Step::Sort(key, direction) => explorer.set_sort(Sort { key, direction }),
            Step::ExpandAll => explorer.expand_all(),
            Step::CollapseAll => explorer.collapse_all(),
            Step::SelectAll => explorer.select_all(),
            Step::Reveal(i) => {
                if let Some(row) = explorer.reveal(at(i)) {
                    assert_eq!(explorer.rows()[row], at(i));
                }
            }
            Step::AskGallery(i) => {
                std::hint::black_box((
                    explorer.gallery_position(at(i)),
                    explorer.gallery_next(at(i)),
                    explorer.gallery_previous(at(i)),
                    explorer.gallery().count(),
                ));
            }
        }
        model.step(step);
    }
    model
}

/// Every checked file, depth first over the display order.
fn naive_gallery(explorer: &Explorer, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    if explorer.is_selected(node) {
        out.push(node);
    }
    for &child in order.children(explorer.tree(), node) {
        naive_gallery(explorer, order, child, out);
    }
}

fn naive_pre_order(explorer: &Explorer, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    out.push(node);
    for &child in order.children(explorer.tree(), node) {
        naive_pre_order(explorer, order, child, out);
    }
}

/// What happens to filter jobs and trees, in any order.
#[derive(Clone, Debug)]
enum Op {
    /// Starts a job and keeps it.
    Job(&'static str, KindFilter),
    /// Runs a kept job, possibly again, and applies its result.
    Run(usize),
    SetFilter(&'static str, KindFilter),
    /// Swaps in one of the generated trees.
    Replace(usize),
    /// Applies a result made by another explorer.
    Foreign(&'static str),
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let kind = prop::sample::select(&KINDS[..]);
    let text = prop::sample::select(&TEXTS[..]);
    let op = prop_oneof![
        3 => (text.clone(), kind.clone()).prop_map(|(t, k)| Op::Job(t, k)),
        4 => (0_usize..8).prop_map(Op::Run),
        1 => (text.clone(), kind).prop_map(|(t, k)| Op::SetFilter(t, k)),
        2 => (0_usize..3).prop_map(Op::Replace),
        1 => text.prop_map(Op::Foreign),
    ];
    prop::collection::vec(op, 0..16)
}

/// A kept job: the filter it runs and the step that made it.
struct Kept {
    text: &'static str,
    kind: KindFilter,
    made_at: usize,
    job: FilterJob,
}

proptest! {
    #[test]
    fn rows_match_a_model_of_the_open_folders(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        let model = play(&mut explorer, &steps);
        let order = DisplayOrder::new(explorer.tree(), explorer.sort());
        let mut expected = Vec::new();
        model.rows(&order, NodeId::ROOT, &mut expected);
        prop_assert_eq!(explorer.rows(), &expected[..]);
        for id in explorer.tree().ids() {
            prop_assert_eq!(explorer.is_open(id), model.is_open(id), "{:?}", id);
            prop_assert_eq!(explorer.can_open(id), model.can_open(id), "{:?}", id);
        }
        let stored: Vec<String> = model.stored.iter().map(|&id| model.tree.path(id)).collect();
        prop_assert_eq!(explorer.open_paths().collect::<Vec<_>>(), stored);
        for (index, &id) in explorer.rows().iter().enumerate() {
            let row = explorer.row(index).unwrap();
            prop_assert_eq!(
                (row.id, row.open, row.can_open, row.check),
                (id, model.is_open(id), model.can_open(id), explorer.check(id))
            );
        }
    }

    #[test]
    fn the_gallery_matches_a_naive_walk(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        play(&mut explorer, &steps);
        let order = DisplayOrder::new(explorer.tree(), explorer.sort());
        let mut expected = Vec::new();
        naive_gallery(&explorer, &order, NodeId::ROOT, &mut expected);
        let got: Vec<NodeId> = explorer.gallery().collect();
        prop_assert_eq!(&got, &expected);
        prop_assert_eq!(explorer.gallery_len(), expected.len());
        let mut all = Vec::new();
        naive_pre_order(&explorer, &order, NodeId::ROOT, &mut all);
        for (at, &node) in all.iter().enumerate() {
            prop_assert_eq!(
                explorer.gallery_position(node),
                expected.iter().position(|&file| file == node)
            );
            let after = all[at + 1..].iter().copied().find(|&n| explorer.is_selected(n));
            let before = all[..at].iter().rev().copied().find(|&n| explorer.is_selected(n));
            prop_assert_eq!(explorer.gallery_next(node), after);
            prop_assert_eq!(explorer.gallery_previous(node), before);
        }
    }

    #[test]
    fn replacing_the_tree_carries_checks_and_open_folders_by_path(
        old in path_sets(),
        new in path_sets(),
        picks in prop::collection::vec(any::<bool>(), 200),
        (text, kind) in (prop::sample::select(&TEXTS[..]), prop::sample::select(&KINDS[..])),
    ) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&old)));
        let tree = Arc::clone(explorer.tree());
        for id in tree.ids().filter(|id| picks.get(id.index()).copied().unwrap_or(false)) {
            if tree.kind(id).is_folder() {
                explorer.set_open(id, true);
            } else {
                explorer.toggle_check(id);
            }
        }
        // A filter shown at the time leaves the stored state alone.
        explorer.set_filter(text, kind);
        let open: BTreeSet<String> = explorer.open_paths().collect();
        let checked: BTreeSet<String> = explorer.selected_paths().collect();
        let full_folders = explorer.snapshot().selection().folders;

        explorer.replace_tree(Arc::new(tree_of(&new)));
        let tree = explorer.tree();
        let kept = |paths: &BTreeSet<String>, wanted: fn(Kind) -> bool| -> BTreeSet<String> {
            paths
                .iter()
                .filter(|path| tree.find(path).is_some_and(|id| wanted(tree.kind(id))))
                .cloned()
                .collect()
        };
        prop_assert_eq!(
            explorer.open_paths().collect::<BTreeSet<_>>(),
            kept(&open, |kind| kind == Kind::Dir)
        );
        // The checked files that are still files, and every file now in a
        // folder that was fully checked, as a saved state restores it.
        let mut expected = kept(&checked, Kind::is_file);
        for folder in &full_folders {
            let found = if folder.is_empty() { Some(NodeId::ROOT) } else { tree.find(folder) };
            for id in found.filter(|&id| !tree.is_file(id)).map(|id| tree.subtree(id)).into_iter().flatten() {
                let node = NodeId::from_index(id);
                if tree.is_file(node) {
                    expected.insert(tree.path(node));
                }
            }
        }
        prop_assert_eq!(explorer.selected_paths().collect::<BTreeSet<_>>(), expected);
        prop_assert_eq!(
            explorer.visibility(),
            &filter(tree, &Query::parse(text), kind)
        );
    }

    #[test]
    fn saved_paths_restore_the_same_view(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        play(&mut explorer, &steps);
        explorer.set_filter("", KindFilter::All);
        let mut restored = Explorer::new(Arc::new(tree_of(&set)));
        restored.set_sort(explorer.sort());
        restored.restore_open(explorer.open_paths());
        restored.restore_selection(explorer.selected_paths());
        prop_assert_eq!(restored.rows(), explorer.rows());
        prop_assert_eq!(
            restored.gallery().collect::<Vec<_>>(),
            explorer.gallery().collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_snapshot_restores_the_same_view(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        play(&mut explorer, &steps);
        let snapshot = explorer.snapshot();
        let saved = snapshot.selection();
        let mut restored = Explorer::new(Arc::new(tree_of(&set)));
        restored.set_sort(explorer.sort());
        restored.restore_open(snapshot.open_paths());
        restored.restore_saved_selection(&saved);
        prop_assert_eq!(restored.selection(), explorer.selection());
        prop_assert_eq!(snapshot.open_paths(), explorer.open_paths().collect::<Vec<_>>());
        // Canonical: no entry lies inside a saved folder, and a folder is
        // listed only when every one of its files is checked.
        let tree = restored.tree();
        for folder in &saved.folders {
            let id = tree.find(folder).unwrap_or(NodeId::ROOT);
            let range = tree.subtree(id);
            prop_assert!(range.clone().filter(|&i| tree.files().contains(i)).all(|i| explorer.selection().is_selected(NodeId::new(u32::try_from(i).unwrap()))));
            for other in saved.folders.iter().chain(&saved.files).filter(|&p| p != folder) {
                let inner = tree.find(other).unwrap_or(NodeId::ROOT).index();
                prop_assert!(!(range.start < inner && inner < range.end), "{other} lies inside {folder}");
            }
        }
    }

    #[test]
    fn filter_jobs_and_tree_swaps_interleave_without_stale_results(
        sets in prop::collection::vec(path_sets(), 1..4),
        ops in ops(),
    ) {
        let trees: Vec<Arc<Tree>> = sets.iter().map(|set| Arc::new(tree_of(set))).collect();
        let mut explorer = Explorer::new(Arc::clone(&trees[0]));
        let mut other = Explorer::new(Arc::new(Tree::from_paths(&["x/1.txt", "y.txt"]).unwrap()));
        let mut kept: Vec<Kept> = Vec::new();
        // The step after which only the newest job's result may apply.
        let mut newest = 0;
        for (at, op) in ops.iter().enumerate() {
            match *op {
                Op::Job(text, kind) => {
                    newest = at;
                    let job = explorer.filter_job(text, kind);
                    kept.push(Kept { text, kind, made_at: at, job });
                }
                Op::Run(k) if !kept.is_empty() => {
                    let run = &kept[k % kept.len()];
                    let current = run.made_at == newest;
                    match run.job.run() {
                        None => prop_assert!(!current, "a current job gave up"),
                        Some(result) => {
                            prop_assert_eq!(explorer.apply_filter(result), current);
                            if current {
                                let expected =
                                    filter(explorer.tree(), &Query::parse(run.text), run.kind);
                                prop_assert_eq!(explorer.visibility(), &expected);
                            }
                        }
                    }
                }
                Op::Run(_) => {}
                Op::SetFilter(text, kind) => {
                    newest = at;
                    explorer.set_filter(text, kind);
                }
                Op::Replace(k) => {
                    newest = at;
                    explorer.replace_tree(Arc::clone(&trees[k % trees.len()]));
                }
                Op::Foreign(text) => {
                    if let Some(result) = other.filter_job(text, KindFilter::All).run() {
                        prop_assert!(!explorer.apply_filter(result));
                    }
                }
            }
            // Everything the explorer shows belongs to its current tree.
            let n = explorer.tree().node_count();
            prop_assert!(explorer.visibility().universe().is_none_or(|u| u.len() == n));
            for index in 0..explorer.rows().len() {
                std::hint::black_box(explorer.row(index));
            }
        }
    }
}
