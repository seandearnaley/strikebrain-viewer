//! How this installation is updated: who owns its files.

use std::path::Path;

/// Who owns this installation, and so how an update reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// A macOS app bundle. Where a check finds that the release signed it,
    /// with a Team ID, and that it can be replaced, the app replaces it.
    MacBundle,
    /// A macOS app bundle the release signed that cannot be replaced: it is
    /// translocated, or its folder cannot be written, as the disk image's
    /// cannot. The checked disk image opens.
    MacUnreplaceable,
    /// A Windows install by the installer, which owns its files.
    WindowsInstaller,
    /// A Linux install from the `.deb`; the package manager owns it.
    Deb,
    /// Anything else: a `.zip`, a `.tar.gz`, a development build. The release
    /// page opens.
    ReleasePage,
}

/// The only place the `.deb` puts the app.
const DEB_PATH: &str = "/usr/bin/strikebrain-viewer";

/// The installer's uninstaller, which sits beside the app it installed.
const UNINSTALLER: &str = "unins000.exe";

impl Owner {
    /// The owner of the installation whose executable is `exe`, by its path
    /// and what sits beside it: only what kind of install it is, without
    /// running anything, as the app opens. Whether a macOS bundle is the
    /// release's, signed with a Team ID, and whether it can be replaced are
    /// asked by each check that finds an update, on the background executor
    /// (`update::macos::owner`).
    pub(crate) fn detect(exe: &Path) -> Self {
        if cfg!(target_os = "macos") {
            if in_app_bundle(exe) {
                Self::MacBundle
            } else {
                Self::ReleasePage
            }
        } else if cfg!(windows) {
            if beside_uninstaller(exe) {
                Self::WindowsInstaller
            } else {
                Self::ReleasePage
            }
        } else if is_deb_path(exe) {
            Self::Deb
        } else {
            Self::ReleasePage
        }
    }
}

/// Whether `exe` is `<X>.app/Contents/MacOS/<exe>`: the layout of a bundle,
/// which the app replaces as a whole.
fn in_app_bundle(exe: &Path) -> bool {
    bundle_of(exe).is_some()
}

/// The bundle `exe` runs in, `<X>.app` of `<X>.app/Contents/MacOS/<exe>`;
/// none if it is not laid out so.
pub(crate) fn bundle_of(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let laid_out = macos.file_name().is_some_and(|name| name == "MacOS")
        && contents.file_name().is_some_and(|name| name == "Contents")
        && app.extension().is_some_and(|extension| extension == "app");
    laid_out.then_some(app)
}

/// Whether the installer's uninstaller sits beside `exe`: only an install
/// through the installer leaves it, a `.zip` does not.
fn beside_uninstaller(exe: &Path) -> bool {
    exe.with_file_name(UNINSTALLER).is_file()
}

/// Whether `exe` is where the `.deb` installs the app: the package manager
/// owns that path, a `.tar.gz` unpacked elsewhere is not it.
fn is_deb_path(exe: &Path) -> bool {
    exe == Path::new(DEB_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_inside_an_app_bundle_is_a_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp
            .path()
            .join("Strikebrain Viewer.app/Contents/MacOS/strikebrain-viewer");
        assert!(in_app_bundle(&exe));
        assert!(!in_app_bundle(&tmp.path().join("strikebrain-viewer")));
        assert!(!in_app_bundle(
            &tmp.path()
                .join("Viewer.app/Contents/Resources/strikebrain-viewer")
        ));
        assert!(!in_app_bundle(Path::new("strikebrain-viewer")));
        assert_eq!(
            bundle_of(&exe),
            Some(tmp.path().join("Strikebrain Viewer.app").as_path())
        );
        assert_eq!(bundle_of(&tmp.path().join("strikebrain-viewer")), None);
    }

    #[test]
    fn only_the_debs_path_is_a_deb() {
        assert!(is_deb_path(Path::new("/usr/bin/strikebrain-viewer")));
        assert!(!is_deb_path(Path::new("/usr/local/bin/strikebrain-viewer")));
        assert!(!is_deb_path(Path::new("/home/me/strikebrain-viewer")));
    }

    #[test]
    fn an_uninstaller_beside_the_executable_marks_the_installer() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("strikebrain-viewer.exe");
        assert!(!beside_uninstaller(&exe));
        std::fs::write(tmp.path().join(UNINSTALLER), b"").unwrap();
        assert!(beside_uninstaller(&exe));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_a_bundle_is_replaceable_and_anything_else_opens_the_page() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp
            .path()
            .join("Strikebrain Viewer.app/Contents/MacOS/strikebrain-viewer");
        assert_eq!(Owner::detect(&exe), Owner::MacBundle);
        assert_eq!(
            Owner::detect(&tmp.path().join("strikebrain-viewer")),
            Owner::ReleasePage
        );
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_the_uninstaller_makes_the_installer_the_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("strikebrain-viewer.exe");
        assert_eq!(Owner::detect(&exe), Owner::ReleasePage);
        std::fs::write(tmp.path().join(UNINSTALLER), b"").unwrap();
        assert_eq!(Owner::detect(&exe), Owner::WindowsInstaller);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn on_linux_only_the_debs_path_is_a_deb() {
        assert_eq!(
            Owner::detect(Path::new("/usr/bin/strikebrain-viewer")),
            Owner::Deb
        );
        assert_eq!(
            Owner::detect(Path::new("/opt/viewer/strikebrain-viewer")),
            Owner::ReleasePage
        );
    }
}
