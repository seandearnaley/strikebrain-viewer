//! Learning of a newer release and installing it. The [`Updater`] entity
//! checks GitHub's latest release 5 s after launch and then once a day by
//! the calendar, or when asked; downloads and verifies the file that
//! updates this installation; and offers it. It follows GPUI Kit's guide,
//! Auto Update: a foreground task coordinates each check, the background
//! executor reads the feed, downloads the file and checks it, and their
//! results come back to the entity, which notifies. The root view owns the
//! updater, saves its two settings, shows its offer in the bar and its
//! answers to a manual check as alerts.
//!
//! Each platform's last step is its own module: on macOS, `macos` checks
//! the disk image's app against the running one and replaces the bundle.

mod download;
mod feed;
mod install;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod test_server;

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
    /// When updates were last checked for, in seconds since the Unix epoch.
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
    /// `Owner::detect` finds as the app opens. On macOS each check that
    /// finds an update asks again whether the release signed the bundle and
    /// whether it can be replaced.
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
    /// signature against the running app's.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(
            dead_code,
            reason = "entered on macOS; Windows checks its installer's signature in a step still to come"
        )
    )]
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
        all(not(test), not(target_os = "macos")),
        expect(
            dead_code,
            reason = "offered on macOS; Windows installs and restarts in a step still to come"
        )
    )]
    Restart,
    /// Another program installs it: the disk image opens, the `.deb` opens
    /// in the software installer, or the release page opens.
    Install,
}

/// What installing the update offered does.
#[derive(Debug)]
enum Plan {
    /// Opens the release's page, where the person installs it.
    Page,
    /// Opens the checked disk image in Finder, where the bundle cannot be
    /// replaced.
    #[cfg(target_os = "macos")]
    Open(PathBuf),
    /// Replaces the bundle `installed` with the checked copy `staged`
    /// beside it and relaunches. If that fails, the old bundle is back in
    /// place and the disk image `image` opens instead.
    #[cfg(target_os = "macos")]
    Swap {
        installed: PathBuf,
        staged: PathBuf,
        image: PathBuf,
    },
}

/// How a manual check ended with nothing to install, for the root view to
/// say. An update found shows its button instead; an automatic check says
/// nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateEvent {
    /// The running version, this one, is the latest.
    UpToDate(semver::Version),
    /// The check failed, for this reason.
    CheckFailed(String),
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
    /// the update.
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

    /// When a check last read the latest release and settled, in seconds
    /// since the Unix epoch.
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
    /// as its plan says. The offer is then used. On macOS a bundle that can
    /// be replaced is swapped for the staged copy here, two renames and the
    /// old app's removal, and the app relaunches as the new version.
    pub(crate) fn install(&mut self, cx: &mut Context<Self>) {
        if self.offer().is_none() {
            return;
        }
        let plan = self.plan.take();
        self.phase = Phase::Idle;
        cx.notify();
        match plan {
            Some(Plan::Page) => {
                if let Some(page) = &self.page {
                    cx.open_url(page);
                }
            }
            #[cfg(target_os = "macos")]
            Some(Plan::Open(image)) => cx.open_with_system(&image),
            #[cfg(target_os = "macos")]
            Some(Plan::Swap {
                installed,
                staged,
                image,
            }) => {
                if macos::swap(&installed, &staged).is_ok() {
                    cx.set_restart_path(installed);
                    cx.restart();
                } else {
                    // The old app is back in place, and the app runs on.
                    cx.open_with_system(&image);
                }
            }
            None => {}
        }
    }

    /// The automatic check, when automatic checks are on and both the last
    /// check that settled and the last attempt are a day old or more by the
    /// calendar.
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
        let downloading = cx.background_spawn(async move {
            let updates = updates.ok_or(download::Error::NoCacheFolder)?;
            let into = download::staging_dir(&updates, &version)?;
            download::file(&agent, &asset, &into, &cancel)
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let downloaded = downloading.await.map_err(|error| error.to_string());
            this.update(cx, |this, cx| this.downloaded(downloaded, owner, cx))
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
        cx: &mut Context<Self>,
    ) {
        let Phase::Downloading { version } = &self.phase else {
            return;
        };
        let version = version.clone();
        match downloaded {
            Ok(file) => self.verify(file, version, owner, cx),
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
        cx: &mut Context<Self>,
    ) {
        let replace = match owner {
            Owner::MacBundle => true,
            Owner::MacUnreplaceable => false,
            _ => return self.make_offer(version, How::Install, Plan::Page, cx),
        };
        self.phase = Phase::Verifying { version };
        cx.notify();
        let verifying = cx.background_spawn(async move {
            macos::prepare(&image, replace)
                .map(|prepared| (prepared, image))
                .map_err(|error| error.to_string())
        });
        self.step = Some(cx.spawn(async move |this, cx| {
            let verified = verifying.await;
            this.update(cx, |this, cx| this.verified(verified, cx))
                .unwrap_or_default();
        }));
    }

    /// Checks the downloaded `file` of `version` as its platform asks, then
    /// offers it. Windows and Linux check their files in steps still to
    /// come; until then the release page installs the update.
    #[cfg(not(target_os = "macos"))]
    fn verify(
        &mut self,
        _file: PathBuf,
        version: semver::Version,
        _owner: Owner,
        cx: &mut Context<Self>,
    ) {
        self.make_offer(version, How::Install, Plan::Page, cx);
    }

    /// The disk image's check's end: the update is offered, or it failed
    /// and the image and any staged copy are gone.
    #[cfg(target_os = "macos")]
    fn verified(
        &mut self,
        verified: Result<(macos::Prepared, PathBuf), String>,
        cx: &mut Context<Self>,
    ) {
        let Phase::Verifying { version } = &self.phase else {
            return;
        };
        let version = version.clone();
        match verified {
            Ok((macos::Prepared::Swap { installed, staged }, image)) => {
                let plan = Plan::Swap {
                    installed,
                    staged,
                    image,
                };
                self.make_offer(version, How::Restart, plan, cx);
            }
            Ok((macos::Prepared::Open, image)) => {
                self.make_offer(version, How::Install, Plan::Open(image), cx);
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
    /// is nothing to install. A check that settled, up to date or with an
    /// update ready, is the last check, which is saved; one that failed is
    /// not, and the first look a day after its attempt tries again.
    fn end(&mut self, phase: Phase, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.asked) {
            match &phase {
                Phase::Idle => cx.emit(UpdateEvent::UpToDate(self.running.clone())),
                Phase::Failed(reason) => cx.emit(UpdateEvent::CheckFailed(reason.clone())),
                _ => {}
            }
        }
        if matches!(phase, Phase::Idle | Phase::Ready(_)) {
            self.settings.last_check = Some(now(cx));
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
/// can be written: `macos::owner`. Blocking.
fn refine(owner: Owner) -> Owner {
    #[cfg(target_os = "macos")]
    if matches!(owner, Owner::MacBundle | Owner::MacUnreplaceable) {
        return macos::owner();
    }
    owner
}

/// Whether a check at `last` is less than a day before `now`, both in
/// seconds since the Unix epoch. One after `now`, as a clock set back
/// leaves it, is not.
fn recent(last: Option<u64>, now: u64) -> bool {
    last.is_some_and(|last| last <= now && now - last < DAY.as_secs())
}

/// The calendar's time, in seconds since the Unix epoch; 0 on a clock set
/// before it. GPUI's clock: the system's, or in tests the test scheduler's,
/// which `advance_clock` moves.
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
    use super::{DAY, recent};

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
