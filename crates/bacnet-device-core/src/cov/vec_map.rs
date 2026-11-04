//! [`VecMap`]: a map kept as one vector sorted by key, for the table's
//! small maps (its flights and the per-peer counts).
//!
//! It answers as the `BTreeMap` it replaced did, in key order, with the
//! entries' lookups a binary search. A `BTreeMap` links its node code once
//! for each key and value type; the vector's code is a fraction of that in
//! a firmware image (#2363).
//!
//! An insert or a removal moves the entries after it. A device holds a few
//! entries (one per subscription or peer); the server, up to one per
//! subscription, 1,024 at its default cap. Measured with 1,024 flights (a
//! release build on a desktop host, 96-octet entries), an insert or a
//! removal in the middle takes about 250 ns against the `BTreeMap`'s 21
//! to 25, at the front about 450, and at the end about 10. A flight's
//! entry goes in when a confirmed report begins and out when it ends, so
//! that is under half a microsecond twice per confirmed notification.

use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub(super) struct VecMap<K, V>(Vec<(K, V)>);

impl<K, V> Default for VecMap<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: Ord, V> VecMap<K, V> {
    fn find(&self, key: &K) -> Result<usize, usize> {
        self.0.binary_search_by(|(at, _)| at.cmp(key))
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.find(key).ok().map(|at| &self.0[at].1)
    }

    pub(super) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.find(key).ok().map(|at| &mut self.0[at].1)
    }

    /// The value at `key`, inserted as its default when absent.
    pub(super) fn or_default(&mut self, key: K) -> &mut V
    where
        V: Default,
    {
        let at = match self.find(&key) {
            Ok(at) => at,
            Err(at) => {
                self.0.insert(at, (key, V::default()));
                at
            }
        };
        &mut self.0[at].1
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        self.find(key).ok().map(|at| self.0.remove(at).1)
    }

    /// Entries in key order.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter().map(|(key, value)| (key, value))
    }

    /// Values in key order.
    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, value)| value)
    }
}
