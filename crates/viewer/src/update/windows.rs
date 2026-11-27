//! The last step of an update on Windows. An installation by the installer,
//! its uninstaller beside the app, is the installer's: the release's
//! installer, run silently, replaces the files it owns and relaunches the
//! app (`bundle/windows.iss`). It must carry a valid Authenticode signature
//! from the running app's publisher, as PowerShell's
//! `Get-AuthenticodeSignature` reads both; a running app with none, a
//! development build, is updated from the release page. **Restart to
//! Update** hashes the installer once more, starts it and quits, so none of
//! its files is in use. Blocking; the updater runs these on GPUI's
//! background executor.
//!
//! PowerShell is Windows PowerShell, by its full path in the system folder,
//! with no profile and no prompts. The file reaches the script in an
//! environment variable, never in its text, so no path can change the
//! command; and the script holds no double quote or backslash, so the
//! command line carries it whole. It writes the subject's UTF-8 bytes
//! straight to standard output, whatever the console's code page.
//!
//! The pure parts, reading the script's output and comparing publishers,
//! are compiled on every platform for their tests.

use std::fmt;
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};

#[cfg(windows)]
use super::install::Owner;

/// Why a step failed.
#[derive(Debug)]
pub(crate) enum Error {
    /// The running app is not one the release signed.
    NotReleased,
    /// The installer is not validly signed by the running app's publisher.
    NotThisApp,
    /// A program, named, could not be started.
    #[cfg(windows)]
    Run(&'static str, io::Error),
    /// PowerShell failed; with the first line it wrote.
    #[cfg(windows)]
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReleased => f.write_str("this copy of the app is not the release's"),
            Self::NotThisApp => f.write_str("the update is not signed as this app"),
            #[cfg(windows)]
            Self::Run(program, error) => write!(f, "{program} did not run: {error}"),
            #[cfg(windows)]
            Self::Failed(said) => write!(f, "powershell failed: {said}"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads who validly signed the file named by `STRIKEBRAIN_VIEWER_SIGNED`:
/// the subject of its signing certificate, as UTF-8 bytes on standard
/// output, and nothing when its signature's status is not `Valid`. Stops
/// at any error, which PowerShell writes to standard error before exiting
/// with 1. No progress records, which could reach an output stream.
const SCRIPT: &str = concat!(
    "$ErrorActionPreference = 'Stop'; ",
    "$ProgressPreference = 'SilentlyContinue'; ",
    "$signature = Get-AuthenticodeSignature -LiteralPath $env:STRIKEBRAIN_VIEWER_SIGNED; ",
    "if ($signature.Status -eq 'Valid') { ",
    "$subject = [Text.Encoding]::UTF8.GetBytes($signature.SignerCertificate.Subject); ",
    "$out = [Console]::OpenStandardOutput(); ",
    "$out.Write($subject, 0, $subject.Length); ",
    "$out.Flush() ",
    "}",
);

/// The environment variable that carries the file to [`SCRIPT`].
const FILE_VARIABLE: &str = "STRIKEBRAIN_VIEWER_SIGNED";

/// The program that reads signatures, named in a reason.
#[cfg(windows)]
const POWERSHELL: &str = "powershell";

/// The installer, named in a reason.
#[cfg(windows)]
const INSTALLER: &str = "the installer";

/// Windows PowerShell, under the system folder.
#[cfg(windows)]
const POWERSHELL_IN_SYSTEM_ROOT: &str = r"System32\WindowsPowerShell\v1.0\powershell.exe";

/// What the installer is run with: no pages, no message boxes, and never a
/// restart of the computer (Inno Setup's command line). It still closes
/// whatever holds its files, as it does whenever it runs silently.
#[cfg(windows)]
const SILENT: [&str; 3] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"];

/// `CREATE_NO_WINDOW`: PowerShell opens no console window of its own, as
/// GPUI starts it to restart the app.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Who owns the running app, which [`Owner::detect`] found beside its
/// uninstaller: [`Owner::WindowsInstaller`] when the app is validly signed,
/// as the release signs it, and [`Owner::ReleasePage`] otherwise, as a
/// development build is not. Runs PowerShell.
#[cfg(windows)]
pub(crate) fn owner() -> Owner {
    match running_publisher() {
        Ok(Some(_)) => Owner::WindowsInstaller,
        _ => Owner::ReleasePage,
    }
}

/// Checks the downloaded `installer` against the running app: it must be
/// validly signed by the same publisher ([`same_publisher`]). An installer
/// that fails is removed: nothing that failed its check is ever run. Runs
/// PowerShell, twice.
#[cfg(windows)]
pub(crate) fn check(installer: &Path) -> Result<(), Error> {
    let checked = check_against_running(installer);
    if checked.is_err() {
        let _removed = std::fs::remove_file(installer);
    }
    checked
}

#[cfg(windows)]
fn check_against_running(installer: &Path) -> Result<(), Error> {
    let running = running_publisher()?;
    // An unsigned app is never updated by the installer: no need to ask.
    let update = match running {
        Some(_) => publisher(installer)?,
        None => None,
    };
    same_publisher(running.as_deref(), update.as_deref())
}

/// Starts `installer`, checked, without its pages ([`SILENT`]), and leaves
/// it running: the app quits next, and the installer relaunches it once it
/// has replaced its files. Nothing is inherited but the environment.
#[cfg(windows)]
pub(crate) fn run(installer: &Path) -> Result<(), Error> {
    Command::new(installer)
        .args(SILENT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|error| Error::Run(INSTALLER, error))
}

/// Who signed the running app's executable, as [`publisher`] reads it.
#[cfg(windows)]
fn running_publisher() -> Result<Option<String>, Error> {
    // Without its own path the app cannot be the release's.
    let exe = std::env::current_exe().map_err(|_| Error::NotReleased)?;
    publisher(&exe)
}

/// Who signed `file`: the subject of its signing certificate, when Windows
/// finds its Authenticode signature valid (whole, from a certificate that
/// chains to a root Windows trusts); none otherwise. The subject, not the
/// thumbprint: the release is signed by Azure Artifact Signing, which
/// issues a new certificate every day, valid for 72 hours, all with the
/// subject of the identity it validated. Runs PowerShell.
#[cfg(windows)]
fn publisher(file: &Path) -> Result<Option<String>, Error> {
    let mut powershell = Command::new(powershell_path()?);
    powershell
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env(FILE_VARIABLE, file)
        .creation_flags(CREATE_NO_WINDOW);
    let output = powershell
        .output()
        .map_err(|error| Error::Run(POWERSHELL, error))?;
    if !output.status.success() {
        let said = first_line(&output.stderr).unwrap_or_else(|| output.status.to_string());
        return Err(Error::Failed(said));
    }
    Ok(publisher_in(&output.stdout))
}

/// Windows PowerShell's full path, in the system folder `SystemRoot` names.
#[cfg(windows)]
fn powershell_path() -> Result<PathBuf, Error> {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join(POWERSHELL_IN_SYSTEM_ROOT))
        .ok_or_else(|| Error::Run(POWERSHELL, io::Error::from(io::ErrorKind::NotFound)))
}

/// The publisher `stdout`, what [`SCRIPT`] wrote, names: the subject's
/// UTF-8 bytes, with at most one line ending after them. None when it wrote
/// nothing, as it does for a file with no valid signature, or anything that
/// is not one line of text.
fn publisher_in(stdout: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(stdout).ok()?;
    let subject = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let readable = !subject.trim().is_empty() && !subject.chars().any(char::is_control);
    readable.then(|| subject.to_owned())
}

/// Whether an installer signed by `update` may replace an app signed by
/// `running`: only an app the release signed, by an installer from the same
/// publisher, to the letter.
fn same_publisher(running: Option<&str>, update: Option<&str>) -> Result<(), Error> {
    let running = running.ok_or(Error::NotReleased)?;
    if update == Some(running) {
        Ok(())
    } else {
        Err(Error::NotThisApp)
    }
}

/// What PowerShell said as it failed: the first line it wrote to standard
/// error, which holds the error's message; the lines after it say where.
fn first_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A subject as Artifact Signing writes an individual's, made up.
    const SUBJECT: &str =
        "CN=Example Developer, O=Example Developer, L=Springfield, S=Example, C=US";

    #[test]
    fn a_publisher_is_read_from_the_scripts_output() {
        assert_eq!(publisher_in(SUBJECT.as_bytes()).as_deref(), Some(SUBJECT));
        // Written as UTF-8, whatever the console's code page.
        let accented = "CN=Zoë Développeuse, O=Zoë Développeuse, L=Montréal, S=Québec, C=CA";
        assert_eq!(publisher_in(accented.as_bytes()).as_deref(), Some(accented));
        // A line ending after it is not part of it.
        for ending in ["\r\n", "\n"] {
            let line = format!("{SUBJECT}{ending}");
            assert_eq!(publisher_in(line.as_bytes()).as_deref(), Some(SUBJECT));
        }
    }

    #[test]
    fn a_file_with_no_valid_signature_has_no_publisher() {
        // The script writes nothing unless the signature is valid.
        assert_eq!(publisher_in(b""), None);
        assert_eq!(publisher_in(b"\r\n"), None);
        assert_eq!(publisher_in(b"  "), None);
    }

    #[test]
    fn output_that_is_not_one_line_of_text_has_no_publisher() {
        // Not UTF-8: the subject in a code page.
        assert_eq!(publisher_in(b"CN=D\xe9veloppeuse"), None);
        for odd in [
            format!("{SUBJECT}\r\n{SUBJECT}"),
            format!("{SUBJECT}\n\n"),
            "CN=Example\0Developer".to_owned(),
            "CN=Example\tDeveloper".to_owned(),
        ] {
            assert_eq!(publisher_in(odd.as_bytes()), None, "{odd:?}");
        }
    }

    #[test]
    fn only_the_running_apps_publisher_may_update_it() {
        same_publisher(Some(SUBJECT), Some(SUBJECT)).unwrap();
        let other = SUBJECT.replace("Springfield", "Shelbyville");
        let lower = SUBJECT.to_lowercase();
        for update in [None, Some(other.as_str()), Some(lower.as_str())] {
            let result = same_publisher(Some(SUBJECT), update);
            assert!(
                matches!(result, Err(Error::NotThisApp)),
                "{update:?} {result:?}"
            );
        }
    }

    #[test]
    fn a_running_app_with_no_publisher_is_never_updated_by_an_installer() {
        // A development build is not signed.
        for update in [None, Some(SUBJECT)] {
            let result = same_publisher(None, update);
            assert!(matches!(result, Err(Error::NotReleased)), "{result:?}");
        }
    }

    #[test]
    fn the_script_reads_the_file_from_the_environment_and_survives_quoting() {
        assert!(SCRIPT.contains(&format!("-LiteralPath $env:{FILE_VARIABLE};")));
        assert!(SCRIPT.contains("$signature.Status -eq 'Valid'"));
        // The command line carries it whole only with neither inside.
        assert!(!SCRIPT.contains(['"', '\\']), "{SCRIPT}");
    }

    #[test]
    fn powershells_error_is_its_first_line() {
        let stderr = b"\r\nGet-AuthenticodeSignature : Cannot find path 'C:\\missing.exe' because it does not exist.\r\nAt line:1 char:79\r\n+ ... Get-AuthenticodeSignature -LiteralPath ...\r\n";
        assert_eq!(
            first_line(stderr).as_deref(),
            Some(
                "Get-AuthenticodeSignature : Cannot find path 'C:\\missing.exe' because it does not exist."
            )
        );
        assert_eq!(first_line(b" \r\n"), None);
    }

    #[test]
    fn the_reasons_say_what_failed() {
        assert_eq!(
            Error::NotReleased.to_string(),
            "this copy of the app is not the release's"
        );
        assert_eq!(
            Error::NotThisApp.to_string(),
            "the update is not signed as this app"
        );
    }

    #[cfg(windows)]
    mod on_windows {
        use super::super::*;

        #[test]
        fn windows_powershell_is_signed_by_microsoft() {
            let powershell = powershell_path().unwrap();
            let publisher = publisher(&powershell).unwrap();
            assert!(
                publisher
                    .as_deref()
                    .is_some_and(|subject| subject.contains("O=Microsoft Corporation")),
                "{publisher:?}"
            );
        }

        #[test]
        fn the_test_runner_is_not_signed_so_the_release_page_updates_it() {
            // Tests run from a development build's executable.
            let exe = std::env::current_exe().unwrap();
            assert_eq!(publisher(&exe).unwrap(), None);
            assert_eq!(owner(), Owner::ReleasePage);
        }

        #[test]
        fn a_file_that_is_not_signed_has_no_publisher() {
            let tmp = tempfile::tempdir().unwrap();
            let file = tmp.path().join("setup.exe");
            std::fs::write(&file, b"not a program").unwrap();
            assert_eq!(publisher(&file).unwrap(), None);
        }

        #[test]
        fn a_missing_file_is_powershells_error() {
            let tmp = tempfile::tempdir().unwrap();
            let result = publisher(&tmp.path().join("missing.exe"));
            let Err(Error::Failed(said)) = &result else {
                panic!("{result:?}");
            };
            assert_ne!(said, "");
        }

        #[test]
        fn an_installer_with_no_released_app_to_check_against_is_removed() {
            let tmp = tempfile::tempdir().unwrap();
            let installer = tmp.path().join("setup.exe");
            std::fs::write(&installer, b"an installer").unwrap();
            let result = check(&installer);
            assert!(matches!(result, Err(Error::NotReleased)), "{result:?}");
            assert!(!installer.exists());
        }
    }
}
