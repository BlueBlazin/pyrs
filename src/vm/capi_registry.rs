use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum CapiPtrProvenance {
    OwnedCompat,
    ExternalRef,
    StaticSingleton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapiPtrLifecycleState {
    Alive,
    PendingFree,
    Freed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapiRefKind {
    Borrowed,
    Owned,
    Stolen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BorrowedRef {
    ptr: usize,
}

impl BorrowedRef {
    pub(crate) fn from_ptr(ptr: *mut std::ffi::c_void) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr: ptr as usize })
    }

    pub(crate) fn ptr(self) -> usize {
        self.ptr
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnedRef {
    ptr: usize,
}

impl OwnedRef {
    pub(crate) fn from_ptr(ptr: *mut std::ffi::c_void) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr: ptr as usize })
    }

    pub(crate) fn ptr(self) -> usize {
        self.ptr
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StolenRef {
    ptr: usize,
}

impl StolenRef {
    pub(crate) fn from_ptr(ptr: *mut std::ffi::c_void) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr: ptr as usize })
    }

    pub(crate) fn ptr(self) -> usize {
        self.ptr
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct CapiPtrEntry {
    pub(crate) ptr: usize,
    pub(crate) generation: u64,
    pub(crate) provenance: CapiPtrProvenance,
    pub(crate) lifecycle: CapiPtrLifecycleState,
    pub(crate) object_id: Option<u64>,
    pub(crate) borrowed_refs: usize,
    pub(crate) owned_refs: usize,
    pub(crate) stolen_refs: usize,
    pub(crate) external_pins: usize,
}

impl CapiPtrEntry {
    fn new(
        ptr: usize,
        generation: u64,
        provenance: CapiPtrProvenance,
        object_id: Option<u64>,
    ) -> Self {
        Self {
            ptr,
            generation,
            provenance,
            lifecycle: CapiPtrLifecycleState::Alive,
            object_id,
            borrowed_refs: 0,
            owned_refs: 0,
            stolen_refs: 0,
            external_pins: 0,
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CapiRegistryStats {
    pub(crate) entries_total: usize,
    pub(crate) entries_alive: usize,
    pub(crate) entries_pending_free: usize,
    pub(crate) entries_freed: usize,
    pub(crate) entries_pinned: usize,
}

#[derive(Debug, Default)]
pub(crate) struct CapiObjectRegistry {
    entries: HashMap<usize, CapiPtrEntry>,
    object_ptr_by_id: HashMap<u64, usize>,
    next_generation: u64,
}

impl CapiObjectRegistry {
    pub(crate) fn register_ptr(
        &mut self,
        ptr: usize,
        provenance: CapiPtrProvenance,
        object_id: Option<u64>,
    ) {
        if ptr == 0 {
            return;
        }
        if let Some(existing) = self.entries.get_mut(&ptr) {
            let was_freed = existing.lifecycle == CapiPtrLifecycleState::Freed;
            existing.provenance = provenance;
            existing.lifecycle = CapiPtrLifecycleState::Alive;
            if was_freed {
                existing.borrowed_refs = 0;
                existing.owned_refs = 0;
                existing.stolen_refs = 0;
                existing.external_pins = 0;
            }
            if object_id.is_some() {
                existing.object_id = object_id;
            }
            if let Some(id) = existing.object_id {
                self.object_ptr_by_id.insert(id, ptr);
            }
            return;
        }
        self.next_generation = self.next_generation.saturating_add(1).max(1);
        let entry = CapiPtrEntry::new(ptr, self.next_generation, provenance, object_id);
        if let Some(id) = object_id {
            self.object_ptr_by_id.insert(id, ptr);
        }
        self.entries.insert(ptr, entry);
    }

    pub(crate) fn record_ref_kind(&mut self, ptr: usize, ref_kind: CapiRefKind) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        match ref_kind {
            CapiRefKind::Borrowed => {
                entry.borrowed_refs = entry.borrowed_refs.saturating_add(1);
            }
            CapiRefKind::Owned => {
                entry.owned_refs = entry.owned_refs.saturating_add(1);
            }
            CapiRefKind::Stolen => {
                entry.stolen_refs = entry.stolen_refs.saturating_add(1);
            }
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pin_external(&mut self, ptr: usize) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        if entry.provenance != CapiPtrProvenance::ExternalRef {
            return;
        }
        entry.external_pins = entry.external_pins.saturating_add(1);
        entry.lifecycle = CapiPtrLifecycleState::Alive;
    }

    pub(crate) fn pin_external_once(&mut self, ptr: usize) -> bool {
        if ptr == 0 {
            return false;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return false;
        };
        if entry.provenance != CapiPtrProvenance::ExternalRef {
            return false;
        }
        if entry.external_pins > 0 {
            return false;
        }
        entry.external_pins = 1;
        entry.lifecycle = CapiPtrLifecycleState::Alive;
        true
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn unpin_external(&mut self, ptr: usize) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        if entry.provenance != CapiPtrProvenance::ExternalRef {
            return;
        }
        if entry.external_pins > 0 {
            entry.external_pins -= 1;
        }
    }

    pub(crate) fn mark_pending_free(&mut self, ptr: usize) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        if entry.lifecycle != CapiPtrLifecycleState::Freed {
            entry.lifecycle = CapiPtrLifecycleState::PendingFree;
        }
    }

    pub(crate) fn mark_alive(&mut self, ptr: usize) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        entry.lifecycle = CapiPtrLifecycleState::Alive;
    }

    pub(crate) fn mark_freed(&mut self, ptr: usize) {
        if ptr == 0 {
            return;
        }
        let Some(entry) = self.entries.get_mut(&ptr) else {
            return;
        };
        entry.lifecycle = CapiPtrLifecycleState::Freed;
        entry.external_pins = 0;
        if let Some(id) = entry.object_id
            && self.object_ptr_by_id.get(&id).copied() == Some(ptr)
        {
            self.object_ptr_by_id.remove(&id);
        }
    }

    pub(crate) fn is_freed(&self, ptr: usize) -> bool {
        self.entries
            .get(&ptr)
            .is_some_and(|entry| entry.lifecycle == CapiPtrLifecycleState::Freed)
    }

    pub(crate) fn should_free_now(&self, ptr: usize) -> bool {
        let Some(entry) = self.entries.get(&ptr) else {
            return false;
        };
        if entry.lifecycle == CapiPtrLifecycleState::Freed {
            return false;
        }
        match entry.provenance {
            CapiPtrProvenance::OwnedCompat => entry.external_pins == 0,
            CapiPtrProvenance::ExternalRef => false,
            CapiPtrProvenance::StaticSingleton => false,
        }
    }

    pub(crate) fn drain_external_pins(&mut self) -> Vec<(usize, usize)> {
        let mut drained = Vec::new();
        for (ptr, entry) in self.entries.iter_mut() {
            if entry.provenance != CapiPtrProvenance::ExternalRef {
                continue;
            }
            if entry.external_pins == 0 {
                continue;
            }
            drained.push((*ptr, entry.external_pins));
            entry.external_pins = 0;
            if entry.lifecycle != CapiPtrLifecycleState::Freed {
                entry.lifecycle = CapiPtrLifecycleState::PendingFree;
            }
        }
        drained
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pointer_for_object_id(&self, object_id: u64) -> Option<usize> {
        self.object_ptr_by_id.get(&object_id).copied()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn stats(&self) -> CapiRegistryStats {
        let mut stats = CapiRegistryStats {
            entries_total: self.entries.len(),
            ..CapiRegistryStats::default()
        };
        for entry in self.entries.values() {
            match entry.lifecycle {
                CapiPtrLifecycleState::Alive => stats.entries_alive += 1,
                CapiPtrLifecycleState::PendingFree => stats.entries_pending_free += 1,
                CapiPtrLifecycleState::Freed => stats.entries_freed += 1,
            }
            if entry.external_pins > 0 {
                stats.entries_pinned += 1;
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::{CapiObjectRegistry, CapiPtrLifecycleState, CapiPtrProvenance, CapiRefKind};

    #[test]
    fn registry_tracks_lifecycle_and_pins() {
        let mut registry = CapiObjectRegistry::default();
        registry.register_ptr(0x10, CapiPtrProvenance::ExternalRef, Some(42));
        registry.record_ref_kind(0x10, CapiRefKind::Owned);
        registry.pin_external(0x10);
        assert!(!registry.should_free_now(0x10));
        registry.mark_pending_free(0x10);
        assert!(!registry.should_free_now(0x10));
        registry.unpin_external(0x10);
        assert!(!registry.should_free_now(0x10));
        registry.mark_freed(0x10);
        let stats = registry.stats();
        assert_eq!(stats.entries_total, 1);
        assert_eq!(stats.entries_freed, 1);
        assert_eq!(registry.pointer_for_object_id(42), None);
    }

    #[test]
    fn registry_reuses_ptr_entry_without_unbounded_growth() {
        let mut registry = CapiObjectRegistry::default();
        for _ in 0..1000 {
            registry.register_ptr(0x22, CapiPtrProvenance::OwnedCompat, Some(1));
            registry.mark_pending_free(0x22);
            if registry.should_free_now(0x22) {
                registry.mark_freed(0x22);
            }
            registry.register_ptr(0x22, CapiPtrProvenance::OwnedCompat, Some(1));
        }
        let stats = registry.stats();
        assert_eq!(stats.entries_total, 1);
        assert_eq!(stats.entries_alive, 1);
    }

    #[test]
    fn external_refs_never_free_from_context_flow() {
        let mut registry = CapiObjectRegistry::default();
        registry.register_ptr(0x44, CapiPtrProvenance::ExternalRef, None);
        registry.record_ref_kind(0x44, CapiRefKind::Borrowed);
        registry.mark_pending_free(0x44);
        assert!(!registry.should_free_now(0x44));
        assert_eq!(
            registry
                .stats()
                .entries_pending_free
                .checked_sub(0)
                .unwrap_or_default(),
            1
        );
        registry.mark_freed(0x44);
        let stats = registry.stats();
        assert_eq!(stats.entries_freed, 1);
        assert!(stats.entries_total >= 1);
        assert_eq!(CapiPtrLifecycleState::Freed, CapiPtrLifecycleState::Freed);
    }

    #[test]
    fn external_pin_once_and_drain_behavior_is_stable() {
        let mut registry = CapiObjectRegistry::default();
        registry.register_ptr(0x99, CapiPtrProvenance::ExternalRef, None);
        assert!(registry.pin_external_once(0x99));
        assert!(!registry.pin_external_once(0x99));
        assert!(!registry.should_free_now(0x99));
        let drained = registry.drain_external_pins();
        assert_eq!(drained, vec![(0x99, 1)]);
        assert!(!registry.should_free_now(0x99));
        assert!(!registry.is_freed(0x99));
        registry.mark_freed(0x99);
        assert!(registry.is_freed(0x99));
    }

    #[test]
    fn external_pin_apis_ignore_non_external_entries() {
        let mut registry = CapiObjectRegistry::default();
        registry.register_ptr(0x77, CapiPtrProvenance::OwnedCompat, None);
        assert!(!registry.pin_external_once(0x77));
        registry.pin_external(0x77);
        assert!(registry.should_free_now(0x77));
        registry.unpin_external(0x77);
        assert!(registry.should_free_now(0x77));
    }

    #[test]
    fn re_registering_freed_entry_resets_pin_state() {
        let mut registry = CapiObjectRegistry::default();
        registry.register_ptr(0x55, CapiPtrProvenance::ExternalRef, None);
        assert!(registry.pin_external_once(0x55));
        registry.mark_freed(0x55);
        assert!(registry.is_freed(0x55));
        registry.register_ptr(0x55, CapiPtrProvenance::OwnedCompat, None);
        assert!(registry.should_free_now(0x55));
    }
}
