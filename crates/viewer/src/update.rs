//! Learning of a newer release and installing it. The [`Updater`] entity
//! checks GitHub's latest release 5 s after launch and then daily, or when
//! asked; downloads and verifies the file that updates this installation;
//! and offers it. It follows GPUI Kit's guide, Auto Update: a foreground
//! task coordinates each check, the background executor reads the feed and
//! downloads the file, and their results come back to the entity, which
//! notifies. The root view owns the updater, saves its two settings, shows
//! its offer in the bar and its answers to a manual check as alerts.

mod download;
mod feed;
mod install;
#[cfg(test)]
mod test_server;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use ureq::Agent;

pub(crate) use download::updates_dir;
use feed::{Asset, Release};
pub use install::Owner;

/// How long after launch the first automatic check waits, so it does not
/// compete with the first scans.
const FIRST_CHECK: Duration = Duration::from_secs(5);

/// How often the automatic checks run while the app does, and how long
/// after the last check a launch checks again.
const DAY: Duration = Duration::from_hours(24);

/// What the root view hands its updater: where to read the latest release,
/// the saved settings, and this installation.
#[derive(Debug)]
pub struct UpdateOptions {
    /// The address of the latest release's JSON: GitHub's, or a feed
    /// served on this machine.
    pub feed: String,
    /// Whether to check automatically: 5 seconds after launch, unless the
    /// last check was within a day, then daily while the app runs.
    pub automatic: bool,
    /// When updates were last checked for, in seconds since the Unix epoch.
    pub last_check: Option<u64>,
    /// The version running: a release is an update when it is greater.
    pub running: semver::Version,
    /// Who owns this installation, and so which file updates it.
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
    #[expect(
        dead_code,
        reason = "entered once a platform checks a downloaded file's signature, which is still to come"
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
        not(test),
        expect(
            dead_code,
            reason = "offered once a platform installs an update and restarts, which is still to come"
        )
    )]
    Restart,
    /// Another program installs it: the disk image opens, the `.deb` opens
    /// in the software installer, or the release page opens.
    Install,
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
    automatic: bool,
    last_check: Option<u64>,
    phase: Phase,
    /// The page of the release found, which the stub install opens.
    page: Option<String>,
    /// Set when the updater goes, so a download under way stops.
    cancel: Arc<AtomicBool>,
    /// The step of the check under way.
    step: Option<Task<()>>,
    /// The automatic checks.
    _schedule: Task<()>,
}

impl EventEmitter<UpdateEvent> for Updater {}

impl Updater {
    /// An updater as `options` say, its automatic checks scheduled on GPUI's
    /// timer.
    pub(crate) fn new(options: UpdateOptions, cx: &mut Context<Self>) -> Self {
        let UpdateOptions {
            feed,
            automatic,
            last_check,
            running,
            owner,
            updates_dir,
        } = options;
        Self {
            feed,
            running,
            owner,
            updates_dir,
            automatic,
            last_check,
            phase: Phase::Idle,
            page: None,
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
        self.automatic
    }

    /// When a check last read the latest release and settled, in seconds
    /// since the Unix epoch.
    pub(crate) fn last_check(&self) -> Option<u64> {
        self.last_check
    }

    /// Turns the automatic checks on or off. A manual check works either
    /// way.
    pub(crate) fn set_automatic(&mut self, on: bool, cx: &mut Context<Self>) {
        self.automatic = on;
        cx.notify();
    }

    /// Checks for a newer release now, unless a check is under way or an
    /// update is ready. A `manual` check tells how it ended when there is
    /// nothing to install; an automatic one says nothing.
    pub(crate) fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Idle | Phase::Failed(_)) {
            return;
        }
        self.phase = Phase::Checking;
        cx.notify();
        let feed = self.feed.clone();
        let reading = cx.background_spawn(async move { read(&feed) });
        self.step = Some(cx.spawn(async move |this, cx| {
            let read = reading.await;
            // The updater is gone only as its window closes.
            this.update(cx, |this, cx| this.found(read, manual, cx))
                .unwrap_or_default();
        }));
    }

    /// Installs the update offered, through whatever owns the installation;
    /// for now every owner's step opens the release page. The offer is
    /// then used.
    pub(crate) fn install(&mut self, cx: &mut Context<Self>) {
        if self.offer().is_none() {
            return;
        }
        if let Some(page) = &self.page {
            cx.open_url(page);
        }
        self.phase = Phase::Idle;
        cx.notify();
    }

    /// The automatic check: only while automatic checks are on.
    fn check_automatically(&mut self, cx: &mut Context<Self>) {
        if self.automatic {
            self.check(false, cx);
        }
    }

    /// What the latest release says: nothing newer, a file to download, or
    /// an update the release page installs: there the owner has no file, or
    /// the release lacks this installation's.
    fn found(
        &mut self,
        read: Result<(Agent, Release), String>,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        let (agent, release) = match read {
            Ok(read) => read,
            Err(reason) => return self.end(Phase::Failed(reason), manual, cx),
        };
        if !feed::newer(&release, &self.running) {
            return self.end(Phase::Idle, manual, cx);
        }
        self.page = Some(release.page.clone());
        match feed::file_for(&release, self.owner).cloned() {
            Some(asset) => self.download(agent, asset, release.version, manual, cx),
            None => self.end(ready(release.version), manual, cx),
        }
    }

    /// Downloads `asset`, the file of `version`, into that version's
    /// folder under the updates folder, on the background executor.
    fn download(
        &mut self,
        agent: Agent,
        asset: Asset,
        version: semver::Version,
        manual: bool,
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
            this.update(cx, |this, cx| this.downloaded(downloaded, manual, cx))
                .unwrap_or_default();
        }));
    }

    /// The download's end: its size and digest were the release's, and the
    /// update is ready, or it failed and nothing is left of it.
    fn downloaded(
        &mut self,
        downloaded: Result<PathBuf, String>,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        let Phase::Downloading { version } = &self.phase else {
            return;
        };
        let phase = match downloaded {
            Ok(_staged) => ready(version.clone()),
            Err(reason) => Phase::Failed(reason),
        };
        self.end(phase, manual, cx);
    }

    /// Ends a check in `phase`. A manual one hears how it went when there
    /// is nothing to install. A check that settled, up to date or with an
    /// update ready, is the last check; one that failed is not, so the next
    /// launch tries again.
    fn end(&mut self, phase: Phase, manual: bool, cx: &mut Context<Self>) {
        if manual {
            match &phase {
                Phase::Idle => cx.emit(UpdateEvent::UpToDate(self.running.clone())),
                Phase::Failed(reason) => cx.emit(UpdateEvent::CheckFailed(reason.clone())),
                _ => {}
            }
        }
        if matches!(phase, Phase::Idle | Phase::Ready(_)) {
            self.last_check = Some(now());
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

/// An update of `version` ready to install. Every owner installs through
/// the stub for now, which opens the release page.
fn ready(version: semver::Version) -> Phase {
    Phase::Ready(Offer {
        version,
        how: How::Install,
    })
}

/// The automatic checks: 5 s after launch, unless the last check was
/// within a day, then every day while the app runs.
fn schedule(cx: &mut Context<Updater>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(FIRST_CHECK).await;
        let first = this.update(cx, |this, cx| {
            if !recent(this.last_check, now()) {
                this.check_automatically(cx);
            }
        });
        // The updater is gone.
        if first.is_err() {
            return;
        }
        loop {
            cx.background_executor().timer(DAY).await;
            if this.update(cx, Updater::check_automatically).is_err() {
                return;
            }
        }
    })
}

/// Reads the latest release at `feed`, with the agent that read it, which
/// then downloads its file. Blocking: runs on the background executor.
fn read(feed: &str) -> Result<(Agent, Release), String> {
    let agent = download::agent();
    let json = download::fetch_json(&agent, feed).map_err(|error| error.to_string())?;
    let release = feed::parse(&json).map_err(|error| error.to_string())?;
    Ok((agent, release))
}

/// Whether a check at `last` is less than a day before `now`, both in
/// seconds since the Unix epoch. One after `now`, as a clock set back
/// leaves it, is not.
fn recent(last: Option<u64>, now: u64) -> bool {
    last.is_some_and(|last| last <= now && now - last < DAY.as_secs())
}

/// Seconds since the Unix epoch; 0 on a clock set before it.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
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
