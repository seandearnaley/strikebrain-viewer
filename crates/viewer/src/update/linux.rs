//! The last step of an update on Linux. An installation from the `.deb`,
//! the app at `/usr/bin/strikebrain-viewer`, is the package manager's: the
//! `.deb`, checked against the release's size and digest as it downloaded
//! and again as it opens, opens with the system. That is GPUI's
//! `open_with_system`, which runs `xdg-open` on it (`gpui-pre-linux`
//! 0.3.12, `platform.rs`): what opens is the desktop's default for a
//! `.deb`, a software installer that installs it through the package
//! manager, or an archive manager, or nothing; and GPUI only logs a failure
//! to start `xdg-open` and never reads its exit status, so the app cannot
//! tell which. GitHub's HTTPS and digest are its authenticity: a `.deb`
//! carries no signature the app could check. Anything else, such as the
//! `.tar.gz` unpacked anywhere, installs from the release page. Either way
//! another program installs: **Install Update**.
//!
//! The same holds wherever `Owner::detect` looks for the `.deb`'s path: any
//! desktop but macOS and Windows. The routing is compiled on every platform
//! for its tests.

use std::path::PathBuf;

use super::Plan;
use super::feed::Asset;
use super::install::Owner;

/// What installing the update does for an installation `owner` owns, whose
/// file `package` downloaded as the release's `asset`: the `.deb` opens
/// with the system once it is checked against `asset` again; any other
/// installation opens the release page.
pub(super) fn plan(owner: Owner, package: PathBuf, asset: Asset) -> Plan {
    match owner {
        Owner::Deb => Plan::Open {
            file: package,
            asset,
        },
        Owner::MacBundle
        | Owner::MacUnreplaceable
        | Owner::WindowsInstaller
        | Owner::ReleasePage => Plan::Page,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `.deb` of 9.9.9, as the release names it.
    const DEB: &str = "strikebrain-viewer_9.9.9_amd64.deb";

    /// The release's `.deb`, as the feed gives it.
    fn asset() -> Asset {
        Asset {
            name: DEB.to_owned(),
            size: 17,
            sha256: [7; 32],
            url: format!("https://example.com/{DEB}"),
        }
    }

    #[test]
    fn the_deb_opens_with_the_system_once_checked_again() {
        let package = PathBuf::from("/home/me/.cache/strikebrain-viewer/updates/9.9.9").join(DEB);
        let plan = plan(Owner::Deb, package.clone(), asset());
        assert!(
            matches!(&plan, Plan::Open { file, asset } if *file == package && asset.name == DEB),
            "{plan:?}"
        );
    }

    #[test]
    fn any_other_installation_opens_the_release_page() {
        for owner in [
            Owner::ReleasePage,
            Owner::WindowsInstaller,
            Owner::MacBundle,
            Owner::MacUnreplaceable,
        ] {
            let plan = plan(owner, PathBuf::from(DEB), asset());
            assert!(matches!(plan, Plan::Page), "{owner:?} {plan:?}");
        }
    }
}
