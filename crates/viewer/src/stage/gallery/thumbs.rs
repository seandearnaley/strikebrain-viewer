//! What the tiles show, kept by file: thumbnails and the tops of text
//! files, in a least-recently-used cache with a byte budget of its own,
//! apart from the file view's full-size images and texts.
//!
//! A picture costs its pixels, a text its bytes, and every entry a little
//! more for itself, so a gallery of icons is bounded too. Making room
//! evicts the entries used longest ago; the tiles on screen are used on
//! every frame, so they go last. An evicted picture is handed back for its
//! GPU texture to be released.
//!
//! Each entry remembers the file's stamp and the tree it was last read or
//! checked in: after a rescan a tile shows what it has while its file is
//! checked again, and is read again only if the file changed. A picture
//! decoded for a smaller box than its tile now has is read again too; one
//! decoded for a larger box serves a smaller tile as it is.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{RenderImage, SharedString};

use super::super::cache::Lru;
use super::super::load::Stamp;
use super::read::Read;

/// How many bytes the tiles' content may take: 128 MB. A 6K display's
/// stage covered in tiles holds some 80 MB of thumbnails at its scale, so
/// a screenful fits with the rows around it.
pub(crate) const THUMBNAIL_BYTES: usize = 128 * 1024 * 1024;

/// What an entry costs besides its content: its path and bookkeeping.
const ENTRY_BYTES: usize = 256;

/// What a tile shows.
#[derive(Clone)]
pub(crate) enum Look {
    /// A picture, scaled down to fit the tile.
    Picture(Arc<RenderImage>),
    /// The top of a text file.
    Text(SharedString),
    /// The file's kind: the file view cannot show it.
    Icon,
}

impl std::fmt::Debug for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Picture(image) => f.debug_tuple("Picture").field(&image.size(0)).finish(),
            Self::Text(head) => f.debug_tuple("Text").field(&head.len()).finish(),
            Self::Icon => f.write_str("Icon"),
        }
    }
}

/// A tile's file to read, and what of it the cache holds already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    /// The stamp of the file as the cache holds it: the read stops at the
    /// file's metadata while it still has that stamp. `None` reads it
    /// whatever it is.
    pub(crate) held: Option<Stamp>,
}

#[derive(Debug)]
struct Entry {
    look: Look,
    stamp: Option<Stamp>,
    /// The box a picture was decoded to fit, in device pixels.
    fit: (u32, u32),
    /// The picture was no larger than its box: it fits any larger one.
    whole: bool,
    /// The tree it was last read or checked in.
    tree: u64,
}

impl Entry {
    /// Whether this entry, read for the tree `tree`, should be read for a
    /// tile whose box is `fit`: checked against its stamp when only the
    /// file may have changed, read whatever it is when its picture is too
    /// small.
    fn reading(&self, tree: u64, fit: (u32, u32)) -> Option<Reading> {
        let small = matches!(self.look, Look::Picture(_))
            && !self.whole
            && (fit.0 > self.fit.0 || fit.1 > self.fit.1);
        if small {
            Some(Reading { held: None })
        } else if self.tree == tree {
            None
        } else {
            Some(Reading { held: self.stamp })
        }
    }
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Thumbs {
    cache: Lru<Arc<Path>, Entry>,
}

impl Thumbs {
    /// An empty cache whose entries may cost `budget` bytes together.
    pub(crate) fn with_budget(budget: usize) -> Self {
        Self {
            cache: Lru::new(budget),
        }
    }

    /// What the tile of `path` shows, marked as just used.
    pub(crate) fn get(&mut self, path: &Arc<Path>) -> Option<Look> {
        self.cache.get(path).map(|entry| entry.look.clone())
    }

    /// Whether the tile of `path`, in the tree `tree` and a box of `fit`,
    /// should be read, leaving its place in the order alone.
    pub(crate) fn wants(&self, path: &Arc<Path>, tree: u64, fit: (u32, u32)) -> Option<Reading> {
        match self.cache.peek(path) {
            Some(entry) => entry.reading(tree, fit),
            None => Some(Reading { held: None }),
        }
    }

    /// The most the content may cost, in bytes.
    pub(crate) fn budget(&self) -> usize {
        self.cache.budget()
    }

    /// Keeps what was read of `path` for the tree `tree`, for a box of
    /// `fit`. Returns the pictures it pushed out, whose textures should be
    /// released.
    pub(crate) fn insert(
        &mut self,
        path: Arc<Path>,
        read: Read,
        fit: (u32, u32),
        tree: u64,
    ) -> Vec<Arc<RenderImage>> {
        let (look, stamp, whole) = match read {
            Read::Unchanged => {
                if let Some(entry) = self.cache.peek_mut(&path) {
                    entry.tree = tree;
                }
                return Vec::new();
            }
            // Nothing was read: the tile is read again when it is wanted.
            Read::Stopped => return Vec::new(),
            Read::Image {
                stamp,
                image,
                whole,
            } => (Look::Picture(image), Some(stamp), whole),
            Read::Text { stamp, head } => (Look::Text(head), Some(stamp), false),
            Read::Unsupported { stamp } => (Look::Icon, stamp, false),
        };
        let cost = ENTRY_BYTES
            + match &look {
                Look::Picture(image) => image.as_bytes(0).map_or(0, <[u8]>::len),
                Look::Text(head) => head.len(),
                Look::Icon => 0,
            };
        let entry = Entry {
            look,
            stamp,
            fit,
            whole,
            tree,
        };
        self.cache
            .insert(path, entry, cost)
            .into_iter()
            .filter_map(|evicted| match evicted.look {
                Look::Picture(image) => Some(image),
                Look::Text(_) | Look::Icon => None,
            })
            .collect()
    }

    /// Every picture held, for release when the gallery goes.
    pub(crate) fn drain(&mut self) -> Vec<Arc<RenderImage>> {
        self.cache
            .drain()
            .filter_map(|entry| match entry.look {
                Look::Picture(image) => Some(image),
                Look::Text(_) | Look::Icon => None,
            })
            .collect()
    }

    /// How many tiles' content the cache holds.
    pub(crate) fn len(&self) -> usize {
        self.cache.len()
    }

    /// What the content held costs, in bytes; never more than the budget.
    pub(crate) fn bytes(&self) -> usize {
        self.cache.used()
    }

    /// The largest picture held, in device pixels: width, then height.
    pub(crate) fn largest(&self) -> Option<(u32, u32)> {
        self.cache
            .iter()
            .filter_map(|(_, entry)| match &entry.look {
                Look::Picture(image) => {
                    let size = image.size(0);
                    Some((
                        u32::try_from(size.width.0).unwrap_or_default(),
                        u32::try_from(size.height.0).unwrap_or_default(),
                    ))
                }
                Look::Text(_) | Look::Icon => None,
            })
            .max_by_key(|(width, height)| u64::from(*width) * u64::from(*height))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use gpui_kit::RenderImage;

    use super::{ENTRY_BYTES, Look, Reading, Thumbs};
    use crate::stage::gallery::read::Read;
    use crate::stage::load::stamp;

    fn picture(side: u32) -> Arc<RenderImage> {
        let frame = image::Frame::new(image::RgbaImage::new(side, side));
        Arc::new(RenderImage::new(vec![frame]))
    }

    /// A stamp of a real file, as every read carries.
    fn some_stamp() -> super::Stamp {
        stamp(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Cargo.toml")
                .as_path(),
        )
        .expect("the manifest is a file")
    }

    fn image(side: u32, whole: bool) -> Read {
        Read::Image {
            stamp: some_stamp(),
            image: picture(side),
            whole,
        }
    }

    fn path(name: &str) -> Arc<Path> {
        Path::new("/gallery").join(name).into()
    }

    #[test]
    fn pictures_stay_within_the_budget_and_the_oldest_go_first() {
        // A 32 x 32 picture is 4 KB of pixels: two fit in 10 KB, a third
        // does not.
        let mut thumbs = Thumbs::with_budget(10 * 1024);
        let cost = 32 * 32 * 4 + ENTRY_BYTES;
        for name in ["a", "b"] {
            let evicted = thumbs.insert(path(name), image(32, true), (64, 64), 0);
            assert_eq!(evicted.len(), 0);
        }
        assert_eq!(thumbs.bytes(), 2 * cost);
        assert_eq!(thumbs.budget(), 10 * 1024);
        // Using a makes b the oldest.
        thumbs.get(&path("a"));
        let evicted = thumbs.insert(path("c"), image(32, true), (64, 64), 0);
        assert_eq!(evicted.len(), 1, "b's texture is released");
        assert!(thumbs.wants(&path("b"), 0, (64, 64)).is_some());
        assert!(thumbs.wants(&path("a"), 0, (64, 64)).is_none());
        assert_eq!(thumbs.len(), 2);
        assert!(thumbs.bytes() <= 10 * 1024);
        assert_eq!(thumbs.largest(), Some((32, 32)));
        assert_eq!(thumbs.drain().len(), 2);
        assert_eq!((thumbs.len(), thumbs.bytes()), (0, 0));
    }

    #[test]
    fn a_rescan_checks_a_tile_again_and_a_larger_tile_reads_it_again() {
        let mut thumbs = Thumbs::with_budget(1 << 20);
        let a = path("a.png");
        thumbs.insert(Arc::clone(&a), image(32, false), (64, 48), 1);
        assert!(matches!(thumbs.get(&a), Some(Look::Picture(_))));
        assert_eq!(thumbs.wants(&a, 1, (64, 48)), None, "up to date");
        // Another tree: checked against its stamp.
        let held = Some(some_stamp());
        assert_eq!(thumbs.wants(&a, 2, (64, 48)), Some(Reading { held }));
        thumbs.insert(Arc::clone(&a), Read::Unchanged, (64, 48), 2);
        assert_eq!(thumbs.wants(&a, 2, (64, 48)), None);
        // A smaller tile shows it as it is.
        assert_eq!(thumbs.wants(&a, 2, (32, 24)), None);
        // A larger tile: decoded again, whatever the stamp.
        assert_eq!(thumbs.wants(&a, 2, (128, 96)), Some(Reading { held: None }));
        // A picture that fitted its box fits any larger one.
        thumbs.insert(Arc::clone(&a), image(32, true), (64, 48), 2);
        assert_eq!(thumbs.wants(&a, 2, (128, 96)), None);
    }

    #[test]
    fn text_and_icons_cost_their_bytes_and_hold_no_texture() {
        let mut thumbs = Thumbs::with_budget(2 * ENTRY_BYTES + 10);
        let text = Read::Text {
            stamp: some_stamp(),
            head: "fn main()".into(),
        };
        thumbs.insert(path("main.rs"), text, (64, 48), 0);
        thumbs.insert(
            path("data.bin"),
            Read::Unsupported { stamp: None },
            (64, 48),
            0,
        );
        assert_eq!(thumbs.bytes(), 2 * ENTRY_BYTES + 9);
        assert_eq!(thumbs.largest(), None);
        // A third pushes the oldest out, which held no picture.
        let evicted = thumbs.insert(path("x.bin"), Read::Unsupported { stamp: None }, (1, 1), 0);
        assert_eq!(evicted.len(), 0);
        assert!(matches!(thumbs.get(&path("x.bin")), Some(Look::Icon)));
        // A file never seen wants reading.
        assert_eq!(
            thumbs.wants(&path("new"), 0, (1, 1)),
            Some(Reading { held: None })
        );
    }
}
