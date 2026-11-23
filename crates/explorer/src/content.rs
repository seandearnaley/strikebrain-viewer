//! What the stage shows for a file, which extensions are images or known
//! binary formats, and how the Size column writes a byte count. Pure
//! functions: the caller reads the file.

use crate::tree::extension_of;

/// How the stage shows a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Viewer {
    /// Decoded and drawn as a picture.
    Image,
    /// Rendered as Markdown.
    Markdown,
    /// Shown as text in the read-only editor.
    Code,
    /// Not shown: only the file's name and size.
    Unsupported,
}

/// Extensions shown as images and passed by the Images kind. Lowercase,
/// without the dot.
pub const IMAGE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "ico", "svg",
];

/// Extensions of formats that are not text and not shown as images, in
/// alphabetical order: images the stage does not decode, audio, video, office
/// documents, archives and disk images, executables and object code, fonts,
/// databases and opaque data. The Text kind leaves them out, and the stage
/// never shows them as text. Lowercase, without the dot. Brainstrike's list,
/// without its search-index and lock-file entries (`Cargo.lock` is text).
pub const BINARY_EXTENSIONS: &[&str] = &[
    "7z", "a", "aac", "aiff", "arw", "avi", "avif", "bin", "br", "bz2", "class", "cr2", "dat",
    "db", "dll", "dmg", "dng", "doc", "docx", "dylib", "eot", "exe", "flac", "flv", "gz", "heic",
    "heif", "iso", "jar", "lz4", "lzma", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "nef", "o",
    "obj", "odp", "ods", "odt", "ogg", "otf", "pdf", "ppt", "pptx", "psd", "pyc", "rar", "raw",
    "so", "sqlite", "sqlite3", "tar", "tgz", "ttf", "war", "wasm", "wav", "webm", "wma", "wmv",
    "woff", "woff2", "xcf", "xls", "xlsx", "xz", "zip", "zst",
];

/// How many bytes of a file's start [`viewer_for`] expects.
pub const HEAD_LEN: usize = 8 * 1024;

/// Whether `ext`, without the dot and in any case, is an image the stage
/// decodes.
#[must_use]
pub fn is_image_extension(ext: &str) -> bool {
    IMAGE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether `ext`, without the dot and in any case, is a known binary format.
#[must_use]
pub fn is_binary_extension(ext: &str) -> bool {
    BINARY_EXTENSIONS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(ext))
}

/// How the stage shows the file called `name` whose first bytes are `head`
/// (up to [`HEAD_LEN`]; fewer for a shorter file). An image extension wins;
/// otherwise a NUL byte in `head` or a known binary extension means
/// [`Viewer::Unsupported`], `md` and `markdown` are [`Viewer::Markdown`], and
/// any other text is [`Viewer::Code`]. Extensions match in any case. The
/// stage's own limits, such as the largest text file it opens, are applied
/// by the stage.
///
/// Learning exercise: Sean may rewrite this body; the tests below define it.
#[must_use]
pub fn viewer_for(name: &str, head: &[u8]) -> Viewer {
    let ext = extension_of(name);
    if is_image_extension(ext) {
        Viewer::Image
    } else if head.contains(&0) || is_binary_extension(ext) {
        Viewer::Unsupported
    } else if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
        Viewer::Markdown
    } else {
        Viewer::Code
    }
}

/// A byte count as the Size column shows it: whole bytes under 1 KB, then
/// KB, MB or GB (powers of 1,024) with one decimal, rounded half up. A value
/// that rounds to 1,024 of a unit moves up to the next unit; GB is the
/// largest.
///
/// ```
/// # use explorer::format_size;
/// assert_eq!(format_size(512), "512 B");
/// assert_eq!(format_size(1536), "1.5 KB");
/// assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
/// ```
#[must_use]
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    // Tenths of a unit, rounded half up; u128 cannot overflow.
    let tenths = |scale: u128| (u128::from(bytes) * 10 + scale / 2) / scale;
    let mut scale = 1024;
    for unit in ["KB", "MB"] {
        let t = tenths(scale);
        if t < 10_240 {
            return format!("{}.{} {unit}", t / 10, t % 10);
        }
        scale *= 1024;
    }
    let t = tenths(scale);
    format!("{}.{} GB", t / 10, t % 10)
}

#[cfg(test)]
mod tests {
    use super::{
        BINARY_EXTENSIONS, IMAGE_EXTENSIONS, Viewer, format_size, is_binary_extension,
        is_image_extension, viewer_for,
    };

    const TEXT: &[u8] = b"fn main() {}\n";
    const BINARY: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    #[test]
    fn an_image_extension_is_an_image_even_with_nul_bytes() {
        assert_eq!(viewer_for("photo.png", BINARY), Viewer::Image);
        assert_eq!(viewer_for("Photo.JPEG", BINARY), Viewer::Image);
        assert_eq!(viewer_for("icon.svg", b"<svg/>"), Viewer::Image);
        for ext in IMAGE_EXTENSIONS {
            assert_eq!(viewer_for(&format!("x.{ext}"), b""), Viewer::Image, "{ext}");
        }
    }

    #[test]
    fn a_nul_byte_makes_any_other_file_unsupported() {
        assert_eq!(viewer_for("notes.txt", b"abc\0def"), Viewer::Unsupported);
        assert_eq!(viewer_for("README.md", b"# hi\0"), Viewer::Unsupported);
        assert_eq!(viewer_for("blob", BINARY), Viewer::Unsupported);
    }

    #[test]
    fn a_known_binary_extension_is_unsupported_without_a_nul() {
        assert_eq!(viewer_for("paper.pdf", b"%PDF-1.7"), Viewer::Unsupported);
        assert_eq!(viewer_for("archive.tar.GZ", TEXT), Viewer::Unsupported);
        assert_eq!(viewer_for("song.mp3", b""), Viewer::Unsupported);
    }

    #[test]
    fn markdown_extensions_are_markdown() {
        assert_eq!(viewer_for("README.md", b"# Title\n"), Viewer::Markdown);
        assert_eq!(viewer_for("notes.MARKDOWN", b""), Viewer::Markdown);
        assert_eq!(viewer_for("page.mdx", b"# x"), Viewer::Code);
    }

    #[test]
    fn other_text_is_code() {
        assert_eq!(viewer_for("main.rs", TEXT), Viewer::Code);
        assert_eq!(viewer_for("Makefile", b"all:\n"), Viewer::Code);
        assert_eq!(viewer_for(".gitignore", b"target/\n"), Viewer::Code);
        assert_eq!(viewer_for("Cargo.lock", b"version = 4\n"), Viewer::Code);
        assert_eq!(viewer_for("empty.txt", b""), Viewer::Code);
        // A dotfile's name is not its extension.
        assert_eq!(viewer_for(".png", TEXT), Viewer::Code);
    }

    #[test]
    fn extension_lists_match_in_any_case_and_do_not_overlap() {
        assert!(is_image_extension("PNG"));
        assert!(is_image_extension("tif"));
        assert!(!is_image_extension("heic"));
        assert!(!is_image_extension(""));
        assert!(is_binary_extension("Zip"));
        assert!(!is_binary_extension("rs"));
        assert!(!is_binary_extension(""));
        for &ext in BINARY_EXTENSIONS {
            assert!(!is_image_extension(ext), "{ext}");
            assert_eq!(ext, ext.to_lowercase(), "{ext}");
        }
    }

    #[test]
    fn sizes_use_bytes_then_one_decimal() {
        for (bytes, text) in [
            (0, "0 B"),
            (1, "1 B"),
            (1023, "1023 B"),
            (1024, "1.0 KB"),
            (1075, "1.0 KB"),
            (1076, "1.1 KB"),
            (1536, "1.5 KB"),
            (10 * 1024 + 512, "10.5 KB"),
            (1024 * 1024 - 1, "1.0 MB"),
            (1024 * 1024, "1.0 MB"),
            (5 * 1024 * 1024 + 300 * 1024, "5.3 MB"),
            (1024 * 1024 * 1024, "1.0 GB"),
            (2048 * 1024 * 1024 * 1024, "2048.0 GB"),
            (u64::MAX, "17179869184.0 GB"),
        ] {
            assert_eq!(format_size(bytes), text, "{bytes}");
        }
    }
}
