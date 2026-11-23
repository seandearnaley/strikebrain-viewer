//! The stage's decoded images: a GPUI [`ImageCache`] that keeps the 8 used
//! most recently and releases an evicted image's GPU texture, the pattern
//! of GPUI's `image_gallery` example.
//!
//! The image on the stage and its gallery neighbours go through it: the
//! stage readies each with [`StageImages::prepare`], and the `img` element
//! on the stage reads it through [`ImageCache::load`]. Each entry remembers
//! the stamp of the file it was decoded from, so a file that changed on
//! disk decodes again while an unchanged one keeps its pixels.

use std::sync::Arc;

use gpui_kit::{
    App, Context, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource, Window,
};

use super::cache::Lru;
use super::load::Stamp;

/// How many decoded images the stage keeps.
pub(crate) const IMAGE_CACHE_SIZE: usize = 8;

/// See the module docs.
pub(crate) struct StageImages {
    images: Lru<Resource, Entry>,
}

impl std::fmt::Debug for StageImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageImages")
            .field("images", &self.images.len())
            .finish()
    }
}

/// One image, loading or loaded. Dropping it cancels a load in flight.
struct Entry {
    image: ImageCacheItem,
    /// The file it was read from; `None` until the stage has stamped it.
    stamp: Option<Stamp>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("stamp", &self.stamp)
            .finish_non_exhaustive()
    }
}

impl StageImages {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            for entry in this.images.drain() {
                release(&entry, None, cx);
            }
        })
        .detach();
        Self {
            images: Lru::new(IMAGE_CACHE_SIZE),
        }
    }

    /// How many images the cache holds, loading or loaded.
    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    /// Readies the image at `resource` as the file is now (`stamp`) and
    /// marks it used: starts decoding it unless it is decoded or decoding
    /// already. A decode of an older version of the file is dropped first.
    pub(crate) fn prepare(
        &mut self,
        resource: &Resource,
        stamp: Stamp,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(entry) = self.images.get(resource) {
            match entry.stamp {
                Some(old) if old != stamp => {}
                _ => {
                    entry.stamp = Some(stamp);
                    return;
                }
            }
        }
        let image = ImageCacheItem::new(resource, cx);
        let entry = Entry {
            image,
            stamp: Some(stamp),
        };
        for evicted in self.images.insert(resource.clone(), entry, 1) {
            release(&evicted, Some(window), cx);
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
        if let Some(entry) = self.images.get(resource) {
            return entry.image.use_image(window);
        }
        // Not prepared: the stage always prepares first, but an image
        // drawn without that still loads, unstamped.
        let image = ImageCacheItem::new(resource, cx);
        let result = image.use_image(window);
        let entry = Entry { image, stamp: None };
        for evicted in self.images.insert(resource.clone(), entry, 1) {
            release(&evicted, Some(window), cx);
        }
        result
    }
}

/// Frees an evicted image's texture in every window. A load still in
/// flight is cancelled when the caller drops the entry.
fn release(entry: &Entry, window: Option<&mut Window>, cx: &mut App) {
    if let Some(Ok(image)) = entry.image.get() {
        cx.drop_image(image, window);
    }
}
