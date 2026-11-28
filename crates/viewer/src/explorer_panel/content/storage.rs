//! Storage management is opened explicitly from More. Filesystem work stays
//! on the background executor, including deletion of a large model.
use super::{Content, ExplorerPanel, Phase};
use crate::search;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, Styled as _,
};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) enum Remove {
    Model,
    Partial,
    Index(PathBuf),
}

pub(super) struct Storage {
    installed: bool,
    partial: u64,
    indexes: Vec<search::StoredIndex>,
}

impl Storage {
    fn read(dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            installed: search::installed(dir),
            partial: search::partial_bytes(dir)?,
            indexes: search::catalog(dir)?,
        })
    }
}

impl ExplorerPanel {
    pub(crate) fn manage_content(&mut self, cx: &mut Context<Self>) {
        self.content.enabled = true;
        if self.content.job.is_some()
            && !matches!(&self.content.phase,Phase::Matches(r) if r.complete)
        {
            cx.notify();
            return;
        }
        self.content.stop();
        self.content.folder = None;
        self.storage_operation(None, cx);
    }

    pub(super) fn storage_operation(&mut self, remove: Option<Remove>, cx: &mut Context<Self>) {
        let Some(dir) = self.content.store.clone() else {
            self.content.phase = Phase::Error("Local search storage is unavailable".into());
            cx.notify();
            return;
        };
        self.content.generation += 1;
        let generation = self.content.generation;
        self.content.phase = Phase::Working("Reading search storage…".into());
        cx.notify();
        let task = cx.background_spawn(async move {
            match remove {
                Some(Remove::Model) => search::remove_model(&dir)?,
                Some(Remove::Index(path)) => search::delete_index(&path)?,
                Some(Remove::Partial) => search::discard(&dir)?,
                None => {}
            }
            Storage::read(&dir)
        });
        cx.spawn(async move |this, cx| {
            let storage = task.await;
            let _alive = this.update(cx, |this, cx| {
                if this.content.generation != generation {
                    return;
                }
                this.content.phase = match storage {
                    Ok(storage) => {
                        this.content.installed = storage.installed;
                        Phase::Storage(storage)
                    }
                    Err(e) => Phase::Unavailable(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remove(
        action: Remove,
        label: &str,
        bytes: u64,
        id: impl Into<gpui_kit::ElementId>,
        cx: &Context<Self>,
    ) -> Button {
        let title = format!("{label} ({})?", explorer::format_size(bytes));
        Button::new(id)
            .ghost()
            .small()
            .label(format!("{label}…"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.content.phase = Phase::Confirm(action.clone(), title.clone());
                cx.notify();
            }))
    }

    pub(super) fn remove_controls(action: &Remove, title: &str, cx: &Context<Self>) -> AnyElement {
        let removal = action.clone();
        v_flex()
            .p_2()
            .gap_1()
            .child(title.to_owned())
            .children(
                matches!(action, Remove::Model)
                    .then_some("Search will need a new download. Folder indexes are kept."),
            )
            .child(
                Button::new("confirm-remove")
                    .small()
                    .label("Remove")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.storage_operation(Some(removal.clone()), cx);
                    })),
            )
            .child(
                Button::new("cancel-remove")
                    .small()
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.manage_content(cx))),
            )
            .into_any_element()
    }

    pub(super) fn storage_controls(storage: &Storage, cx: &Context<Self>) -> AnyElement {
        let mut panel = v_flex().gap_1().p_2().text_sm().child("EmbeddingGemma 2");
        if storage.installed {
            panel = panel
                .child(format!(
                    "Installed · {}",
                    explorer::format_size(search::total())
                ))
                .child(Self::confirm_remove(
                    Remove::Model,
                    "Remove Model",
                    search::total(),
                    "remove-model",
                    cx,
                ));
        } else {
            panel = panel
                .child(format!(
                    "{} MB download",
                    search::total().div_ceil(1_000_000)
                ))
                .child(Self::primary(
                    "search-start",
                    if storage.partial == 0 {
                        "Download Model"
                    } else {
                        "Resume Download"
                    },
                    cx,
                ));
            if storage.partial > 0 {
                panel = panel.child(Self::confirm_remove(
                    Remove::Partial,
                    "Discard Download",
                    storage.partial,
                    "discard-part",
                    cx,
                ));
            }
        }
        for (i, index) in storage.indexes.iter().enumerate() {
            panel = panel
                .child(format!(
                    "{} · {}",
                    index.label,
                    explorer::format_size(index.bytes)
                ))
                .child(Self::confirm_remove(
                    Remove::Index(index.path.clone()),
                    "Delete Folder Index",
                    index.bytes,
                    ("remove-index", i),
                    cx,
                ));
        }
        panel
            .child(
                Button::new("search-done")
                    .ghost()
                    .small()
                    .label("Done")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.content.phase = Phase::Idle;
                        this.content.enabled = false;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

impl Content {
    pub(super) fn stop(&mut self) {
        self.generation += 1;
        self.job = None;
    }
}
