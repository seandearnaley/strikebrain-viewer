//! Running the filter: on the UI thread for small trees, or as a
//! [`FilterJob`] on a background thread, applied with a generation check.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::Explorer;
use crate::filter::filter_until;
use crate::{KindFilter, Query, Tree, Visibility, filter};

/// One filter run, made by [`Explorer::filter_job`] for a background
/// thread. It holds its own `Arc` of the tree, so it can outlive changes to
/// the explorer.
#[derive(Clone, Debug)]
pub struct FilterJob {
    tree: Arc<Tree>,
    text: String,
    query: Query,
    kind: KindFilter,
    generation: u64,
    /// The explorer's newest generation.
    latest: Arc<AtomicU64>,
}

impl FilterJob {
    /// Runs the filter. Returns `None` when a newer job, a synchronous
    /// [`Explorer::set_filter`] or a [`Explorer::replace_tree`] superseded
    /// this one; the run notices between passes and stops early, so an
    /// outdated keystroke costs little.
    #[must_use]
    pub fn run(&self) -> Option<FilterResult> {
        let superseded = || self.latest.load(Ordering::Relaxed) != self.generation;
        let visibility = filter_until(&self.tree, &self.query, self.kind, &superseded)?;
        Some(FilterResult {
            text: self.text.clone(),
            kind: self.kind,
            generation: self.generation,
            visibility,
        })
    }
}

/// A finished [`FilterJob`], to hand back to [`Explorer::apply_filter`] on
/// the UI thread.
#[derive(Clone, Debug)]
pub struct FilterResult {
    text: String,
    kind: KindFilter,
    generation: u64,
    visibility: Visibility,
}

impl Explorer {
    /// Starts a filter for `text` and `kind`: returns the job to run off the
    /// UI thread. Every earlier job is superseded; only this one's result
    /// will apply.
    #[must_use]
    pub fn filter_job(&mut self, text: &str, kind: KindFilter) -> FilterJob {
        let generation = self.next_generation();
        self.requested = (text.to_owned(), kind);
        FilterJob {
            tree: Arc::clone(&self.tree),
            text: text.to_owned(),
            query: Query::parse(text),
            kind,
            generation,
            latest: Arc::clone(&self.generation),
        }
    }

    /// Shows a finished job's result and rebuilds the rows. Returns `false`,
    /// changing nothing, when the result is stale: a newer job was started,
    /// or the tree was replaced, after its job was made.
    pub fn apply_filter(&mut self, result: FilterResult) -> bool {
        if result.generation != self.generation.load(Ordering::Relaxed) {
            return false;
        }
        self.install(result.text, result.kind, result.visibility);
        true
    }

    /// Runs the filter for `text` and `kind` here and shows it: for small
    /// trees, and for tests. Supersedes every running job.
    pub fn set_filter(&mut self, text: &str, kind: KindFilter) {
        self.next_generation();
        self.requested = (text.to_owned(), kind);
        let visibility = filter(&self.tree, &Query::parse(text), kind);
        self.install(text.to_owned(), kind, visibility);
    }

    /// Moves to a new filter generation and returns it.
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}
