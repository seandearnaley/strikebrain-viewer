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

    /// Appends entries `range` of `other`, in one copy of their bytes.
    pub(crate) fn extend_from(
        &mut self,
        other: &Self,
        range: std::ops::Range<usize>,
    ) -> Result<(), TreeError> {
        if range.is_empty() {
            return Ok(());
        }
        let bytes = other.span(range.start).start..other.span(range.end - 1).end;
        let base = self.buf.len();
        self.buf.push_str(
            other
                .buf
                .get(bytes.clone())
                .expect("entry offsets lie on char boundaries"),
        );
        u32::try_from(self.buf.len()).map_err(|_| TreeError::TooLarge)?;
        let shift = |end: u32| {
            u32::try_from(end as usize - bytes.start + base).expect("checked against the length")
        };
        self.ends
            .extend(other.ends[range].iter().map(|&end| shift(end)));
        Ok(())
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

    /// Each entry's end offset in [`Self::as_str`].
    pub(crate) fn ends(&self) -> &[u32] {
        &self.ends
    }

    /// The whole buffer.
    pub(crate) fn as_str(&self) -> &str {
        &self.buf
    }

    /// Total bytes held.
    pub(crate) fn byte_len(&self) -> usize {
        self.buf.len()
    }

    /// The bytes entries `range` hold.
    pub(crate) fn bytes_of(&self, range: std::ops::Range<usize>) -> usize {
        if range.is_empty() {
            return 0;
        }
        self.span(range.end - 1).end - self.span(range.start).start
    }

    /// Bytes reserved, held or not.
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.buf.capacity()
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
    fn haystack_entries_are_lowercase_and_nul_terminated() {
        let mut arena = NameArena::default();
        for s in ["Ab", "c", "DEF"] {
            arena.push_lowercase_terminated(s).unwrap();
        }
        assert_eq!(arena.as_str(), "ab\0c\0def\0");
        assert_eq!(arena.ends(), [3, 5, 9]);
        assert_eq!(arena.get(2), "def\0");
    }

    #[test]
    fn entries_copy_from_another_arena() {
        let mut from = NameArena::default();
        for s in ["skip", "é", "", "kept"] {
            from.push(s).unwrap();
        }
        let mut arena = NameArena::default();
        arena.push("first").unwrap();
        arena.extend_from(&from, 1..4).unwrap();
        arena.extend_from(&from, 2..2).unwrap();
        assert_eq!(arena.as_str(), "firstékept");
        assert_eq!(arena.get(1), "é");
        assert_eq!(arena.get(2), "");
        assert_eq!(arena.get(3), "kept");
        assert_eq!(arena.ends().len(), 4);
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
