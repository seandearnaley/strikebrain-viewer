//! How this installation is updated: who owns its files.

use std::path::Path;

/// Who owns this installation, and so how an update reaches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// A macOS app bundle the app can replace itself.
    MacBundle,
    /// A macOS app bundle that cannot be replaced; the disk image is opened.
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
    /// and what sits beside it. Whether a macOS bundle can be replaced, and
    /// whether the release signed it, is checked later; this says only what
    /// kind of install it is.
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
    let Some(macos) = exe.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    let Some(app) = contents.parent() else {
        return false;
    };
    macos.file_name().is_some_and(|name| name == "MacOS")
        && contents.file_name().is_some_and(|name| name == "Contents")
        && app.extension().is_some_and(|extension| extension == "app")
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
