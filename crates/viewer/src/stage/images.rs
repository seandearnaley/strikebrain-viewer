//! The stage's decoded images: a GPUI [`ImageCache`] that keeps at most 8
//! images and 512 MB of decoded pixels, and releases an evicted image's GPU
//! texture, the pattern of GPUI's `image_gallery` example.
//!
//! The image on the stage and its gallery neighbours go through it: the
//! stage readies each with [`StageImages::prepare`], and the `img` element
//! on the stage reads it through [`ImageCache::load`]. Each entry remembers
//! the stamp of the file it was decoded from, so a file that changed on
//! disk decodes again while an unchanged one keeps its pixels.
//!
//! An image's size is known once it is decoded, every frame of an
//! animation counted: the cache then makes room for it, evicting the
//! images used longest ago but never the one on the stage, which stays
//! whatever its size.

use std::sync::Arc;

use gpui_kit::{
    App, Context, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource, Window,
};

use super::cache::Lru;
use super::load::Stamp;

/// How many decoded images the stage keeps.
pub(crate) const IMAGE_CACHE_SIZE: usize = 8;

/// How many bytes of decoded pixels the stage keeps: 512 MB, some five
/// 24-megapixel photos (96 MB each) or two 8000 x 6000 ones (183 MB). The
/// image on the stage stays even when it alone is larger.
pub(crate) const IMAGE_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// See the module docs.
pub(crate) struct StageImages {
    images: Lru<Resource, Entry>,
    /// The image on the stage: never evicted to make room.
    on_stage: Option<Resource>,
}

impl std::fmt::Debug for StageImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageImages")
            .field("images", &self.images.len())
            .field("bytes", &self.images.used())
            .finish_non_exhaustive()
    }
}

/// One image, loading or loaded. Dropping it cancels a load in flight.
struct Entry {
    image: ImageCacheItem,
    /// The file it was read from; `None` until the stage has stamped it.
    stamp: Option<Stamp>,
    /// Whether its decoded size is counted against the budget yet.
    counted: bool,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("stamp", &self.stamp)
            .field("counted", &self.counted)
            .finish_non_exhaustive()
    }
}

impl Entry {
    fn new(resource: &Resource, stamp: Option<Stamp>, cx: &mut App) -> Self {
        Self {
            image: ImageCacheItem::new(resource, cx),
            stamp,
            counted: false,
        }
    }
}

impl StageImages {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self::with_budget(IMAGE_CACHE_BYTES, cx)
    }

    /// A cache that keeps at most `bytes` of decoded pixels.
    pub(crate) fn with_budget(bytes: usize, cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            for entry in this.images.drain() {
                release(&entry, None, cx);
            }
        })
        .detach();
        Self {
            images: Lru::with_max_len(bytes, IMAGE_CACHE_SIZE),
            on_stage: None,
        }
    }

    /// How many images the cache holds, loading or loaded.
    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    /// How many bytes of decoded pixels the cache holds, as far as it has
    /// counted them.
    pub(crate) fn bytes(&self) -> usize {
        self.images.used()
    }

    /// Dimensions of the decoded first frame, in source pixels.
    pub(super) fn dimensions(
        &self,
        resource: &Resource,
        window: &Window,
    ) -> Option<gpui_kit::Size<gpui_kit::DevicePixels>> {
        let image = self.images.peek(resource)?.image.use_image(window)?.ok()?;
        Some(image.size(0))
    }

    /// No image is on the stage: every image may go to make room.
    pub(crate) fn leave_stage(&mut self) {
        self.on_stage = None;
    }

    /// Readies the image at `resource` as the file is now (`stamp`) and
    /// marks it used: starts decoding it unless it is decoded or decoding
    /// already. A decode of an older version of the file is dropped first.
    /// With `on_stage`, it is the image the stage shows from now on.
    pub(crate) fn prepare(
        &mut self,
        resource: &Resource,
        stamp: Stamp,
        on_stage: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if on_stage {
            self.on_stage = Some(resource.clone());
        }
        let current = self
            .images
            .get(resource)
            .is_some_and(|entry| match entry.stamp {
                Some(old) if old != stamp => false,
                _ => {
                    entry.stamp = Some(stamp);
                    true
                }
            });
        if !current {
            let entry = Entry::new(resource, Some(stamp), cx);
            self.insert(resource, entry, window, cx);
        }
        self.count_decoded(window, cx);
    }

    /// Stores `entry`, releasing what it pushes out.
    fn insert(&mut self, resource: &Resource, entry: Entry, window: &mut Window, cx: &mut App) {
        let keep = self.on_stage.as_ref();
        for evicted in self.images.insert_keeping(resource.clone(), entry, 0, keep) {
            release(&evicted, Some(window), cx);
        }
    }

    /// Counts the images decoded since the last look against the budget,
    /// making room as it goes. A load that failed costs nothing.
    fn count_decoded(&mut self, window: &mut Window, cx: &mut App) {
        let decoded: Vec<(Resource, usize)> = self
            .images
            .iter()
            .filter(|(_, entry)| !entry.counted)
            .filter_map(|(resource, entry)| {
                let bytes = entry.image.get()?.map_or(0, |image| decoded_bytes(&image));
                Some((resource.clone(), bytes))
            })
            .collect();
        for (resource, bytes) in decoded {
            if let Some(entry) = self.images.peek_mut(&resource) {
                entry.counted = true;
            }
            let keep = self.on_stage.as_ref();
            for evicted in self.images.set_cost(&resource, bytes, keep) {
                release(&evicted, Some(window), cx);
            }
        }
    }
}

impl ImageCache for StageImages {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let result = if let Some(entry) = self.images.get(resource) {
            entry.image.use_image(window)
        } else {
            // Not prepared: the stage always prepares first, but an image
            // drawn without that still loads, unstamped.
            let entry = Entry::new(resource, None, cx);
            let result = entry.image.use_image(window);
            self.insert(resource, entry, window, cx);
            result
        };
        self.count_decoded(window, cx);
        result
    }
}

/// The bytes of pixels a decoded image holds, every frame counted.
fn decoded_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .filter_map(|frame| image.as_bytes(frame))
        .map(<[u8]>::len)
        .sum()
}

/// Frees an evicted image's texture in every window. A load still in
/// flight is cancelled when the caller drops the entry.
fn release(entry: &Entry, window: Option<&mut Window>, cx: &mut App) {
    if let Some(Ok(image)) = entry.image.get() {
        cx.drop_image(image, window);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, Entity, RenderImage, Resource, TestAppContext};

    use super::{StageImages, decoded_bytes};
    use crate::stage::load;

    fn png(dir: &Path, name: &str, side: u32) -> PathBuf {
        let image = image::RgbaImage::from_pixel(side, side, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn every_frame_counts() {
        let frame = || image::Frame::new(image::RgbaImage::new(16, 8));
        let image = RenderImage::new(vec![frame(), frame(), frame()]);
        assert_eq!(decoded_bytes(&image), 3 * 16 * 8 * 4);
    }

    #[gpui_kit::test]
    fn decoded_pixels_stay_within_the_budget_but_the_stage_keeps_its_image(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        // 64 x 64 is 16 KB of pixels: three fit in 50 KB, a fourth does not.
        let small: Vec<PathBuf> = (0..4)
            .map(|ix| png(dir.path(), &format!("{ix}.png"), 64))
            .collect();
        let large = png(dir.path(), "large.png", 128);
        let images: Entity<StageImages> = cx.new(|cx| StageImages::with_budget(50 * 1024, cx));
        let window = cx.add_empty_window();
        let prepare = |window: &mut gpui_kit::VisualTestContext, path: &Path, on_stage: bool| {
            let stamp = load::stamp(path).unwrap();
            let resource = Resource::Path(Arc::from(path));
            window.update(|window, cx| {
                images.update(cx, |images, cx| {
                    images.prepare(&resource, stamp, on_stage, window, cx);
                });
            });
            window.run_until_parked();
            // Counted when the cache is next asked.
            window.update(|window, cx| {
                images.update(cx, |images, cx| images.count_decoded(window, cx));
            });
        };
        let bytes = |window: &mut gpui_kit::VisualTestContext| {
            window.update(|_, cx| images.read(cx).bytes())
        };
        for path in &small[..3] {
            prepare(window, path, false);
        }
        assert_eq!(bytes(window), 3 * 16 * 1024);
        prepare(window, &small[3], true);
        assert_eq!(bytes(window), 3 * 16 * 1024, "the oldest went");
        assert_eq!(window.update(|_, cx| images.read(cx).len()), 3);

        // A picture on the stage larger than the whole budget stays alone.
        prepare(window, &large, true);
        assert_eq!(bytes(window), 64 * 1024);
        assert_eq!(window.update(|_, cx| images.read(cx).len()), 1);
        // A neighbour decoded beside it makes no room for itself.
        prepare(window, &small[0], false);
        assert_eq!(bytes(window), 64 * 1024);
        assert_eq!(window.update(|_, cx| images.read(cx).len()), 1);
    }
}
