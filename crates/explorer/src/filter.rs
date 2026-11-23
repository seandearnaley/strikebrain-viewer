//! The filter: which nodes a query and a kind let through.
//!
//! [`filter`] is a pure function of the tree, the parsed [`Query`] and the
//! [`KindFilter`]. Each include or exclude term is one `memmem` pass over the
//! tree's haystack (every lowercased name in id order, `'\0'`-separated);
//! the passes combine as bitsets, excluded folders drop their id ranges, and
//! one backwards pass over the visible ids marks every ancestor. The root
//! folder is not a row, so the filter never tests its name.

use std::convert::Infallible;

use fixedbitset::FixedBitSet;
use memchr::memmem::Finder;

use crate::tree::push_lowercase;
use crate::{NodeId, Tree, bits, is_binary_extension, is_image_extension};

/// A parsed filter query: lowercase terms a name must contain (`include`)
/// or must not contain (`exclude`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Query {
    /// Parses the filter input as brainstrike does. The text splits on
    /// whitespace; a term that starts with `-` and is longer than one
    /// character is an exclude term without its `-`; every other term,
    /// a lone `-` included, is an include term. Terms are lowercased the way
    /// the tree lowercases names, so matching is a plain substring test.
    ///
    /// Learning exercise: Sean may rewrite this body; the tests below define it.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut query = Self::default();
        for term in text.split_whitespace() {
            let (list, body) = match term.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (&mut query.exclude, rest),
                _ => (&mut query.include, term),
            };
            let mut lower = String::with_capacity(body.len());
            push_lowercase(&mut lower, body);
            list.push(lower);
        }
        query
    }

    /// Terms every visible name contains, lowercased.
    #[must_use]
    pub fn include(&self) -> &[String] {
        &self.include
    }

    /// Terms that hide a file, or a folder with everything in it, lowercased.
    #[must_use]
    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }

    /// Whether the query has no terms at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }
}

/// Which files the kind control lets through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KindFilter {
    /// Every file.
    #[default]
    All,
    /// Files with an image extension ([`crate::IMAGE_EXTENSIONS`]).
    Images,
    /// Files that are neither images nor a known binary format
    /// ([`crate::BINARY_EXTENSIONS`]). Files without an extension pass.
    Text,
}

impl KindFilter {
    /// Whether a file with extension `ext` (no dot, any case) passes.
    #[must_use]
    pub fn passes(self, ext: &str) -> bool {
        match self {
            Self::All => true,
            Self::Images => is_image_extension(ext),
            Self::Text => !is_image_extension(ext) && !is_binary_extension(ext),
        }
    }
}

/// Which nodes a filter lets through. Inactive (the default) shows
/// everything; active, it holds one visible bit per node and one bit per
/// folder that the filter shows open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visibility {
    sets: Option<Sets>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Sets {
    visible: FixedBitSet,
    /// Folders with a visible descendant.
    open: FixedBitSet,
}

impl Visibility {
    /// The inactive filter: every node visible, no folder opened.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Whether a query or a kind other than [`KindFilter::All`] is in force.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.sets.is_some()
    }

    /// Whether the node passes. Always true when inactive. The root is
    /// visible under an active filter only when something below it is.
    #[must_use]
    pub fn is_visible(&self, id: NodeId) -> bool {
        self.sets
            .as_ref()
            .is_none_or(|sets| sets.visible.contains(id.index()))
    }

    /// Whether the active filter shows the folder open because something
    /// below it is visible. An inactive filter opens nothing.
    #[must_use]
    pub fn opens(&self, id: NodeId) -> bool {
        self.sets
            .as_ref()
            .is_some_and(|sets| sets.open.contains(id.index()))
    }

    /// The visible set as a selection universe: `None` when inactive.
    #[must_use]
    pub fn universe(&self) -> Option<&FixedBitSet> {
        self.sets.as_ref().map(|sets| &sets.visible)
    }
}

/// Applies `query` and `kind` to `tree`. With no terms and
/// [`KindFilter::All`] the filter is inactive and every node is visible,
/// empty folders included. Otherwise:
///
/// - a file is visible when its name contains every include term and no
///   exclude term, its kind passes, and no folder above it is removed;
/// - a folder whose name contains an exclude term is removed with
///   everything in it;
/// - a folder that is not removed is visible when something below it is
///   visible, or when its own name contains every include term of a
///   non-empty include list.
///
/// Names match in lowercase, as substrings; paths never match.
#[must_use]
pub fn filter(tree: &Tree, query: &Query, kind: KindFilter) -> Visibility {
    let Ok(visibility) = run(tree, query, kind, &mut || Ok::<(), Infallible>(()));
    visibility
}

/// The filter, stopping with `check`'s error at the first checkpoint where
/// it returns one.
fn run<E>(
    tree: &Tree,
    query: &Query,
    kind: KindFilter,
    check: &mut dyn FnMut() -> Result<(), E>,
) -> Result<Visibility, E> {
    if query.is_empty() && kind == KindFilter::All {
        return Ok(Visibility::all());
    }
    let files = tree.files();

    // Nodes whose names contain every include term; `None` with no terms.
    let mut matched: Option<FixedBitSet> = None;
    for term in query.include() {
        check()?;
        matched = Some(match matched {
            Some(found) if found.count_ones(..) < tree.node_count() / REFINE_BELOW => {
                refine(tree, &found, term)
            }
            Some(mut found) => {
                found.intersect_with(&containing(tree, term));
                found
            }
            None => containing(tree, term),
        });
    }
    let mut excluded = FixedBitSet::with_capacity(tree.node_count());
    for term in query.exclude() {
        check()?;
        excluded.union_with(&containing(tree, term));
    }
    excluded.remove(NodeId::ROOT.index());
    check()?;

    let mut visible = files.clone();
    if kind != KindFilter::All {
        visible.intersect_with(&kind_passes(tree, kind));
    }
    if let Some(found) = &matched {
        visible.intersect_with(found);
        // Folders that match by name.
        let mut folders = found.clone();
        folders.difference_with(files);
        folders.remove(NodeId::ROOT.index());
        visible.union_with(&folders);
    }
    visible.difference_with(&excluded);
    // A removed folder takes its whole range; a nested one is already gone.
    let mut removed_to = 0;
    for folder in excluded.ones() {
        if folder >= removed_to && !files.contains(folder) {
            let range = tree.subtree(NodeId::from_index(folder));
            removed_to = range.end;
            visible.remove_range(range);
        }
    }
    check()?;

    // Parents have smaller ids, so walking visible ids downwards reaches
    // each parent after every child that made it visible.
    let mut open = FixedBitSet::with_capacity(tree.node_count());
    let mut cursor = tree.node_count();
    while let Some(id) = bits::prev_one(&visible, cursor) {
        let Some(parent) = tree.parent(NodeId::from_index(id)) else {
            break;
        };
        visible.insert(parent.index());
        open.insert(parent.index());
        cursor = id;
    }
    Ok(Visibility {
        sets: Some(Sets { visible, open }),
    })
}

/// A later include term checks the names already matched one by one, rather
/// than the whole haystack, when fewer than one node in this many matched.
const REFINE_BELOW: usize = 16;

/// Nodes whose lowercase name contains `term`: one pass over the haystack.
fn containing(tree: &Tree, term: &str) -> FixedBitSet {
    let mut hits = FixedBitSet::with_capacity(tree.node_count());
    // Entries are separated by '\0', which no name holds.
    if term.contains('\0') {
        return hits;
    }
    let haystack = tree.haystack().as_bytes();
    let ends = tree.haystack_ends();
    let finder = Finder::new(term.as_bytes());
    let (mut node, mut from) = (0, 0);
    while let Some(found) = finder.find(&haystack[from..]) {
        node = entry_at(ends, node, from + found);
        hits.insert(node);
        // One hit per name is enough: carry on from the next entry.
        from = ends[node] as usize;
    }
    hits
}

/// The entry holding byte `offset`: the first index from `start` on whose
/// end is past it. Gallops from `start`, so a hit in the next few entries
/// costs a step or two and a distant one a binary search.
fn entry_at(ends: &[u32], start: usize, offset: usize) -> usize {
    let past = |entry: usize| ends[entry] as usize > offset;
    if past(start) {
        return start;
    }
    // `ends[low]` is at or before `offset`.
    let (mut low, mut step) = (start, 1);
    loop {
        let high = (low + step).min(ends.len() - 1);
        if past(high) {
            let window = &ends[low + 1..high];
            return low + 1 + window.partition_point(|&end| end as usize <= offset);
        }
        low = high;
        step *= 2;
    }
}

/// The nodes of `found` whose lowercase name also contains `term`.
fn refine(tree: &Tree, found: &FixedBitSet, term: &str) -> FixedBitSet {
    let finder = Finder::new(term.as_bytes());
    let mut hits = FixedBitSet::with_capacity(tree.node_count());
    for node in found.ones() {
        let name = tree.lower_name(NodeId::from_index(node));
        if finder.find(name.as_bytes()).is_some() {
            hits.insert(node);
        }
    }
    hits
}

/// Files whose extension `kind` lets through. Each interned extension is
/// judged once; only files outside the table look at their own name.
fn kind_passes(tree: &Tree, kind: KindFilter) -> FixedBitSet {
    let table: Vec<bool> = tree
        .extensions()
        .iter()
        .map(|ext| kind.passes(ext))
        .collect();
    let mut passing = FixedBitSet::with_capacity(tree.node_count());
    for file in tree.files().ones() {
        let id = NodeId::from_index(file);
        let passes = match tree.ext_id(id) {
            0 => kind.passes(tree.extension(id)),
            ext => table[usize::from(ext)],
        };
        passing.set(file, passes);
    }
    passing
}

#[cfg(test)]
mod tests;
