//! The network: reading the feed and downloading a release's file into the
//! cache folder, checked against the size and digest the release gives.
//! Blocking; the updater runs these on GPUI's background executor.
//!
//! Every request starts at an address [`allowed`] passes: `https`, or
//! `http` to this machine for tests and a local feed. ureq follows up to
//! ten redirects to any `Location`, whatever its scheme, so a request that
//! starts at `https` is held to `https` on every redirect: ureq checks each
//! hop before connecting and fails with `RequireHttpsOnly`. Otherwise a
//! redirect to `http` would carry the feed, whose digests are the only
//! check of a `.deb`, in plain text. A request that starts at `http` to
//! this machine follows redirects as ureq does.
//!
//! A request to this machine never goes through a proxy: the environment's
//! proxy (`HTTPS_PROXY` and the like) could not reach a feed served here,
//! for tests or the check by hand.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest as _, Sha256};
use ureq::config::ConfigBuilder;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::typestate::{AgentScope, WithoutBody};
use ureq::{Agent, RequestBuilder};

use super::feed::Asset;

/// How long one step of a request may take: resolving the host, connecting
/// with the TLS handshake, or one read or write. There is no limit on the
/// whole: a large file on a slow connection takes as long as it takes.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The user agent GitHub's API asks for: the app and its version, nothing
/// about the person.
const USER_AGENT: &str = concat!("strikebrain-viewer/", env!("CARGO_PKG_VERSION"));

/// GitHub's JSON media type, which the feed asks for.
const GITHUB_JSON: &str = "application/vnd.github+json";

/// The version of GitHub's API the feed is read as.
const GITHUB_API_VERSION: &str = "2022-11-28";

/// The folder under the cache folder that holds each version's files.
const UPDATES: &str = "updates";

/// The feed is a few kilobytes: its whole request, redirects included,
/// gets this long.
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

/// How much of a file one read takes.
const CHUNK: usize = 64 * 1024;

/// Why a request or a download failed.
#[derive(Debug)]
pub(crate) enum Error {
    /// The address is neither `https` nor `http` to this machine.
    NotHttps(String),
    /// The request failed, or the server answered with an error status.
    Http(ureq::Error),
    /// The cache folder or the file could not be written.
    Io(io::Error),
    /// The platform gives no cache folder: there is no home folder.
    NoCacheFolder,
    /// The file is not the size the release gives.
    Size,
    /// The file's SHA-256 digest is not the release's.
    Digest,
    /// The download was cancelled.
    Cancelled,
    /// The asset's name is not a bare file name, so it could land outside
    /// the folder.
    NotAFileName(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotHttps(url) => write!(f, "not an https address: {url}"),
            Self::Http(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::NoCacheFolder => f.write_str("there is no cache folder"),
            Self::Size => f.write_str("the file is not the size the release gives"),
            Self::Digest => f.write_str("the file's digest is not the release's"),
            Self::Cancelled => f.write_str("the download was cancelled"),
            Self::NotAFileName(name) => write!(f, "not a file name: {name}"),
        }
    }
}

impl std::error::Error for Error {}

/// The agent for the feed and the downloads: certificates checked by the
/// platform's verifier against the system's store, the app's user agent,
/// and a 30 s limit on each step. A 4xx or 5xx reply is an error
/// (`ureq::Error::StatusCode`), as ureq does by default. A proxy comes from
/// the environment (`HTTPS_PROXY` and the like), as ureq does by default,
/// for any host but this machine.
pub(crate) fn agent() -> Agent {
    config().build().new_agent()
}

fn config() -> ConfigBuilder<AgentScope> {
    Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent(USER_AGENT)
        .timeout_resolve(Some(TIMEOUT))
        .timeout_connect(Some(TIMEOUT))
        .timeout_per_read(Some(TIMEOUT))
        .timeout_per_write(Some(TIMEOUT))
}

/// Whether `url` may be requested: `https`, or `http` to this machine
/// (`127.0.0.1`, `::1` or `localhost`), where tests and a local feed for
/// the check by hand serve.
pub(crate) fn allowed(url: &str) -> bool {
    let Ok(uri) = url.parse::<Uri>() else {
        return false;
    };
    match uri.scheme_str() {
        Some("https") => true,
        Some("http") => uri.host().is_some_and(is_loopback),
        _ => false,
    }
}

/// Whether `host`, as a URL writes it (an IPv6 address in brackets), names
/// this machine.
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().is_ok_and(|ip| {
        ip == IpAddr::V4(Ipv4Addr::LOCALHOST) || ip == IpAddr::V6(Ipv6Addr::LOCALHOST)
    })
}

/// Whether `url` is an address on this machine.
fn on_this_machine(url: &str) -> bool {
    url.parse::<Uri>()
        .is_ok_and(|uri| uri.host().is_some_and(is_loopback))
}

/// Whether `url` starts at `https`, and so must stay on it.
fn starts_https(url: &str) -> bool {
    url.parse::<Uri>()
        .is_ok_and(|uri| uri.scheme_str() == Some("https"))
}

/// A GET of `url`, refused before any request unless [`allowed`] passes
/// it, held to `https` on every redirect when it starts there, and with no
/// proxy when it is on this machine.
fn get(agent: &Agent, url: &str) -> Result<RequestBuilder<WithoutBody>, Error> {
    if !allowed(url) {
        return Err(Error::NotHttps(url.to_owned()));
    }
    let request = agent.get(url).config().https_only(starts_https(url));
    let request = if on_this_machine(url) {
        request.proxy(None)
    } else {
        request
    };
    Ok(request.build())
}

/// The feed's reply, as bytes for `feed::parse`, with GitHub's JSON media
/// type and API version, within [`FEED_TIMEOUT`]. ureq caps the reply at
/// 10 MB.
pub(crate) fn fetch_json(agent: &Agent, url: &str) -> Result<Vec<u8>, Error> {
    let mut response = get(agent, url)?
        .config()
        .timeout_global(Some(FEED_TIMEOUT))
        .build()
        .header("Accept", GITHUB_JSON)
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .call()
        .map_err(Error::Http)?;
    response.body_mut().read_to_vec().map_err(Error::Http)
}

/// Downloads `asset` into `into` as `<name>.part`, hashing as it streams
/// and stopping past the release's size or when `cancel` is set, then
/// renames it to `<name>` once its size and digest are the release's. On
/// any failure the partial file is removed, so nothing partly downloaded is
/// ever left to open. A name that is not a bare file name is refused
/// before any request.
pub(crate) fn file(
    agent: &Agent,
    asset: &Asset,
    into: &Path,
    cancel: &AtomicBool,
) -> Result<PathBuf, Error> {
    if Path::new(&asset.name).file_name() != Some(OsStr::new(&asset.name)) {
        return Err(Error::NotAFileName(asset.name.clone()));
    }
    let part = into.join(format!("{}.part", asset.name));
    let path = into.join(&asset.name);
    let done = fetch_into(agent, asset, &part, cancel)
        .and_then(|()| fs::rename(&part, &path).map_err(Error::Io));
    if let Err(error) = done {
        let _removed = fs::remove_file(&part);
        return Err(error);
    }
    Ok(path)
}

/// Streams `asset` into `part`, checked; leaves `part` to the caller.
fn fetch_into(agent: &Agent, asset: &Asset, part: &Path, cancel: &AtomicBool) -> Result<(), Error> {
    let mut response = get(agent, &asset.url)?.call().map_err(Error::Http)?;
    // ureq's limit hands over at most the size and fails on one byte more.
    let mut body = response.body_mut().with_config().limit(asset.size).reader();
    let mut out = fs::File::create(part).map_err(Error::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK];
    let mut length = 0_u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(match ureq::Error::from(error) {
                    ureq::Error::BodyExceedsLimit(_) => Error::Size,
                    other => Error::Http(other),
                });
            }
        };
        let chunk = &buffer[..read];
        hasher.update(chunk);
        out.write_all(chunk).map_err(Error::Io)?;
        length += read as u64;
    }
    if length != asset.size {
        return Err(Error::Size);
    }
    if hasher.finalize() != asset.sha256 {
        return Err(Error::Digest);
    }
    // On disk before the rename, so the file under its own name is whole.
    out.sync_all().map_err(Error::Io)
}

/// Checks the file at `path`, downloaded and checked earlier, against
/// `asset`'s size and SHA-256 digest once more, as it is about to be
/// opened or run: it may have sat in the cache folder for days. A file
/// that is no longer the release's, or cannot be read, is removed.
#[cfg_attr(
    all(not(test), not(any(target_os = "macos", windows))),
    expect(
        dead_code,
        reason = "used on macOS and Windows; Linux opens its checked file in a step still to come"
    )
)]
pub(crate) fn check_again(path: &Path, asset: &Asset) -> Result<(), Error> {
    let checked = same(path, asset);
    if checked.is_err() {
        let _removed = fs::remove_file(path);
    }
    checked
}

/// Whether the file at `path` is `asset`'s size and has its digest.
#[cfg_attr(
    all(not(test), not(any(target_os = "macos", windows))),
    expect(dead_code, reason = "only `check_again` reads a file back")
)]
fn same(path: &Path, asset: &Asset) -> Result<(), Error> {
    let mut file = fs::File::open(path).map_err(Error::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK];
    let mut length = 0_u64;
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::Io(error)),
        };
        hasher.update(&buffer[..read]);
        length += read as u64;
        if length > asset.size {
            return Err(Error::Size);
        }
    }
    if length != asset.size {
        return Err(Error::Size);
    }
    if hasher.finalize() != asset.sha256 {
        return Err(Error::Digest);
    }
    Ok(())
}

/// The cache folder's `updates`, where each version's files download.
pub(crate) fn updates_dir() -> Result<PathBuf, Error> {
    crate::persist::project_dirs()
        .map(|dirs| dirs.cache_dir().join(UPDATES))
        .ok_or(Error::NoCacheFolder)
}

/// The folder `version`'s files download into: `updates/<version>`, made
/// if missing. Everything else in `updates`, older versions' files, is
/// removed. `updates` is [`updates_dir`], or a temporary folder in tests.
pub(crate) fn staging_dir(updates: &Path, version: &semver::Version) -> Result<PathBuf, Error> {
    let name = version.to_string();
    let dir = updates.join(&name);
    fs::create_dir_all(&dir).map_err(Error::Io)?;
    for entry in fs::read_dir(updates).map_err(Error::Io)?.flatten() {
        if entry.file_name() == name.as_str() {
            continue;
        }
        let path = entry.path();
        // An older version's file that will not go is left for the next
        // update; it does not stop this one.
        let _removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::net::TcpListener;
    use std::sync::atomic::Ordering;

    use super::super::test_server::{Reply, Server, plain_as_tls};
    use super::*;

    const NAME: &str = "Strikebrain-Viewer-9.9.9-macos-arm64.dmg";

    /// An empty listing.
    const NOTHING: [OsString; 0] = [];

    /// An asset named as a release names its DMG, whose size and digest
    /// are those of `body`, at `url`.
    fn asset(body: &[u8], url: String) -> Asset {
        Asset {
            name: NAME.to_owned(),
            size: u64::try_from(body.len()).unwrap(),
            sha256: Sha256::digest(body).into(),
            url,
        }
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

    fn download(asset: &Asset, into: &Path) -> Result<PathBuf, Error> {
        file(&agent(), asset, into, &AtomicBool::new(false))
    }

    #[test]
    fn the_right_bytes_land_under_the_assets_name() {
        let server = Server::start();
        let body = b"a disk image".repeat(10_000);
        server.route("/dmg", Reply::ok(body.clone()));
        let tmp = tempfile::tempdir().unwrap();
        let path = download(&asset(&body, server.url("/dmg")), tmp.path()).unwrap();
        assert_eq!(path, tmp.path().join(NAME));
        assert_eq!(fs::read(&path).unwrap(), body);
        assert_eq!(listing(tmp.path()), [NAME]);
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].line, "GET /dmg HTTP/1.1");
    }

    #[test]
    fn a_redirect_is_followed() {
        let server = Server::start();
        let body = b"moved".to_vec();
        server.route("/dmg", Reply::ok(body.clone()));
        server.route("/release", Reply::redirect(&server.url("/dmg")));
        let tmp = tempfile::tempdir().unwrap();
        let path = download(&asset(&body, server.url("/release")), tmp.path()).unwrap();
        assert_eq!(fs::read(path).unwrap(), body);
    }

    #[test]
    fn a_wrong_digest_leaves_nothing() {
        let server = Server::start();
        server.route("/dmg", Reply::ok(b"tampered".to_vec()));
        let tmp = tempfile::tempdir().unwrap();
        // The size matches; only the digest differs.
        let expected = asset(b"original", server.url("/dmg"));
        let result = download(&expected, tmp.path());
        assert!(matches!(result, Err(Error::Digest)), "{result:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
    }

    #[test]
    fn a_body_of_another_size_stops_and_leaves_nothing() {
        let server = Server::start();
        server.route("/long", Reply::ok(b"0123456789 and more".to_vec()));
        server.route("/short", Reply::ok(b"0123".to_vec()));
        let tmp = tempfile::tempdir().unwrap();
        let long = download(&asset(b"0123456789", server.url("/long")), tmp.path());
        assert!(matches!(long, Err(Error::Size)), "{long:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
        let short = download(&asset(b"0123456789", server.url("/short")), tmp.path());
        assert!(matches!(short, Err(Error::Size)), "{short:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
    }

    #[test]
    fn a_missing_file_or_a_refused_connection_is_an_error() {
        let server = Server::start();
        let tmp = tempfile::tempdir().unwrap();
        let missing = download(&asset(b"x", server.url("/nothing")), tmp.path());
        assert!(
            matches!(missing, Err(Error::Http(ureq::Error::StatusCode(404)))),
            "{missing:?}"
        );
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let refused = download(&asset(b"x", format!("http://{closed}/dmg")), tmp.path());
        assert!(matches!(refused, Err(Error::Http(_))), "{refused:?}");
        assert!(matches!(
            fetch_json(&agent(), &format!("http://{closed}/latest")),
            Err(Error::Http(_))
        ));
        assert_eq!(listing(tmp.path()), NOTHING);
    }

    #[test]
    fn a_cancelled_download_stops_and_leaves_nothing() {
        let server = Server::start();
        let body = b"never kept".to_vec();
        server.route("/dmg", Reply::ok(body.clone()));
        let tmp = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::Relaxed);
        let result = file(
            &agent(),
            &asset(&body, server.url("/dmg")),
            tmp.path(),
            &cancel,
        );
        assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
    }

    #[test]
    fn the_feed_request_names_the_app_and_asks_for_githubs_json() {
        let server = Server::start();
        server.route("/latest", Reply::ok(br#"{"tag_name":"v9.9.9"}"#.to_vec()));
        let json = fetch_json(&agent(), &server.url("/latest")).unwrap();
        assert_eq!(json, br#"{"tag_name":"v9.9.9"}"#);
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.line, "GET /latest HTTP/1.1");
        assert_eq!(
            request.header("user-agent"),
            Some(concat!("strikebrain-viewer/", env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(
            request.header("accept"),
            Some("application/vnd.github+json")
        );
        assert_eq!(request.header("x-github-api-version"), Some("2022-11-28"));
    }

    #[test]
    fn only_https_or_http_to_this_machine_is_allowed() {
        for url in [
            "https://api.github.com/repos/seandearnaley/strikebrain-viewer/releases/latest",
            "HTTPS://objects.githubusercontent.com/x",
            "http://127.0.0.1:8080/latest",
            "http://localhost/latest",
            "http://LOCALHOST:1/latest",
            "http://[::1]:9/latest",
            "http://other@127.0.0.1/latest",
        ] {
            assert!(allowed(url), "{url}");
        }
        for url in [
            "http://example.com/latest",
            "http://127.0.0.2/latest",
            "http://127.0.0.1.example.com/latest",
            "http://localhost.example.com/latest",
            "http://127.0.0.1@example.com/latest",
            "ftp://127.0.0.1/latest",
            "file:///tmp/latest",
            "/latest",
            "not a url",
            "",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn an_address_that_is_not_allowed_is_refused_before_any_request() {
        let tmp = tempfile::tempdir().unwrap();
        // The shipped agent, with the environment's proxy: no request is made.
        let result = file(
            &agent(),
            &asset(b"x", "http://example.com/dmg".to_owned()),
            tmp.path(),
            &AtomicBool::new(false),
        );
        assert!(matches!(result, Err(Error::NotHttps(_))), "{result:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
        let feed = fetch_json(&agent(), "http://example.com/latest");
        assert!(matches!(feed, Err(Error::NotHttps(_))), "{feed:?}");
    }

    #[test]
    fn an_https_request_reaches_the_tls_handshake_without_panicking() {
        // The server hangs up on a TLS handshake. Getting that far means the
        // agent built its TLS configuration: a crypto provider is wired in
        // and the platform verifier loads.
        let server = Server::start();
        let url = server.url("/latest").replacen("http://", "https://", 1);
        let result = fetch_json(&agent(), &url);
        assert!(matches!(result, Err(Error::Http(_))), "{result:?}");
        assert_eq!(server.requests().len(), 0);
    }

    #[test]
    fn a_file_is_checked_again_and_removed_if_it_is_not_the_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(NAME);
        let release = asset(b"the release", String::new());
        fs::write(&path, b"the release").unwrap();
        check_again(&path, &release).unwrap();
        assert_eq!(listing(tmp.path()), [NAME]);
        // Changed since it was checked: the same size, other bytes.
        fs::write(&path, b"the rel3ase").unwrap();
        let changed = check_again(&path, &release);
        assert!(matches!(changed, Err(Error::Digest)), "{changed:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
        for other in [&b"the release and more"[..], b"the"] {
            fs::write(&path, other).unwrap();
            let resized = check_again(&path, &release);
            assert!(matches!(resized, Err(Error::Size)), "{resized:?}");
            assert_eq!(listing(tmp.path()), NOTHING);
        }
        let gone = check_again(&path, &release);
        assert!(matches!(gone, Err(Error::Io(_))), "{gone:?}");
    }

    #[test]
    fn staging_keeps_this_versions_folder_and_removes_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let updates = tmp.path().join("updates");
        fs::create_dir_all(updates.join("0.1.2")).unwrap();
        fs::write(updates.join("0.1.2/old.dmg"), b"old").unwrap();
        fs::create_dir_all(updates.join("0.1.3")).unwrap();
        fs::write(updates.join("0.1.3/kept.dmg"), b"kept").unwrap();
        fs::write(updates.join("stray"), b"stray").unwrap();
        let dir = staging_dir(&updates, &semver::Version::new(0, 1, 3)).unwrap();
        assert_eq!(dir, updates.join("0.1.3"));
        assert_eq!(listing(&updates), ["0.1.3"]);
        assert_eq!(listing(&dir), ["kept.dmg"]);
        // A first update makes the folders.
        let fresh = tmp.path().join("fresh/updates");
        let dir = staging_dir(&fresh, &semver::Version::new(1, 0, 0)).unwrap();
        assert!(dir.is_dir());
        assert_eq!(dir, fresh.join("1.0.0"));
    }

    #[test]
    fn updates_stage_in_the_cache_folder_named_for_the_app() {
        let updates = updates_dir().unwrap();
        assert!(updates.ends_with("updates"));
        #[cfg(target_os = "macos")]
        assert!(
            updates.ends_with(format!("Library/Caches/{}/updates", crate::persist::APP_ID)),
            "{}",
            updates.display()
        );
    }

    #[test]
    fn a_body_far_longer_than_the_size_is_cut_off() {
        let server = Server::start();
        // Far more than a socket's buffers hold, so the server can write it
        // all only if the client reads it all.
        let body = vec![b'x'; 32 << 20];
        server.route("/dmg", Reply::ok(body.clone()));
        let tmp = tempfile::tempdir().unwrap();
        let result = download(&asset(&body[..10], server.url("/dmg")), tmp.path());
        assert!(matches!(result, Err(Error::Size)), "{result:?}");
        assert_eq!(listing(tmp.path()), NOTHING);
        let requests = server.stop();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].whole, "the client read the whole body");
    }

    #[test]
    fn a_request_to_this_machine_skips_the_proxy() {
        let server = Server::start();
        server.route("/latest", Reply::ok(b"{}".to_vec()));
        // A proxy nothing listens on: a request through it is refused.
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let proxy = ureq::Proxy::new(&format!("http://{closed}")).unwrap();
        let agent = config().proxy(Some(proxy)).build().new_agent();
        assert_eq!(fetch_json(&agent, &server.url("/latest")).unwrap(), b"{}");
        let tmp = tempfile::tempdir().unwrap();
        let body = b"through no proxy".to_vec();
        server.route("/dmg", Reply::ok(body.clone()));
        let dmg = asset(&body, server.url("/dmg"));
        let path = file(&agent, &dmg, tmp.path(), &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(path).unwrap(), body);
    }

    #[test]
    fn a_request_that_starts_at_https_is_held_to_https() {
        assert!(starts_https(
            "https://api.github.com/repos/x/y/releases/latest"
        ));
        assert!(starts_https("HTTPS://objects.githubusercontent.com/x"));
        assert!(!starts_https("http://127.0.0.1:8080/latest"));
        assert!(!starts_https("http://localhost/latest"));
        assert!(!starts_https("not a url"));
    }

    #[test]
    fn a_request_that_starts_at_https_is_not_redirected_to_http() {
        let server = Server::start();
        let agent = plain_as_tls(config().build());
        server.route("/json", Reply::ok(b"{}".to_vec()));
        server.route("/dmg", Reply::ok(b"x".to_vec()));
        server.route("/latest", Reply::redirect(&server.url("/json")));
        server.route("/release", Reply::redirect(&server.url("/dmg")));
        let feed = fetch_json(&agent, &server.https_url("/latest"));
        assert!(
            matches!(feed, Err(Error::Http(ureq::Error::RequireHttpsOnly(_)))),
            "{feed:?}"
        );
        let tmp = tempfile::tempdir().unwrap();
        let dmg = asset(b"x", server.https_url("/release"));
        let result = file(&agent, &dmg, tmp.path(), &AtomicBool::new(false));
        assert!(
            matches!(result, Err(Error::Http(ureq::Error::RequireHttpsOnly(_)))),
            "{result:?}"
        );
        assert_eq!(listing(tmp.path()), NOTHING);
        // A redirect that stays on https is followed.
        server.route("/secure", Reply::redirect(&server.https_url("/json")));
        assert_eq!(
            fetch_json(&agent, &server.https_url("/secure")).unwrap(),
            b"{}"
        );
        let lines: Vec<_> = server.stop().into_iter().map(|r| r.line).collect();
        assert_eq!(
            lines,
            [
                "GET /latest HTTP/1.1",
                "GET /release HTTP/1.1",
                "GET /secure HTTP/1.1",
                "GET /json HTTP/1.1",
            ]
        );
    }

    #[test]
    fn a_name_that_is_not_a_bare_file_name_is_refused_before_any_request() {
        let server = Server::start();
        server.route("/dmg", Reply::ok(b"x".to_vec()));
        let tmp = tempfile::tempdir().unwrap();
        let into = tmp.path().join("into");
        fs::create_dir(&into).unwrap();
        for name in [
            "../evil.dmg",
            "sub/evil.dmg",
            "/tmp/evil.dmg",
            "..",
            ".",
            "",
        ] {
            let mut bad = asset(b"x", server.url("/dmg"));
            bad.name = name.to_owned();
            let result = download(&bad, &into);
            assert!(
                matches!(result, Err(Error::NotAFileName(_))),
                "{name:?}: {result:?}"
            );
        }
        assert_eq!(listing(tmp.path()), ["into"]);
        assert_eq!(listing(&into), NOTHING);
        assert_eq!(server.stop().len(), 0);
    }
}
