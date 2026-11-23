use std::path::PathBuf;

use proptest::prelude::*;

use super::{KindFilter, Query, Visibility, filter, filter_until};
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
fn names_whose_lowercase_changes_length_match_at_the_right_node() {
    let names = [
        "\u{212A}elvin.txt",
        "STRAẞE.md",
        "İzmir/a.txt",
        "zz-k.txt",
        "plain",
    ];
    let (tree, visibility) = run(&names, "k", KindFilter::All);
    assert_eq!(
        visible(&tree, &visibility),
        ["\u{212A}elvin.txt", "zz-k.txt"]
    );
    let (tree, visibility) = run(&names, "ẞe", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["STRAẞE.md"]);
    let (tree, visibility) = run(&names, "i\u{307}z", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["İzmir"]);
    let (tree, visibility) = run(&names, "-İ plain", KindFilter::All);
    assert_eq!(visible(&tree, &visibility), ["plain"]);
}

#[test]
fn a_cancelled_filter_gives_no_result() {
    let tree = Tree::from_paths(&["a.txt"]).unwrap();
    let query = Query::parse("a -b");
    assert_eq!(filter_until(&tree, &query, KindFilter::All, &|| true), None);
    // Each later checkpoint also stops the run.
    for allowed in 1..4 {
        let calls = std::cell::Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > allowed
        };
        assert_eq!(
            filter_until(&tree, &query, KindFilter::All, &cancelled),
            None
        );
    }
    // An inactive filter has nothing to cancel.
    assert_eq!(
        filter_until(&tree, &Query::default(), KindFilter::All, &|| true),
        Some(Visibility::all())
    );
}

#[test]
fn a_term_holding_a_nul_matches_nothing() {
    let (tree, visibility) = run(&["a.txt"], "a\0", KindFilter::All);
    assert!(visibility.is_active());
    assert_eq!(visible(&tree, &visibility), Vec::<String>::new());
}

#[test]
fn visible_file_counts_cover_any_id_range() {
    let (tree, visibility) = run(&["a/x.md", "a/y.rs", "b.md", "c.rs"], "md", KindFilter::All);
    let a = id(&tree, "a");
    assert_eq!(visibility.file_count(&tree, tree.subtree(a)), 1);
    assert_eq!(visibility.file_count(&tree, tree.subtree(NodeId::ROOT)), 2);
    let all = Visibility::all();
    assert_eq!(all.file_count(&tree, tree.subtree(NodeId::ROOT)), 4);
    assert_eq!(all.stamp(), 0);
    assert_ne!(visibility.stamp(), 0);
    let again = filter(&tree, &Query::parse("md"), KindFilter::All);
    // Equal contents, but each run is its own universe.
    assert_eq!(again, visibility);
    assert_ne!(again.stamp(), visibility.stamp());
}

#[test]
fn visibilities_print_and_compare_by_what_they_show() {
    let (tree, visibility) = run(&["a.txt", "b.txt"], "a", KindFilter::All);
    let text = format!("{visibility:?}");
    assert!(text.contains("visible: [0, 1]"), "{text}");
    assert!(format!("{:?}", Visibility::all()).contains("active: false"));
    assert_ne!(visibility, Visibility::all());
    assert_ne!(Visibility::all(), visibility);
    assert_eq!(tree.node_count(), 3);
}

#[test]
fn a_universe_is_the_visible_set() {
    let (tree, visibility) = run(&["a.txt", "b.txt"], "a", KindFilter::All);
    let universe = visibility.universe().unwrap();
    assert!(universe.contains(id(&tree, "a.txt").index()));
    assert!(!universe.contains(id(&tree, "b.txt").index()));
}

/// Terms that hit, miss, overlap and nest across the generated names, some
/// of them changing byte length when lowercased.
const TERMS: [&str; 34] = [
    "src", "SRC", "s", "a", ".", "md", "é", "Ä", "b1", "b", "x", "e", "z", "readme", "1", "2",
    "gz", ".git", "-", "q", "-src", "-a", "-md", "-.", "-e", "-x.d", "İ", "i\u{307}", "ß", "ẞ",
    "k", "\u{212A}", "-ß", "-k",
];

fn queries() -> impl Strategy<Value = (String, KindFilter)> {
    (
        prop::collection::vec(prop::sample::select(&TERMS[..]), 0..4),
        prop::sample::select(&[KindFilter::All, KindFilter::Images, KindFilter::Text][..]),
    )
        .prop_map(|(terms, kind)| (terms.join(" "), kind))
}

/// Lowercase one character at a time, as the oracle's own fold: it does
/// not use the tree's lowercased names or the query's lowercased terms.
fn fold(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// The query's include and exclude terms, folded, by the grammar's words.
fn folded_terms(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut include, mut exclude) = (Vec::new(), Vec::new());
    for term in text.split_whitespace() {
        match term.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => exclude.push(fold(rest)),
            _ => include.push(fold(term)),
        }
    }
    (include, exclude)
}

/// The extension of a folded name: after the last dot, unless the dot
/// starts the name.
fn extension(folded: &str) -> &str {
    match folded.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    }
}

fn contains_every(name: &str, terms: &[String]) -> bool {
    terms.iter().all(|term| name.contains(term.as_str()))
}

/// The node or a folder above it, below the root, names an exclude term.
fn removed(tree: &Tree, exclude: &[String], node: NodeId) -> bool {
    let mut current = node;
    while current != NodeId::ROOT {
        let name = fold(tree.name(current));
        if exclude.iter().any(|term| name.contains(term.as_str())) {
            return true;
        }
        current = tree.parent(current).unwrap();
    }
    false
}

/// The filter's rules for one node, by recursion over its children.
fn brute_visible(tree: &Tree, text: &str, kind: KindFilter, node: NodeId) -> bool {
    let (include, exclude) = folded_terms(text);
    if include.is_empty() && exclude.is_empty() && kind == KindFilter::All {
        return true;
    }
    let any_child = || {
        tree.children(node)
            .iter()
            .any(|&child| brute_visible(tree, text, kind, child))
    };
    if node == NodeId::ROOT {
        return any_child();
    }
    if removed(tree, &exclude, node) {
        return false;
    }
    let name = fold(tree.name(node));
    if tree.is_file(node) {
        contains_every(&name, &include) && kind.passes(extension(&name))
    } else {
        any_child() || (!include.is_empty() && contains_every(&name, &include))
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
            let expected = brute_visible(&tree, &text, kind, node);
            prop_assert_eq!(visibility.is_visible(node), expected, "{:?}", tree.path(node));
            let opens = visibility.is_active()
                && tree.children(node).iter().any(|&child| brute_visible(&tree, &text, kind, child));
            prop_assert_eq!(visibility.opens(node), opens, "{:?}", tree.path(node));
        }
    }
}
