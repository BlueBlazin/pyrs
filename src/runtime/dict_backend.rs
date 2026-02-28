//! Internal dict storage backend.
//!
//! Implements CPython-style open addressing with perturb probing while preserving
//! insertion order in the `entries` vector.

use super::{Value, value_key_equal, value_lookup_hash};

const PERTURB_SHIFT: usize = 5;
const MIN_TABLE_SIZE: usize = 8;
const LOAD_NUMERATOR: usize = 2;
const LOAD_DENOMINATOR: usize = 3;
const MAX_PERTURB_ROUNDS: usize = (usize::BITS as usize / PERTURB_SHIFT) + 2;
const NO_SLOT: usize = usize::MAX;

#[derive(Debug, Clone, Copy)]
enum DictSlot {
    Empty,
    Dummy,
    Occupied { hash: u64, entry: usize },
}

#[derive(Debug, Clone)]
/// Dense entry storage plus hash-probe slot table.
///
/// `entries` preserves logical insertion order; `slots` accelerates hash lookups.
pub(super) struct DictBackend {
    entries: Vec<(Value, Value)>,
    entry_hashes: Vec<Option<u64>>,
    entry_slots: Vec<usize>,
    slots: Vec<DictSlot>,
    used: usize,
    filled: usize,
}

#[derive(Debug, Clone, Copy)]
enum SlotLookup {
    Found { entry: usize },
    Vacant(usize),
}

impl DictBackend {
    pub(super) fn new(initial_entries: Vec<(Value, Value)>) -> Self {
        let mut out = Self {
            entries: Vec::new(),
            entry_hashes: Vec::new(),
            entry_slots: Vec::new(),
            slots: Vec::new(),
            used: 0,
            filled: 0,
        };
        for (key, value) in initial_entries {
            out.insert(key, value);
        }
        out
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.entry_hashes.clear();
        self.entry_slots.clear();
        self.slots.clear();
        self.used = 0;
        self.filled = 0;
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, (Value, Value)> {
        self.entries.iter()
    }

    pub(super) fn iter_mut(&mut self) -> std::slice::IterMut<'_, (Value, Value)> {
        self.entries.iter_mut()
    }

    pub(super) fn to_vec(&self) -> Vec<(Value, Value)> {
        self.entries.clone()
    }

    pub(super) fn entry_at(&self, index: usize) -> &(Value, Value) {
        &self.entries[index]
    }

    pub(super) fn set_value_at(&mut self, index: usize, value: Value) {
        self.entries[index].1 = value;
    }

    pub(super) fn into_entries(self) -> Vec<(Value, Value)> {
        self.entries
    }

    pub(super) fn find(&self, key: &Value) -> Option<&Value> {
        let index = self.find_index(key)?;
        Some(&self.entries[index].1)
    }

    pub(super) fn find_with_hash(&self, key: &Value, hash: u64) -> Option<&Value> {
        let index = self.find_index_with_hash(key, hash)?;
        Some(&self.entries[index].1)
    }

    pub(super) fn candidate_indices_for_hash(&self, hash: u64) -> Vec<usize> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        if self.slots.is_empty() {
            return self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, (key, _))| {
                    (value_lookup_hash(key) == Some(hash)).then_some(index)
                })
                .collect();
        }
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        let mut perturb = hash as usize;
        let mut candidates = Vec::new();
        // CPython probing may need extra rounds while perturb collapses to zero.
        for _ in 0..(self.slots.len() + MAX_PERTURB_ROUNDS) {
            match self.slots[slot] {
                DictSlot::Empty => break,
                DictSlot::Dummy => {}
                DictSlot::Occupied {
                    hash: slot_hash,
                    entry,
                } => {
                    if slot_hash == hash {
                        candidates.push(entry);
                    }
                }
            }
            slot = ((slot * 5).wrapping_add(perturb).wrapping_add(1)) & mask;
            perturb >>= PERTURB_SHIFT;
        }
        candidates
    }

    pub(super) fn contains_key(&self, key: &Value) -> bool {
        self.find_index(key).is_some()
    }

    pub(super) fn contains_key_with_hash(&self, key: &Value, hash: u64) -> bool {
        self.find_index_with_hash(key, hash).is_some()
    }

    /// Insert/update a key using runtime hash semantics when hashable.
    pub(super) fn insert(&mut self, key: Value, value: Value) {
        let Some(hash) = value_lookup_hash(&key) else {
            if let Some(index) = self.find_index(&key) {
                self.entries[index].1 = value;
                return;
            }
            self.entries.push((key, value));
            self.entry_hashes.push(None);
            self.entry_slots.push(NO_SLOT);
            self.used = self.entries.len();
            return;
        };
        self.insert_with_hash(key, value, hash);
    }

    /// Insert/update using a caller-provided hash (C-API compatibility path).
    pub(super) fn insert_with_hash(&mut self, key: Value, value: Value, hash: u64) {
        if self.slots.is_empty() {
            self.resize_slots(MIN_TABLE_SIZE);
        }

        let mut slot = match self.lookup_slot(&key, hash) {
            SlotLookup::Found { entry } => {
                self.entries[entry].1 = value;
                self.entry_hashes[entry] = Some(hash);
                return;
            }
            SlotLookup::Vacant(slot) => slot,
        };
        let usable_slots = (self.slots.len() * LOAD_NUMERATOR) / LOAD_DENOMINATOR;
        if self.filled + 1 > usable_slots {
            self.resize_slots(self.slots.len() * 2);
            slot = self.lookup_vacant_slot(hash);
        }

        let entry = self.entries.len();
        self.entries.push((key, value));
        self.entry_hashes.push(Some(hash));
        self.entry_slots.push(slot);
        self.used += 1;
        if matches!(self.slots[slot], DictSlot::Empty) {
            self.filled += 1;
        }
        self.slots[slot] = DictSlot::Occupied { hash, entry };
    }

    pub(super) fn remove_key(&mut self, key: &Value) -> Option<(Value, Value)> {
        let index = self.find_index(key)?;
        Some(self.remove(index))
    }

    pub(super) fn remove_key_with_hash(
        &mut self,
        key: &Value,
        hash: u64,
    ) -> Option<(Value, Value)> {
        let index = self.find_index_with_hash(key, hash)?;
        Some(self.remove(index))
    }

    pub(super) fn remove(&mut self, index: usize) -> (Value, Value) {
        let removed_slot = self.entry_slots[index];
        if removed_slot != NO_SLOT {
            self.remove_slot_for_entry(index, removed_slot);
        }
        let (removed_key, removed_value) = self.entries.remove(index);
        self.entry_hashes.remove(index);
        self.entry_slots.remove(index);
        if removed_slot != NO_SLOT {
            self.adjust_slot_indices_after_remove(index);
        }
        self.used = self.entries.len();
        self.maybe_resize_after_remove();
        (removed_key, removed_value)
    }

    pub(super) fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&(Value, Value)) -> bool,
    {
        let old_entries = std::mem::take(&mut self.entries);
        let old_hashes = std::mem::take(&mut self.entry_hashes);
        self.entries = Vec::with_capacity(old_entries.len());
        self.entry_hashes = Vec::with_capacity(old_hashes.len());
        for (entry, hash) in old_entries.into_iter().zip(old_hashes.into_iter()) {
            if f(&entry) {
                self.entries.push(entry);
                self.entry_hashes.push(hash);
            }
        }
        self.rebuild_slots();
    }

    fn find_index(&self, key: &Value) -> Option<usize> {
        if let Some(hash) = value_lookup_hash(key) {
            return self.find_index_with_hash(key, hash);
        }
        self.entries
            .iter()
            .position(|(stored, _)| value_key_equal(stored, key))
    }

    fn find_index_with_hash(&self, key: &Value, hash: u64) -> Option<usize> {
        if !self.slots.is_empty() {
            if let SlotLookup::Found { entry } = self.lookup_slot(key, hash) {
                return Some(entry);
            }
            return None;
        }
        self.entries
            .iter()
            .position(|(stored, _)| value_key_equal(stored, key))
    }

    fn maybe_resize_after_remove(&mut self) {
        if self.entries.is_empty() {
            self.clear();
            return;
        }
        let dummy_count = self.filled.saturating_sub(self.used);
        if dummy_count > self.used {
            self.resize_slots(self.slots.len().max(MIN_TABLE_SIZE));
            return;
        }
        if self.slots.len() <= MIN_TABLE_SIZE {
            return;
        }
        if self.used * 8 <= self.slots.len() {
            self.resize_slots(self.slots.len() / 2);
        }
    }

    fn resize_slots(&mut self, target: usize) {
        let size = next_power_of_two(target.max(MIN_TABLE_SIZE));
        self.slots = vec![DictSlot::Empty; size];
        self.entry_slots = vec![NO_SLOT; self.entries.len()];
        self.filled = 0;
        self.used = self.entries.len();
        for entry in 0..self.entries.len() {
            let mut hash = self.entry_hashes.get(entry).copied().flatten();
            if hash.is_none() {
                hash = value_lookup_hash(&self.entries[entry].0);
                self.entry_hashes[entry] = hash;
            }
            if let Some(hash) = hash {
                let slot = self.lookup_vacant_slot(hash);
                self.slots[slot] = DictSlot::Occupied { hash, entry };
                self.entry_slots[entry] = slot;
                self.filled += 1;
            }
        }
    }

    fn rebuild_slots(&mut self) {
        if self.entries.is_empty() {
            self.clear();
            return;
        }
        self.resize_slots(self.entries.len().max(MIN_TABLE_SIZE));
    }

    /// Find a writable slot index for the given hash (empty or first dummy).
    fn lookup_vacant_slot(&self, hash: u64) -> usize {
        debug_assert!(!self.slots.is_empty());
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        let mut perturb = hash as usize;
        let mut first_dummy = None;
        // CPython probing may need extra rounds while perturb collapses to zero.
        for _ in 0..(self.slots.len() + MAX_PERTURB_ROUNDS) {
            match self.slots[slot] {
                DictSlot::Empty => return first_dummy.unwrap_or(slot),
                DictSlot::Dummy => {
                    if first_dummy.is_none() {
                        first_dummy = Some(slot);
                    }
                }
                DictSlot::Occupied { .. } => {}
            }
            slot = ((slot * 5).wrapping_add(perturb).wrapping_add(1)) & mask;
            perturb >>= PERTURB_SHIFT;
        }
        first_dummy.unwrap_or(slot)
    }

    /// Probe for an existing key slot or first acceptable insertion slot.
    fn lookup_slot(&self, key: &Value, hash: u64) -> SlotLookup {
        debug_assert!(!self.slots.is_empty());
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        let mut perturb = hash as usize;
        let mut first_dummy = None;

        // CPython probing may need extra rounds while perturb collapses to zero.
        for _ in 0..(self.slots.len() + MAX_PERTURB_ROUNDS) {
            match self.slots[slot] {
                DictSlot::Empty => return SlotLookup::Vacant(first_dummy.unwrap_or(slot)),
                DictSlot::Dummy => {
                    if first_dummy.is_none() {
                        first_dummy = Some(slot);
                    }
                }
                DictSlot::Occupied {
                    hash: slot_hash,
                    entry,
                } => {
                    if slot_hash == hash && value_key_equal(&self.entries[entry].0, key) {
                        return SlotLookup::Found { entry };
                    }
                }
            }
            slot = ((slot * 5).wrapping_add(perturb).wrapping_add(1)) & mask;
            perturb >>= PERTURB_SHIFT;
        }
        SlotLookup::Vacant(first_dummy.unwrap_or(slot))
    }

    fn remove_slot_for_entry(&mut self, expected_entry: usize, slot_index: usize) {
        if let Some(slot) = self.slots.get_mut(slot_index)
            && let DictSlot::Occupied { entry, .. } = slot
            && *entry == expected_entry
        {
            *slot = DictSlot::Dummy;
        }
    }

    fn adjust_slot_indices_after_remove(&mut self, removed_index: usize) {
        for slot_index in self.entry_slots.iter().skip(removed_index) {
            if *slot_index == NO_SLOT {
                continue;
            }
            if let Some(DictSlot::Occupied { entry, .. }) = self.slots.get_mut(*slot_index)
                && *entry > removed_index
            {
                *entry -= 1;
            }
        }
    }
}

fn next_power_of_two(value: usize) -> usize {
    if value <= 1 {
        return 1;
    }
    value.next_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::{DictBackend, MIN_TABLE_SIZE, value_lookup_hash};
    use crate::runtime::Value;

    fn collide_on_mask(mask: usize, count: usize) -> Vec<Value> {
        let mut out = Vec::new();
        let mut candidate = 0i64;
        while out.len() < count {
            let value = Value::Int(candidate);
            let hash = value_lookup_hash(&value).expect("int hash");
            if ((hash as usize) & mask) == 0 {
                out.push(value);
            }
            candidate += 1;
        }
        out
    }

    #[test]
    fn probing_handles_multiple_collisions_and_deletes() {
        let mut backend = DictBackend::new(Vec::new());
        backend.resize_slots(8);
        let keys = collide_on_mask(backend.slots.len() - 1, 5);
        for (idx, key) in keys.iter().enumerate() {
            backend.insert(key.clone(), Value::Int(idx as i64));
        }
        for (idx, key) in keys.iter().enumerate() {
            assert_eq!(backend.find(key), Some(&Value::Int(idx as i64)));
        }
        let removed = backend.remove_key(&keys[2]);
        assert_eq!(removed, Some((keys[2].clone(), Value::Int(2))));
        assert_eq!(backend.find(&keys[2]), None);
        assert_eq!(backend.find(&keys[3]), Some(&Value::Int(3)));

        let new_key = collide_on_mask(backend.slots.len() - 1, 8)
            .pop()
            .expect("new colliding key");
        backend.insert(new_key.clone(), Value::Int(42));
        assert_eq!(backend.find(&new_key), Some(&Value::Int(42)));
    }

    #[test]
    fn insertion_order_is_preserved_after_update_and_reinsert() {
        let mut backend = DictBackend::new(vec![
            (Value::Str("a".to_string()), Value::Int(1)),
            (Value::Str("b".to_string()), Value::Int(2)),
            (Value::Str("c".to_string()), Value::Int(3)),
        ]);
        backend.insert(Value::Str("b".to_string()), Value::Int(20));
        assert_eq!(
            backend.to_vec(),
            vec![
                (Value::Str("a".to_string()), Value::Int(1)),
                (Value::Str("b".to_string()), Value::Int(20)),
                (Value::Str("c".to_string()), Value::Int(3)),
            ]
        );
        assert_eq!(
            backend.remove_key(&Value::Str("b".to_string())),
            Some((Value::Str("b".to_string()), Value::Int(20)))
        );
        backend.insert(Value::Str("b".to_string()), Value::Int(200));
        assert_eq!(
            backend.to_vec(),
            vec![
                (Value::Str("a".to_string()), Value::Int(1)),
                (Value::Str("c".to_string()), Value::Int(3)),
                (Value::Str("b".to_string()), Value::Int(200)),
            ]
        );
    }

    #[test]
    fn tombstone_saturation_at_min_table_size_does_not_hang() {
        let mut backend = DictBackend::new(Vec::new());
        backend.resize_slots(MIN_TABLE_SIZE);
        let keys = collide_on_mask(backend.slots.len() - 1, backend.slots.len());
        for (idx, key) in keys.iter().enumerate() {
            backend.insert(key.clone(), Value::Int(idx as i64));
        }
        for key in keys.iter().skip(1) {
            let _ = backend.remove_key(key);
        }

        let missing = Value::Int(987654321);
        assert_eq!(backend.find(&missing), None);
        backend.insert(missing.clone(), Value::Int(99));
        assert_eq!(backend.find(&missing), Some(&Value::Int(99)));
    }

    #[test]
    fn string_key_lookup_remains_correct_after_dense_inserts() {
        let backend = DictBackend::new(vec![
            (
                Value::Str("i1".to_string()),
                Value::Str("2147483648".to_string()),
            ),
            (
                Value::Str("float".to_string()),
                Value::Str("43.0e12".to_string()),
            ),
            (Value::Str("i2".to_string()), Value::Str("17".to_string())),
            (Value::Str("s1".to_string()), Value::Str("abc".to_string())),
            (Value::Str("s2".to_string()), Value::Str("def".to_string())),
        ]);

        assert_eq!(
            backend.find(&Value::Str("s2".to_string())),
            Some(&Value::Str("def".to_string()))
        );
        assert!(backend.contains_key(&Value::Str("s2".to_string())));
    }

    #[test]
    fn remove_by_index_keeps_slot_backrefs_in_sync() {
        let mut backend = DictBackend::new(Vec::new());
        backend.resize_slots(64);
        let keys = collide_on_mask(backend.slots.len() - 1, 20);
        for (idx, key) in keys.iter().enumerate() {
            backend.insert(key.clone(), Value::Int(idx as i64));
        }

        let removed = backend.remove(7);
        assert_eq!(removed, (keys[7].clone(), Value::Int(7)));
        for (idx, key) in keys.iter().enumerate() {
            if idx == 7 {
                assert_eq!(backend.find(key), None);
            } else {
                assert_eq!(backend.find(key), Some(&Value::Int(idx as i64)));
            }
        }

        let removed_again = backend.remove(0);
        assert_eq!(removed_again, (keys[0].clone(), Value::Int(0)));
        assert_eq!(backend.find(&keys[0]), None);
        assert_eq!(backend.find(&keys[19]), Some(&Value::Int(19)));

        let replacement = collide_on_mask(backend.slots.len() - 1, 40)
            .pop()
            .expect("replacement key");
        backend.insert(replacement.clone(), Value::Int(999));
        assert_eq!(backend.find(&replacement), Some(&Value::Int(999)));
    }

    #[test]
    fn retain_rebuilds_slot_backrefs() {
        let mut backend = DictBackend::new(Vec::new());
        backend.resize_slots(32);
        let keys = collide_on_mask(backend.slots.len() - 1, 12);
        for (idx, key) in keys.iter().enumerate() {
            backend.insert(key.clone(), Value::Int(idx as i64));
        }

        backend.retain(|(_, value)| matches!(value, Value::Int(v) if v % 2 == 0));
        for (idx, key) in keys.iter().enumerate() {
            if idx % 2 == 0 {
                assert_eq!(backend.find(key), Some(&Value::Int(idx as i64)));
            } else {
                assert_eq!(backend.find(key), None);
            }
        }

        let extra = collide_on_mask(backend.slots.len() - 1, 24)
            .pop()
            .expect("extra key");
        backend.insert(extra.clone(), Value::Int(1234));
        assert_eq!(backend.find(&extra), Some(&Value::Int(1234)));
    }

    #[test]
    fn resize_preserves_insert_with_hash_entries() {
        let mut backend = DictBackend::new(Vec::new());
        // First resize threshold from 8 -> 16 is crossed on the sixth insert.
        for idx in 0..9 {
            let key = Value::Int(idx);
            let forced_hash = 10_000 + idx as u64;
            backend.insert_with_hash(key.clone(), Value::Int(idx), forced_hash);
            assert_eq!(
                backend.find_with_hash(&key, forced_hash),
                Some(&Value::Int(idx))
            );
        }
        for idx in 0..9 {
            let key = Value::Int(idx);
            let forced_hash = 10_000 + idx as u64;
            assert_eq!(
                backend.find_with_hash(&key, forced_hash),
                Some(&Value::Int(idx))
            );
        }
    }
}
