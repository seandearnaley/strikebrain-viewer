//! Learning of a newer release and installing it. The [`Updater`] entity
//! checks GitHub's latest release 5 s after launch and then once a day by
//! the calendar, or when asked; downloads and verifies the file that
//! updates this installation; and offers it. It follows GPUI Kit's guide,
//! Auto Update: a foreground task coordinates each check, the background
//! executor reads the feed, downloads the file and checks it, and their
//! results come back to the entity, which notifies. The root view owns the
//! updater, saves its two settings, shows its offer in the bar, and shows
//! as alerts its answers to a manual check and why an install failed.
//!
//! Each platform's last step is its own module: on macOS, `macos` checks
//! the disk image's app against the running one and replaces the bundle;
//! on Windows, `windows` checks the installer's signature against the
//! running app's and runs it, and it relaunches the app; on Linux, `linux`
//! opens the `.deb` with `xdg-open`.

mod download;
mod feed;
mod install;
#[cfg(any(not(any(target_os = "macos", windows)), test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod test_server;
#[cfg(any(windows, test))]
mod windows;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, EventEmitter, Task};
use ureq::Agent;

pub(crate) use download::updates_dir;
use feed::{Asset, Release};
pub use install::Owner;

/// How long after launch the first automatic check waits, so it does not
/// compete with the first scans.
const FIRST_CHECK: Duration = Duration::from_secs(5);

/// How long after the last check, by the calendar, an automatic check is
/// due.
const DAY: Duration = Duration::from_hours(24);

/// How often the updater looks at the calendar for a check that is due.
/// GPUI's timers stop while the computer sleeps, so a day-long timer would
/// stretch over the nights a laptop sleeps.
const HOUR: Duration = Duration::from_hours(1);

/// The update settings the state file keeps. The default, with nothing
/// saved, checks automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateSettings {
    /// Whether to check automatically: 5 seconds after launch, then a day
    /// after the last check, by the calendar.
    pub automatic: bool,
    /// When a check last found the app up to date, in seconds since the
    /// Unix epoch. None since a check found an update.
    pub last_check: Option<u64>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            automatic: true,
            last_check: None,
        }
    }
}

/// What the root view hands its updater, with the saved settings: where to
/// read the latest release, and this installation.
#[derive(Debug)]
pub struct UpdateOptions {
    /// The address of the latest release's JSON: GitHub's, or a feed
    /// served on this machine.
    pub feed: String,
    /// The version running: a release is an update when it is greater.
    pub running: semver::Version,
    /// Who owns this installation, and so which file updates it: what
    /// `Owner::detect` finds as the app opens. On macOS and Windows each
    /// check that finds an update asks again whether the release signed the
    /// app, and on macOS whether the bundle can be replaced.
    pub owner: Owner,
    /// Where each version's files download: the cache folder's `updates`,
    /// or a temporary folder in tests. `None` where the platform gives no
    /// cache folder: a check that finds a file to download then fails.
    pub updates_dir: Option<PathBuf>,
}

/// Where the updater is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Waiting for a check.
    Idle,
    /// Reading the latest release.
    Checking,
    /// Downloading the file of `version`, checking its size and digest as
    /// it streams.
    Downloading { version: semver::Version },
    /// Checking the downloaded file of `version` as its platform asks: its
    /// signature against the running app's. Again as the update's button is
    /// pressed: on macOS the staged copy's signature, on Windows the
    /// installer's digest, and the digest of a file that opens with the
    /// system: the disk image, or the `.deb`.
    Verifying { version: semver::Version },
    /// An update is ready to install. It stays until used or the app quits.
    Ready(Offer),
    /// The last check failed, for this reason.
    Failed(String),
}

/// An update ready to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) version: semver::Version,
    pub(crate) how: How,
}

/// How an update installs, which names the bar's button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum How {
    /// The app is replaced, by itself or by the installer, and restarts:
    /// the macOS bundle, an install by the Windows installer.
    #[cfg_attr(
        all(not(test), not(any(target_os = "macos", windows))),
        expect(
            dead_code,
            reason = "offered on macOS and Windows; elsewhere another program installs"
        )
    )]
    Restart,
    /// Another program installs it: the disk image opens, the `.deb` opens
    /// with `xdg-open`, or the release page opens.
    Install,
}

/// What installing the update offered does.
#[derive(Debug)]
enum Plan {
    /// Opens the release's page, where the person installs it.
    Page,
    /// Opens `file` with the system once it is checked again against
    /// `asset`, the release's file: on macOS the disk image in Finder,
    /// where the bundle cannot be replaced; on Linux the `.deb`, with
    /// `xdg-open`.
    #[cfg_attr(
        all(windows, not(test)),
        expect(dead_code, reason = "Windows runs its installer and opens no file")
    )]
    Open { file: PathBuf, asset: Asset },
    /// Replaces the bundle with the staged copy and relaunches.
    #[cfg(target_os = "macos")]
    Swap(Swap),
    /// Runs `installer`, the release's file `asset` of `version`, once it
    /// is checked against `asset` again, and quits; the installer
    /// relaunches the app.
    #[cfg(windows)]
    Run {
        version: semver::Version,
        installer: PathBuf,
        asset: Asset,
    },
}

/// The macOS swap an update offers, as [`Updater::swap_in`] carries it out.
#[cfg(target_os = "macos")]
#[derive(Debug)]
struct Swap {
    /// The version the staged app is.
    version: semver::Version,
    /// The running bundle, which the staged app replaces.
    installed: PathBuf,
    /// The checked copy of the update's app, beside the running bundle.
    staged: PathBuf,
    /// Who signed the running app, whose requirement the copy must satisfy
    /// again before it moves in.
    signer: macos::Signer,
    /// The disk image the copy came from, which opens if the swap does not
    /// happen.
    image: PathBuf,
    /// The release's file, which the disk image is checked against again
    /// before it opens.
    asset: Asset,
}

/// What the root view says: how a manual check ended with nothing to
/// install, or why an install failed after its button was pressed. An
/// update found shows its button instead; an automatic check says nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateEvent {
    /// The running version, this one, is the latest.
    UpToDate(semver::Version),
    /// The check failed, for this reason.
    CheckFailed(String),
    /// The install failed, for this reason, and the release page opened,
    /// or the file to install from did.
    InstallFailed(String),
}

/// Checks for a newer release, on its schedule or when asked, one check at
/// a time; downloads and verifies the file that updates this installation;
/// and offers the update until it is used or the app quits.
pub(crate) struct Updater {
    feed: String,
    running: semver::Version,
    owner: Owner,
    updates_dir: Option<PathBuf>,
    settings: UpdateSettings,
    phase: Phase,
    /// Whether someone asked for the check under way, and so hears how it
    /// ends.
    asked: bool,
    /// The page of the release found, which opens where the person installs
    /// the update: only an address `download::allowed` passes.
    page: Option<String>,
    /// What installing the update offered does; set with the offer.
    plan: Option<Plan>,
    /// When the last check started, settled or not, in seconds since the
    /// Unix epoch. Kept only while the app runs: an automatic check waits a
    /// day after it too, so a release whose file fails to download is not
    /// downloaded again every hour.
    last_attempt: Option<u64>,
    /// Set when the updater goes, so a download under way stops.
    cancel: Arc<AtomicBool>,
    /// The step of the check under way.
    step: Option<Task<()>>,
    /// The automatic checks.
    _schedule: Task<()>,
}

impl EventEmitter<UpdateEvent> for Updater {}

impl Updater {
    /// An updater as `options` and `settings` say, its automatic checks
    /// scheduled on GPUI's timer.
    pub(crate) fn new(
        options: UpdateOptions,
        settings: UpdateSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        let UpdateOptions {
            feed,
            running,
            owner,
            updates_dir,
        } = options;
        Self {
            feed,
            running,
            owner,
            updates_dir,
            settings,
            phase: Phase::Idle,
            asked: false,
            page: None,
            plan: None,
            last_attempt: None,
            cancel: Arc::default(),
            step: None,
            _schedule: schedule(cx),
        }
    }

    /// The update ready to install, if any.
    pub(crate) fn offer(&self) -> Option<&Offer> {
        match &self.phase {
            Phase::Ready(offer) => Some(offer),
            _ => None,
        }
    }

    /// Whether updates are checked for automatically.
    pub(crate) fn automatic(&self) -> bool {
        self.settings.automatic
    }

    /// When a check last found the app up to date, in seconds since the
    /// Unix epoch.
    pub(crate) fn last_check(&self) -> Option<u64> {
        self.settings.last_check
    }

    /// Turns the automatic checks on or off. A manual check works either
    /// way.
    pub(crate) fn set_automatic(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.automatic = on;
        cx.notify();
    }

    /// Checks for a newer release now. A `manual` check hears how it ended
    /// when there is nothing to install; an automatic one says nothing.
    /// While a check is under way, a manual one hears how that one ends.
    /// While an update is ready there is nothing new to say: the offer
    /// stands, in the bar, or there once the presentation ends.
    pub(crate) fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        match self.phase {
            Phase::Idle | Phase::Failed(_) => {}
            Phase::Checking | Phase::Downloading { .. } | Phase::Verifying { .. } => {
                self.asked |= manual;
                return;
            }
            Phase::Ready(_) => return,
        }
        self.asked = manual;
        self.last_attempt = Some(now(cx));
        self.phase = Phase::Checking;
        cx.notify();
        let feed = self.feed.clone();
        let running = self.running.clone();
        let owner = self.owner;
        let reading = cx.background_spawn(async move { read(&feed, &running, owner) });
        self.step = Some(cx.spawn(async move |this, cx| {
            let read = reading.await;
            // The updater is gone only as its window closes.
            this.update(cx, |this, cx| this.found(read, cx))
                .unwrap_or_default();
        }));
    }

    /// Installs the update offered, through whatever owns the installation,
    /// as its plan says. The offer is then used. A file that opens with the
    /// system is checked against the release again first
    /// ([`Self::open_checked`]); on macOS a staged copy is checked again
    /// before it replaces the bundle ([`Self::swap_in`]); on Windows the
    /// installer is checked against the release again before it runs
    /// (`Self::run_installer`). An install that fails is never silent
    /// ([`Self::install_failed`]): the root view says why, and the release
    /// page opens, or the file to install from.
    pub(crate) fn install(&mut self, cx: &mut Context<Self>) {
        let Some(version) = self.offer().map(|offer| offer.version.clone()) else {
            return;
        };
        let Some(plan) = self.plan.take() else {
            return;
        };
        match plan {
            Plan::Page => {
                self.phase = Phase::Idle;
                cx.notify();
                if let Err(reason) = self.open_page(cx) {
                    self.install_failed(reason, false, cx);
                }
            }
            Plan::Open { file, asset } => self.open_checked(version, file, asset, None, cx),
            #[cfg(target_os = "macos")]
            Plan::Swap(swap) => self.swap_in(swap, cx),
            #[cfg(windows)]
            Plan::Run {
                version,
                installer,
                asset,
            } => self.run_installer(version, installer, asset, cx),
        }
    }

    /// Checks the staged copy again on the background executor, as
    /// **Restart to Update** asks for it: it may have sat beside the app
    /// for days. Meanwhile the update is verifying again, so the button
    /// goes and no check starts.
    #[cfg(target_os = "macos")]
    fn swap_in(&mut self, swap: Swap, cx: &mut Context<Self>) {
        self.phase = Phase::Verifying {
            version: swap.version.clone(),
        };
        cx.notify();
        let (staged, signer) = (swap.staged.clone(), swap.signer.clone());
        let checking = cx.background_spawn(async move { macos::check_staged(&staged, &signer) });
        self.step = Some(cx.spawn(async move |this, cx| {
            let checked = checking.await;
            this.update(cx, |this, cx| this.swapped(checked, swap, cx))
                .unwrap_or_default();
        }));
    }

    /// The second check's end. Where it passed and the swap worked, the app
    /// relaunches as the new version. Otherwise the install failed, for a
    /// reason the root view gives, and the disk image, checked against the
    /// release again, opens so the person can install it; an image that is
    /// no longer the release's is removed, and the release page opens in
    /// its place. A copy that failed its check is removed, and the old app
    /// is still in place. A swap that failed has put the old app back, or,
    /// if that failed too, left it in the staging folder, which the reason
    /// names.
    #[cfg(target_os = "macos")]
    fn swapped(&mut self, checked: Result<(), macos::Error>, swap: Swap, cx: &mut Context<Self>) {
        let Swap {
            version,
            installed,
            staged,
            image,
            asset,
            ..
        } = swap;
        match checked.and_then(|()| macos::swap(&installed, &staged)) {
            Ok(()) => {
                cx.set_restart_path(installed);
                cx.restart();
            }
            Err(error) => self.open_checked(version, image, asset, Some(error.to_string()), cx),
        }
    }

    /// Hashes the installer of `version` again on the background executor,
    /// as **Restart to Update** asks for it: it may have sat in the cache
    /// folder for days. One that is still the release's file `asset` starts
    /// there too, off the UI thread, where starting a process can run the
    /// window's messages. Meanwhile the update is verifying again, so the
    /// button goes and no check starts.
    #[cfg(windows)]
    fn run_installer(
        &mut self,
        version: semver::Version,
        installer: PathBuf,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        self.phase = Phase::Verifying { version };
        cx.notify();
        let starting = cx.background_spawn(async move {
            download::check_again(&installer, &asset).map_err(|error| error.to_string())?;
            windows::run(&installer).map_err(|error| error.to_string())
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let started = starting.await;
            this.update(cx, |this, cx| this.started(started, cx))
                .unwrap_or_default();
        }));
    }

    /// The installer's start. Once it runs, the app quits, so none of its
    /// files is in use, and the installer relaunches it. Otherwise the
    /// install failed, for a reason the root view gives, and the release
    /// page opens so the person can install it. An installer that is no
    /// longer the release's was removed.
    #[cfg(windows)]
    fn started(&mut self, started: Result<(), String>, cx: &mut Context<Self>) {
        match started {
            Ok(()) => cx.quit(),
            Err(reason) => self.install_failed(reason, true, cx),
        }
    }

    /// Opens `file`, the release's file `asset` of `version`, with the
    /// system: the disk image in Finder, or the `.deb` with `xdg-open`. It
    /// is checked against `asset` again first, on the background executor:
    /// it may have sat in the cache folder for days. Meanwhile the update
    /// is verifying again, so the button goes and no check starts. `failed`
    /// is why the install it stands in for failed, if one did: the macOS
    /// swap.
    fn open_checked(
        &mut self,
        version: semver::Version,
        file: PathBuf,
        asset: Asset,
        failed: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.phase = Phase::Verifying { version };
        cx.notify();
        let checking = cx.background_spawn(async move {
            download::check_again(&file, &asset)
                .map(|()| file)
                .map_err(|error| error.to_string())
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let checked = checking.await;
            this.update(cx, |this, cx| this.opened(checked, failed, cx))
                .unwrap_or_default();
        }));
    }

    /// The file's second check's end. A file still the release's opens;
    /// one that is not was removed, and the install failed, the release
    /// page opening in its place. Either way an install that failed before,
    /// as `failed` says, is reported. A manual check asked for meanwhile
    /// hears only that: what opened is its answer.
    fn opened(
        &mut self,
        checked: Result<PathBuf, String>,
        failed: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match (checked, failed) {
            (Ok(file), None) => {
                cx.open_with_system(&file);
                self.asked = false;
                self.phase = Phase::Idle;
                cx.notify();
            }
            (Ok(file), Some(reason)) => {
                cx.open_with_system(&file);
                self.install_failed(reason, false, cx);
            }
            (Err(reason), failed) => self.install_failed(failed.unwrap_or(reason), true, cx),
        }
    }

    /// Ends an install that failed after its button was pressed, for
    /// `reason`, which the root view gives, as the answer to a manual check
    /// asked for meanwhile too. With `page`, the release's page opens,
    /// where the person can install the update by hand. As for a check
    /// that failed, the next attempt waits a day.
    fn install_failed(&mut self, reason: String, page: bool, cx: &mut Context<Self>) {
        self.asked = false;
        self.last_attempt = Some(now(cx));
        if page {
            // The install's reason is the one to give; a page refused too
            // adds nothing to it.
            let _opened = self.open_page(cx);
        }
        cx.emit(UpdateEvent::InstallFailed(reason.clone()));
        self.phase = Phase::Failed(reason);
        cx.notify();
    }

    /// Opens the page of the release found, where the person installs the
    /// update by hand, as [`download::allowed`] allows: the feed's page
    /// could be any address, and the system would open any.
    fn open_page(&self, cx: &mut App) -> Result<(), String> {
        let page = self.page.as_deref().unwrap_or_default();
        if !download::allowed(page) {
            return Err(download::Error::NotHttps(page.to_owned()).to_string());
        }
        cx.open_url(page);
        Ok(())
    }

    /// The automatic check, when automatic checks are on and both the last
    /// check that found the app up to date and the last attempt are a day
    /// old or more by the calendar.
    fn check_if_due(&mut self, cx: &mut Context<Self>) {
        let now = now(cx);
        if self.settings.automatic
            && !recent(self.settings.last_check, now)
            && !recent(self.last_attempt, now)
        {
            self.check(false, cx);
        }
    }

    /// What the latest release says: nothing newer, a file to download, or
    /// an update the release page installs: there the owner, as the check
    /// found it, has no file, or the release lacks this installation's.
    fn found(&mut self, read: Result<(Agent, Release, Owner), String>, cx: &mut Context<Self>) {
        let (agent, release, owner) = match read {
            Ok(read) => read,
            Err(reason) => return self.end(Phase::Failed(reason), cx),
        };
        if !feed::newer(&release, &self.running) {
            return self.end(Phase::Idle, cx);
        }
        self.page = Some(release.page.clone());
        match feed::file_for(&release, owner).cloned() {
            Some(asset) => self.download(agent, asset, release.version, owner, cx),
            None => self.make_offer(release.version, How::Install, Plan::Page, cx),
        }
    }

    /// Downloads `asset`, the file of `version` for an installation `owner`
    /// owns, into that version's folder under the updates folder, on the
    /// background executor.
    fn download(
        &mut self,
        agent: Agent,
        asset: Asset,
        version: semver::Version,
        owner: Owner,
        cx: &mut Context<Self>,
    ) {
        self.phase = Phase::Downloading {
            version: version.clone(),
        };
        cx.notify();
        let updates = self.updates_dir.clone();
        let cancel = Arc::clone(&self.cancel);
        let wanted = asset.clone();
        let downloading = cx.background_spawn(async move {
            let updates = updates.ok_or(download::Error::NoCacheFolder)?;
            let into = download::staging_dir(&updates, &version)?;
            download::file(&agent, &wanted, &into, &cancel)
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let downloaded = downloading.await.map_err(|error| error.to_string());
            this.update(cx, |this, cx| this.downloaded(downloaded, owner, asset, cx))
                .unwrap_or_default();
        }));
    }

    /// The download's end: its size and digest were the release's, and the
    /// file goes on to its platform's check, or it failed and nothing is
    /// left of it.
    fn downloaded(
        &mut self,
        downloaded: Result<PathBuf, String>,
        owner: Owner,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        let Phase::Downloading { version } = &self.phase else {
            return;
        };
        let version = version.clone();
        match downloaded {
            Ok(file) => self.verify(file, version, owner, asset, cx),
            Err(reason) => self.end(Phase::Failed(reason), cx),
        }
    }

    /// Checks the downloaded disk image `image` of `version` on the
    /// background executor, staging its app where the bundle can be
    /// replaced, then offers it: **Restart to Update** for a bundle that
    /// can be replaced, **Install Update**, which opens the image, for one
    /// that cannot. Another owner's file, which tests download on a Mac,
    /// is installed from the release page, as elsewhere.
    #[cfg(target_os = "macos")]
    fn verify(
        &mut self,
        image: PathBuf,
        version: semver::Version,
        owner: Owner,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        let replace = match owner {
            Owner::MacBundle => true,
            Owner::MacUnreplaceable => false,
            _ => return self.make_offer(version, How::Install, Plan::Page, cx),
        };
        let release = version.clone();
        self.phase = Phase::Verifying { version };
        cx.notify();
        let verifying = cx.background_spawn(async move {
            macos::prepare(&image, &release, replace)
                .map(|prepared| (prepared, image))
                .map_err(|error| error.to_string())
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let verified = verifying.await;
            this.update(cx, |this, cx| this.verified(verified, asset, cx))
                .unwrap_or_default();
        }));
    }

    /// Checks the downloaded installer `installer` of `version` on the
    /// background executor against the running app's signature, then
    /// offers it: **Restart to Update**, which runs it. Another owner's
    /// file, which tests download, is installed from the release page.
    #[cfg(windows)]
    fn verify(
        &mut self,
        installer: PathBuf,
        version: semver::Version,
        owner: Owner,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        if owner != Owner::WindowsInstaller {
            return self.make_offer(version, How::Install, Plan::Page, cx);
        }
        self.phase = Phase::Verifying { version };
        cx.notify();
        let verifying = cx.background_spawn(async move {
            windows::check(&installer)
                .map(|()| installer)
                .map_err(|error| error.to_string())
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let verified = verifying.await;
            this.update(cx, |this, cx| this.verified(verified, asset, cx))
                .unwrap_or_default();
        }));
    }

    /// Offers the downloaded `.deb` `package` of `version` at once: its
    /// check, against the release's size and digest, was the download's.
    /// **Install Update** opens it with `xdg-open`. Another
    /// owner's file, which tests download, is installed from the release
    /// page.
    #[cfg(not(any(target_os = "macos", windows)))]
    fn verify(
        &mut self,
        package: PathBuf,
        version: semver::Version,
        owner: Owner,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        let plan = linux::plan(owner, package, asset);
        self.make_offer(version, How::Install, plan, cx);
    }

    /// The disk image's check's end: the update is offered, or it failed
    /// and the image and any staged copy are gone.
    #[cfg(target_os = "macos")]
    fn verified(
        &mut self,
        verified: Result<(macos::Prepared, PathBuf), String>,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        let Phase::Verifying { version } = &self.phase else {
            return;
        };
        let version = version.clone();
        match verified {
            Ok((
                macos::Prepared::Swap {
                    installed,
                    staged,
                    signer,
                },
                image,
            )) => {
                let plan = Plan::Swap(Swap {
                    version: version.clone(),
                    installed,
                    staged,
                    signer,
                    image,
                    asset,
                });
                self.make_offer(version, How::Restart, plan, cx);
            }
            Ok((macos::Prepared::Open, image)) => {
                let plan = Plan::Open { file: image, asset };
                self.make_offer(version, How::Install, plan, cx);
            }
            Err(reason) => self.end(Phase::Failed(reason), cx),
        }
    }

    /// The installer's check's end: the update is offered, or it failed and
    /// the installer is gone.
    #[cfg(windows)]
    fn verified(
        &mut self,
        verified: Result<PathBuf, String>,
        asset: Asset,
        cx: &mut Context<Self>,
    ) {
        let Phase::Verifying { version } = &self.phase else {
            return;
        };
        let version = version.clone();
        match verified {
            Ok(installer) => {
                let plan = Plan::Run {
                    version: version.clone(),
                    installer,
                    asset,
                };
                self.make_offer(version, How::Restart, plan, cx);
            }
            Err(reason) => self.end(Phase::Failed(reason), cx),
        }
    }

    /// Offers the update of `version`, which installs as `how` names it and
    /// `plan` does, until it is used or the app quits.
    fn make_offer(
        &mut self,
        version: semver::Version,
        how: How,
        plan: Plan,
        cx: &mut Context<Self>,
    ) {
        self.plan = Some(plan);
        self.end(Phase::Ready(Offer { version, how }), cx);
    }

    /// Ends a check in `phase`. One asked for hears how it went when there
    /// is nothing to install. A check that found the app up to date is the
    /// last check, which is saved. One that found an update clears it: the
    /// app is no longer known to be up to date, so if the update is not
    /// used, the next launch checks again 5 s in and offers it again. One
    /// that failed leaves it, and the first look a day after its attempt
    /// tries again.
    fn end(&mut self, phase: Phase, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.asked) {
            match &phase {
                Phase::Idle => cx.emit(UpdateEvent::UpToDate(self.running.clone())),
                Phase::Failed(reason) => cx.emit(UpdateEvent::CheckFailed(reason.clone())),
                _ => {}
            }
        }
        match phase {
            Phase::Idle => self.settings.last_check = Some(now(cx)),
            Phase::Ready(_) => self.settings.last_check = None,
            _ => {}
        }
        self.phase = phase;
        cx.notify();
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        // A download under way stops at its next read and removes what it
        // wrote.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The automatic checks: a look 5 s after launch, then every hour, for a
/// check that is due.
fn schedule(cx: &mut Context<Updater>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let mut wait = FIRST_CHECK;
        loop {
            cx.background_executor().timer(wait).await;
            // The updater is gone.
            if this.update(cx, Updater::check_if_due).is_err() {
                return;
            }
            wait = HOUR;
        }
    })
}

/// Reads the latest release at `feed`, with the agent that read it, which
/// then downloads its file, and the owner of this installation. That is
/// `owner`, as [`Owner::detect`] found it, asked of the system again
/// ([`refine`]) when the release is newer than `running`. Blocking: runs on
/// the background executor.
fn read(
    feed: &str,
    running: &semver::Version,
    owner: Owner,
) -> Result<(Agent, Release, Owner), String> {
    let agent = download::agent();
    let json = download::fetch_json(&agent, feed).map_err(|error| error.to_string())?;
    let release = feed::parse(&json).map_err(|error| error.to_string())?;
    // Only an update needs to know: on macOS asking runs `codesign` and
    // writes beside the app.
    let owner = if feed::newer(&release, running) {
        refine(owner)
    } else {
        owner
    };
    Ok((agent, release, owner))
}

/// The owner of this installation, which [`Owner::detect`] found to be
/// `owner`, as the system says now. On macOS a bundle is the release's
/// only if signed with a Team ID, and can be replaced only where its folder
/// can be written: `macos::owner`. On Windows an install by the installer
/// is updated by it only if the app is validly signed: `windows::owner`.
/// Blocking.
fn refine(owner: Owner) -> Owner {
    #[cfg(target_os = "macos")]
    if matches!(owner, Owner::MacBundle | Owner::MacUnreplaceable) {
        return macos::owner();
    }
    #[cfg(windows)]
    if owner == Owner::WindowsInstaller {
        return windows::owner();
    }
    owner
}

/// Whether a check at `last` is less than a day before `now`, both in
/// seconds since the Unix epoch. One after `now`, as a clock set back
/// leaves it, is not.
fn recent(last: Option<u64>, now: u64) -> bool {
    last.is_some_and(|last| last <= now && now - last < DAY.as_secs())
}

/// The calendar's time, in seconds since the Unix epoch, on GPUI's clock:
/// the system's, or in tests the test scheduler's, which `advance_clock`
/// moves. The system's is chrono's `Utc::now`, which panics on a clock set
/// before 1970; only the test clock can be set there, and then this is 0.
fn now(cx: &App) -> u64 {
    let seconds = cx
        .background_executor()
        .scheduler_executor()
        .scheduler()
        .clock()
        .utc_now()
        .timestamp();
    u64::try_from(seconds).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use sha2::{Digest as _, Sha256};

    use super::feed::Asset;
    use super::{
        DAY, How, Offer, Owner, Phase, Plan, UpdateEvent, UpdateOptions, UpdateSettings, Updater,
        recent,
    };

    /// The release's page, which opens where nothing else installs.
    const PAGE: &str = "https://github.com/seandearnaley/strikebrain-viewer/releases/tag/v9.9.9";

    /// The version offered.
    fn offered() -> semver::Version {
        semver::Version::new(9, 9, 9)
    }

    /// An updater offering 9.9.9, whose install does what `plan` says, and
    /// whose release has the page `page`. It checks nothing on its own.
    fn offering(cx: &mut TestAppContext, plan: Plan, page: &str) -> Entity<Updater> {
        cx.new(|cx| {
            let options = UpdateOptions {
                feed: String::new(),
                running: semver::Version::new(0, 0, 1),
                owner: Owner::ReleasePage,
                updates_dir: None,
            };
            let settings = UpdateSettings {
                automatic: false,
                last_check: None,
            };
            let mut updater = Updater::new(options, settings, cx);
            updater.page = Some(page.to_owned());
            updater.plan = Some(plan);
            updater.phase = Phase::Ready(Offer {
                version: offered(),
                how: How::Install,
            });
            updater
        })
    }

    /// The release's file `name`, with the size and digest of `bytes`.
    fn asset(name: &str, bytes: &[u8]) -> Asset {
        Asset {
            name: name.to_owned(),
            size: u64::try_from(bytes.len()).unwrap(),
            sha256: Sha256::digest(bytes).into(),
            url: String::new(),
        }
    }

    /// What `updater` says while `act` runs on it and every task it starts
    /// finishes.
    fn said(
        cx: &mut TestAppContext,
        updater: &Entity<Updater>,
        act: impl FnOnce(&mut Updater, &mut gpui_kit::Context<Updater>),
    ) -> Vec<UpdateEvent> {
        let mut events = cx.events(updater);
        updater.update(cx, act);
        cx.run_until_parked();
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    }

    /// Whether `updater`'s install failed and its next attempt waits a day.
    fn failed(cx: &mut TestAppContext, updater: &Entity<Updater>) -> bool {
        updater.read_with(cx, |updater, _| {
            matches!(updater.phase, Phase::Failed(_)) && updater.last_attempt.is_some()
        })
    }

    #[gpui_kit::test]
    fn a_file_changed_since_the_offer_is_removed_and_the_release_page_opens_instead(
        cx: &mut TestAppContext,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let name = "strikebrain-viewer_9.9.9_amd64.deb";
        let file = tmp.path().join(name);
        // The size the release gives, other bytes.
        std::fs::write(&file, b"the other release").unwrap();
        let plan = Plan::Open {
            file: file.clone(),
            asset: asset(name, b"the newer release"),
        };
        let updater = offering(cx, plan, PAGE);
        let events = said(cx, &updater, Updater::install);
        assert_eq!(
            events,
            [UpdateEvent::InstallFailed(
                "the file's digest is not the release's".to_owned()
            )]
        );
        assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
        assert!(!file.exists());
        assert!(failed(cx, &updater));
        assert!(updater.read_with(cx, |updater, _| updater.offer().is_none()));
    }

    #[gpui_kit::test]
    fn a_release_page_that_is_not_https_is_never_opened(cx: &mut TestAppContext) {
        let page = "http://example.com/releases/tag/v9.9.9";
        let updater = offering(cx, Plan::Page, page);
        let events = said(cx, &updater, Updater::install);
        assert_eq!(
            events,
            [UpdateEvent::InstallFailed(format!(
                "not an https address: {page}"
            ))]
        );
        assert_eq!(cx.opened_url(), None);
        assert!(failed(cx, &updater));
    }

    #[gpui_kit::test]
    fn the_release_page_opens_where_nothing_else_installs(cx: &mut TestAppContext) {
        let updater = offering(cx, Plan::Page, PAGE);
        let events = said(cx, &updater, Updater::install);
        assert_eq!(events, []);
        assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
        assert!(updater.read_with(cx, |updater, _| updater.phase == Phase::Idle));
    }

    /// A swap of 9.9.9 into `apps`, as `macos::stage` leaves it, from the
    /// disk image `image`, which no longer holds the release's bytes.
    #[cfg(target_os = "macos")]
    fn swap_from_a_changed_image(apps: &std::path::Path) -> super::Swap {
        let installed = apps.join("Strikebrain Viewer.app");
        std::fs::create_dir_all(installed.join("Contents/MacOS")).unwrap();
        let staged = apps.join(".Strikebrain Viewer.update/Strikebrain Viewer.app");
        std::fs::create_dir_all(staged.join("Contents/MacOS")).unwrap();
        let name = "Strikebrain-Viewer-9.9.9-macos-arm64.dmg";
        let image = apps.join(name);
        std::fs::write(&image, b"the other release").unwrap();
        super::Swap {
            version: offered(),
            installed,
            staged,
            signer: super::macos::Signer::made_up(),
            image,
            asset: asset(name, b"the newer release"),
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui_kit::test]
    fn a_staged_app_that_fails_its_check_at_the_click_says_why(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let swap = swap_from_a_changed_image(tmp.path());
        let (installed, image) = (swap.installed.clone(), swap.image.clone());
        let updater = offering(cx, Plan::Swap(swap), PAGE);
        // The staged copy is not signed: codesign refuses it.
        let events = said(cx, &updater, Updater::install);
        let [UpdateEvent::InstallFailed(reason)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(reason.starts_with("codesign failed: "), "{reason}");
        // The image changed too: the release page opens in its place.
        assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
        assert!(!image.exists());
        assert!(installed.is_dir());
        assert!(!tmp.path().join(".Strikebrain Viewer.update").exists());
        assert!(failed(cx, &updater));
    }

    #[cfg(target_os = "macos")]
    #[gpui_kit::test]
    fn a_swap_that_strands_the_old_app_says_where_it_is(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let swap = swap_from_a_changed_image(tmp.path());
        let old = swap.staged.with_file_name("old.app");
        let stranded = super::macos::Error::Stranded {
            old: old.clone(),
            error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            restore: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        let updater = offering(cx, Plan::Page, PAGE);
        let events = said(cx, &updater, |updater, cx| {
            updater.swapped(Err(stranded), swap, cx);
        });
        let [UpdateEvent::InstallFailed(reason)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(reason.contains(&old.display().to_string()), "{reason}");
        assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
        assert!(failed(cx, &updater));
    }

    #[cfg(windows)]
    #[gpui_kit::test]
    fn an_installer_that_does_not_start_says_why_and_opens_the_release_page(
        cx: &mut TestAppContext,
    ) {
        let updater = offering(cx, Plan::Page, PAGE);
        let events = said(cx, &updater, |updater, cx| {
            updater.started(Err("the installer did not run".to_owned()), cx);
        });
        assert_eq!(
            events,
            [UpdateEvent::InstallFailed(
                "the installer did not run".to_owned()
            )]
        );
        assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
        assert!(failed(cx, &updater));
    }

    #[test]
    fn a_check_less_than_a_day_old_is_recent() {
        let now = 1_764_158_400;
        let day = DAY.as_secs();
        assert!(!recent(None, now));
        assert!(recent(Some(now), now));
        assert!(recent(Some(now - day + 1), now));
        assert!(!recent(Some(now - day), now));
        assert!(!recent(Some(0), now));
        // A check after now, as a clock set back leaves it, checks again.
        assert!(!recent(Some(now + 1), now));
    }
}
