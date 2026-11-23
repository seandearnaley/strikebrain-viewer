//! Word-at-a-time operations over a range of bits. `fixedbitset` counts and
//! sets ranges of one set, but cannot combine two sets over a range, which a
//! folder's state under a filter needs.

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

/// Bits set in both `a` and `b` within `range`.
///
/// # Panics
///
/// If `range` reaches past either set.
pub(crate) fn count_and(a: &FixedBitSet, b: &FixedBitSet, range: Range<usize>) -> usize {
    let (a, b) = (a.as_slice(), b.as_slice());
    words(range)
        .map(|(w, mask)| (a[w] & b[w] & mask).count_ones() as usize)
        .sum()
}

/// Sets in `dest` every bit of `range` that is set in `a` and, if given, in `b`.
///
/// # Panics
///
/// If `range` reaches past any of the sets.
pub(crate) fn or_and(
    dest: &mut FixedBitSet,
    a: &FixedBitSet,
    b: Option<&FixedBitSet>,
    range: Range<usize>,
) {
    let a = a.as_slice();
    let b = b.map(FixedBitSet::as_slice);
    let dest = dest.as_mut_slice();
    for (w, mask) in words(range) {
        let extra = b.map_or(usize::MAX, |b| b[w]);
        dest[w] |= a[w] & extra & mask;
    }
}

#[cfg(test)]
mod tests {
    use fixedbitset::FixedBitSet;
    use proptest::prelude::*;

    use super::{count_and, or_and};

    fn set(len: usize, bits: &[bool]) -> FixedBitSet {
        let mut set = FixedBitSet::with_capacity(len);
        for (i, &on) in bits.iter().enumerate().take(len) {
            set.set(i, on);
        }
        set
    }

    #[test]
    fn whole_words_and_partial_edges_count() {
        let a = set(200, &[true; 200]);
        let mut b = FixedBitSet::with_capacity(200);
        b.insert_range(10..150);
        assert_eq!(count_and(&a, &b, 0..200), 140);
        assert_eq!(count_and(&a, &b, 64..128), 64);
        assert_eq!(count_and(&a, &b, 5..12), 2);
        assert_eq!(count_and(&a, &b, 70..70), 0);
        assert_eq!(count_and(&a, &b, 200..200), 0);
    }

    proptest! {
        #[test]
        fn count_and_matches_a_bit_loop(
            len in 0_usize..300,
            a in prop::collection::vec(any::<bool>(), 300),
            b in prop::collection::vec(any::<bool>(), 300),
            cut in (0_usize..300, 0_usize..300),
        ) {
            let (a, b) = (set(len, &a), set(len, &b));
            let (start, end) = (cut.0.min(cut.1).min(len), cut.0.max(cut.1).min(len));
            let naive = (start..end).filter(|&i| a.contains(i) && b.contains(i)).count();
            prop_assert_eq!(count_and(&a, &b, start..end), naive);
        }

        #[test]
        fn or_and_matches_a_bit_loop(
            len in 0_usize..300,
            dest in prop::collection::vec(any::<bool>(), 300),
            a in prop::collection::vec(any::<bool>(), 300),
            b in prop::option::of(prop::collection::vec(any::<bool>(), 300)),
            cut in (0_usize..300, 0_usize..300),
        ) {
            let (mut got, a) = (set(len, &dest), set(len, &a));
            let b = b.map(|b| set(len, &b));
            let (start, end) = (cut.0.min(cut.1).min(len), cut.0.max(cut.1).min(len));
            let mut want = got.clone();
            for i in start..end {
                if a.contains(i) && b.as_ref().is_none_or(|b| b.contains(i)) {
                    want.insert(i);
                }
            }
            or_and(&mut got, &a, b.as_ref(), start..end);
            prop_assert_eq!(got, want);
        }
    }
}
