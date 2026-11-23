use std::path::PathBuf;

use proptest::prelude::*;

use super::{KindFilter, Query, Visibility, filter};
use crate::tree::RawTree;
use crate::tree::tests::{path_sets, tree_of};
use crate::{Kind, NodeId, Tree};

fn id(tree: &Tree, path: &str) -> NodeId {
    tree.find(path)
        .unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// The visible nodes' paths, root left out, in id order.
fn visible(tree: &Tree, visibility: &Visibility) -> Vec<String> {
    tree.ids()
        .skip(1)
        .filter(|&node| visibility.is_visible(node))
        .map(|node| tree.path(node))
        .collect()
}

fn run(paths: &[&str], text: &str, kind: KindFilter) -> (Tree, Visibility) {
    let tree = Tree::from_paths(paths).unwrap();
    let visibility = filter(&tree, &Query::parse(text), kind);
    (tree, visibility)
}

fn terms(list: &[&str]) -> Vec<String> {
    list.iter().map(|&term| term.to_owned()).collect()
}

#[test]
fn a_query_splits_on_whitespace_and_lowercases() {
    let query = Query::parse("  Foo\tBAR\nbaz  ");
    assert_eq!(query.include(), terms(&["foo", "bar", "baz"]));
    assert_eq!(query.exclude(), terms(&[]));
    assert!(!query.is_empty());
}

#[test]
fn a_dash_term_longer_than_one_character_excludes() {
    let query = Query::parse("a -B c --d -");
    assert_eq!(query.include(), terms(&["a", "c", "-"]));
    assert_eq!(query.exclude(), terms(&["b", "-d"]));
}

#[test]
fn an_empty_or_blank_query_has_no_terms() {
    assert!(Query::parse("").is_empty());
    assert!(Query::parse(" \t\n ").is_empty());
    assert_eq!(Query::parse(""), Query::default());
}

#[test]
fn terms_lowercase_like_names() {
    assert_eq!(
        Query::parse("ÄRGER İ").include(),
        terms(&["ärger", "i\u{307}"])
    );
}

#[test]
fn an_inactive_filter_shows_everything_and_opens_nothing() {
    let (tree, visibility) = run(&["a/b.txt", "empty/"], " ", KindFilter::All);
    assert!(!visibility.is_active());
    assert_eq!(visibility, Visibility::all());
    assert_eq!(visibility.universe(), None);
    for node in tree.ids() {
        assert!(visibility.is_visible(node));
        assert!(!visibility.opens(node));
    }
}

#[test]
fn every_include_term_must_match() {
    let (tree, visibility) = run(
        &["alpha beta.txt", "alpha.txt", "beta.txt", "BETA-ALPHA.md"],
        "beta alpha",
        KindFilter::All,
    );
    assert!(visibility.is_active());
    assert_eq!(
        visible(&tree, &visibility),
        ["alpha beta.txt", "BETA-ALPHA.md"]
    );
}

#[test]
fn terms_match_names_never_paths() {
    let (tree, visibility) = run(&["docs/readme.txt"], "docs/readme", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), Vec::<String>::new());
}

#[test]
fn the_filter_shows_the_ancestors_of_a_deep_match_open() {
    let (tree, visibility) = run(
        &["deep/er/target.txt", "deep/er/other.txt", "side/x.txt"],
        "target",
        KindFilter::All,
    );
    assert_eq!(
        visible(&tree, &visibility),
        ["deep", "deep/er", "deep/er/target.txt"]
    );
    assert!(visibility.opens(id(&tree, "deep")));
    assert!(visibility.opens(id(&tree, "deep/er")));
    assert!(visibility.opens(NodeId::ROOT));
    assert!(!visibility.opens(id(&tree, "side")));
    assert!(visibility.is_visible(NodeId::ROOT));
}

#[test]
fn an_exclude_term_hides_files_and_removes_folders_with_everything_in_them() {
    let (tree, visibility) = run(
        &[
            "keep/a.txt",
            "skip/a.txt",
            "skip/deeper/a.txt",
            "a-skip.txt",
            "a.txt",
        ],
        "a -SKIP",
        KindFilter::All,
    );
    assert_eq!(visible(&tree, &visibility), ["keep", "keep/a.txt", "a.txt"]);
}

#[test]
fn an_exclude_only_query_hides_empty_folders() {
    let (tree, visibility) = run(&["empty/", "a/x.txt", "a/y.log"], "-log", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["a", "a/x.txt"]);
}

#[test]
fn a_folder_whose_name_matches_stays_visible_without_matching_files() {
    let (tree, visibility) = run(
        &["photos/notes.txt", "photos-old/", "other/"],
        "photo",
        KindFilter::All,
    );
    assert_eq!(visible(&tree, &visibility), ["photos", "photos-old"]);
    assert!(!visibility.opens(id(&tree, "photos")));
}

#[test]
fn a_folder_name_match_needs_every_include_term() {
    let (tree, visibility) = run(&["red/blue.txt", "red blue/"], "red blue", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["red blue"]);
}

#[test]
fn kinds_pass_images_or_text_by_extension() {
    let paths = [
        "a.png",
        "b.PNG",
        "c.rs",
        "d",
        "e.pdf",
        "f.svg",
        "g.tar.gz",
        ".env",
        "dir/h.webp",
    ];
    let (tree, images) = run(&paths, "", KindFilter::Images);
    assert!(images.is_active());
    assert_eq!(
        visible(&tree, &images),
        ["dir", "dir/h.webp", "a.png", "b.PNG", "f.svg"]
    );
    let (tree, text) = run(&paths, "", KindFilter::Text);
    assert_eq!(visible(&tree, &text), [".env", "c.rs", "d"]);
}

#[test]
fn a_kind_and_a_query_both_apply() {
    let (tree, visibility) = run(
        &["cat.png", "cat.txt", "dog.png"],
        "cat",
        KindFilter::Images,
    );
    assert_eq!(visible(&tree, &visibility), ["cat.png"]);
}

#[test]
fn matching_ignores_case_in_any_script() {
    let (tree, visibility) = run(&["Ärger.TXT", "other.txt"], "ÄRGER", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["Ärger.TXT"]);
}

#[test]
fn a_lone_dash_is_an_include_term() {
    let (tree, visibility) = run(&["a-b.txt", "ab.txt"], "-", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["a-b.txt"]);
}

#[test]
fn the_root_name_is_never_tested() {
    let mut raw = RawTree::new("project", false).unwrap();
    raw.push(0, "project-notes.txt", Kind::File).unwrap();
    raw.push(0, "main.rs", Kind::File).unwrap();
    let tree = raw.build(PathBuf::from("/project")).unwrap();
    let excluded = filter(&tree, &Query::parse("-project"), KindFilter::All);
    assert_eq!(visible(&tree, &excluded), ["main.rs"]);
    let included = filter(&tree, &Query::parse("project"), KindFilter::All);
    assert_eq!(visible(&tree, &included), ["project-notes.txt"]);
    let nothing = filter(&tree, &Query::parse("projectx"), KindFilter::All);
    assert!(!nothing.is_visible(NodeId::ROOT));
}

#[test]
fn many_hits_in_a_row_and_far_apart_are_all_found() {
    let mut paths: Vec<String> = (0..300).map(|i| format!("hit{i:03}")).collect();
    paths.extend((0..2000).map(|i| format!("miss{i:04}")));
    paths.push("zz-hit".to_owned());
    let list: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (tree, visibility) = run(&list, "hit", KindFilter::All);
    assert_eq!(visible(&tree, &visibility).len(), 301);
    assert!(visibility.is_visible(id(&tree, "zz-hit")));
}

#[test]
fn a_universe_is_the_visible_set() {
    let (tree, visibility) = run(&["a.txt", "b.txt"], "a", KindFilter::All);
    let universe = visibility.universe().unwrap();
    assert!(universe.contains(id(&tree, "a.txt").index()));
    assert!(!universe.contains(id(&tree, "b.txt").index()));
}

/// Terms that hit, miss, overlap and nest across the generated names.
const TERMS: [&str; 26] = [
    "src", "SRC", "s", "a", ".", "md", "é", "Ä", "b1", "b", "x", "e", "z", "readme", "1", "2",
    "gz", ".git", "-", "q", "-src", "-a", "-md", "-.", "-e", "-x.d",
];

fn queries() -> impl Strategy<Value = (String, KindFilter)> {
    (
        prop::collection::vec(prop::sample::select(&TERMS[..]), 0..4),
        prop::sample::select(&[KindFilter::All, KindFilter::Images, KindFilter::Text][..]),
    )
        .prop_map(|(terms, kind)| (terms.join(" "), kind))
}

fn contains_every(name: &str, terms: &[String]) -> bool {
    terms.iter().all(|term| name.contains(term.as_str()))
}

/// The node or a folder above it, below the root, names an exclude term.
fn removed(tree: &Tree, query: &Query, node: NodeId) -> bool {
    let mut current = node;
    while current != NodeId::ROOT {
        let name = tree.lower_name(current);
        if query
            .exclude()
            .iter()
            .any(|term| name.contains(term.as_str()))
        {
            return true;
        }
        current = tree.parent(current).unwrap();
    }
    false
}

/// The filter's rules for one node, by recursion over its children.
fn brute_visible(tree: &Tree, query: &Query, kind: KindFilter, node: NodeId) -> bool {
    if query.is_empty() && kind == KindFilter::All {
        return true;
    }
    let any_child = || {
        tree.children(node)
            .iter()
            .any(|&child| brute_visible(tree, query, kind, child))
    };
    if node == NodeId::ROOT {
        return any_child();
    }
    if removed(tree, query, node) {
        return false;
    }
    let name = tree.lower_name(node);
    if tree.is_file(node) {
        contains_every(name, query.include()) && kind.passes(tree.extension(node))
    } else {
        any_child() || (!query.include().is_empty() && contains_every(name, query.include()))
    }
}

proptest! {
    #[test]
    fn visibility_matches_a_per_node_check(set in path_sets(), (text, kind) in queries()) {
        let tree = tree_of(&set);
        let query = Query::parse(&text);
        let visibility = filter(&tree, &query, kind);
        prop_assert_eq!(visibility.is_active(), !query.is_empty() || kind != KindFilter::All);
        for node in tree.ids() {
            let expected = brute_visible(&tree, &query, kind, node);
            prop_assert_eq!(visibility.is_visible(node), expected, "{:?}", tree.path(node));
            let opens = visibility.is_active()
                && tree.children(node).iter().any(|&child| brute_visible(&tree, &query, kind, child));
            prop_assert_eq!(visibility.opens(node), opens, "{:?}", tree.path(node));
        }
    }
}
