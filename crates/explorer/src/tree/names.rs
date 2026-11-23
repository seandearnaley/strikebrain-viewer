//! Many short strings in one buffer, and the lowercasing the filter shares.

use super::TreeError;

/// Many strings in one `String` buffer, with one end offset per entry.
///
/// Entry `i` spans `ends[i - 1]..ends[i]` (or `0..ends[0]`), so a tree of a
/// million names makes two allocations instead of a million.
#[derive(Clone, Debug, Default)]
pub(crate) struct NameArena {
    buf: String,
    ends: Vec<u32>,
}

impl NameArena {
    /// An empty arena with room for `entries` strings of `bytes` total bytes.
    pub(crate) fn with_capacity(entries: usize, bytes: usize) -> Self {
        Self {
            buf: String::with_capacity(bytes),
            ends: Vec::with_capacity(entries),
        }
    }

    /// Appends one entry.
    pub(crate) fn push(&mut self, s: &str) -> Result<(), TreeError> {
        self.buf.push_str(s);
        self.close_entry()
    }

    /// Appends `s` lowercased and followed by `'\0'`: one haystack entry.
    pub(crate) fn push_lowercase_terminated(&mut self, s: &str) -> Result<(), TreeError> {
        push_lowercase(&mut self.buf, s);
        self.buf.push('\0');
        self.close_entry()
    }

    fn close_entry(&mut self) -> Result<(), TreeError> {
        let end = u32::try_from(self.buf.len()).map_err(|_| TreeError::TooLarge)?;
        self.ends.push(end);
        Ok(())
    }

    /// Entry `i`.
    pub(crate) fn get(&self, i: usize) -> &str {
        self.buf.get(self.span(i)).expect(
            "entry offsets are recorded after whole strings, so they lie on char boundaries",
        )
    }

    /// The byte range of entry `i` in [`Self::as_str`].
    pub(crate) fn span(&self, i: usize) -> std::ops::Range<usize> {
        let start = match i {
            0 => 0,
            _ => self.ends[i - 1] as usize,
        };
        start..self.ends[i] as usize
    }

    /// The index of the entry whose span contains byte `offset`.
    pub(crate) fn entry_at(&self, offset: usize) -> usize {
        self.ends.partition_point(|&end| end as usize <= offset)
    }

    /// The whole buffer.
    pub(crate) fn as_str(&self) -> &str {
        &self.buf
    }

    /// Total bytes held.
    pub(crate) fn byte_len(&self) -> usize {
        self.buf.len()
    }
}

/// Appends `s` to `out` in lowercase, one character at a time.
///
/// The filter lowercases its terms with this same function, so a name and a
/// term always fold the same way. Unlike `str::to_lowercase`, it has no
/// word-final sigma rule and allocates nothing.
pub(crate) fn push_lowercase(out: &mut String, s: &str) {
    if s.is_ascii() {
        let start = out.len();
        out.push_str(s);
        if let Some(tail) = out.get_mut(start..) {
            tail.make_ascii_lowercase();
        }
    } else {
        for c in s.chars() {
            out.extend(c.to_lowercase());
        }
    }
}

/// The extension of a lowercased file name: the text after its last dot,
/// unless that dot starts the name. `"a.tar.gz"` gives `"gz"`; `".gitignore"`
/// and `"notes"` give `""`.
pub(crate) fn extension_of(lower_name: &str) -> &str {
    match lower_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext,
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{NameArena, extension_of, push_lowercase};

    #[test]
    fn entries_come_back_in_order() {
        let mut arena = NameArena::default();
        for s in ["alpha", "", "é", "z"] {
            arena.push(s).unwrap();
        }
        assert_eq!(arena.get(0), "alpha");
        assert_eq!(arena.get(1), "");
        assert_eq!(arena.get(2), "é");
        assert_eq!(arena.get(3), "z");
        assert_eq!(arena.as_str(), "alphaéz");
        assert_eq!(arena.byte_len(), 8);
    }

    #[test]
    fn entry_at_maps_a_byte_offset_to_its_entry() {
        let mut arena = NameArena::default();
        for s in ["ab", "c", "def"] {
            arena.push_lowercase_terminated(s).unwrap();
        }
        assert_eq!(arena.as_str(), "ab\0c\0def\0");
        let found: Vec<usize> = (0..arena.byte_len()).map(|o| arena.entry_at(o)).collect();
        assert_eq!(found, [0, 0, 0, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn lowercasing_handles_ascii_and_unicode() {
        let mut out = String::from("keep ");
        push_lowercase(&mut out, "MiXeD");
        push_lowercase(&mut out, " ÄÖÜ İ");
        assert_eq!(out, "keep mixed äöü i\u{307}");
    }

    #[test]
    fn extensions_skip_dotfiles_and_bare_names() {
        assert_eq!(extension_of("a.tar.gz"), "gz");
        assert_eq!(extension_of("photo.png"), "png");
        assert_eq!(extension_of(".gitignore"), "");
        assert_eq!(extension_of("notes"), "");
        assert_eq!(extension_of("trailing."), "");
        assert_eq!(extension_of(".config.toml"), "toml");
    }
}
