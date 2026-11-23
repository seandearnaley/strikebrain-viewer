//! Engine benchmarks, one group per row of the design's Budgets table that
//! the engine owns, plus the costs around them that have no budget.
//!
//! `just bench` runs them; `just bench filter` runs one group. Set
//! `EXPLORER_SCAN_ROOT` to a large folder (the reference corpus is a
//! brainstrike checkout) to scan it: without it the open group scans a
//! generated 6,000-file folder and skips the ignored-files rows.
//!
//! | Group | Budget |
//! | --- | --- |
//! | `open` | 6,000 files on screen in 100 ms; 1.65 M entries with ignored shown in 3 s |
//! | `filter` | keystroke to updated rows on 1 M names in 16 ms |
//! | `check` | check or clear a folder of 1 M files in 1 ms |
//! | `rows` | expand or collapse with 1 M visible rows in 16 ms |
//! | `folders` | a 6,000-file folder added to a 1 M-entry tree, its rows on screen, in 100 ms |
//! | `other` | none: sort, tree replacement, gallery questions (each first one after a change remakes the gallery list) |

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use criterion::{BatchSize, Criterion, SamplingMode, criterion_group, criterion_main};
use explorer::{
    Direction, Explorer, KindFilter, NodeId, ScanOptions, Sort, SortKey, Tree, read_sizes, scan,
};

/// Extensions the synthetic files cycle through.
const EXTENSIONS: [&str; 5] = ["txt", "rs", "png", "md", "json"];

/// 10 top folders, 100 folders in each, 1,000 files in each of those:
/// 1,001,011 nodes, 1,000,000 of them files.
fn million() -> Arc<Tree> {
    let mut owned = Vec::with_capacity(1_000_000);
    for top in 0..10 {
        for mid in 0..100 {
            for file in 0..1000_u64 {
                let ext =
                    EXTENSIONS[usize::try_from(file).expect("under 1,000") % EXTENSIONS.len()];
                owned.push((format!("Top {top}/dir{mid}/file{file}.{ext}"), file));
            }
        }
    }
    let paths: Vec<(&str, u64)> = owned.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    Arc::new(Tree::from_paths_with_sizes(&paths).expect("the paths are valid and distinct"))
}

/// The folder named by `EXPLORER_SCAN_ROOT`, if set.
fn scan_root() -> Option<PathBuf> {
    std::env::var_os("EXPLORER_SCAN_ROOT").map(PathBuf::from)
}

/// A temporary folder of 6,000 files in 60 folders of 3 levels.
fn generated_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary folder");
    for a in 0..6 {
        for b in 0..10 {
            let folder = dir.path().join(format!("pkg{a}/mod{b}"));
            std::fs::create_dir_all(&folder).expect("a folder in the temporary folder");
            for f in 0..100 {
                std::fs::write(folder.join(format!("file{f}.rs")), "fn main() {}\n")
                    .expect("a file in the temporary folder");
            }
        }
    }
    dir
}

fn listed(root: &Path, show_ignored: bool) -> Tree {
    scan(root, ScanOptions { show_ignored }, &AtomicBool::new(false))
        .expect("EXPLORER_SCAN_ROOT names a readable folder")
}

fn open(c: &mut Criterion) {
    let mut group = c.benchmark_group("open");
    let generated;
    let small = if let Some(root) = scan_root() {
        root
    } else {
        generated = generated_folder();
        generated.path().to_path_buf()
    };
    group.bench_function("scan and show, default rules", |b| {
        b.iter(|| Explorer::new(Arc::new(listed(&small, false))));
    });
    group.bench_function("read sizes, default rules", |b| {
        b.iter_batched(
            || listed(&small, false),
            |tree| read_sizes(&tree, &AtomicBool::new(false)).expect("never cancelled"),
            BatchSize::PerIteration,
        );
    });
    if let Some(root) = scan_root() {
        // Seconds per run: ten runs of one iteration each.
        group
            .sample_size(10)
            .sampling_mode(SamplingMode::Flat)
            .measurement_time(Duration::from_secs(80));
        group.bench_function("scan and show, ignored shown", |b| {
            b.iter(|| Explorer::new(Arc::new(listed(&root, true))));
        });
        group.bench_function("read sizes, ignored shown", |b| {
            b.iter_batched(
                || listed(&root, true),
                |tree| read_sizes(&tree, &AtomicBool::new(false)).expect("never cancelled"),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

fn filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter");
    let mut explorer = Explorer::new(million());
    for (name, text, kind) in [
        ("common term", "file1", KindFilter::All),
        ("rare term", "file999.", KindFilter::All),
        ("two terms", "file1 .png", KindFilter::All),
        ("exclude, most rows open", "-dir1", KindFilter::All),
        ("images", "", KindFilter::Images),
        ("clear", "", KindFilter::All),
    ] {
        group.bench_function(format!("1M names, {name}"), |b| {
            b.iter(|| explorer.set_filter(black_box(text), kind));
        });
    }
    if let Some(root) = scan_root() {
        let mut real = Explorer::new(Arc::new(listed(&root, true)));
        for text in ["index", "test .ts", "-node_modules"] {
            group.bench_function(format!("real names, {text}"), |b| {
                b.iter(|| real.set_filter(black_box(text), KindFilter::All));
            });
        }
    }
    group.finish();
}

fn check(c: &mut Criterion) {
    let mut group = c.benchmark_group("check");
    let mut explorer = Explorer::new(million());
    let top = explorer
        .tree()
        .find("Top 0")
        .expect("the synthetic tree has it");
    group.bench_function("root of 1M files, check then clear", |b| {
        b.iter(|| explorer.toggle_check(black_box(NodeId::ROOT)));
    });
    group.bench_function("folder of 100k files, check then clear", |b| {
        b.iter(|| explorer.toggle_check(black_box(top)));
    });
    explorer.set_filter("file1", KindFilter::All);
    group.bench_function("root of 1M files under a filter", |b| {
        b.iter(|| explorer.toggle_check(black_box(NodeId::ROOT)));
    });
    group.bench_function("root state of 1M files under a filter", |b| {
        b.iter(|| explorer.check(black_box(NodeId::ROOT)));
    });
    group.finish();
}

fn rows(c: &mut Criterion) {
    let mut group = c.benchmark_group("rows");
    group.sample_size(50);
    let mut explorer = Explorer::new(million());
    explorer.expand_all();
    assert_eq!(explorer.rows().len(), 1_001_010);
    let top = explorer
        .tree()
        .find("Top 0")
        .expect("the synthetic tree has it");
    group.bench_function("toggle a folder, 1M rows", |b| {
        b.iter(|| explorer.toggle_folder(black_box(top)));
    });
    group.bench_function("expand all, 1M rows", |b| {
        b.iter(|| explorer.expand_all());
    });
    group.bench_function("collapse all", |b| {
        b.iter(|| explorer.collapse_all());
    });
    explorer.set_sort(Sort {
        key: SortKey::Type,
        direction: Direction::Ascending,
    });
    explorer.expand_all();
    group.bench_function("toggle a folder, 1M rows, type sort", |b| {
        b.iter(|| explorer.toggle_folder(black_box(top)));
    });
    group.finish();
}

/// Adding, removing and rescanning one small folder beside a million
/// entries. The scan has landed, with its sizes: what is timed is joining
/// the trees, carrying the view across and rebuilding the rows.
fn folders(c: &mut Criterion) {
    let mut group = c.benchmark_group("folders");
    group.sample_size(20);
    let generated = generated_folder();
    let small = listed(generated.path(), false);
    read_sizes(&small, &AtomicBool::new(false)).expect("never cancelled");
    let mut base = Explorer::empty();
    base.add_folder(&million(), 0, None)
        .expect("a million entries fit");
    let large = Arc::clone(base.tree());
    base.add_folder(&small, 1, None)
        .expect("a million entries and a few more fit");
    let both = Arc::clone(base.tree());
    let small_top = both.children(NodeId::ROOT)[1];

    // Each run starts from a fresh view of the same tree; making it is
    // not timed, and neither is dropping the result.
    let fresh = |tree: &Arc<Tree>, open: bool| {
        let mut explorer = Explorer::new(Arc::clone(tree));
        if open {
            explorer.expand_all();
            explorer.select_all();
        }
        explorer
    };
    for (name, open) in [("closed", false), ("1M rows open and checked", true)] {
        group.bench_function(format!("add 6,000 files to 1M, {name}"), |b| {
            b.iter_batched(
                || fresh(&large, open),
                |mut explorer| {
                    explorer
                        .add_folder(&small, 1, None)
                        .expect("a million entries and a few more fit");
                    explorer
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.bench_function("rescan 6,000 files beside 1M", |b| {
        b.iter_batched(
            || fresh(&both, false),
            |mut explorer| {
                explorer
                    .replace_folder(small_top, &small, None)
                    .expect("the same tree fits again");
                explorer
            },
            BatchSize::PerIteration,
        );
    });
    group.bench_function("remove 6,000 files beside 1M", |b| {
        b.iter_batched(
            || fresh(&both, false),
            |mut explorer| {
                explorer.remove_folder(small_top, None);
                explorer
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

fn other(c: &mut Criterion) {
    let mut group = c.benchmark_group("other");
    group.sample_size(20);
    let tree = million();
    let mut explorer = Explorer::new(Arc::clone(&tree));
    for (name, key) in [
        ("name", SortKey::Name),
        ("type", SortKey::Type),
        ("size", SortKey::Size),
    ] {
        // Alternates directions, so every call sorts.
        let mut direction = Direction::Descending;
        group.bench_function(format!("sort 1M by {name}"), |b| {
            b.iter(|| {
                direction = direction.flipped();
                explorer.set_sort(Sort { key, direction });
            });
        });
    }

    explorer.set_sort(Sort::default());
    explorer.select_all();
    explorer.expand_all();
    let twin = million();
    let mut next = [Arc::clone(&twin), Arc::clone(&tree)].into_iter().cycle();
    group.bench_function("replace a 1M tree, all checked and open", |b| {
        b.iter(|| explorer.replace_tree(next.next().expect("the cycle never ends")));
    });

    explorer.clear_selection();
    let first = explorer
        .tree()
        .find("Top 0/dir0/file0.txt")
        .expect("the synthetic tree has it");
    let last = explorer
        .tree()
        .find("Top 9/dir99/file999.json")
        .expect("the synthetic tree has it");
    explorer.toggle_check(first);
    explorer.toggle_check(last);
    for (name, sort) in [
        ("name sort", Sort::default()),
        (
            "type sort",
            Sort {
                key: SortKey::Type,
                direction: Direction::Ascending,
            },
        ),
    ] {
        explorer.set_sort(sort);
        group.bench_function(format!("gallery next across 1M, {name}"), |b| {
            b.iter(|| explorer.gallery_next(black_box(first)));
        });
        group.bench_function(format!("gallery position across 1M, {name}"), |b| {
            b.iter(|| explorer.gallery_position(black_box(last)));
        });
    }
    // The first question after a click remakes the gallery list.
    explorer.select_all();
    group.bench_function("click, then gallery position, 1M checked, type sort", |b| {
        b.iter(|| {
            explorer.toggle_check(black_box(first));
            explorer.gallery_position(black_box(last))
        });
    });
    group.finish();
}

criterion_group!(benches, open, filter, check, rows, folders, other);
criterion_main!(benches);
