//! The icons the app draws: the Kit's default set plus the symlink icon,
//! which the default set leaves out.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(ExtraIcons, [FileSymlink]);

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
    fn loads_the_symlink_icon_and_the_default_icons() {
        let symlink = gpui_kit::assets::IconName::FileSymlink.path();
        assert!(AppAssets.load(&symlink).unwrap().is_some());
        assert!(AppAssets.load("icons/folder.svg").unwrap().is_some());
        assert!(AppAssets.list("icons/").unwrap().contains(&symlink));
    }
}
