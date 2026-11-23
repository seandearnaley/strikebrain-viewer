//! Filtering as you type: each change of the query or the kind starts one
//! engine filter job on the background executor and replaces the task of
//! the one before, which stops it. The engine's generation check drops a
//! result that is no longer the latest, so a stale query never lands.

use explorer::{FilterResult, KindFilter};
use gpui_kit::{AppContext as _, Context};

use super::AppState;

impl AppState {
    /// The filter text asked for, as typed.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The kind asked for.
    #[must_use]
    pub fn kind(&self) -> KindFilter {
        self.kind
    }

    /// Filters the tree by `text`: every word must be in a name, and a word
    /// starting with `-` must not be. Runs in the background; the rows change
    /// when the result lands.
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.query {
            return;
        }
        text.clone_into(&mut self.query);
        self.start_filter(cx);
    }

    /// Filters the tree to files of `kind`.
    pub fn set_kind(&mut self, kind: KindFilter, cx: &mut Context<Self>) {
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        self.start_filter(cx);
        cx.notify();
    }

    /// Whether the filter asked for is not the inactive one.
    pub(super) fn filter_asked(&self) -> bool {
        !self.query.is_empty() || self.kind != KindFilter::All
    }

    /// Starts a job for the filter asked for. With no tree yet, the filter
    /// applies when the tree arrives.
    fn start_filter(&mut self, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let job = explorer.filter_job(&self.query, self.kind);
        // Dropping the task this replaces cancels it; the engine also stops
        // the superseded job between its passes.
        self.filter_task = Some(cx.spawn(async move |this, cx| {
            let Some(result) = cx.background_spawn(async move { job.run() }).await else {
                return Ok(());
            };
            this.update(cx, |this, cx| this.finish_filter(result, cx))
        }));
    }

    /// Shows a filter result if it is still the latest.
    fn finish_filter(&mut self, result: FilterResult, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        if explorer.apply_filter(result) {
            self.rows_replaced(cx);
        }
    }
}
