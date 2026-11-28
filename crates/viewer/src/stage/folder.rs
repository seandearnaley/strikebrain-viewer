//! The directory on the stage. The Kit list provides virtualization,
//! keyboard selection, scrolling and activation for its immediate children.

use std::path::PathBuf;

use explorer::{Kind, NodeId, format_size};
use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, IndexPath, h_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window, div, prelude::*,
};

use crate::state::AppState;

pub(super) type FolderList = ListState<Contents>;

pub(super) struct Contents {
    state: Entity<AppState>,
    folder: NodeId,
    generation: u64,
    directory: PathBuf,
    selected: Option<(NodeId, String)>,
}

/// Keep the list's scroll handle and selected child when the same folder is
/// rebuilt or reordered. Opening another directory starts its own list.
pub(super) fn sync(
    existing: Option<&Entity<FolderList>>,
    state: &Entity<AppState>,
    window: &mut Window,
    cx: &mut Context<super::Stage>,
) -> Entity<FolderList> {
    let model = state.read(cx);
    let (generation, folder) = model.staged();
    let folder = folder.expect("folder view has a current directory");
    let explorer = model.explorer().expect("folder view has an explorer");
    let directory = explorer.tree().abs_path(folder);
    if let Some(list) = existing.filter(|list| list.read(cx).delegate().directory == directory) {
        let selected = list
            .read(cx)
            .delegate()
            .selected
            .as_ref()
            .and_then(|(_, name)| {
                explorer
                    .children(folder)
                    .iter()
                    .position(|id| explorer.tree().name(*id) == name)
                    .map(IndexPath::new)
            });
        list.update(cx, |list, cx| {
            let contents = list.delegate_mut();
            contents.generation = generation;
            contents.folder = folder;
            list.set_selected_index(selected, window, cx);
            cx.notify();
        });
        return list.clone();
    }
    let delegate = Contents {
        state: state.clone(),
        folder,
        generation,
        directory,
        selected: None,
    };
    cx.new(|cx| ListState::new(delegate, window, cx))
}

impl Contents {
    fn explorer<'a>(&self, cx: &'a App) -> Option<&'a explorer::Explorer> {
        let state = self.state.read(cx);
        if state.staged() != (self.generation, Some(self.folder)) {
            return None;
        }
        state.explorer()
    }
}

impl ListDelegate for Contents {
    type Item = ListItem;

    fn items_count(&self, _: usize, cx: &App) -> usize {
        self.explorer(cx)
            .map_or(0, |e| e.children(self.folder).len())
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        cx: &mut Context<FolderList>,
    ) -> Option<ListItem> {
        let e = self.explorer(cx)?;
        let id = *e.children(self.folder).get(ix.row)?;
        let tree = e.tree();
        let name = tree.name(id).to_owned();
        let icon = match tree.kind(id) {
            Kind::Dir | Kind::Unreadable => Icon::new(IconName::Folder).text_color(cx.theme().info),
            Kind::Symlink => Icon::new(gpui_kit::assets::IconName::FileSymlink),
            Kind::File => Icon::new(crate::row::file_icon(tree.extension(id))),
        };
        let size = tree.size(id).map(format_size);
        Some(
            ListItem::new(("entry", id.index()))
                .accessibility_label(name.clone())
                .child(
                    h_flex()
                        .id(("folder-entry", id.index()))
                        .test_support()
                        .w_full()
                        .h_8()
                        .gap_2()
                        .px_2()
                        .child(icon.size_4())
                        .child(div().flex_1().min_w_0().truncate().child(name))
                        .when_some(size, |row, size| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(size),
                            )
                        }),
                ),
        )
    }

    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        cx: &mut Context<FolderList>,
    ) {
        self.selected = index.and_then(|ix| {
            let e = self.explorer(cx)?;
            let id = *e.children(self.folder).get(ix.row)?;
            Some((id, e.tree().name(id).to_owned()))
        });
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<FolderList>) {
        let Some(&(id, _)) = self.selected.as_ref() else {
            return;
        };
        self.state
            .update(cx, |state, cx| state.navigate(self.generation, id, cx));
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut Context<FolderList>) -> impl IntoElement {
        let text = self.explorer(cx).map_or("", |e| {
            if e.tree().kind(self.folder) == Kind::Unreadable {
                "Folder unavailable"
            } else if e.tree().is_partial(self.folder) {
                "Reading folder…"
            } else {
                "Empty folder"
            }
        });
        div()
            .id("folder-empty")
            .test_support()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }
}
