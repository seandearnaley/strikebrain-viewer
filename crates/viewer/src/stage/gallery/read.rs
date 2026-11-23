//! Reading one tile, on the background executor: the top of a text file,
//! or a picture decoded no larger than the tile at the display's scale.
//!
//! The checks are the stage's (`stage/load.rs`): only regular files, opened
//! without waiting; the engine's `viewer_for` on the first 8 KB; the
//! stage's size limits, so a tile shows what the file view would and a
//! file the file view cannot show is its kind's icon.
//!
//! A picture is decoded as GPUI's own loader decodes it, by the `image`
//! crate, then scaled down to fit the box with its proportions kept, never
//! up, and turned as its EXIF orientation says (after scaling, so a turned
//! photo is never copied at full size). An animation keeps its first
//! frame. Anything the `image` crate does not know is tried as an SVG, as
//! GPUI does, drawn by GPUI's renderer at its own size, or at the box's
//! when that is smaller.
//!
//! A read the gallery no longer wants is told to stop: it looks before
//! reading a picture whole and before decoding it.

use std::fs::{self, File};
use std::io::{self, Cursor, Read as _};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{HEAD_LEN, Viewer, viewer_for};
use gpui_kit::{DevicePixels, RenderImage, SharedString, SvgRenderer, SvgSize, size};
use image::metadata::Orientation;
use image::{DynamicImage, Frame, ImageDecoder as _, ImageFormat, ImageReader};

use super::super::load::{IMAGE_LIMIT, Stamp, TEXT_LIMIT, image_limit, open_regular};

/// The most lines of a text file a tile keeps: more than the tallest tile
/// shows at 9 px.
const HEAD_LINES: usize = 48;

/// The most characters of a line a tile keeps: more than the widest tile
/// shows at 9 px.
const HEAD_COLUMNS: usize = 160;

/// What a tile shows of a file.
#[derive(Clone)]
pub(crate) enum Read {
    /// The file is as the tile has it: the stamp matched.
    Unchanged,
    /// A picture, scaled down to the box. `whole` when it was no larger
    /// than the box: it fits any larger box as it is.
    Image {
        stamp: Stamp,
        image: Arc<RenderImage>,
        whole: bool,
    },
    /// The top of a text, code or Markdown file.
    Text { stamp: Stamp, head: SharedString },
    /// Anything the file view cannot show: its kind's icon.
    Unsupported { stamp: Option<Stamp> },
    /// Told to stop before it read the picture whole or decoded it.
    Stopped,
}

impl std::fmt::Debug for Read {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unchanged => f.write_str("Unchanged"),
            Self::Image { image, whole, .. } => f
                .debug_struct("Image")
                .field("size", &image.size(0))
                .field("whole", whole)
                .finish_non_exhaustive(),
            Self::Text { head, .. } => f.debug_struct("Text").field("head", head).finish(),
            Self::Unsupported { .. } => f.write_str("Unsupported"),
            Self::Stopped => f.write_str("Stopped"),
        }
    }
}

/// Reads what the tile of the file at `path`, named `name`, shows, unless
/// it is as `held`: a picture fitted inside `fit`, in device pixels. Ends
/// early once `stop` is set.
pub(crate) fn read(
    path: &Path,
    name: &str,
    held: Option<Stamp>,
    fit: (u32, u32),
    svg: &SvgRenderer,
    stop: &AtomicBool,
) -> Read {
    try_read(path, name, held, fit, svg, stop).unwrap_or(Read::Unsupported { stamp: None })
}

fn try_read(
    path: &Path,
    name: &str,
    held: Option<Stamp>,
    fit: (u32, u32),
    svg: &SvgRenderer,
    stop: &AtomicBool,
) -> io::Result<Read> {
    let stopped = || stop.load(Ordering::Relaxed);
    // Follows a symlink, so a link to a FIFO is caught before it is opened.
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Ok(Read::Unsupported { stamp: None });
    }
    if held == Some(Stamp::of(&metadata)) {
        return Ok(Read::Unchanged);
    }
    let Some((mut file, metadata)) = open_regular(path)? else {
        return Ok(Read::Unsupported { stamp: None });
    };
    let stamp = Stamp::of(&metadata);
    let mut bytes = Vec::with_capacity(HEAD_LEN);
    file.by_ref()
        .take(HEAD_LEN as u64)
        .read_to_end(&mut bytes)?;
    let unsupported = Read::Unsupported { stamp: Some(stamp) };
    Ok(match viewer_for(name, &bytes) {
        Viewer::Unsupported => unsupported,
        Viewer::Image if stamp.len() > image_limit(name, &bytes) => unsupported,
        Viewer::Image if stopped() => Read::Stopped,
        Viewer::Image => {
            let Some(bytes) = read_picture(file, bytes, stamp.len(), name)? else {
                return Ok(unsupported);
            };
            if stopped() {
                return Ok(Read::Stopped);
            }
            match picture(&bytes, fit, svg) {
                Some((image, whole)) => Read::Image {
                    stamp,
                    image,
                    whole,
                },
                None => unsupported,
            }
        }
        _ if stamp.len() > TEXT_LIMIT => unsupported,
        _ => Read::Text {
            stamp,
            head: head(&bytes, stamp.len() <= HEAD_LEN as u64),
        },
    })
}

/// Reads the rest of a picture of `len` bytes after `bytes`, its head, into
/// the same buffer, sized for the whole file at once: a buffer grown by
/// doubling leaves the allocator holding every size it passed through, some
/// 30 MB a photo on macOS. `None` when the file, as read, is past the limit
/// for a picture named `name`: it grew since its size was read, and is not
/// decoded.
fn read_picture(
    file: File,
    mut bytes: Vec<u8>,
    len: u64,
    name: &str,
) -> io::Result<Option<Vec<u8>>> {
    let len = usize::try_from(len.min(IMAGE_LIMIT)).unwrap_or(usize::MAX);
    bytes.reserve_exact(len.saturating_sub(bytes.len()).saturating_add(1));
    // One byte past the largest limit tells a file that grew since.
    let rest = (IMAGE_LIMIT + 1).saturating_sub(bytes.len() as u64);
    file.take(rest).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= image_limit(name, &bytes)).then_some(bytes))
}

/// The top of a text: its first lines, each cut short, tabs as four
/// spaces and other control characters as spaces. `whole` when `bytes`
/// is the whole file; otherwise its last line may be cut and goes.
fn head(bytes: &[u8], whole: bool) -> SharedString {
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if !whole && lines.len() > 1 {
        lines.pop();
    }
    let mut head = String::new();
    for (ix, line) in lines.iter().take(HEAD_LINES).enumerate() {
        if ix > 0 {
            head.push('\n');
        }
        let mut columns = 0;
        for c in line.chars() {
            let (c, width) = match c {
                '\t' => (' ', 4),
                c if c.is_control() => (' ', 1),
                c => (c, 1),
            };
            if columns + width > HEAD_COLUMNS {
                break;
            }
            columns += width;
            head.extend(std::iter::repeat_n(c, width));
        }
    }
    head.into()
}

/// A picture from its bytes, fitted inside `fit`, and whether it already
/// fitted. `None` when it cannot be decoded.
fn picture(bytes: &[u8], fit: (u32, u32), svg: &SvgRenderer) -> Option<(Arc<RenderImage>, bool)> {
    match image::guess_format(bytes) {
        Ok(format) => raster(bytes, format, fit),
        Err(_) => vector(bytes, fit, svg),
    }
}

/// Decodes a raster picture, scales it down to fit and turns it as its
/// EXIF orientation says. A quarter turn swaps the box first; turning the
/// small picture spares a turned copy of the large one. GPUI draws BGRA.
fn raster(bytes: &[u8], format: ImageFormat, fit: (u32, u32)) -> Option<(Arc<RenderImage>, bool)> {
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().ok()?;
    let image = DynamicImage::from_decoder(decoder).ok()?;
    let quarter = matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    let fit = if quarter { (fit.1, fit.0) } else { fit };
    let whole = image.width() <= fit.0 && image.height() <= fit.1;
    let mut image = if whole {
        image
    } else {
        image.thumbnail(fit.0, fit.1)
    };
    image.apply_orientation(orientation);
    let mut pixels = image.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some((Arc::new(RenderImage::new(vec![Frame::new(pixels)])), whole))
}

/// Draws an SVG at its own size, or at the largest size inside `fit` when
/// its own is larger. A first drawing at a sixteenth of its size, cheap,
/// tells its size and proportions; GPUI draws every scale at twice the
/// pixels, so its own size is sixteen times that drawing whatever the
/// factor. One too small to draw that way is small enough to draw as it is.
fn vector(bytes: &[u8], fit: (u32, u32), svg: &SvgRenderer) -> Option<(Arc<RenderImage>, bool)> {
    let parsed = svg.parse_svg(bytes).ok()?;
    let probe = svg
        .render_parsed(&parsed, 1. / 16.)
        .ok()
        .and_then(|image| pixels(&image));
    let mut proportions = probe;
    if probe.is_none_or(|(width, height)| width * 16 <= fit.0 && height * 16 <= fit.1) {
        let natural = svg.render_parsed(&parsed, 1.).ok()?;
        let (width, height) = pixels(&natural)?;
        if width <= fit.0 && height <= fit.1 {
            return Some((natural, true));
        }
        proportions = Some((width, height));
    }
    let (width, height) = proportions?;
    // Width alone sets the size; the height follows the picture's. The
    // first drawing's proportions, finer than the probe's, set the second.
    let mut side = scaled(fit.0, 1.).min(scaled(fit.1, f64::from(width) / f64::from(height)));
    let mut drawn = None;
    for _ in 0..3 {
        let side_px = i32::try_from(side.max(1)).ok()?;
        let image = svg
            .render_parsed(
                &parsed,
                SvgSize::Size(size(DevicePixels(side_px), DevicePixels(side_px))),
            )
            .ok()?;
        let (drawn_width, drawn_height) = pixels(&image)?;
        let ratio = f64::from(drawn_width.max(1)) / f64::from(drawn_height.max(1));
        let best = fit.0.min(scaled(fit.1, ratio)).max(1);
        let fits = drawn_height <= fit.1;
        if fits {
            drawn = Some(image);
        }
        if fits && best <= side {
            break;
        }
        side = best;
    }
    // Scaled down to its box: not its own size, so not whole.
    drawn.map(|image| (image, false))
}

/// A drawn picture's size in device pixels.
fn pixels(image: &RenderImage) -> Option<(u32, u32)> {
    let size = image.size(0);
    Some((
        u32::try_from(size.width.0).ok()?,
        u32::try_from(size.height.0).ok()?,
    ))
}

/// `side` times `scale`, rounded down, within a `u32`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the product is clamped to a u32's range first"
)]
fn scaled(side: u32, scale: f64) -> u32 {
    (f64::from(side) * scale)
        .floor()
        .clamp(0., f64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use gpui_kit::{SvgRenderer, TestAppContext};

    use explorer::HEAD_LEN;

    use std::sync::atomic::AtomicBool;

    use super::{HEAD_COLUMNS, HEAD_LINES, Read, head};
    use crate::stage::load::{IMAGE_LIMIT, Stamp, TEXT_LIMIT, stamp};

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn encoded(image: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        encoded(
            &image::DynamicImage::ImageRgba8(image),
            image::ImageFormat::Png,
        )
    }

    /// Reads a tile, never told to stop.
    fn read(
        path: &Path,
        name: &str,
        held: Option<Stamp>,
        fit: (u32, u32),
        svg: &SvgRenderer,
    ) -> Read {
        super::read(path, name, held, fit, svg, &AtomicBool::new(false))
    }

    /// A JPEG of `width` x `height` whose EXIF orientation says to turn it
    /// a quarter clockwise (6), its left half red.
    pub(crate) fn turned_jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            }
        });
        let plain = encoded(
            &image::DynamicImage::ImageRgb8(image),
            image::ImageFormat::Jpeg,
        );
        // APP1: "Exif", a big-endian TIFF header, one entry: Orientation
        // (0x0112), a SHORT, one of it, 6; then no next IFD.
        let exif: &[u8] =
            b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
        let len = u16::try_from(exif.len() + 2).unwrap().to_be_bytes();
        let mut bytes = plain[..2].to_vec();
        bytes.extend([0xFF, 0xE1]);
        bytes.extend(len);
        bytes.extend(exif);
        bytes.extend(&plain[2..]);
        bytes
    }

    fn svg(cx: &mut TestAppContext) -> SvgRenderer {
        cx.update(|cx| cx.svg_renderer())
    }

    /// The size a read picture came out at, and whether it fitted already.
    fn picture(read: &Read) -> ((i32, i32), bool) {
        match read {
            Read::Image { image, whole, .. } => {
                let size = image.size(0);
                ((size.width.0, size.height.0), *whole)
            }
            other => panic!("a picture, not {other:?}"),
        }
    }

    #[gpui_kit::test]
    fn a_picture_is_scaled_down_to_fit_its_box_and_never_up(cx: &mut TestAppContext) {
        let svg = svg(cx);
        let dir = tempfile::tempdir().unwrap();
        let wide = write(dir.path(), "wide.png", &png(800, 400));
        assert_eq!(
            picture(&read(&wide, "wide.png", None, (256, 192), &svg)),
            ((256, 128), false)
        );
        let tall = write(dir.path(), "tall.png", &png(300, 900));
        assert_eq!(
            picture(&read(&tall, "tall.png", None, (256, 192), &svg)),
            ((64, 192), false)
        );
        let small = write(dir.path(), "small.png", &png(40, 30));
        assert_eq!(
            picture(&read(&small, "small.png", None, (256, 192), &svg)),
            ((40, 30), true)
        );
    }

    #[gpui_kit::test]
    fn a_turned_photo_is_turned_and_fitted_to_its_box(cx: &mut TestAppContext) {
        let svg = svg(cx);
        let dir = tempfile::tempdir().unwrap();
        let small = write(dir.path(), "small.jpg", &turned_jpeg(80, 40));
        assert_eq!(
            picture(&read(&small, "small.jpg", None, (256, 192), &svg)),
            ((40, 80), true)
        );
        // Larger than its box once turned: scaled to the turned box.
        let large = write(dir.path(), "large.jpg", &turned_jpeg(800, 400));
        let read = read(&large, "large.jpg", None, (256, 192), &svg);
        assert_eq!(picture(&read), ((96, 192), false));
        // Turned clockwise, the red left half is the top: BGRA, red last
        // of the three.
        let Read::Image { image, .. } = read else {
            panic!("a picture");
        };
        let top_left = &image.as_bytes(0).unwrap()[..4];
        assert!(top_left[2] > 150 && top_left[0] < 100, "{top_left:?}");
    }

    #[gpui_kit::test]
    fn a_read_told_to_stop_ends_before_its_full_read(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "photo.png", &png(400, 300));
        let stopped = super::read(
            &path,
            "photo.png",
            None,
            (64, 64),
            &svg(cx),
            &AtomicBool::new(true),
        );
        assert!(matches!(stopped, Read::Stopped), "{stopped:?}");
    }

    #[gpui_kit::test]
    fn pixels_are_bgra_as_gpui_draws_them(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let red = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 10, 20, 255]));
        let path = write(
            dir.path(),
            "red.png",
            &encoded(
                &image::DynamicImage::ImageRgba8(red),
                image::ImageFormat::Png,
            ),
        );
        let Read::Image { image, .. } = read(&path, "red.png", None, (64, 64), &svg(cx)) else {
            panic!("a picture");
        };
        assert_eq!(&image.as_bytes(0).unwrap()[..4], [20, 10, 200, 255]);
    }

    #[gpui_kit::test]
    fn an_animation_keeps_its_first_frame(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let frames = [[200, 60, 60, 255], [60, 60, 200, 255]].map(|pixel| {
            image::Frame::new(image::RgbaImage::from_pixel(16, 16, image::Rgba(pixel)))
        });
        let mut bytes = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut bytes)
            .encode_frames(frames)
            .unwrap();
        let path = write(dir.path(), "moving.gif", &bytes);
        let Read::Image { image, .. } = read(&path, "moving.gif", None, (64, 64), &svg(cx)) else {
            panic!("a picture");
        };
        assert_eq!(image.frame_count(), 1);
        // The first frame's red, as BGRA.
        assert_eq!(image.as_bytes(0).unwrap()[2], 200);
    }

    #[gpui_kit::test]
    fn an_svg_is_drawn_at_its_own_size_or_the_box(cx: &mut TestAppContext) {
        let svg = svg(cx);
        let dir = tempfile::tempdir().unwrap();
        let shape = |side: u32| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{side}" height="{}"><rect width="{side}" height="{}" fill="red"/></svg>"#,
                side / 2,
                side / 2
            )
        };
        let small = write(dir.path(), "small.svg", shape(20).as_bytes());
        let ((width, height), whole) = picture(&read(&small, "small.svg", None, (256, 192), &svg));
        assert!(whole);
        assert!(width <= 256 && height <= 192, "{width} x {height}");
        let large = write(dir.path(), "large.svg", shape(2000).as_bytes());
        let ((width, height), whole) = picture(&read(&large, "large.svg", None, (256, 192), &svg));
        assert!(!whole);
        assert_eq!((width, height), (256, 128));
        // Tall: the box's height sets its size.
        let tall = r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="3000"><rect width="300" height="3000" fill="red"/></svg>"#;
        let tall = write(dir.path(), "tall.svg", tall.as_bytes());
        let ((width, height), whole) = picture(&read(&tall, "tall.svg", None, (256, 192), &svg));
        assert!(!whole);
        assert_eq!((width, height), (19, 190));
        // Too small for a sixteenth of it to be drawn: its own size.
        let tiny = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#;
        let tiny = write(dir.path(), "tiny.svg", tiny.as_bytes());
        let ((width, height), whole) = picture(&read(&tiny, "tiny.svg", None, (256, 192), &svg));
        assert!(whole);
        assert!(width <= 16 && height <= 16, "{width} x {height}");
    }

    #[gpui_kit::test]
    fn a_text_tile_holds_the_top_of_its_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "main.rs",
            b"fn main() {\n\tprintln!(\"hi\");\n}\n",
        );
        let Read::Text { head, stamp: held } = read(&path, "main.rs", None, (64, 64), &svg(cx))
        else {
            panic!("text");
        };
        assert_eq!(head.as_ref(), "fn main() {\n    println!(\"hi\");\n}");
        assert_eq!(stamp(&path), Some(held));
        // The same file again: nothing is read past its metadata.
        assert!(matches!(
            read(&path, "main.rs", Some(held), (64, 64), &svg(cx)),
            Read::Unchanged
        ));
    }

    /// The file at `path` opened, with its first 8 KB read.
    fn headed(path: &Path) -> (std::fs::File, Vec<u8>) {
        use std::io::Read as _;
        let mut file = std::fs::File::open(path).unwrap();
        let mut bytes = Vec::with_capacity(HEAD_LEN);
        file.by_ref()
            .take(HEAD_LEN as u64)
            .read_to_end(&mut bytes)
            .unwrap();
        (file, bytes)
    }

    #[test]
    fn a_picture_is_read_into_one_buffer_of_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let len = 300_000;
        let path = write(dir.path(), "large.png", &vec![7; len]);
        let (file, head) = headed(&path);
        let bytes = super::read_picture(file, head, len as u64, "large.png")
            .unwrap()
            .expect("within its limit");
        assert_eq!(bytes.len(), len);
        assert!(bytes.capacity() <= len + 1, "{}", bytes.capacity());
    }

    #[test]
    fn a_picture_past_its_limit_once_read_is_not_decoded() {
        // A GIF that grew past an animation's limit since its size was
        // read: the size it was read at is all the reader knew.
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "moving.gif", b"GIF89a");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(crate::stage::load::ANIMATED_IMAGE_LIMIT + 1)
            .unwrap();
        let (file, head) = headed(&path);
        assert!(
            super::read_picture(file, head, 1000, "moving.gif")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_head_keeps_so_many_lines_and_columns_and_drops_a_cut_line() {
        let long = "x".repeat(HEAD_COLUMNS + 10);
        let text = format!("{long}\n{}", "line\n".repeat(HEAD_LINES + 5));
        let kept = head(text.as_bytes(), true);
        let lines: Vec<&str> = kept.lines().collect();
        assert_eq!(lines.len(), HEAD_LINES);
        assert_eq!(lines[0].len(), HEAD_COLUMNS);
        // Cut by the end of the head: the last, partial line goes.
        assert_eq!(head(b"one\ntwo\nthr", false).as_ref(), "one\ntwo");
        assert_eq!(
            head(b"only one line, cut", false).as_ref(),
            "only one line, cut"
        );
        // Control characters do not reach the text system.
        assert_eq!(head(b"a\x1bb\x0cc", true).as_ref(), "a b c");
    }

    #[gpui_kit::test]
    fn what_the_file_view_cannot_show_is_unsupported(cx: &mut TestAppContext) {
        let svg = svg(cx);
        let dir = tempfile::tempdir().unwrap();
        let unsupported = |path: &Path, name: &str| {
            matches!(
                read(path, name, None, (64, 64), &svg),
                Read::Unsupported { .. }
            )
        };
        let binary = write(dir.path(), "data.bin", b"\0\x01\x02");
        assert!(unsupported(&binary, "data.bin"));
        let broken = write(dir.path(), "broken.png", b"not a png at all");
        assert!(unsupported(&broken, "broken.png"));
        let big = write(dir.path(), "big.txt", b"text head\n");
        std::fs::File::options()
            .write(true)
            .open(&big)
            .unwrap()
            .set_len(TEXT_LIMIT + 1)
            .unwrap();
        assert!(unsupported(&big, "big.txt"));
        let huge = write(dir.path(), "huge.png", &png(4, 4));
        std::fs::File::options()
            .write(true)
            .open(&huge)
            .unwrap()
            .set_len(IMAGE_LIMIT + 1)
            .unwrap();
        assert!(unsupported(&huge, "huge.png"));
        let folder = dir.path().join("folder.txt");
        std::fs::create_dir(&folder).unwrap();
        assert!(unsupported(&folder, "folder.txt"));
        assert!(unsupported(&dir.path().join("missing.txt"), "missing.txt"));
    }
}
