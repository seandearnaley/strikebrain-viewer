//! The icons the app draws: the Kit's default set plus the few it leaves
//! out: the picture, code and symlink files, Collapse all, Select all,
//! Clear and Add Folder.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    ExtraIcons,
    [
        FileImage,
        FileCode,
        FileSymlink,
        ChevronsDownUp,
        ListChecks,
        ListX,
        FolderPlus
    ]
);

/// The app's asset source: pass it to `Application::with_assets`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(svg) => Ok(Some(svg)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_extra_icons_and_the_default_icons() {
        use gpui_kit::assets::IconName;
        let symlink = IconName::FileSymlink.path();
        for icon in [
            IconName::FileImage,
            IconName::FileCode,
            IconName::FileText,
            IconName::FileSymlink,
            IconName::ChevronsDownUp,
            IconName::ListChecks,
            IconName::ListX,
            IconName::FolderPlus,
        ] {
            assert!(AppAssets.load(&icon.path()).unwrap().is_some());
        }
        assert!(AppAssets.load("icons/folder.svg").unwrap().is_some());
        assert!(AppAssets.list("icons/").unwrap().contains(&symlink));
    }
}
