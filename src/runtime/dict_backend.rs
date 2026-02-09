use super::{value_key_equal, value_lookup_hash, Value};

const PERTURB_SHIFT: usize = 5;
const MIN_TABLE_SIZE: usize = 8;
const LOAD_NUMERATOR: usize = 2;
const LOAD_DENOMINATOR: usize = 3;

#[derive(Debug, Clone, Copy)]
enum DictSlot {
    Empty,
    Dummy,
    Occupied { hash: u64, entry: usize },
}

#[derive(Debug, Clone)]
pub(super) struct DictBackend {
    entries: Vec<(Value, Value)>,
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

    pub(super) fn contains_key(&self, key: &Value) -> bool {
        self.find_index(key).is_some()
    }

    pub(super) fn contains_key_with_hash(&self, key: &Value, hash: u64) -> bool {
        self.find_index_with_hash(key, hash).is_some()
    }

    pub(super) fn insert(&mut self, key: Value, value: Value) {
        if let Some(index) = self.find_index(&key) {
            self.entries[index].1 = value;
            return;
        }
        let Some(hash) = value_lookup_hash(&key) else {
            self.entries.push((key, value));
            self.used = self.entries.len();
            return;
        };

        self.ensure_insert_capacity();
        let slot = match self.lookup_slot(&key, hash) {
            SlotLookup::Found { entry, .. } => {
                self.entries[entry].1 = value;
                return;
            }
            SlotLookup::Vacant(slot) => slot,
        };

        let entry = self.entries.len();
        self.entries.push((key, value));
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

    pub(super) fn remove_key_with_hash(&mut self, key: &Value, hash: u64) -> Option<(Value, Value)> {
        let index = self.find_index_with_hash(key, hash)?;
        Some(self.remove(index))
    }

    pub(super) fn remove(&mut self, index: usize) -> (Value, Value) {
        if value_lookup_hash(&self.entries[index].0).is_some() {
            self.remove_slot_for_entry(index);
        }
        let (removed_key, removed_value) = self.entries.remove(index);
        if value_lookup_hash(&removed_key).is_some() {
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
        self.entries.retain(|entry| f(entry));
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

    fn ensure_insert_capacity(&mut self) {
        if self.slots.is_empty() {
            self.resize_slots(MIN_TABLE_SIZE);
            return;
        }
        let usable_slots = (self.slots.len() * LOAD_NUMERATOR) / LOAD_DENOMINATOR;
        if self.filled + 1 > usable_slots {
            self.resize_slots(self.slots.len() * 2);
        }
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
        self.filled = 0;
        self.used = self.entries.len();
        for (entry, (key, _)) in self.entries.iter().enumerate() {
            if let Some(hash) = value_lookup_hash(key) {
                let slot = self.lookup_vacant_slot(hash);
                self.slots[slot] = DictSlot::Occupied { hash, entry };
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

    fn lookup_vacant_slot(&self, hash: u64) -> usize {
        debug_assert!(!self.slots.is_empty());
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        let mut perturb = hash as usize;
        let mut first_dummy = None;
        for _ in 0..self.slots.len() {
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

    fn lookup_slot(&self, key: &Value, hash: u64) -> SlotLookup {
        debug_assert!(!self.slots.is_empty());
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        let mut perturb = hash as usize;
        let mut first_dummy = None;

        for _ in 0..self.slots.len() {
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

    fn remove_slot_for_entry(&mut self, expected_entry: usize) {
        for slot in &mut self.slots {
            if let DictSlot::Occupied {
                entry,
                ..
            } = slot
            {
                if *entry == expected_entry {
                    *slot = DictSlot::Dummy;
                    return;
                }
            }
        }
    }

    fn adjust_slot_indices_after_remove(&mut self, removed_index: usize) {
        for slot in &mut self.slots {
            if let DictSlot::Occupied { entry, .. } = slot {
                if *entry > removed_index {
                    *entry -= 1;
                }
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
    use super::*;

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
}
