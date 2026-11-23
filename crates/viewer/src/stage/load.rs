//! Reading a file for the stage. Everything here blocks on the disk, so it
//! runs on the background executor, never on the UI thread.
//!
//! [`load`] reads the first 8 KB and asks the engine's
//! [`viewer_for`](explorer::viewer_for) which viewer the file needs; for text
//! it then reads the rest, up to [`TEXT_LIMIT`], and decodes it lossily.
//! Only regular files are read: a symlink to a FIFO, a device or a folder
//! is unsupported, since opening a FIFO waits for a writer and reading a
//! device may never end. (The scan leaves FIFOs and devices out; a symlink
//! is a leaf whatever it points to.) The metadata is read first, and the
//! file is opened without waiting and asked again, so a file swapped for
//! a FIFO in between cannot hold a thread.

use std::fs::{self, File, Metadata};
use std::io::{self, Read as _};
use std::path::Path;
use std::time::SystemTime;

use explorer::{HEAD_LEN, Viewer, viewer_for};
use gpui_kit::SharedString;

/// Text files larger than this are unsupported: 10 MB. Measured in
/// milestone 3 (release, M4 Max): a 10 MB file of 360,000 lines reaches
/// the screen in about 0.1 s and scrolls at 2.6 ms a frame; its colours
/// follow some 13 s later from the background. See the spec's Risks.
pub(crate) const TEXT_LIMIT: u64 = 10 * 1024 * 1024;

/// What a file looked like when it was read: its size and when it last
/// changed. A cached copy is still the file's while its stamp matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }

    /// The file's size in bytes.
    pub(crate) fn len(self) -> u64 {
        self.len
    }
}

/// What the stage needs to show a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Loaded {
    /// An image. GPUI's image loader reads and decodes it.
    Image(Stamp),
    /// Text for the Markdown or the code view. `text` is `None` when the
    /// copy the caller holds, stamped as `cached`, is still the file's.
    Text {
        viewer: Viewer,
        stamp: Stamp,
        text: Option<SharedString>,
    },
    /// Anything else: a binary file, text over the limit, a symlink to
    /// something that is not a file, a file that cannot be read. `len` is
    /// the file's size when it could be read.
    Unsupported { len: Option<u64> },
    /// The file is as it was when the stage showed it: the same stamp.
    /// Nothing was read past its metadata.
    Unchanged,
}

/// What the caller holds of a file already.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    /// The stamp of the file as the stage shows it now.
    pub(crate) shown: Option<Stamp>,
    /// The stamp of a copy of its text in the cache.
    pub(crate) text: Option<Stamp>,
}

/// Reads what the stage needs to show the file at `path`, named `name`,
/// given what the caller already `held`.
pub(crate) fn load(path: &Path, name: &str, held: Held) -> Loaded {
    try_load(path, name, held).unwrap_or(Loaded::Unsupported { len: None })
}

/// The stamp of a regular file, for preloading an image: `None` for
/// anything else or a file that cannot be read.
pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    fs::metadata(path)
        .ok()
        .filter(Metadata::is_file)
        .map(|metadata| Stamp::of(&metadata))
}

fn try_load(path: &Path, name: &str, held: Held) -> io::Result<Loaded> {
    // Follows a symlink, so a link to a FIFO is caught before it is opened.
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Ok(Loaded::Unsupported { len: None });
    }
    let stamp = Stamp::of(&metadata);
    if held.shown == Some(stamp) {
        return Ok(Loaded::Unchanged);
    }
    let cached = held.text;
    // The file may have changed into something else since: ask the open
    // file.
    let Some((mut file, metadata)) = open_regular(path)? else {
        return Ok(Loaded::Unsupported { len: None });
    };
    let stamp = Stamp::of(&metadata);
    let mut bytes = Vec::with_capacity(HEAD_LEN);
    file.by_ref()
        .take(HEAD_LEN as u64)
        .read_to_end(&mut bytes)?;

    let viewer = viewer_for(name, &bytes);
    let unsupported = Loaded::Unsupported {
        len: Some(stamp.len),
    };
    Ok(match viewer {
        Viewer::Image => Loaded::Image(stamp),
        Viewer::Unsupported => unsupported,
        _ if stamp.len > TEXT_LIMIT => unsupported,
        _ if cached == Some(stamp) => Loaded::Text {
            viewer,
            stamp,
            text: None,
        },
        _ => {
            // One byte past the limit tells a file that grew since its
            // size was read.
            let rest = TEXT_LIMIT + 1 - bytes.len() as u64;
            file.take(rest).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > TEXT_LIMIT {
                unsupported
            } else {
                Loaded::Text {
                    viewer,
                    stamp,
                    text: Some(decode(bytes)),
                }
            }
        }
    })
}

/// Opens the regular file at `path` for reading, with its metadata as the
/// open file gives it; `None` for anything else. The open never waits:
/// a FIFO put in the file's place since its metadata was read would
/// otherwise wait for a writer, and hold a thread, forever.
fn open_regular(path: &Path) -> io::Result<Option<(File, Metadata)>> {
    let file = open_without_waiting(path)?;
    let metadata = file.metadata()?;
    Ok(metadata.is_file().then_some((file, metadata)))
}

/// Opens `path` for reading with `O_NONBLOCK`, which reads of a regular
/// file ignore.
#[cfg(unix)]
fn open_without_waiting(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_without_waiting(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Text from bytes: UTF-8 as it is, anything else lossily, each invalid
/// sequence becoming U+FFFD.
pub(crate) fn decode(bytes: Vec<u8>) -> SharedString {
    match String::from_utf8(bytes) {
        Ok(text) => text.into(),
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn text(loaded: &Loaded) -> Option<&str> {
        match loaded {
            Loaded::Text {
                text: Some(text), ..
            } => Some(text),
            _ => None,
        }
    }

    #[test]
    fn each_kind_of_file_finds_its_viewer() {
        let dir = tempfile::tempdir().unwrap();
        let notes = write(dir.path(), "notes.md", b"# Notes\n");
        let code = write(dir.path(), "main.rs", b"fn main() {}\n");
        let image = write(dir.path(), "photo.png", b"not really a png");
        let binary = write(dir.path(), "data.txt", b"a\0b");

        let notes = load(&notes, "notes.md", Held::default());
        assert!(matches!(
            notes,
            Loaded::Text {
                viewer: Viewer::Markdown,
                ..
            }
        ));
        assert_eq!(text(&notes), Some("# Notes\n"));
        let code = load(&code, "main.rs", Held::default());
        assert!(matches!(
            code,
            Loaded::Text {
                viewer: Viewer::Code,
                ..
            }
        ));
        // An image extension wins; decoding is GPUI's, later.
        assert!(matches!(
            load(&image, "photo.png", Held::default()),
            Loaded::Image(_)
        ));
        assert_eq!(
            load(&binary, "data.txt", Held::default()),
            Loaded::Unsupported { len: Some(3) }
        );
    }

    #[test]
    fn invalid_utf8_is_decoded_lossily() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "latin1.txt", b"na\xefve ok\n");
        assert_eq!(
            text(&load(&path, "latin1.txt", Held::default())),
            Some("na\u{fffd}ve ok\n")
        );
    }

    #[test]
    fn text_over_the_limit_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "big.log", &[b'x'; HEAD_LEN]);
        // Sparse past the limit: a text head, too big a file.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(TEXT_LIMIT + 1)
            .unwrap();
        assert_eq!(
            load(&path, "big.log", Held::default()),
            Loaded::Unsupported {
                len: Some(TEXT_LIMIT + 1)
            }
        );
    }

    #[test]
    fn text_longer_than_the_head_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let line = "0123456789abcdef\n".repeat(2_000);
        let path = write(dir.path(), "long.txt", line.as_bytes());
        let loaded = load(&path, "long.txt", Held::default());
        assert_eq!(text(&loaded), Some(line.as_str()));
    }

    #[test]
    fn a_cached_copy_with_the_same_stamp_is_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.txt", b"one");
        let Loaded::Text { stamp, .. } = load(&path, "a.txt", Held::default()) else {
            panic!("text");
        };
        let text_held = Held {
            text: Some(stamp),
            ..Held::default()
        };
        assert_eq!(
            load(&path, "a.txt", text_held),
            Loaded::Text {
                viewer: Viewer::Code,
                stamp,
                text: None
            }
        );
        // A different size is a different stamp: read again.
        fs::write(&path, b"three").unwrap();
        assert_eq!(text(&load(&path, "a.txt", text_held)), Some("three"));
    }

    #[test]
    fn a_file_as_the_stage_shows_it_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.md", b"# one");
        let Loaded::Text { stamp, .. } = load(&path, "a.md", Held::default()) else {
            panic!("text");
        };
        let shown = Held {
            shown: Some(stamp),
            ..Held::default()
        };
        assert_eq!(load(&path, "a.md", shown), Loaded::Unchanged);
        fs::write(&path, b"# two, longer").unwrap();
        assert_eq!(text(&load(&path, "a.md", shown)), Some("# two, longer"));
    }

    #[test]
    fn only_regular_files_are_opened() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        assert_eq!(
            load(&link, "link.txt", Held::default()),
            Loaded::Unsupported { len: None }
        );
        assert_eq!(stamp(&link), None);
        // A device: never read, whatever its name says.
        assert_eq!(
            load(Path::new("/dev/zero"), "zero.png", Held::default()),
            Loaded::Unsupported { len: None }
        );
        assert_eq!(
            load(
                &dir.path().join("missing.md"),
                "missing.md",
                Held::default()
            ),
            Loaded::Unsupported { len: None }
        );
    }

    #[test]
    fn opening_never_waits_on_a_fifo() {
        // What `load` meets if a file is swapped for a FIFO between reading
        // its metadata and opening it: opening returns at once, and the
        // open file says it is not a regular file.
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe.txt");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let (sender, opened) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let regular = open_regular(&fifo).map(|file| file.is_some());
            sender.send(regular).unwrap();
        });
        let regular = opened
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("opening a FIFO returns at once");
        assert!(!regular.unwrap());
    }

    #[test]
    fn a_symlink_to_a_fifo_is_never_opened() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let link = dir.path().join("pipe.txt");
        std::os::unix::fs::symlink(&fifo, &link).unwrap();
        // Opening the FIFO for reading would wait for a writer forever.
        assert_eq!(
            load(&link, "pipe.txt", Held::default()),
            Loaded::Unsupported { len: None }
        );
        assert_eq!(stamp(&link), None);
    }
}
