//! Reading one tile, on the background executor: the top of a text file,
//! or a picture decoded no larger than the tile at the display's scale.
//!
//! The checks are the stage's (`stage/load.rs`): only regular files, opened
//! without waiting; the engine's `viewer_for` on the first 8 KB; the
//! stage's size limits, so a tile shows what the file view would and a
//! file the file view cannot show is its kind's icon.
//!
//! A picture is decoded as GPUI's own loader decodes it, by the `image`
//! crate with its EXIF orientation applied, then scaled down to fit the
//! box with its proportions kept, never up. An animation keeps its first
//! frame. Anything the `image` crate does not know is tried as an SVG, as
//! GPUI does, drawn by GPUI's renderer at its own size, or at the box's
//! when that is smaller.

use std::fs;
use std::io::{self, Cursor, Read as _};
use std::path::Path;
use std::sync::Arc;

use explorer::{HEAD_LEN, Viewer, viewer_for};
use gpui_kit::{DevicePixels, RenderImage, SharedString, SvgRenderer, SvgSize, size};
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
        }
    }
}

/// Reads what the tile of the file at `path`, named `name`, shows, unless
/// it is as `held`: a picture fitted inside `fit`, in device pixels.
pub(crate) fn read(
    path: &Path,
    name: &str,
    held: Option<Stamp>,
    fit: (u32, u32),
    svg: &SvgRenderer,
) -> Read {
    try_read(path, name, held, fit, svg).unwrap_or(Read::Unsupported { stamp: None })
}

fn try_read(
    path: &Path,
    name: &str,
    held: Option<Stamp>,
    fit: (u32, u32),
    svg: &SvgRenderer,
) -> io::Result<Read> {
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
        Viewer::Image => {
            // One byte past the limit tells a file that grew since.
            let rest = IMAGE_LIMIT + 1 - bytes.len() as u64;
            file.take(rest).read_to_end(&mut bytes)?;
            match picture(&bytes, fit, svg) {
                Some((image, whole)) if bytes.len() as u64 <= IMAGE_LIMIT => Read::Image {
                    stamp,
                    image,
                    whole,
                },
                _ => unsupported,
            }
        }
        _ if stamp.len() > TEXT_LIMIT => unsupported,
        _ => Read::Text {
            stamp,
            head: head(&bytes, stamp.len() <= HEAD_LEN as u64),
        },
    })
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

/// Decodes a raster picture and scales it down to fit. GPUI draws BGRA.
fn raster(bytes: &[u8], format: ImageFormat, fit: (u32, u32)) -> Option<(Arc<RenderImage>, bool)> {
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    let whole = image.width() <= fit.0 && image.height() <= fit.1;
    if !whole {
        image = image.thumbnail(fit.0, fit.1);
    }
    let mut pixels = image.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some((Arc::new(RenderImage::new(vec![Frame::new(pixels)])), whole))
}

/// Draws an SVG at its own size, or again at the largest size inside
/// `fit` when its own is larger.
fn vector(bytes: &[u8], fit: (u32, u32), svg: &SvgRenderer) -> Option<(Arc<RenderImage>, bool)> {
    let parsed = svg.parse_svg(bytes).ok()?;
    let natural = svg.render_parsed(&parsed, 1.).ok()?;
    let drawn = natural.size(0);
    let (width, height) = (
        u32::try_from(drawn.width.0).ok()?,
        u32::try_from(drawn.height.0).ok()?,
    );
    if width <= fit.0 && height <= fit.1 {
        return Some((natural, true));
    }
    // Width alone sets the size; the height follows the picture's.
    let scale = (f64::from(fit.0) / f64::from(width)).min(f64::from(fit.1) / f64::from(height));
    let fitted = i32::try_from(scaled(width, scale)).ok()?.max(1);
    let image = svg
        .render_parsed(
            &parsed,
            SvgSize::Size(size(DevicePixels(fitted), DevicePixels(fitted))),
        )
        .ok()?;
    Some((image, false))
}

/// `side` times `scale`, which is below 1, rounded down.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the product is non-negative and below `side`, a u32"
)]
fn scaled(side: u32, scale: f64) -> u32 {
    (f64::from(side) * scale).floor() as u32
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use gpui_kit::{SvgRenderer, TestAppContext};

    use super::{HEAD_COLUMNS, HEAD_LINES, Read, head, read};
    use crate::stage::load::{IMAGE_LIMIT, TEXT_LIMIT, stamp};

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
