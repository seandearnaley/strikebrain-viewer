//! A least-recently-used map with a budget, for the stage's caches.
//!
//! Each entry has a cost, and the costs together stay within the budget;
//! the cache may also hold at most so many entries. The text cache gives
//! each text its length in bytes and a budget of 32 MB. The image cache
//! holds at most 8 images and 512 MB of decoded pixels: an image's cost is
//! known only once it is decoded, so it comes in at no cost and is given
//! its cost then ([`Lru::set_cost`]), which may evict others, but never the
//! image on the stage. Making room evicts the entries used longest ago and
//! hands them back, so the caller can release what they hold (an image's
//! GPU texture).

use std::collections::HashMap;
use std::hash::Hash;

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Lru<K, V> {
    budget: usize,
    /// The most entries it holds, whatever they cost.
    max_len: usize,
    used: usize,
    /// Counts uses; an entry's stamp is the count at its last use.
    clock: u64,
    entries: HashMap<K, Slot<V>>,
}

#[derive(Debug)]
struct Slot<V> {
    value: V,
    cost: usize,
    last_used: u64,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    /// An empty cache whose entries may cost `budget` together.
    pub(crate) fn new(budget: usize) -> Self {
        Self::with_max_len(budget, usize::MAX)
    }

    /// An empty cache of at most `max_len` entries, which may cost `budget`
    /// together.
    pub(crate) fn with_max_len(budget: usize, max_len: usize) -> Self {
        Self {
            budget,
            max_len,
            used: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    /// The entry for `key`, marked as just used.
    pub(crate) fn get(&mut self, key: &K) -> Option<&mut V> {
        self.clock += 1;
        let slot = self.entries.get_mut(key)?;
        slot.last_used = self.clock;
        Some(&mut slot.value)
    }

    /// The entry for `key`, leaving its place in the order alone.
    pub(crate) fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|slot| &slot.value)
    }

    /// The entry for `key`, to change, leaving its place in the order alone.
    pub(crate) fn peek_mut(&mut self, key: &K) -> Option<&mut V> {
        self.entries.get_mut(key).map(|slot| &mut slot.value)
    }

    /// Every entry, in no particular order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(key, slot)| (key, &slot.value))
    }

    /// Stores `value` as the most recently used entry and returns what it
    /// pushed out: a value it replaced under the same key, then the least
    /// recently used entries until the new one fits. A value that costs
    /// more than the whole budget is not kept; it comes straight back.
    pub(crate) fn insert(&mut self, key: K, value: V, cost: usize) -> Vec<V> {
        self.insert_keeping(key, value, cost, None)
    }

    /// Stores `value` as [`Lru::insert`] does, but never evicts `keep` to
    /// make room.
    pub(crate) fn insert_keeping(
        &mut self,
        key: K,
        value: V,
        cost: usize,
        keep: Option<&K>,
    ) -> Vec<V> {
        let mut evicted: Vec<V> = self.remove(&key).into_iter().collect();
        if cost > self.budget {
            evicted.push(value);
            return evicted;
        }
        while self.used + cost > self.budget || self.entries.len() >= self.max_len {
            let Some(oldest) = self.oldest(keep) else {
                break;
            };
            evicted.extend(self.remove(&oldest));
        }
        self.clock += 1;
        self.used += cost;
        self.entries.insert(
            key,
            Slot {
                value,
                cost,
                last_used: self.clock,
            },
        );
        evicted
    }

    /// Gives the entry for `key` the cost it turned out to have, then
    /// evicts the entries used longest ago, `key` among them but never
    /// `keep`, until the costs fit the budget or only `keep` is left.
    /// Returns what it evicted.
    pub(crate) fn set_cost(&mut self, key: &K, cost: usize, keep: Option<&K>) -> Vec<V> {
        let Some(slot) = self.entries.get_mut(key) else {
            return Vec::new();
        };
        self.used = self.used - slot.cost + cost;
        slot.cost = cost;
        let mut evicted = Vec::new();
        while self.used > self.budget {
            let Some(oldest) = self.oldest(keep) else {
                break;
            };
            evicted.extend(self.remove(&oldest));
        }
        evicted
    }

    /// The key used longest ago, other than `keep`.
    fn oldest(&self, keep: Option<&K>) -> Option<K> {
        self.entries
            .iter()
            .filter(|(key, _)| Some(*key) != keep)
            .min_by_key(|(_, slot)| slot.last_used)
            .map(|(key, _)| key.clone())
    }

    /// Takes the entry for `key` out.
    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        let slot = self.entries.remove(key)?;
        self.used -= slot.cost;
        Some(slot.value)
    }

    /// Takes every entry out.
    pub(crate) fn drain(&mut self) -> impl Iterator<Item = V> + '_ {
        self.used = 0;
        self.entries.drain().map(|(_, slot)| slot.value)
    }

    /// How many entries the cache holds.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// What the entries cost together; never more than the budget.
    pub(crate) fn used(&self) -> usize {
        self.used
    }

    /// What the entries may cost together.
    pub(crate) fn budget(&self) -> usize {
        self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::Lru;

    #[test]
    fn eight_images_fit_and_a_ninth_evicts_the_least_recently_used() {
        let mut cache = Lru::new(8);
        for image in 0..8 {
            assert_eq!(cache.insert(image, image, 1), Vec::<i32>::new());
        }
        // Using 0 makes 1 the oldest.
        assert_eq!(cache.get(&0), Some(&mut 0));
        assert_eq!(cache.insert(8, 8, 1), [1]);
        assert_eq!(cache.len(), 8);
        assert!(cache.peek(&1).is_none());
        assert!(cache.peek(&0).is_some());

        // Never more than eight, whatever the order of use.
        for image in 9..100 {
            cache.get(&(image % 7));
            assert!(cache.insert(image, image, 1).len() <= 1);
            assert_eq!(cache.len(), 8);
            assert_eq!(cache.used(), 8);
        }
    }

    #[test]
    fn at_most_so_many_entries_whatever_they_cost() {
        let mut cache = Lru::with_max_len(100, 2);
        cache.insert("a", 1, 0);
        cache.insert("b", 2, 0);
        assert_eq!(cache.insert("c", 3, 0), [1]);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_cost_learned_later_evicts_the_oldest_but_never_the_kept() {
        // Images: their cost is known once decoded.
        let mut cache = Lru::with_max_len(10, 8);
        for key in ["a", "b", "c", "d"] {
            cache.insert(key, key, 0);
        }
        assert_eq!(cache.set_cost(&"a", 4, None), Vec::<&str>::new());
        assert_eq!(cache.set_cost(&"b", 4, None), Vec::<&str>::new());
        // c needs room: a, used longest ago, goes.
        assert_eq!(cache.set_cost(&"c", 4, None), ["a"]);
        assert_eq!(cache.used(), 8);
        // d is on the stage and larger than the whole budget: kept, and
        // everything else goes.
        let mut evicted = cache.set_cost(&"d", 12, Some(&"d"));
        evicted.sort_unstable();
        assert_eq!(evicted, ["b", "c"]);
        assert_eq!((cache.len(), cache.used()), (1, 12));
        // A neighbour decoded next to it is not kept: the stage's image is.
        assert_eq!(
            cache.insert_keeping("e", "e", 0, Some(&"d")),
            Vec::<&str>::new()
        );
        assert_eq!(cache.set_cost(&"e", 1, Some(&"d")), ["e"]);
        assert!(cache.peek(&"d").is_some());
    }

    #[test]
    fn peeking_leaves_the_order_alone() {
        let mut cache = Lru::new(2);
        cache.insert("a", 1, 1);
        cache.insert("b", 2, 1);
        assert_eq!(cache.peek(&"a"), Some(&1));
        assert_eq!(cache.insert("c", 3, 1), [1]);
    }

    #[test]
    fn texts_stay_within_their_byte_budget() {
        const MB: usize = 1024 * 1024;
        let mut cache = Lru::new(32 * MB);
        for text in 0..10 {
            cache.insert(text, text, 10 * MB);
            assert!(cache.used() <= 32 * MB, "{} bytes", cache.used());
        }
        // Three 10 MB texts fit in 32 MB; the newest three are kept.
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.used(), 30 * MB);
        assert!(cache.peek(&7).is_some() && cache.peek(&9).is_some());

        // A small text evicts nothing while it fits.
        assert_eq!(cache.insert(100, 100, MB), Vec::<i32>::new());
        // Another 10 MB evicts the oldest until it fits.
        assert_eq!(cache.insert(101, 101, 10 * MB), [7]);
        assert_eq!(cache.used(), 31 * MB);
    }

    #[test]
    fn a_value_larger_than_the_budget_is_not_kept() {
        let mut cache = Lru::new(10);
        cache.insert("small", 1, 4);
        assert_eq!(cache.insert("huge", 2, 11), [2]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used(), 4);
    }

    #[test]
    fn replacing_a_key_returns_the_old_value_and_its_cost() {
        let mut cache = Lru::new(10);
        cache.insert("a", 1, 6);
        assert_eq!(cache.insert("a", 2, 3), [1]);
        assert_eq!(cache.used(), 3);
        assert_eq!(cache.remove(&"a"), Some(2));
        assert_eq!(cache.used(), 0);
        cache.insert("b", 3, 5);
        assert_eq!(cache.budget(), 10);
        assert_eq!(cache.drain().collect::<Vec<_>>(), [3]);
        assert_eq!((cache.len(), cache.used()), (0, 0));
    }
}
