//! Word-at-a-time operations over a range of bits. `fixedbitset` counts and
//! sets ranges of one set, but cannot copy one set's range into another,
//! which checking a folder needs. [`Ranks`] counts any range in constant
//! time, which a folder's state on every frame needs.

use std::ops::Range;

use fixedbitset::FixedBitSet;

const WORD: usize = usize::BITS as usize;

/// Each word index touched by `range`, with the mask of the range's bits in it.
fn words(range: Range<usize>) -> impl Iterator<Item = (usize, usize)> {
    let Range { start, end } = range;
    (start / WORD..end.div_ceil(WORD)).map(move |word| {
        let base = word * WORD;
        let low = start.max(base) - base;
        let high = end.min(base + WORD) - base;
        let mask = match high - low {
            WORD => usize::MAX,
            width => ((1 << width) - 1) << low,
        };
        (word, mask)
    })
}

/// Sets in `dest` every bit of `range` that is set in `a`.
///
/// # Panics
///
/// If `range` reaches past either set.
pub(crate) fn or_range(dest: &mut FixedBitSet, a: &FixedBitSet, range: Range<usize>) {
    let a = a.as_slice();
    let dest = dest.as_mut_slice();
    for (w, mask) in words(range) {
        dest[w] |= a[w] & mask;
    }
}

/// Running counts of a bit set, one per word, so that the bits set in any
/// range cost two lookups and two masked popcounts instead of a pass over
/// the range. The set itself is not kept: each question passes a function
/// giving word `w` of the set the counts were made from, which lets one
/// count a combination of sets, such as `a & b`, without building it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ranks {
    /// `before[w]` is the number of set bits in words `0..w`; one entry
    /// more than there are words.
    before: Box<[u32]>,
}

impl Ranks {
    /// The running counts of `words`, in order.
    ///
    /// # Panics
    ///
    /// Past `u32::MAX` set bits, more than a tree has nodes.
    pub(crate) fn new(words: impl Iterator<Item = usize>) -> Self {
        let mut before = Vec::with_capacity(words.size_hint().0 + 1);
        let mut total = 0_u32;
        before.push(total);
        for word in words {
            total = total
                .checked_add(word.count_ones())
                .expect("a tree has fewer than u32::MAX nodes");
            before.push(total);
        }
        Self {
            before: before.into_boxed_slice(),
        }
    }

    /// Set bits in `range`; `word(w)` is word `w` of the counted set.
    ///
    /// # Panics
    ///
    /// If `range` reaches past the counted words.
    pub(crate) fn count(&self, word: impl Fn(usize) -> usize, range: Range<usize>) -> usize {
        if range.is_empty() {
            return 0;
        }
        self.below(&word, range.end) - self.below(&word, range.start)
    }

    /// Set bits below bit `i`.
    fn below(&self, word: &impl Fn(usize) -> usize, i: usize) -> usize {
        let (w, bit) = (i / WORD, i % WORD);
        let whole = self.before[w] as usize;
        match bit {
            0 => whole,
            bit => whole + (word(w) & ((1 << bit) - 1)).count_ones() as usize,
        }
    }
}

/// The lowest set bit at `from` or above, if any.
pub(crate) fn next_one(set: &FixedBitSet, from: usize) -> Option<usize> {
    if from >= set.len() {
        return None;
    }
    let blocks = set.as_slice();
    let mut w = from / WORD;
    let mut word = blocks[w] & (usize::MAX << (from % WORD));
    loop {
        if word != 0 {
            return Some(w * WORD + word.trailing_zeros() as usize);
        }
        w += 1;
        word = *blocks.get(w)?;
    }
}

/// The set bit with `n` set bits below it, if any: a word's population at
/// a time, then the bits of the one word that holds it.
pub(crate) fn nth_one(set: &FixedBitSet, mut n: usize) -> Option<usize> {
    for (w, &word) in set.as_slice().iter().enumerate() {
        let ones = word.count_ones() as usize;
        if n >= ones {
            n -= ones;
            continue;
        }
        let mut word = word;
        for _ in 0..n {
            // Clears the lowest set bit.
            word &= word - 1;
        }
        return Some(w * WORD + word.trailing_zeros() as usize);
    }
    None
}

/// The highest set bit below `before`, if any.
///
/// # Panics
///
/// If `before` is past the end of the set.
pub(crate) fn prev_one(set: &FixedBitSet, before: usize) -> Option<usize> {
    assert!(before <= set.len(), "{before} is past the set");
    let last = before.checked_sub(1)?;
    let blocks = set.as_slice();
    let mut w = last / WORD;
    let keep = last % WORD;
    let mut word = blocks[w]
        & match keep {
            k if k == WORD - 1 => usize::MAX,
            k => (1 << (k + 1)) - 1,
        };
    loop {
        if word != 0 {
            return Some(w * WORD + (WORD - 1 - word.leading_zeros() as usize));
        }
        w = w.checked_sub(1)?;
        word = blocks[w];
    }
}

#[cfg(test)]
mod tests {
    use fixedbitset::FixedBitSet;
    use proptest::prelude::*;

    use super::{Ranks, next_one, nth_one, or_range, prev_one};

    fn set(len: usize, bits: &[bool]) -> FixedBitSet {
        let mut set = FixedBitSet::with_capacity(len);
        for (i, &on) in bits.iter().enumerate().take(len) {
            set.set(i, on);
        }
        set
    }

    #[test]
    fn ranks_count_whole_words_and_partial_edges() {
        let mut b = FixedBitSet::with_capacity(200);
        b.insert_range(10..150);
        let words = b.as_slice();
        let ranks = Ranks::new(words.iter().copied());
        let count = |range| ranks.count(|w| words[w], range);
        assert_eq!(count(0..200), 140);
        assert_eq!(count(64..128), 64);
        assert_eq!(count(5..12), 2);
        assert_eq!(count(70..70), 0);
        assert_eq!(count(200..200), 0);
        assert_eq!(count(0..192), 140);
    }

    #[test]
    fn prev_one_finds_the_highest_bit_below() {
        let mut set = FixedBitSet::with_capacity(200);
        for bit in [0, 63, 64, 130] {
            set.insert(bit);
        }
        assert_eq!(prev_one(&set, 200), Some(130));
        assert_eq!(prev_one(&set, 130), Some(64));
        assert_eq!(prev_one(&set, 64), Some(63));
        assert_eq!(prev_one(&set, 63), Some(0));
        assert_eq!(prev_one(&set, 0), None);
        assert_eq!(prev_one(&FixedBitSet::with_capacity(70), 70), None);
    }

    proptest! {
        #[test]
        fn next_one_matches_a_bit_loop(
            len in 0_usize..300,
            bits in prop::collection::vec(any::<bool>(), 300),
            from in 0_usize..310,
        ) {
            let set = set(len, &bits);
            prop_assert_eq!(next_one(&set, from), (from..len).find(|&i| set.contains(i)));
        }

        #[test]
        fn nth_one_matches_a_bit_loop(
            len in 0_usize..300,
            bits in prop::collection::vec(any::<bool>(), 300),
            n in 0_usize..310,
        ) {
            let set = set(len, &bits);
            prop_assert_eq!(nth_one(&set, n), set.ones().nth(n));
        }

        #[test]
        fn prev_one_matches_a_bit_loop(
            len in 0_usize..300,
            bits in prop::collection::vec(any::<bool>(), 300),
            before in 0_usize..300,
        ) {
            let set = set(len, &bits);
            let before = before.min(len);
            prop_assert_eq!(prev_one(&set, before), (0..before).rev().find(|&i| set.contains(i)));
        }

        #[test]
        fn ranks_count_a_range_like_a_bit_loop(
            len in 0_usize..300,
            a in prop::collection::vec(any::<bool>(), 300),
            b in prop::collection::vec(any::<bool>(), 300),
            cut in (0_usize..300, 0_usize..300),
        ) {
            let (a, b) = (set(len, &a), set(len, &b));
            let (a_words, b_words) = (a.as_slice(), b.as_slice());
            let both = |w: usize| a_words[w] & b_words[w];
            let ranks = Ranks::new((0..a_words.len()).map(both));
            let (start, end) = (cut.0.min(cut.1).min(len), cut.0.max(cut.1).min(len));
            let naive = |range: std::ops::Range<usize>| {
                range.filter(|&i| a.contains(i) && b.contains(i)).count()
            };
            prop_assert_eq!(ranks.count(both, start..end), naive(start..end));
            prop_assert_eq!(ranks.count(both, 0..len), naive(0..len));
        }

        #[test]
        fn or_range_matches_a_bit_loop(
            len in 0_usize..300,
            dest in prop::collection::vec(any::<bool>(), 300),
            a in prop::collection::vec(any::<bool>(), 300),
            cut in (0_usize..300, 0_usize..300),
        ) {
            let (mut got, a) = (set(len, &dest), set(len, &a));
            let (start, end) = (cut.0.min(cut.1).min(len), cut.0.max(cut.1).min(len));
            let mut want = got.clone();
            for i in start..end {
                if a.contains(i) {
                    want.insert(i);
                }
            }
            or_range(&mut got, &a, start..end);
            prop_assert_eq!(got, want);
        }
    }
}
