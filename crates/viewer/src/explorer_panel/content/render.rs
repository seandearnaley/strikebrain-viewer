//! The first-use prompt, progress and navigable ranked matches.
use super::{ExplorerPanel, Phase, storage};
use crate::search;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _,
    TestSupportExt as _, div, prelude::*,
};
use std::path::Path;

impl ExplorerPanel {
    pub(super) fn primary(id: &'static str, label: &'static str, cx: &Context<Self>) -> Button {
        Button::new(id)
            .small()
            .primary()
            .label(label)
            .on_click(cx.listener(|this, _, _, cx| this.start_content(cx)))
    }

    fn offer_controls(&self, cx: &Context<Self>) -> AnyElement {
        let content = &self.content;
        let folder = content
            .folder
            .as_deref()
            .and_then(Path::file_name)
            .map_or_default(|name| name.to_string_lossy());
        let mut panel = v_flex()
            .id("search-consent")
            .test_support()
            .gap_1()
            .px_2()
            .pb_2()
            .text_sm();
        let has_folder = content.folder.is_some();
        let text = match (has_folder, content.installed) {
            (true, false) => format!(
                "Download the search model and index images and text in {folder} on this device."
            ),
            (true, true) => format!("Index images and text in {folder} on this device."),
            (false, false) => "Download the search model for offline use.".into(),
            (false, true) => "EmbeddingGemma 2 is installed.".into(),
        };
        panel = panel.child(text);
        if !content.installed {
            panel = panel.child(format!(
                "{} MB download",
                search::total().div_ceil(1_000_000)
            ));
        }
        if !content.installed && content.saved_bytes > 0 {
            panel = panel.child(format!(
                "{} MB already saved",
                content.saved_bytes / 1_000_000
            ));
        }
        let mut actions = h_flex().gap_1().flex_wrap();
        if has_folder || !content.installed {
            let label = if !has_folder {
                "Download Model"
            } else if content.installed {
                "Index Folder"
            } else if content.saved_bytes > 0 {
                "Resume & Index"
            } else {
                "Download & Index"
            };
            actions = actions.child(Self::primary("search-start", label, cx));
        }
        panel = panel.child(
            actions.child(
                Button::new("search-dismiss")
                    .small()
                    .ghost()
                    .label("Not Now")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_content(cx))),
            ),
        );

        panel.into_any_element()
    }

    pub(in crate::explorer_panel) fn search_controls(&self, cx: &Context<Self>) -> AnyElement {
        let content = &self.content;
        let mut panel = v_flex()
            .id("search-progress")
            .test_support()
            .gap_1()
            .px_2()
            .pb_2()
            .text_sm();
        match &content.phase {
            Phase::Idle => return div().into_any_element(),
            Phase::Offer => return self.offer_controls(cx),
            Phase::Working(label) => panel = panel.child(label.clone()),
            Phase::Download(n) => {
                let total = search::total();
                let percent = u8::try_from(n.saturating_mul(100) / total).unwrap_or(100);
                panel = panel
                    .child(format!(
                        "Downloading model · {} of {} MB",
                        n / 1_000_000,
                        total.div_ceil(1_000_000)
                    ))
                    .child(
                        Progress::new("model-progress")
                            .accessibility_label("Downloading search model")
                            .value(f32::from(percent))
                            .xsmall(),
                    );
            }
            Phase::Pausing => panel = panel.child("Pausing…"),
            Phase::Paused | Phase::Error(_) => {
                panel = panel
                    .child(crate::title_bar::activity_label(
                        "search-interrupted",
                        if let Phase::Error(s) = &content.phase {
                            s.clone().into()
                        } else {
                            "Paused".into()
                        },
                        cx,
                    ))
                    .child(Self::primary("search-resume", "Resume", cx));
                if !content.installed && content.saved_bytes > 0 {
                    panel = panel.child(Self::confirm_remove(
                        storage::Remove::Partial,
                        "Discard Download",
                        content.saved_bytes,
                        "discard-download",
                        cx,
                    ));
                }
            }
            Phase::Storage(storage) => return Self::storage_controls(storage, cx),
            Phase::Confirm(action, title) => return Self::remove_controls(action, title, cx),
            Phase::Unavailable(reason) => panel = panel.child(reason.clone()),
            Phase::Ready => panel = panel.child("Model ready"),
            Phase::Matches(r) => panel = panel.child(self.match_status(r, cx)),
        }
        if content.job.is_some()
            && !matches!(content.phase, Phase::Pausing)
            && !matches!(&content.phase,Phase::Matches(r) if r.complete)
        {
            panel = panel.child(
                Button::new("search-pause")
                    .small()
                    .ghost()
                    .label("Pause")
                    .on_click(cx.listener(|this, _, _, cx| this.pause_content(cx))),
            );
        }
        panel.into_any_element()
    }

    fn match_status(&self, r: &search::Results, cx: &Context<Self>) -> AnyElement {
        let folder = self
            .content
            .folder
            .as_deref()
            .and_then(Path::file_name)
            .map_or_default(|name| name.to_string_lossy());
        let mut panel = v_flex().gap_1();
        let status = if r.unavailable.is_some() {
            format!("{folder} unavailable · {} cached files", r.indexed)
        } else if r.complete {
            format!("{} indexed", r.indexed)
        } else {
            format!(
                "Indexing {folder} · {} of {}",
                r.indexed + r.failed,
                r.total
            )
        };
        panel = panel.child(crate::title_bar::activity_label(
            "search-status",
            status.into(),
            cx,
        ));
        if let Some(reason) = &r.unavailable {
            panel = panel.child(reason.clone()).child(
                Button::new("reconnect-index")
                    .small()
                    .label("Check Folder Again")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.content.stop();
                        this.start_content(cx);
                    })),
            );
        }
        if let Some(e) = &r.last_error {
            panel = panel.child(format!("{} skipped · {e}", r.failed));
        }
        if r.hits.is_empty() && r.query == self.content.query {
            panel = panel.child(format!("No matches in {} indexed files", r.indexed));
        }
        panel.into_any_element()
    }

    pub(in crate::explorer_panel) fn background_search(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if self.content.enabled {
            return None;
        }
        let label = match &self.content.phase {
            Phase::Matches(r) if !r.complete => {
                format!("Indexing · {} of {}", r.indexed + r.failed, r.total)
            }
            Phase::Download(n) => format!("Downloading model · {} MB", n / 1_000_000),
            Phase::Working(s) => s.clone(),
            _ => return None,
        };
        Some(
            crate::title_bar::activity_label("search-activity", label.into(), cx)
                .into_any_element(),
        )
    }

    pub(in crate::explorer_panel) fn search_hits(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let Phase::Matches(r) = &self.content.phase else {
            return None;
        };
        if !self.content.enabled
            || r.query != self.content.query
            || self.content.input.read(cx).value().trim() != r.query
            || self.search_folder(cx) != self.content.folder
        {
            return None;
        }
        let body = v_flex()
            .id("content-results")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(crate::keys::EXPLORER)
            .flex_1()
            .min_h_0()
            .on_action(
                cx.listener(|this, _: &crate::keys::FocusNext, _, cx| this.move_hit(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &crate::keys::FocusPrevious, _, cx| this.move_hit(false, cx)),
            )
            .on_action(cx.listener(|this, _: &crate::keys::ShowFile, _, cx| this.activate_hit(cx)))
            .child(
                gpui_kit::uniform_list(
                    "ranked-files",
                    r.hits.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let Phase::Matches(results) = &this.content.phase else {
                            return Vec::new();
                        };
                        results.hits[range.clone()]
                            .iter()
                            .zip(range)
                            .map(|(path, i)| {
                                let label = SharedString::from(
                                    this.content
                                        .folder
                                        .as_deref()
                                        .and_then(|folder| path.strip_prefix(folder).ok())
                                        .unwrap_or(path)
                                        .to_string_lossy()
                                        .into_owned(),
                                );
                                let file = path.clone();
                                let button = Button::new(("search-hit", i))
                                    .small()
                                    .label(label)
                                    .disabled(results.unavailable.is_some());
                                let button = if this.content.selected.as_ref() == Some(path) {
                                    button.outline()
                                } else {
                                    button.ghost()
                                };
                                button.on_click(cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus_handle, cx);
                                    this.open_hit(&file, cx);
                                }))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.content.result_scroll)
                .size_full(),
            );
        Some(body.into_any_element())
    }
}
