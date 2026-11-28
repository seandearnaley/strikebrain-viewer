//! In-app Markdown navigation. Resolve URLs against the document that was
//! clicked, and only pass local filesystem paths to the viewer.
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, ListOffset, PromptLevel, Window};
use markdown::mdast::Node;
use percent_encoding::percent_decode_str;
use url::Url;

use super::{Content, Stage};

fn destination(source: &Path, href: &str) -> Option<(PathBuf, Option<String>)> {
    let url = Url::from_file_path(source).ok()?.join(href).ok()?;
    if url.scheme() != "file" || url.host_str().is_some_and(|host| host != "localhost") {
        return None;
    }
    let path = url.to_file_path().ok()?;
    let fragment = url
        .fragment()
        .filter(|f| !f.is_empty())
        .map(|f| percent_decode_str(f).decode_utf8_lossy().into_owned());
    Some((path, fragment))
}

impl Stage {
    pub(super) fn follow_link(
        &mut self,
        source: &Path,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, fragment)) = destination(source, href) else {
            return;
        };
        // A document can remain on screen while another is loading.
        if self.asked_file_path(cx).as_deref() != Some(source) {
            return;
        }
        self.link_task = None;
        self.section = None;
        if let Some(file) = self
            .state
            .read(cx)
            .explorer()
            .and_then(|e| e.tree().find_abs(&path).map(|id| e.tree().is_file(id)))
        {
            self.state
                .update(cx, |s, cx| s.follow_document(path, file, fragment, cx));
            return;
        }
        self.link_task = Some(cx.spawn_in(window, async move |this, cx| {
            let target = cx
                .background_spawn(async move {
                    let absolute = explorer::canonical(&path)?;
                    let metadata = std::fs::metadata(&absolute)?;
                    if !metadata.is_dir() && !metadata.is_file() {
                        return Err(std::io::Error::other(
                            "The link does not point to a file or folder.",
                        ));
                    }
                    Ok((absolute, metadata.is_file()))
                })
                .await;
            this.update_in(cx, |this, window, cx| match target {
                Ok((path, file)) => this
                    .state
                    .update(cx, |s, cx| s.follow_document(path, file, fragment, cx)),
                Err(err) => drop(window.prompt(
                    PromptLevel::Warning,
                    "Couldn’t open link",
                    Some(&err.to_string()),
                    &["OK"],
                    cx,
                )),
            })
        }));
    }

    fn asked_file_path(&self, cx: &Context<Self>) -> Option<PathBuf> {
        let state = self.state.read(cx);
        Some(state.explorer()?.tree().abs_path(state.current()?))
    }

    /// The text view parses asynchronously. Match the rendered revision to
    /// the source before translating the heading's byte range into a scroll.
    pub(super) fn reveal_fragment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Content::Markdown(file) = &self.content else {
            self.section = None;
            return;
        };
        let Some(fragment) = self.state.read(cx).fragment() else {
            if self
                .section
                .take()
                .is_some_and(|(path, _)| path == file.path)
            {
                self.text_views.markdown.update(cx, |view, cx| {
                    view.list_state().scroll_to(ListOffset::default());
                    cx.notify();
                });
            }
            return;
        };
        if self
            .section
            .as_ref()
            .is_some_and(|(path, section)| *path == file.path && section == fragment)
        {
            return;
        }
        if self.asked_file_path(cx).as_deref() != Some(file.path.as_ref()) {
            return;
        }
        let rendered = self.text_views.markdown.read(cx).rendered_text();
        if rendered.source() != self.text_views.markdown_source.as_ref() {
            return;
        }
        let fragment = fragment.to_owned();
        self.section = Some((Arc::clone(&file.path), fragment.clone()));
        let source = self.text_views.markdown_source.clone();
        let target = Arc::clone(&file.path);
        self.heading_task = Some(cx.spawn_in(window, async move |this, cx| {
            let search = fragment.clone();
            let (source, range) = cx
                .background_spawn(async move {
                    let range = heading(&source, &search);
                    (source, range)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.section.as_ref() != Some(&(target, fragment.clone())) {
                    return;
                }
                let rendered = this.text_views.markdown.read(cx).rendered_text();
                if rendered.source() != source.as_ref() {
                    return;
                }
                if let Some(range) = range.and_then(|range| rendered.range_for_source(range)) {
                    this.text_views
                        .markdown
                        .update(cx, |view, cx| view.reveal_range(range, cx))
                        .expect("the range belongs to this rendered Markdown revision");
                } else {
                    drop(window.prompt(
                        PromptLevel::Info,
                        "Heading not found",
                        Some(&fragment),
                        &["OK"],
                        cx,
                    ));
                }
            })
        }));
    }
}

/// Match rendered heading text, including duplicate-heading suffixes. The
/// parser's source positions distinguish headings from identical body text.
fn heading(source: &str, fragment: &str) -> Option<Range<usize>> {
    let doc = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).ok()?;
    let mut stack = vec![&doc];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if matches!(node, Node::Heading(_)) {
            let base: String = node
                .to_string()
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
                .map(|c| if c == ' ' { '-' } else { c })
                .collect();
            let mut slug = base.clone();
            let mut n = 0;
            while !seen.insert(slug.clone()) {
                n += 1;
                slug = format!("{base}-{n}");
            }
            if slug == fragment {
                let pos = node.position()?;
                return Some(pos.start.offset..pos.end.offset);
            }
        }
        if let Some(children) = node.children() {
            stack.extend(children.iter().rev());
        }
    }
    None
}
