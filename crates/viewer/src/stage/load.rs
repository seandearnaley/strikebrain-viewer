//! Reading a file for the stage. Everything here blocks on the disk, so it
//! runs on the background executor, never on the UI thread. The gallery's
//! tiles read through the same checks (`stage/gallery/read.rs`).
//!
//! [`load`] reads the first 8 KB and asks the engine's
//! [`viewer_for`](explorer::viewer_for) which viewer the file needs; for text
//! it then reads the rest, up to [`TEXT_LIMIT`], and decodes it lossily.
//! Markdown over [`MARKDOWN_LIMIT`] opens in the code view; a picture over
//! [`IMAGE_LIMIT`], or an animation over [`ANIMATED_IMAGE_LIMIT`], is
//! unsupported.
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

const MB: u64 = 1024 * 1024;

/// Pictures larger than this on disk are unsupported: 64 MB. GPUI decodes a
/// picture whole, at full size: a 64 MB JPEG is some 100 to 200 megapixels,
/// 0.4 to 0.8 GB of pixels, more than the image cache's budget alone (the
/// cache keeps the picture on the stage whatever its size). Measured in
/// milestone 3: an 8000 x 6000 photo of 44 MB decodes in 0.6 s to 183 MB.
pub(crate) const IMAGE_LIMIT: u64 = 64 * MB;

/// GIFs and animated WebP pictures larger than this are unsupported:
/// 16 MB. GPUI decodes every frame up front, and an animation compresses
/// far better than a photo: a 16 MB GIF can hold a thousand frames.
pub(crate) const ANIMATED_IMAGE_LIMIT: u64 = 16 * MB;

/// The gallery's pictures either side of the stage's are decoded ahead only
/// up to this size on disk: 8 MB, a photo of some 25 megapixels, 100 MB of
/// pixels. Two of them and the picture on the stage fit the image cache.
pub(crate) const PRELOAD_LIMIT: u64 = 8 * MB;

/// Markdown files larger than this open in the code view: 256 KB.
/// Measured (release, M4 Max): a document's first frame takes 0.18 to 0.21 s
/// at 256 KB, 0.32 to 0.34 s at 384 KB, 0.5 s at 512 KB, 1.5 s at 1 MB and
/// 5 s at 2 MB, the cost growing with the square of the size. See the
/// spec's Risks.
pub(crate) const MARKDOWN_LIMIT: u64 = 256 * 1024;

/// What a file looked like when it was read: its size and when it last
/// changed. A cached copy is still the file's while its stamp matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    pub(super) fn of(metadata: &Metadata) -> Self {
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

    let viewer = match viewer_for(name, &bytes) {
        // Too long to lay out as Markdown in good time: its source.
        Viewer::Markdown if stamp.len > MARKDOWN_LIMIT => Viewer::Code,
        viewer => viewer,
    };
    let unsupported = Loaded::Unsupported {
        len: Some(stamp.len),
    };
    Ok(match viewer {
        Viewer::Image if stamp.len > image_limit(name, &bytes) => unsupported,
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

/// The largest picture the stage decodes, by its size on disk: lower for an
/// animation, whose every frame is decoded at once.
pub(super) fn image_limit(name: &str, head: &[u8]) -> u64 {
    if is_animated(name, head) {
        ANIMATED_IMAGE_LIMIT
    } else {
        IMAGE_LIMIT
    }
}

/// Whether a picture may hold many frames: a GIF, by its name or its
/// content (GPUI decodes by content), or a WebP whose header says it is
/// animated (`RIFF`, size, `WEBPVP8X`, chunk size, then flags; bit 1 is
/// animation).
fn is_animated(name: &str, head: &[u8]) -> bool {
    let gif_name = name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("gif"));
    let animated_webp = head.starts_with(b"RIFF")
        && head.get(8..16) == Some(b"WEBPVP8X".as_slice())
        && head.get(20).is_some_and(|flags| flags & 0x02 != 0);
    gif_name || head.starts_with(b"GIF8") || animated_webp
}

/// Opens the regular file at `path` for reading, with its metadata as the
/// open file gives it; `None` for anything else. The open never waits:
/// a FIFO put in the file's place since its metadata was read would
/// otherwise wait for a writer, and hold a thread, forever.
pub(crate) fn open_regular(path: &Path) -> io::Result<Option<(File, Metadata)>> {
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

    /// A sparse file of `len` bytes that starts with `head`.
    fn sparse(dir: &Path, name: &str, head: &[u8], len: u64) -> std::path::PathBuf {
        let path = write(dir, name, head);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len)
            .unwrap();
        path
    }

    #[test]
    fn pictures_over_their_limits_are_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let viewer = |path: &Path, name: &str| match load(path, name, Held::default()) {
            Loaded::Image(_) => "image",
            Loaded::Unsupported { .. } => "unsupported",
            other => panic!("{other:?}"),
        };
        let still = sparse(dir.path(), "still.png", b"", IMAGE_LIMIT);
        assert_eq!(viewer(&still, "still.png"), "image");
        let still = sparse(dir.path(), "still.png", b"", IMAGE_LIMIT + 1);
        assert_eq!(viewer(&still, "still.png"), "unsupported");

        // Every frame of an animation is decoded at once: a lower limit,
        // for a GIF by its name or its content, and an animated WebP.
        let gif = sparse(dir.path(), "a.gif", b"GIF89a", ANIMATED_IMAGE_LIMIT);
        assert_eq!(viewer(&gif, "a.gif"), "image");
        let gif = sparse(dir.path(), "a.gif", b"GIF89a", ANIMATED_IMAGE_LIMIT + 1);
        assert_eq!(viewer(&gif, "a.gif"), "unsupported");
        let disguised = sparse(dir.path(), "b.png", b"GIF89a", ANIMATED_IMAGE_LIMIT + 1);
        assert_eq!(viewer(&disguised, "b.png"), "unsupported");
        let mut animated = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
        animated.push(0x02);
        let webp = sparse(dir.path(), "c.webp", &animated, ANIMATED_IMAGE_LIMIT + 1);
        assert_eq!(viewer(&webp, "c.webp"), "unsupported");
        let still_webp = b"RIFF\0\0\0\0WEBPVP8 ";
        let webp = sparse(dir.path(), "d.webp", still_webp, ANIMATED_IMAGE_LIMIT + 1);
        assert_eq!(viewer(&webp, "d.webp"), "image");
    }

    #[test]
    fn markdown_over_its_limit_opens_as_code() {
        let dir = tempfile::tempdir().unwrap();
        let at_limit = "# A\n\n".repeat(usize::try_from(MARKDOWN_LIMIT).unwrap() / 5);
        let path = write(dir.path(), "notes.md", at_limit.as_bytes());
        let viewer = |path: &Path| match load(path, "notes.md", Held::default()) {
            Loaded::Text { viewer, .. } => viewer,
            other => panic!("text, not {other:?}"),
        };
        assert_eq!(viewer(&path), Viewer::Markdown);
        let over = format!(
            "{at_limit}{}",
            "x".repeat(usize::try_from(MARKDOWN_LIMIT).unwrap())
        );
        let path = write(dir.path(), "notes.md", over.as_bytes());
        assert_eq!(viewer(&path), Viewer::Code);
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
