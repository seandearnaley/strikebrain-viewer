//! Updates: the automatic checks and the manual one against a feed served
//! here, the bar's button, the two settings saved, the alerts of a manual
//! check, and the More menu's update commands where there is no menu bar.
//! Nothing reaches the network: the feed and the files come from a local
//! server, and time is GPUI's clock, advanced by the test.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, OwnedMenuItem, Role, TestAppContext};
use sha2::{Digest as _, Sha256};
use viewer::keys::{CheckForUpdates, StartPresenting, ToggleCheckAutomatically};
use viewer::{
    AppState, Launch, Owner, SAVE_DELAY, Session, StateStore, UpdateOptions, UpdateSettings,
    open_main_window,
};

use crate::{
    PROJECT, click, dispatch, folder, node, open_app_with_session, press, read, row, tree,
};

/// The release's page, which the stub install opens.
const PAGE: &str = "https://github.com/seandearnaley/strikebrain-viewer/releases/tag/v9.9.9";

/// A day of GPUI's clock.
const DAY: Duration = Duration::from_hours(24);

/// A local HTTP/1.1 server on `127.0.0.1`: it answers each path served with
/// its bytes and anything else with 404, one request per connection, and
/// records each request's path. It stops when dropped.
struct Server {
    address: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free local port");
        let address = listener.local_addr().expect("the listener's address");
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (routes, requests, stop) = (routes.clone(), requests.clone(), stop.clone());
            move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        // A client that hangs up is its test's to notice.
                        let _answered = answer(&stream, &routes, &requests);
                    }
                }
            }
        });
        Self {
            address,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// Answers `path` with `body` from now on.
    fn serve(&self, path: &str, body: impl Into<Vec<u8>>) {
        lock(&self.routes).insert(path.to_owned(), body.into());
    }

    /// The `http` URL of `path` on this server.
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// The paths requested so far, in order.
    fn requests(&self) -> Vec<String> {
        lock(&self.requests).clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop, which then sees the flag.
        let _woken = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

/// The value behind `mutex`, even if a failing test left it poisoned.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads one request and writes its reply.
fn answer(
    stream: &TcpStream,
    routes: &Mutex<HashMap<String, Vec<u8>>>,
    requests: &Mutex<Vec<String>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // The headers, up to the blank line.
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? <= "\r\n".len() {
            break;
        }
    }
    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
    lock(requests).push(path.clone());
    let body = lock(routes).get(&path).cloned();
    let status = if body.is_some() {
        "200 OK"
    } else {
        "404 Not Found"
    };
    let body = body.unwrap_or_default();
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

/// The SHA-256 digest of `bytes` in hex, as GitHub gives it.
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("a String takes any text");
            hex
        })
}

/// The `.deb` of `version`, as the release workflow names it.
fn deb(version: &str) -> String {
    format!("strikebrain-viewer_{version}_amd64.deb")
}

/// GitHub's latest-release reply for `version`, carrying its `.deb` at
/// `url` with the size and digest of `file`.
fn release(version: &str, file: &[u8], url: &str) -> String {
    format!(
        r#"{{"tag_name": "v{version}", "html_url": "{PAGE}", "assets": [{{"name": "{}", "size": {}, "digest": "sha256:{}", "browser_download_url": "{url}"}}]}}"#,
        deb(version),
        file.len(),
        sha256(file)
    )
}

/// The version this build runs as.
fn running() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo's package version is SemVer")
}

/// Serves a release of `version` at `/latest` whose `.deb`, `file`, is at
/// `/deb`.
fn serve_release(server: &Server, version: &str, file: &[u8]) {
    server.serve("/latest", release(version, file, &server.url("/deb")));
    server.serve("/deb", file.to_vec());
}

/// An updater's options for an installation from the `.deb`, whose file
/// is checked against the release's digest alone, reading `server`'s feed
/// and downloading into `updates`.
fn options(server: &Server, updates: &Path) -> UpdateOptions {
    UpdateOptions {
        feed: server.url("/latest"),
        running: running(),
        owner: Owner::Deb,
        updates_dir: Some(updates.to_owned()),
    }
}

/// Opens the app with no folder and an updater with `options`, saving to
/// `store` if given.
fn open(
    cx: &mut TestAppContext,
    options: UpdateOptions,
    store: Option<StateStore>,
) -> AnyWindowHandle {
    let session = Session {
        updates: Some(options),
        store,
        ..Session::default()
    };
    open_app_with_session(cx, Launch::Empty, session).0
}

/// Advances GPUI's clock by `by` and lets every task finish.
fn wait(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

/// The calendar's time on GPUI's clock, which the test scheduler keeps, in
/// seconds since the Unix epoch, as the saved state counts them.
fn now(cx: &TestAppContext) -> u64 {
    let seconds = cx
        .executor()
        .scheduler_executor()
        .scheduler()
        .clock()
        .utc_now()
        .timestamp();
    u64::try_from(seconds).expect("the test clock is past 1970")
}

/// Moves the calendar on by `asleep`, as a computer asleep that long finds
/// it on waking: GPUI's timers do not run meanwhile.
fn sleep(cx: &mut TestAppContext, asleep: Duration) {
    let executor = cx.executor();
    let mut woken = executor.scheduler_executor().scheduler().clock().utc_now();
    woken += asleep;
    executor
        .dispatcher()
        .as_test()
        .expect("tests run on the test scheduler")
        .scheduler()
        .clock()
        .set_utc_now(woken);
}

/// The bar's update button after a fresh frame: its accessible label and
/// its description, the tooltip. `None` when the bar shows none.
fn update_button(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<(String, String)> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find("update").map(|button| {
            (
                button.label().unwrap_or_default().to_owned(),
                button.description().unwrap_or_default().to_owned(),
            )
        })
    })
    .expect("read the bar")
}

/// The button an update of 9.9.9 shows where another program installs it.
fn install_9_9_9() -> (String, String) {
    ("Install Update".to_owned(), "Version 9.9.9".to_owned())
}

/// The names of the files in `dir`; none if it does not exist.
fn files_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .map(|entry| {
                    let entry = entry.expect("a readable entry");
                    entry.file_name().to_string_lossy().into_owned()
                })
                .collect()
        },
    )
}

/// Whether the app menu's Check Automatically is checked.
fn check_automatically_checked(cx: &mut TestAppContext) -> bool {
    let menus = cx.update(|cx| cx.get_menus().unwrap_or_default());
    menus
        .iter()
        .flat_map(|menu| &menu.items)
        .find_map(|item| match item {
            OwnedMenuItem::Action { name, checked, .. } if name == "Check Automatically" => {
                Some(*checked)
            }
            _ => None,
        })
        .expect("the app menu has Check Automatically")
}

#[gpui_kit::test]
fn a_newer_release_is_downloaded_and_offered_five_seconds_after_launch(cx: &mut TestAppContext) {
    let server = Server::start();
    let file = b"the newer release".to_vec();
    serve_release(&server, "9.9.9", &file);
    let updates = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let state_file = state.path().join("state.json");
    let launched = now(cx);
    let handle = open(
        cx,
        options(&server, updates.path()),
        Some(StateStore::at(&state_file)),
    );

    // Not before 5 s: the first scans have the machine.
    wait(cx, Duration::from_millis(4_999));
    assert!(server.requests().is_empty(), "{:?}", server.requests());
    assert_eq!(update_button(cx, handle), None);

    wait(cx, Duration::from_millis(1));
    assert_eq!(server.requests(), ["/latest", "/deb"]);
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));
    let staged = updates.path().join("9.9.9").join(deb("9.9.9"));
    assert_eq!(std::fs::read(staged).unwrap(), file);
    // An automatic check says nothing.
    assert!(!cx.has_pending_prompt());

    // The check is saved, with automatic checks still on.
    wait(cx, SAVE_DELAY);
    let saved = StateStore::at(&state_file).load().unwrap();
    assert!(saved.check_updates);
    assert!(saved.last_update_check >= Some(launched));
}

#[gpui_kit::test]
fn install_update_opens_the_release_page_and_the_button_goes(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, "9.9.9", b"the newer release");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));
    // An update found by a manual check shows only its button.
    assert!(!cx.has_pending_prompt());
    click(cx, handle, "update");
    assert_eq!(cx.opened_url().as_deref(), Some(PAGE));
    assert_eq!(update_button(cx, handle), None);
}

#[gpui_kit::test]
fn where_the_release_page_installs_an_update_is_offered_without_a_download(
    cx: &mut TestAppContext,
) {
    let server = Server::start();
    serve_release(&server, "9.9.9", b"the newer release");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(
        cx,
        UpdateOptions {
            owner: Owner::ReleasePage,
            ..options(&server, updates.path())
        },
        None,
    );

    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(server.requests(), ["/latest"]);
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));
    assert_eq!(files_in(updates.path()), Vec::<String>::new());
}

#[gpui_kit::test]
fn the_app_checks_again_every_day_it_runs(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    wait(cx, Duration::from_secs(5));
    assert_eq!(server.requests(), ["/latest"]);
    wait(cx, DAY.checked_sub(Duration::from_millis(1)).unwrap());
    assert_eq!(server.requests(), ["/latest"]);
    wait(cx, Duration::from_millis(1));
    assert_eq!(server.requests(), ["/latest", "/latest"]);
    // Up to date: nothing to show, nothing to say.
    assert_eq!(update_button(cx, handle), None);
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn the_checks_keep_calendar_time_across_a_sleep(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    open(cx, options(&server, updates.path()), None);

    wait(cx, Duration::from_secs(5));
    assert_eq!(server.requests(), ["/latest"]);
    // Two days asleep: the calendar moves on, GPUI's timers do not.
    sleep(cx, Duration::from_hours(48));
    wait(cx, Duration::from_hours(1));
    assert_eq!(server.requests(), ["/latest", "/latest"]);
}

#[gpui_kit::test]
fn a_manual_check_asked_for_during_an_automatic_one_is_answered(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    // Run to the automatic check's start: its read handed to the
    // background executor, not yet run.
    let executor = cx.executor();
    let dispatcher = executor
        .dispatcher()
        .as_test()
        .expect("tests run on the test scheduler");
    dispatcher
        .scheduler()
        .clock()
        .advance(Duration::from_secs(5));
    while dispatcher.tick(true) {}
    while dispatcher.scheduler().pending_task_counts().1 == 0 && dispatcher.tick(false) {}
    assert!(server.requests().is_empty(), "{:?}", server.requests());

    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(CheckForUpdates), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(server.requests(), ["/latest"]);
    assert_eq!(
        cx.pending_prompt().map(|(title, _)| title).as_deref(),
        Some("Strikebrain Viewer is up to date")
    );
    cx.simulate_prompt_answer("OK");
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn a_manual_check_asked_for_twice_checks_once_and_answers_once(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(CheckForUpdates), cx);
        window.dispatch_action(Box::new(CheckForUpdates), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(server.requests(), ["/latest"]);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("OK");
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn the_update_button_is_not_shown_while_presenting(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, "9.9.9", b"the newer release");
    let updates = tempfile::tempdir().unwrap();
    let session = Session {
        updates: Some(UpdateOptions {
            owner: Owner::ReleasePage,
            ..options(&server, updates.path())
        }),
        ..Session::default()
    };
    let (handle, view) = open_app_with_session(cx, tree(PROJECT), session);
    let readme = node(cx, &view, "README.md");
    click(cx, handle, row(readme));
    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));

    dispatch(cx, handle, StartPresenting);
    assert!(read(cx, &view, AppState::is_presenting));
    assert_eq!(update_button(cx, handle), None);
    press(cx, handle, "escape");
    assert!(!read(cx, &view, AppState::is_presenting));
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));
}

#[gpui_kit::test]
fn more_is_on_the_bar_with_nothing_open_where_there_is_no_menu_bar(cx: &mut TestAppContext) {
    let server = Server::start();
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);
    let more = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("more").is_some()
        })
        .unwrap()
    };
    if cfg!(target_os = "macos") {
        // The app menu has the update commands.
        assert!(!more(cx));
        return;
    }
    assert!(more(cx));
    click(cx, handle, "more");
    // The explorer's commands stay, disabled where there is no tree; the
    // Kit's menu items do not expose that state to assistive technology.
    let names = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .within("popup-menu")
                .find_all_by_role(Role::MenuItem)
                .iter()
                .filter_map(|item| item.label().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        names,
        [
            "Expand All",
            "Collapse All",
            "Show Ignored",
            "Refresh",
            "Select All",
            "Deselect All",
            "Check for Updates…",
            "Check Automatically"
        ]
    );
}

#[gpui_kit::test]
fn a_launch_within_a_day_of_the_last_check_waits_for_the_daily_one(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, "9.9.9", b"the newer release");
    let updates = tempfile::tempdir().unwrap();
    let session = Session {
        update_settings: UpdateSettings {
            last_check: Some(now(cx) - 60 * 60),
            ..UpdateSettings::default()
        },
        updates: Some(options(&server, updates.path())),
        ..Session::default()
    };
    let handle = open_app_with_session(cx, Launch::Empty, session).0;

    wait(cx, Duration::from_secs(5));
    assert!(server.requests().is_empty(), "{:?}", server.requests());
    // The hourly look that finds the last check a day old: 23 h on.
    let almost = Duration::from_hours(23).checked_sub(Duration::from_millis(1));
    wait(cx, almost.unwrap());
    assert!(server.requests().is_empty(), "{:?}", server.requests());
    wait(cx, Duration::from_millis(1));
    assert_eq!(server.requests(), ["/latest", "/deb"]);
    assert_eq!(update_button(cx, handle), Some(install_9_9_9()));
}

#[gpui_kit::test]
fn with_check_automatically_off_only_a_manual_check_checks(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let state_file = state.path().join("state.json");
    let handle = open(
        cx,
        options(&server, updates.path()),
        Some(StateStore::at(&state_file)),
    );
    assert!(check_automatically_checked(cx));

    dispatch(cx, handle, ToggleCheckAutomatically);
    assert!(!check_automatically_checked(cx));
    wait(cx, SAVE_DELAY);
    assert!(!StateStore::at(&state_file).load().unwrap().check_updates);

    wait(cx, Duration::from_secs(5));
    wait(cx, DAY);
    assert!(server.requests().is_empty(), "{:?}", server.requests());

    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(server.requests(), ["/latest"]);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("OK");

    // On again, and saved.
    dispatch(cx, handle, ToggleCheckAutomatically);
    assert!(check_automatically_checked(cx));
    wait(cx, SAVE_DELAY);
    assert!(StateStore::at(&state_file).load().unwrap().check_updates);
}

#[gpui_kit::test]
fn a_manual_check_finding_the_running_version_says_it_is_up_to_date(cx: &mut TestAppContext) {
    let server = Server::start();
    serve_release(&server, &running().to_string(), b"this version");
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(server.requests(), ["/latest"]);
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Strikebrain Viewer is up to date".to_owned(),
            format!("Version {}", env!("CARGO_PKG_VERSION"))
        ))
    );
    cx.simulate_prompt_answer("OK");
    assert!(!cx.has_pending_prompt());
    assert_eq!(update_button(cx, handle), None);
}

#[gpui_kit::test]
fn a_check_that_cannot_connect_is_silent_unless_asked_for(cx: &mut TestAppContext) {
    // A port nothing listens on: the connection is refused.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let updates = tempfile::tempdir().unwrap();
    let handle = open(
        cx,
        UpdateOptions {
            feed: format!("http://{closed}/latest"),
            running: running(),
            owner: Owner::Deb,
            updates_dir: Some(updates.path().to_owned()),
        },
        None,
    );

    wait(cx, Duration::from_secs(5));
    assert!(!cx.has_pending_prompt());
    assert_eq!(update_button(cx, handle), None);

    dispatch(cx, handle, CheckForUpdates);
    let (title, message) = cx.pending_prompt().expect("an alert");
    assert_eq!(title, "Couldn't check for updates");
    assert_ne!(message, "");
    cx.simulate_prompt_answer("OK");
}

#[gpui_kit::test]
fn a_file_that_is_not_the_releases_is_never_offered(cx: &mut TestAppContext) {
    let server = Server::start();
    // The size the release gives, other bytes.
    server.serve(
        "/latest",
        release("9.9.9", b"the newer release", &server.url("/deb")),
    );
    server.serve("/deb", b"the other release".to_vec());
    let updates = tempfile::tempdir().unwrap();
    let handle = open(cx, options(&server, updates.path()), None);

    wait(cx, Duration::from_secs(5));
    assert_eq!(server.requests(), ["/latest", "/deb"]);
    assert_eq!(update_button(cx, handle), None);
    assert!(!cx.has_pending_prompt());
    assert_eq!(
        files_in(&updates.path().join("9.9.9")),
        Vec::<String>::new()
    );

    // Asked for, it says why.
    dispatch(cx, handle, CheckForUpdates);
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Couldn't check for updates".to_owned(),
            "the file's digest is not the release's".to_owned()
        ))
    );
    cx.simulate_prompt_answer("OK");
    assert_eq!(update_button(cx, handle), None);
}

#[gpui_kit::test]
fn a_check_whose_download_fails_is_tried_again_a_day_later(cx: &mut TestAppContext) {
    let server = Server::start();
    // The size the release gives, other bytes: every download fails.
    server.serve(
        "/latest",
        release("9.9.9", b"the newer release", &server.url("/deb")),
    );
    server.serve("/deb", b"the other release".to_vec());
    let updates = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let state_file = state.path().join("state.json");
    open(
        cx,
        options(&server, updates.path()),
        Some(StateStore::at(&state_file)),
    );
    let attempt = ["/latest", "/deb"];

    wait(cx, Duration::from_secs(5));
    assert_eq!(server.requests(), attempt);
    // The hourly looks of the next day find the attempt too recent.
    wait(cx, DAY.checked_sub(Duration::from_millis(1)).unwrap());
    assert_eq!(server.requests(), attempt);
    // A day after it, by the calendar, the next attempt.
    wait(cx, Duration::from_millis(1));
    assert_eq!(server.requests(), attempt.repeat(2));
    // Two days asleep: the first look on waking tries again.
    sleep(cx, Duration::from_hours(48));
    wait(cx, Duration::from_hours(1));
    assert_eq!(server.requests(), attempt.repeat(3));
    assert!(!cx.has_pending_prompt());

    // Only a check that settles is saved as the last one.
    cx.quit();
    let saved = StateStore::at(&state_file).load().unwrap();
    assert_eq!(saved.last_update_check, None);
}

#[gpui_kit::test]
fn the_update_commands_are_on_offer_only_with_an_updater(cx: &mut TestAppContext) {
    let available = |cx: &mut TestAppContext, handle: AnyWindowHandle| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.is_action_available(&CheckForUpdates, cx)
                && window.is_action_available(&ToggleCheckAutomatically, cx)
        })
        .unwrap()
    };
    let (without, _) = open_app_with_session(cx, Launch::Empty, Session::default());
    assert!(!available(cx, without));
    let server = Server::start();
    let updates = tempfile::tempdir().unwrap();
    let with = open(cx, options(&server, updates.path()), None);
    assert!(available(cx, with));
}

#[gpui_kit::test]
fn more_ends_with_the_update_commands_where_there_is_no_menu_bar(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let server = Server::start();
    let updates = tempfile::tempdir().unwrap();
    let session = Session {
        updates: Some(options(&server, updates.path())),
        ..Session::default()
    };
    let (handle, _) =
        open_app_with_session(cx, Launch::Folders(vec![dir.path().to_owned()]), session);
    click(cx, handle, "more");
    let items = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // The Kit draws its separators as menu items with no name.
            window
                .within("popup-menu")
                .find_all_by_role(Role::MenuItem)
                .iter()
                .map(|item| (item.label().map(ToOwned::to_owned), item.checked()))
                .collect::<Vec<_>>()
        })
        .unwrap();
    let names: Vec<&str> = items
        .iter()
        .filter_map(|(label, _)| label.as_deref())
        .collect();
    let explorers = [
        "Expand All",
        "Collapse All",
        "Show Ignored",
        "Refresh",
        "Select All",
        "Deselect All",
    ];
    if cfg!(target_os = "macos") {
        // The app menu has them.
        assert_eq!(names, explorers);
        return;
    }
    assert_eq!(names[..explorers.len()], explorers);
    let last: Vec<_> = items[items.len() - 3..]
        .iter()
        .map(|(label, _)| label.as_deref())
        .collect();
    assert_eq!(
        last,
        [
            None,
            Some("Check for Updates…"),
            Some("Check Automatically")
        ]
    );
    assert_eq!(items[items.len() - 1].1, Some(true));
}

#[gpui_kit::test]
fn a_window_without_an_updater_keeps_the_saved_update_settings(cx: &mut TestAppContext) {
    let state = tempfile::tempdir().unwrap();
    let state_file = state.path().join("state.json");
    std::fs::write(
        &state_file,
        r#"{"version": 2, "check_updates": false, "last_update_check": 7}"#,
    )
    .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    let store = StateStore::at(&state_file);
    cx.update(|cx| open_main_window(Vec::new(), Some(store), false, false, cx))
        .unwrap();
    cx.run_until_parked();
    // The quit saves the whole state.
    cx.quit();
    let saved = StateStore::at(&state_file).load().unwrap();
    assert!(!saved.check_updates);
    assert_eq!(saved.last_update_check, Some(7));
}
