//! The last step of an update on macOS. The downloaded disk image is
//! attached read-only and hidden. Its one app must pass `codesign`'s strict
//! check of its whole signature against a requirement naming the running
//! app's Team ID and bundle identifier, and Gatekeeper's assessment, and its
//! own version must be the release's. Where the running bundle can be
//! replaced, that app is copied beside it, onto the same volume, and the
//! copy is checked the same way. **Restart to
//! Update** checks the copy's signature once more, swaps the two bundles
//! with two renames and relaunches. Where the bundle cannot be replaced,
//! the checked disk image opens in Finder instead. Blocking; the updater
//! runs the checks and the copy on GPUI's background executor, and the
//! swap as it relaunches.
//!
//! The tools are the system's, by their full paths. A path given to one is
//! always absolute: `codesign` reads an argument that starts with a digit
//! as a process ID.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use super::install::{Owner, bundle_of};

/// Attaches and detaches disk images.
const HDIUTIL: &str = "/usr/bin/hdiutil";
/// Checks and reports code signatures.
const CODESIGN: &str = "/usr/bin/codesign";
/// Asks Gatekeeper whether code may run.
const SPCTL: &str = "/usr/sbin/spctl";
/// Copies a bundle whole: its links, modes and extended attributes.
const DITTO: &str = "/usr/bin/ditto";
/// Reads a value from a property list.
const PLUTIL: &str = "/usr/bin/plutil";

/// The folder beside the installed app that holds the new one until the
/// swap: on the same volume, so the swap is two renames.
const STAGING: &str = ".Strikebrain Viewer.update";

/// The name the old app takes in the staging folder while the new one
/// moves into its place.
const OLD: &str = "old.app";

/// How long a Team ID is: ten capital letters and digits. An app with none,
/// signed ad hoc or by Apple, reports `not set`.
const TEAM_ID_LENGTH: usize = 10;

/// The exit status of `codesign --verify` when the code is properly signed
/// but fails the requirement given with `-R`/`--test-requirement`.
const FAILED_REQUIREMENT: i32 = 3;

/// A part of the path of a translocated app: macOS runs a quarantined app
/// that was never moved from where it was downloaded from a randomised,
/// read-only mount.
const TRANSLOCATED: &str = "/AppTranslocation/";

/// Why a step failed.
#[derive(Debug)]
pub(crate) enum Error {
    /// The running app is not a bundle the release signed.
    NotReleased,
    /// A tool, named by its path, could not be started.
    Run(&'static str, io::Error),
    /// A tool, named by its path, failed; with the last line it wrote.
    Failed(&'static str, String),
    /// The disk image does not hold exactly one app.
    NoApp,
    /// The app is validly signed, but not with the running app's Team ID as
    /// the same bundle identifier.
    NotThisApp,
    /// The app is the release's, but its own version, `found`, if it
    /// gives one, is not the release's version, `release`: an older build
    /// in a newer release.
    Version {
        found: Option<String>,
        release: semver::Version,
    },
    /// The staging folder could not be made, or a bundle could not move.
    Io(io::Error),
    /// The new app could not move into place (`error`), and the old one
    /// could not move back either (`restore`): nothing is at the installed
    /// path, and the old app is at `old`, in the staging folder.
    Stranded {
        old: PathBuf,
        error: io::Error,
        restore: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReleased => f.write_str("this copy of the app is not the release's"),
            Self::Run(tool, error) => write!(f, "{} did not run: {error}", name(tool)),
            Self::Failed(tool, said) => write!(f, "{} failed: {said}", name(tool)),
            Self::NoApp => f.write_str("the disk image does not hold one app"),
            Self::NotThisApp => f.write_str("the update is not signed as this app"),
            Self::Version {
                found: Some(found),
                release,
            } => write!(f, "the update's app is version {found}, not {release}"),
            Self::Version { found: None, .. } => f.write_str("the update's app gives no version"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Stranded {
                old,
                error,
                restore,
            } => write!(
                f,
                "the update could not move into place ({error}), nor the old app back ({restore}); the old app is at {}",
                old.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A tool's name, from its path.
fn name(tool: &str) -> &str {
    tool.rsplit('/').next().unwrap_or(tool)
}

/// Who signed an app, as `codesign` reports it: the Team ID of the
/// developer's certificate and the app's bundle identifier. Both hold only
/// the characters such names may, so they can be written into a
/// requirement as they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Signer {
    team: String,
    id: String,
}

impl Signer {
    /// The code requirement an update must satisfy, as `codesign -R` takes
    /// it: literal source after `=`, in Apple's Code Signing Requirement
    /// Language. A certificate Apple issued (`anchor apple generic`), the
    /// same identifier, and a signing certificate whose subject's
    /// organizational unit, which holds the Team ID in Apple's developer
    /// certificates, is the same team.
    fn requirement(&self) -> String {
        format!(
            r#"=anchor apple generic and identifier "{}" and certificate leaf[subject.OU] = "{}""#,
            self.id, self.team
        )
    }
}

#[cfg(test)]
impl Signer {
    /// The release's signer, with a Team ID made up, for tests elsewhere in
    /// the updater.
    pub(crate) fn made_up() -> Self {
        Self {
            team: "ABCDE12345".to_owned(),
            id: "ai.brainstrike.viewer".to_owned(),
        }
    }
}

/// An update checked and ready to install.
#[derive(Debug)]
pub(crate) enum Prepared {
    /// Its app is staged at `staged`, to replace the bundle `installed`,
    /// which `signer` signed.
    Swap {
        installed: PathBuf,
        staged: PathBuf,
        signer: Signer,
    },
    /// The disk image passed, to open in Finder.
    Open,
}

/// Who owns the running app, which [`Owner::detect`] found in a bundle.
/// The release's signed bundle is [`Owner::MacBundle`] where it can be
/// replaced and [`Owner::MacUnreplaceable`] where it cannot. A bundle with
/// no Team ID, signed ad hoc, never installs over itself, and neither does
/// an executable outside a bundle: [`Owner::ReleasePage`]. Runs `codesign`
/// and writes beside the bundle to find out.
pub(crate) fn owner() -> Owner {
    match running() {
        Some(app) if signed_by(&app).is_some() => {
            if can_replace(&app) {
                Owner::MacBundle
            } else {
                Owner::MacUnreplaceable
            }
        }
        _ => Owner::ReleasePage,
    }
}

/// The running app's bundle, from its executable's path.
fn running() -> Option<PathBuf> {
    bundle_at(&std::env::current_exe().ok()?)
}

/// The bundle the executable `exe` runs in, by its canonical path: absolute,
/// with no link in it, so what is moved is the bundle itself.
fn bundle_at(exe: &Path) -> Option<PathBuf> {
    let exe = explorer::canonical(exe).ok()?;
    bundle_of(&exe).map(Path::to_path_buf)
}

/// Checks the downloaded disk image `image` of the release `version`
/// against the running app and, when `replace`, stages its app beside the
/// running one. An image that fails is removed: nothing that failed its
/// check is ever opened. Runs the tools.
pub(crate) fn prepare(
    image: &Path,
    version: &semver::Version,
    replace: bool,
) -> Result<Prepared, Error> {
    let prepared = prepare_for_running(image, version, replace);
    if prepared.is_err() {
        let _removed = fs::remove_file(image);
    }
    prepared
}

fn prepare_for_running(
    image: &Path,
    version: &semver::Version,
    replace: bool,
) -> Result<Prepared, Error> {
    let installed = running().ok_or(Error::NotReleased)?;
    let signer = signed_by(&installed).ok_or(Error::NotReleased)?;
    if replace {
        let staged = stage(image, version, &installed, &signer)?;
        Ok(Prepared::Swap {
            installed,
            staged,
            signer,
        })
    } else {
        mounted(image, |app| {
            check(app, &signer).and_then(|()| check_version(app, version))
        })?;
        Ok(Prepared::Open)
    }
}

/// Who signed `app`, from `codesign --display --verbose=2`: none when it is
/// not a signed bundle, or is signed with no Team ID. Only the running
/// app's own signature is read this way; an update is held to it by
/// `codesign`'s requirement check, never by reading its report. Runs
/// `codesign`.
pub(crate) fn signed_by(app: &Path) -> Option<Signer> {
    let mut display = Command::new(CODESIGN);
    display.args(["--display", "--verbose=2"]).arg(app);
    let output = run(CODESIGN, &mut display).ok()?;
    // codesign writes what it displays to standard error.
    signer(&String::from_utf8_lossy(&output.stderr))
}

/// The signer `report`, what `codesign --display --verbose=2` wrote, names
/// in its `TeamIdentifier=` and `Identifier=` lines. None unless each key
/// is on exactly one line, the Team ID is ten capital letters and digits
/// (not `not set`), and the identifier holds only letters, digits, `.` and
/// `-`. The report's first line is the executable's path, which may hold
/// a newline and so seem to add lines: a key found twice, or a value with
/// any other character, is refused.
fn signer(report: &str) -> Option<Signer> {
    let field = |key: &str| {
        // Split on `\n` alone: `lines` would also drop a `\r` before it.
        let mut values = report.split('\n').filter_map(|line| line.strip_prefix(key));
        match (values.next(), values.next()) {
            (Some(value), None) => Some(value),
            _ => None,
        }
    };
    let team = field("TeamIdentifier=")?;
    let id = field("Identifier=")?;
    let team_id = team.len() == TEAM_ID_LENGTH
        && team
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
    let bundle_id = !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
    (team_id && bundle_id).then(|| Signer {
        team: team.to_owned(),
        id: id.to_owned(),
    })
}

/// Whether the bundle `installed` can be replaced: it is not translocated,
/// and a folder can be made beside it, as the staging folder will be. A
/// read-only volume, such as the disk image the app may be run from, or a
/// folder the person may not write, refuses that.
pub(crate) fn can_replace(installed: &Path) -> bool {
    !installed.to_string_lossy().contains(TRANSLOCATED) && installed.parent().is_some_and(writable)
}

/// Whether a folder can be made in `dir`. The folder made to find out is
/// removed.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!("{STAGING}.{}", std::process::id()));
    let made = fs::create_dir(&probe).is_ok();
    if made {
        let _removed = fs::remove_dir(&probe);
    }
    made
}

/// Stages the app in the disk image `image`, the release `version`'s,
/// beside the bundle `installed`: checked as [`check`] says, then copied by
/// `ditto` into `<installed's folder>/.Strikebrain Viewer.update/`, made
/// afresh, and the copy checked the same way, its own version too
/// ([`check_version`]), so what moves into place is what passed. The image
/// is always detached, and on any failure the staging folder is removed.
/// Runs the tools.
pub(crate) fn stage(
    image: &Path,
    version: &semver::Version,
    installed: &Path,
    signer: &Signer,
) -> Result<PathBuf, Error> {
    let staging = installed.with_file_name(STAGING);
    mounted(image, |app| {
        check(app, signer)?;
        let staged = staging.join(app.file_name().ok_or(Error::NoApp)?);
        let copied = copy(app, &staging, &staged)
            .and_then(|()| check(&staged, signer))
            .and_then(|()| check_version(&staged, version));
        if copied.is_err() {
            let _removed = fs::remove_dir_all(&staging);
        }
        copied.map(|()| staged)
    })
}

/// Checks `staged`, the copy [`stage`] made, once more as it is about to
/// move into place: it may have sat beside the app for days. Its signature
/// must still be whole and satisfy `signer`'s requirement ([`satisfies`]).
/// A copy that fails is removed with its staging folder. Runs `codesign`.
pub(crate) fn check_staged(staged: &Path, signer: &Signer) -> Result<(), Error> {
    let checked = satisfies(staged, signer);
    if checked.is_err() {
        remove_staging(staged);
    }
    checked
}

/// Copies `app` to `staged`, in the folder `staging`, which is made afresh:
/// whatever an earlier attempt left there goes first.
fn copy(app: &Path, staging: &Path, staged: &Path) -> Result<(), Error> {
    if let Err(error) = fs::remove_dir_all(staging)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(Error::Io(error));
    }
    fs::create_dir(staging).map_err(Error::Io)?;
    let mut ditto = Command::new(DITTO);
    ditto.arg(app).arg(staged);
    run(DITTO, &mut ditto).map(drop)
}

/// Attaches the disk image `image` at a mount point of its own, hands
/// `then` the one app in it, and detaches it, whatever `then` returned.
fn mounted<T>(image: &Path, then: impl FnOnce(&Path) -> Result<T, Error>) -> Result<T, Error> {
    mounted_with(image, attach, detach, then)
}

/// [`mounted`], attaching with `attach` and detaching with `detach`, which
/// a test replaces. Once `attach` succeeds, `detach` always runs.
fn mounted_with<T>(
    image: &Path,
    attach: impl FnOnce(&Path, &Path) -> Result<(), Error>,
    detach: impl FnOnce(&Path),
    then: impl FnOnce(&Path) -> Result<T, Error>,
) -> Result<T, Error> {
    let mount = mount_point();
    if let Err(error) = attach(image, &mount) {
        let _removed = fs::remove_dir(&mount);
        return Err(error);
    }
    let result = the_app(&mount).and_then(|app| then(&app));
    detach(&mount);
    result
}

/// Attaches the disk image `image` at `mount`: read-only, hidden from
/// Finder and not opened there.
fn attach(image: &Path, mount: &Path) -> Result<(), Error> {
    let mut attach = Command::new(HDIUTIL);
    attach
        .args([
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(mount)
        .arg(image);
    run(HDIUTIL, &mut attach).map(drop)
}

/// A mount point no other attach uses: in the temporary folder, which on
/// macOS is the person's own, named for this process and a count. hdiutil
/// makes it, and removes it on detach.
fn mount_point() -> PathBuf {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let count = COUNT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "strikebrain-viewer-update-{}-{count}",
        std::process::id()
    ))
}

/// Detaches the image mounted at `mount`, by force if something holds it
/// open, and removes the mount point if hdiutil left it. An image that
/// will not detach stays attached, hidden, until the computer restarts.
fn detach(mount: &Path) {
    let detach = |force: bool| {
        let mut detach = Command::new(HDIUTIL);
        detach.arg("detach").arg(mount);
        if force {
            detach.arg("-force");
        }
        run(HDIUTIL, &mut detach)
    };
    if detach(false).is_err() {
        let _forced = detach(true);
    }
    // Only an empty folder goes: never anything on a volume still mounted.
    let _removed = fs::remove_dir(mount);
}

/// The one app at the top of the image mounted at `mount`: a folder, not a
/// link, named `*.app`.
fn the_app(mount: &Path) -> Result<PathBuf, Error> {
    let mut apps = fs::read_dir(mount)
        .map_err(Error::Io)?
        .flatten()
        .filter(|entry| {
            Path::new(&entry.file_name()).extension() == Some(OsStr::new("app"))
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path());
    match (apps.next(), apps.next()) {
        (Some(app), None) => Ok(app),
        _ => Err(Error::NoApp),
    }
}

/// Checks `app` as the update of the app `signer` signed: it satisfies
/// `signer`'s requirement ([`satisfies`]), and Gatekeeper accepts it to run
/// (`spctl --assess --type execute`), which a notarized Developer ID app
/// passes.
fn check(app: &Path, signer: &Signer) -> Result<(), Error> {
    satisfies(app, signer)?;
    let mut assess = Command::new(SPCTL);
    assess.args(["--assess", "--type", "execute"]).arg(app);
    run(SPCTL, &mut assess).map(drop)
}

/// Whether `app`'s signature is valid and seals all of it, nested code too,
/// and satisfies [`Signer::requirement`]: `codesign --verify --deep
/// --strict --test-requirement <requirement>`. `codesign` evaluates the
/// requirement against the signature itself; nothing it prints is read.
/// It exits 3 for code properly signed that fails the requirement.
fn satisfies(app: &Path, signer: &Signer) -> Result<(), Error> {
    let mut verify = Command::new(CODESIGN);
    verify
        .args(["--verify", "--deep", "--strict", "--test-requirement"])
        .arg(signer.requirement())
        .arg(app);
    let output = verify
        .output()
        .map_err(|error| Error::Run(CODESIGN, error))?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(FAILED_REQUIREMENT) => Err(Error::NotThisApp),
        _ => Err(Error::Failed(CODESIGN, said(&output))),
    }
}

/// Checks that `app`'s own version, as its `Info.plist` gives it, is
/// `release`'s: a release re-uploaded with an older build inside, validly
/// signed, would otherwise downgrade the app. Runs `plutil`.
fn check_version(app: &Path, release: &semver::Version) -> Result<(), Error> {
    let found = version_of(app)?;
    if found
        .as_deref()
        .is_some_and(|found| same_version(found, release))
    {
        Ok(())
    } else {
        Err(Error::Version {
            found,
            release: release.clone(),
        })
    }
}

/// The version `app` gives as its `CFBundleShortVersionString`, which the
/// bundle's template sets from Cargo's; none if that is not a string.
/// `plutil -extract CFBundleShortVersionString xml1 -o -` writes the value
/// as a property list of its own, read by [`plist_string`]: plutil's `raw`
/// format, which writes the bare string, came with macOS 12, and the app
/// runs on macOS 11. Runs `plutil`.
fn version_of(app: &Path) -> Result<Option<String>, Error> {
    let mut extract = Command::new(PLUTIL);
    extract
        .args(["-extract", "CFBundleShortVersionString", "xml1", "-o", "-"])
        .arg(app.join("Contents/Info.plist"));
    let output = run(PLUTIL, &mut extract)?;
    Ok(plist_string(&String::from_utf8_lossy(&output.stdout)).map(str::to_owned))
}

/// The string the property list `xml`, as `plutil -extract … xml1` wrote
/// it, holds: the text of its one `<string>` element, as written. None if
/// it holds another type, or more than one string. XML escapes are left as
/// they are: a version has none, and one that held any is not a version.
fn plist_string(xml: &str) -> Option<&str> {
    let (_, rest) = xml.split_once("<string>")?;
    let (value, rest) = rest.split_once("</string>")?;
    (!rest.contains("<string>")).then_some(value)
}

/// Whether `found`, a bundle's version, is `release`, to the letter: a
/// version with nothing around it, equal to the release's.
fn same_version(found: &str, release: &semver::Version) -> bool {
    semver::Version::parse(found).is_ok_and(|version| version == *release)
}

/// Runs `command`, the tool `tool`, with its standard input closed, and
/// returns its output if it succeeded. Each tool exits 0 on success and
/// another status on failure.
fn run(tool: &'static str, command: &mut Command) -> Result<Output, Error> {
    let output = command.output().map_err(|error| Error::Run(tool, error))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(Error::Failed(tool, said(&output)))
    }
}

/// What a tool that failed said: the last line it wrote to standard error,
/// or else to standard output, which follows any warning; else its exit
/// status.
fn said(output: &Output) -> String {
    [&output.stderr, &output.stdout]
        .into_iter()
        .find_map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| output.status.to_string())
}

/// Replaces the bundle `installed` with `staged`, the copy [`stage`] made
/// and [`check_staged`] checked again. The installed app moves into the
/// staging folder as `old.app` and the staged one into its place, both
/// renames on one volume; then the staging folder goes, the old app with
/// it. If the installed app cannot move aside, nothing has moved and the
/// staging folder goes. If the staged app cannot move in, the old one moves
/// back and the staging folder goes; if the old one cannot move back
/// either, nothing is at `installed`, the staging folder stays with the old
/// app in it, and the error ([`Error::Stranded`]) says where it is.
pub(crate) fn swap(installed: &Path, staged: &Path) -> Result<(), Error> {
    swap_with(installed, staged, |from, to| fs::rename(from, to))
}

/// [`swap`], moving each bundle with `rename`, which a test makes fail.
fn swap_with(
    installed: &Path,
    staged: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), Error> {
    let old = staged.with_file_name(OLD);
    if let Err(error) = rename(installed, &old) {
        remove_staging(staged);
        return Err(Error::Io(error));
    }
    if let Err(error) = rename(staged, installed) {
        return match rename(&old, installed) {
            Ok(()) => {
                remove_staging(staged);
                Err(Error::Io(error))
            }
            Err(restore) => Err(Error::Stranded {
                old,
                error,
                restore,
            }),
        };
    }
    // An old app that will not go is removed with the next update's
    // staging folder.
    remove_staging(staged);
    Ok(())
}

/// Removes the staging folder `staged` is in, the old app with it: only a
/// folder named as [`stage`] names it, whatever a caller passes.
fn remove_staging(staged: &Path) {
    if let Some(staging) = staged.parent()
        && staging.file_name() == Some(OsStr::new(STAGING))
    {
        let _removed = fs::remove_dir_all(staging);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::ExitStatus;

    use super::*;

    /// `codesign --display --verbose=2` on the release's app, with the
    /// Team ID made up.
    const RELEASED: &str = "\
Executable=/Applications/Strikebrain Viewer.app/Contents/MacOS/strikebrain-viewer
Identifier=ai.brainstrike.viewer
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 size=52897 flags=0x10000(runtime) hashes=1646+3 location=embedded
Signature size=8994
Authority=Developer ID Application: Example Developer (ABCDE12345)
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=Nov 25, 2026 at 11:18:17 PM
Notarization Ticket=stapled
Info.plist entries=16
TeamIdentifier=ABCDE12345
Runtime Version=26.5.0
Sealed Resources version=2 rules=13 files=2
Internal requirements count=1 size=184
Total signatures=1
Chosen signature=1
";

    /// The same for the app `just bundle` signs ad hoc.
    const AD_HOC: &str = "\
Executable=/Users/me/strikebrain-viewer/target/release/bundle/osx/Strikebrain Viewer.app/Contents/MacOS/strikebrain-viewer
Identifier=ai.brainstrike.viewer
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 size=53110 flags=0x10002(adhoc,runtime) hashes=1653+3 location=embedded
Signature=adhoc
Info.plist entries=16
TeamIdentifier=not set
Runtime Version=27.2.0
Sealed Resources version=2 rules=13 files=2
Internal requirements count=0 size=12
Total signatures=1
Chosen signature=1
";

    /// The same for an app Apple signed, which has a platform identifier
    /// and no Team ID.
    const APPLE: &str = "\
Executable=/System/Applications/Calculator.app/Contents/MacOS/Calculator
Identifier=com.apple.calculator
Format=app bundle with Mach-O universal (arm64e arm64e.x1)
CodeDirectory v=20400 size=3629 flags=0x0(none) hashes=103+7 location=embedded
Platform identifier=26
Signature size=4202
Authority=macOS Software Signing
Authority=Apple Code Signing Certification Authority
Authority=Apple Root CA
Signed Time=Aug 8, 2026 at 11:50:01 PM
Info.plist entries=35
TeamIdentifier=not set
Sealed Resources version=2 rules=2 files=0
Internal requirements count=1 size=68
Total signatures=1
Chosen signature=1
";

    /// `report` without its line that starts with `key`.
    fn without(report: &str, key: &str) -> String {
        report
            .lines()
            .filter(|line| !line.starts_with(key))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A folder laid out as an app bundle, its executable holding `text`.
    fn bundle(app: &Path, text: &str) {
        let macos = app.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("strikebrain-viewer"), text).unwrap();
    }

    /// What the bundle `app`'s executable holds.
    fn executable(app: &Path) -> String {
        fs::read_to_string(app.join("Contents/MacOS/strikebrain-viewer")).unwrap()
    }

    /// The names in `dir`, sorted.
    fn listing(dir: &Path) -> Vec<OsString> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    /// The release's signer, as the recorded report names it.
    fn ours() -> Signer {
        Signer::made_up()
    }

    /// An installed app holding "old" in `apps` and a staged one holding
    /// "new" beside it, as `stage` leaves them.
    fn installed_and_staged(apps: &Path) -> (PathBuf, PathBuf) {
        let installed = apps.join("Strikebrain Viewer.app");
        bundle(&installed, "old");
        let staged = apps.join(STAGING).join("Strikebrain Viewer.app");
        bundle(&staged, "new");
        (installed, staged)
    }

    #[test]
    fn a_signer_is_read_from_codesigns_report() {
        assert_eq!(
            signer(RELEASED),
            Some(Signer {
                team: "ABCDE12345".to_owned(),
                id: "ai.brainstrike.viewer".to_owned(),
            })
        );
    }

    #[test]
    fn an_app_without_a_team_id_has_no_signer() {
        assert_eq!(signer(AD_HOC), None);
        assert_eq!(signer(APPLE), None);
        assert_eq!(signer(""), None);
        assert_eq!(signer(&without(RELEASED, "Identifier=")), None);
        assert_eq!(signer(&without(RELEASED, "TeamIdentifier=")), None);
        assert_eq!(signer("Identifier=\nTeamIdentifier=ABCDE12345\n"), None);
    }

    #[test]
    fn a_report_with_lines_injected_through_the_executables_path_has_no_signer() {
        // An app signed by another team, whose executable's name carries a
        // newline and this app's keys, which come first in the report.
        let injected = RELEASED
            .replace(
                "Identifier=ai.brainstrike.viewer",
                "Identifier=com.example.other",
            )
            .replace("TeamIdentifier=ABCDE12345", "TeamIdentifier=OTHER12345")
            .replacen(
                "/MacOS/strikebrain-viewer\n",
                "/MacOS/x\nIdentifier=ai.brainstrike.viewer\nTeamIdentifier=ABCDE12345\n",
                1,
            );
        assert!(injected.starts_with("Executable="), "{injected}");
        assert_eq!(signer(&injected), None);
    }

    #[test]
    fn a_signer_must_be_a_team_id_and_a_bundle_identifier() {
        let report = |team: &str, id: &str| format!("Identifier={id}\nTeamIdentifier={team}\n");
        assert_eq!(
            signer(&report("ABCDE12345", "ai.brainstrike.viewer")),
            Some(ours())
        );
        for (team, id) in [
            ("abcde12345", "ai.brainstrike.viewer"),
            ("ABCDE1234", "ai.brainstrike.viewer"),
            ("ABCDE123456", "ai.brainstrike.viewer"),
            ("ABCDE-1234", "ai.brainstrike.viewer"),
            ("not set", "ai.brainstrike.viewer"),
            ("ABCDE12345", "ai.\"brainstrike\""),
            ("ABCDE12345", "ai.brainstrike.viewer\r"),
            ("ABCDE12345", "ai brainstrike"),
            ("ABCDE12345", "ai\\brainstrike"),
        ] {
            assert_eq!(signer(&report(team, id)), None, "{team:?} {id:?}");
        }
    }

    #[test]
    fn the_requirement_names_apples_anchor_the_identifier_and_the_team() {
        assert_eq!(
            ours().requirement(),
            r#"=anchor apple generic and identifier "ai.brainstrike.viewer" and certificate leaf[subject.OU] = "ABCDE12345""#
        );
    }

    #[test]
    fn an_app_not_signed_as_this_one_does_not_satisfy_its_requirement() {
        // Every Mac has Calculator, signed by Apple: valid, but not ours.
        let calculator = Path::new("/System/Applications/Calculator.app");
        let result = satisfies(calculator, &ours());
        assert!(matches!(result, Err(Error::NotThisApp)), "{result:?}");
        let tmp = tempfile::tempdir().unwrap();
        let unsigned = tmp.path().join("Unsigned.app");
        bundle(&unsigned, "unsigned");
        let result = satisfies(&unsigned, &ours());
        assert!(
            matches!(result, Err(Error::Failed(CODESIGN, _))),
            "{result:?}"
        );
    }

    #[test]
    fn a_staged_app_that_fails_its_second_check_is_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, staged) = installed_and_staged(tmp.path());
        let result = check_staged(&staged, &ours());
        assert!(
            matches!(result, Err(Error::Failed(CODESIGN, _))),
            "{result:?}"
        );
        assert_eq!(listing(tmp.path()), ["Strikebrain Viewer.app"]);
        assert_eq!(executable(&installed), "old");
    }

    #[test]
    fn the_running_bundle_is_found_by_its_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        bundle(&real.join("Strikebrain Viewer.app"), "app");
        symlink(&real, tmp.path().join("link")).unwrap();
        let exe = tmp
            .path()
            .join("link/Strikebrain Viewer.app/Contents/MacOS/strikebrain-viewer");
        let canonical = explorer::canonical(tmp.path()).unwrap();
        assert_eq!(
            bundle_at(&exe),
            Some(canonical.join("real/Strikebrain Viewer.app"))
        );
        assert_eq!(bundle_at(&tmp.path().join("missing")), None);
    }

    #[test]
    fn the_image_is_detached_whatever_its_check_finds() {
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("update.dmg");
        let attached = RefCell::new(Vec::<PathBuf>::new());
        let detached = RefCell::new(Vec::<PathBuf>::new());
        let attach = |with_app: bool| {
            let attached = &attached;
            move |_image: &Path, mount: &Path| {
                fs::create_dir(mount).unwrap();
                if with_app {
                    bundle(&mount.join("Strikebrain Viewer.app"), "new");
                }
                attached.borrow_mut().push(mount.to_owned());
                Ok(())
            }
        };
        let detach = |mount: &Path| {
            detached.borrow_mut().push(mount.to_owned());
            fs::remove_dir_all(mount).unwrap();
        };
        // The check fails.
        let result: Result<(), Error> =
            mounted_with(&image, attach(true), detach, |_| Err(Error::NotThisApp));
        assert!(matches!(result, Err(Error::NotThisApp)), "{result:?}");
        // The image holds no app.
        let result: Result<(), Error> = mounted_with(&image, attach(false), detach, |_| Ok(()));
        assert!(matches!(result, Err(Error::NoApp)), "{result:?}");
        // The check passes.
        let result = mounted_with(&image, attach(true), detach, |app| Ok(app.to_owned()));
        assert!(result.unwrap().ends_with("Strikebrain Viewer.app"));
        assert_eq!(*detached.borrow(), *attached.borrow());
        assert_eq!(detached.borrow().len(), 3);

        // An image that will not attach is neither checked nor detached.
        let checked = Cell::new(false);
        let result: Result<(), Error> = mounted_with(
            &image,
            |_, _| Err(Error::NoApp),
            detach,
            |_| {
                checked.set(true);
                Ok(())
            },
        );
        assert!(matches!(result, Err(Error::NoApp)), "{result:?}");
        assert!(!checked.get());
        assert_eq!(detached.borrow().len(), 3);
    }

    #[test]
    fn a_folder_that_is_not_signed_has_no_signer() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Unsigned.app");
        bundle(&app, "unsigned");
        assert_eq!(signed_by(&app), None);
        assert_eq!(signed_by(&tmp.path().join("Missing.app")), None);
    }

    #[test]
    fn the_test_runner_is_not_a_released_bundle() {
        // Tests run from a bare executable, as a development build does:
        // the release page updates it.
        assert_eq!(owner(), Owner::ReleasePage);
    }

    #[test]
    fn swap_puts_the_new_bundle_in_place_and_removes_the_old_and_the_staging_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, staged) = installed_and_staged(tmp.path());
        swap(&installed, &staged).unwrap();
        assert_eq!(executable(&installed), "new");
        assert_eq!(listing(tmp.path()), ["Strikebrain Viewer.app"]);
    }

    #[test]
    fn when_the_new_bundle_cannot_move_in_the_old_one_is_put_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, staged) = installed_and_staged(tmp.path());
        let refused = |from: &Path, to: &Path| {
            if from == staged {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                fs::rename(from, to)
            }
        };
        let result = swap_with(&installed, &staged, refused);
        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
        assert_eq!(executable(&installed), "old");
        // The staged copy goes with its folder.
        assert_eq!(listing(tmp.path()), ["Strikebrain Viewer.app"]);
    }

    #[test]
    fn when_the_old_bundle_cannot_move_aside_nothing_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, staged) = installed_and_staged(tmp.path());
        let refused = |from: &Path, to: &Path| {
            if from == installed {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                fs::rename(from, to)
            }
        };
        let result = swap_with(&installed, &staged, refused);
        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
        assert_eq!(executable(&installed), "old");
        assert_eq!(listing(tmp.path()), ["Strikebrain Viewer.app"]);
    }

    #[test]
    fn when_the_old_bundle_cannot_move_back_it_is_kept_in_the_staging_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, staged) = installed_and_staged(tmp.path());
        let old = staged.with_file_name("old.app");
        let refused = |from: &Path, to: &Path| {
            if from == staged || from == old {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                fs::rename(from, to)
            }
        };
        let result = swap_with(&installed, &staged, refused);
        let Err(Error::Stranded { old: kept, .. }) = &result else {
            panic!("{result:?}");
        };
        assert_eq!(kept, &old);
        assert!(!installed.exists());
        assert_eq!(executable(&old), "old");
        assert_eq!(executable(&staged), "new");
        // The message says where the old app is.
        let message = result.unwrap_err().to_string();
        assert!(message.contains(&old.display().to_string()), "{message}");
    }

    #[test]
    fn a_folder_that_is_not_the_staging_folder_is_never_removed() {
        // A path that is not in the staging folder never has its folder
        // removed, whatever a caller passes.
        let tmp = tempfile::tempdir().unwrap();
        let installed = tmp.path().join("apps/Strikebrain Viewer.app");
        bundle(&installed, "old");
        let elsewhere = tmp.path().join("elsewhere/Strikebrain Viewer.app");
        bundle(&elsewhere, "new");
        fs::write(tmp.path().join("elsewhere/keep"), "keep").unwrap();
        swap(&installed, &elsewhere).unwrap();
        assert_eq!(executable(&installed), "new");
        assert_eq!(listing(&tmp.path().join("elsewhere")), ["keep", "old.app"]);
    }

    #[test]
    fn a_bundle_in_a_folder_that_can_be_written_can_be_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = tmp.path().join("Strikebrain Viewer.app");
        bundle(&installed, "old");
        assert!(can_replace(&installed));
        // The check leaves nothing behind.
        assert_eq!(listing(tmp.path()), ["Strikebrain Viewer.app"]);
    }

    #[test]
    fn a_bundle_under_a_read_only_folder_cannot_be_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let apps = tmp.path().join("Applications");
        let installed = apps.join("Strikebrain Viewer.app");
        bundle(&installed, "old");
        fs::set_permissions(&apps, fs::Permissions::from_mode(0o555)).unwrap();
        let replaceable = can_replace(&installed);
        // Writable again, so the temporary folder can go.
        fs::set_permissions(&apps, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!replaceable);
        assert_eq!(listing(&apps), ["Strikebrain Viewer.app"]);
    }

    #[test]
    fn a_translocated_bundle_cannot_be_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = tmp
            .path()
            .join("AppTranslocation/0A1B2C3D/d/Strikebrain Viewer.app");
        bundle(&installed, "old");
        assert!(!can_replace(&installed));
    }

    #[test]
    fn the_one_app_in_an_image_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mount = tmp.path();
        symlink("/Applications", mount.join("Applications")).unwrap();
        // A link named as an app is not one.
        symlink("/Applications", mount.join("Link.app")).unwrap();
        fs::write(mount.join("Readme.app"), "a file").unwrap();
        assert!(matches!(the_app(mount), Err(Error::NoApp)));
        bundle(&mount.join("Strikebrain Viewer.app"), "new");
        assert_eq!(
            the_app(mount).unwrap(),
            mount.join("Strikebrain Viewer.app")
        );
        bundle(&mount.join("Other.app"), "other");
        assert!(matches!(the_app(mount), Err(Error::NoApp)));
    }

    #[test]
    fn an_image_with_no_released_app_to_check_against_is_removed() {
        // Tests run outside a bundle, as a development build does.
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("update.dmg");
        for replace in [true, false] {
            fs::write(&image, "a disk image").unwrap();
            let result = prepare(&image, &semver::Version::new(9, 9, 9), replace);
            assert!(matches!(result, Err(Error::NotReleased)), "{result:?}");
            assert_eq!(listing(tmp.path()), Vec::<OsString>::new());
        }
    }

    #[test]
    fn a_tool_that_failed_is_reported_by_the_last_line_it_wrote() {
        let output = Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: b"/dev/disk5\tGUID_partition_scheme\n".to_vec(),
            stderr:
                b"hdiutil: WARNING: deprecated.\nhdiutil: attach failed - image not recognized\n\n"
                    .to_vec(),
        };
        let last = said(&output);
        assert_eq!(last, "hdiutil: attach failed - image not recognized");
        assert_eq!(
            Error::Failed(HDIUTIL, last).to_string(),
            "hdiutil failed: hdiutil: attach failed - image not recognized"
        );
        let to_stdout = Output {
            stderr: Vec::new(),
            ..output.clone()
        };
        assert_eq!(said(&to_stdout), "/dev/disk5\tGUID_partition_scheme");
        let silent = Output {
            stdout: Vec::new(),
            stderr: Vec::new(),
            ..output
        };
        assert_eq!(said(&silent), "exit status: 1");
    }

    /// A bundle at `app` whose `Info.plist` gives `version` as its version,
    /// as the bundle's template writes it.
    fn bundle_of_version(app: &Path, version: &str) {
        bundle(app, "app");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>ai.brainstrike.viewer</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
</dict>
</plist>
"#
        );
        fs::write(app.join("Contents/Info.plist"), plist).unwrap();
    }

    #[test]
    fn only_the_releases_own_version_is_the_release() {
        let release = semver::Version::new(0, 1, 3);
        assert!(same_version("0.1.3", &release));
        for other in [
            "0.1.2",
            "0.1.4",
            "1.0.0",
            "0.1.3-rc.1",
            "v0.1.3",
            "0.1.3 ",
            "0.1",
            "",
        ] {
            assert!(!same_version(other, &release), "{other:?}");
        }
    }

    #[test]
    fn a_string_is_read_from_what_plutil_extracts() {
        let extracted = |value: &str| {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n{value}\n</plist>\n"
            )
        };
        assert_eq!(
            plist_string(&extracted("<string>0.1.3</string>")),
            Some("0.1.3")
        );
        assert_eq!(plist_string(&extracted("<string></string>")), Some(""));
        for other in [
            "<integer>3</integer>",
            "<array><string>0.1.3</string><string>0.1.4</string></array>",
            "<string>0.1.3",
            "",
        ] {
            assert_eq!(plist_string(&extracted(other)), None, "{other:?}");
        }
    }

    #[test]
    fn an_apps_version_is_read_from_its_info_plist() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Strikebrain Viewer.app");
        bundle_of_version(&app, "9.9.9");
        assert_eq!(version_of(&app).unwrap().as_deref(), Some("9.9.9"));
        check_version(&app, &semver::Version::new(9, 9, 9)).unwrap();
        // A release whose app is an older build: a downgrade.
        let older = check_version(&app, &semver::Version::new(10, 0, 0));
        let Err(error @ Error::Version { .. }) = &older else {
            panic!("{older:?}");
        };
        assert_eq!(
            error.to_string(),
            "the update's app is version 9.9.9, not 10.0.0"
        );
        // A version that is not a string.
        let odd = tmp.path().join("Odd.app");
        bundle_of_version(&odd, "9.9.9");
        let plist = odd.join("Contents/Info.plist");
        let text = fs::read_to_string(&plist).unwrap();
        let text = text.replace("<string>9.9.9</string>", "<integer>9</integer>");
        fs::write(&plist, text).unwrap();
        let not_a_string = check_version(&odd, &semver::Version::new(9, 9, 9));
        let Err(error @ Error::Version { found: None, .. }) = &not_a_string else {
            panic!("{not_a_string:?}");
        };
        assert_eq!(error.to_string(), "the update's app gives no version");
        // No Info.plist: plutil says why.
        let bare = tmp.path().join("Bare.app");
        bundle(&bare, "bare");
        let missing = check_version(&bare, &semver::Version::new(9, 9, 9));
        assert!(
            matches!(missing, Err(Error::Failed(PLUTIL, _))),
            "{missing:?}"
        );
    }

    #[test]
    fn a_file_that_is_not_a_disk_image_stages_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("not.dmg");
        fs::write(&image, "not a disk image").unwrap();
        let apps = tmp.path().join("Applications");
        let installed = apps.join("Strikebrain Viewer.app");
        bundle(&installed, "old");
        let signer = Signer {
            team: "ABCDE12345".to_owned(),
            id: "ai.brainstrike.viewer".to_owned(),
        };
        let result = stage(&image, &semver::Version::new(9, 9, 9), &installed, &signer);
        assert!(matches!(result, Err(Error::Failed(..))), "{result:?}");
        assert_eq!(listing(&apps), ["Strikebrain Viewer.app"]);
    }
}
