use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CapiPerfSnapshot {
    pub richcompare_calls: u64,
    pub richcompare_bool_calls: u64,
    pub richcompare_slot_attempts: u64,
    pub richcompare_dunder_fallback_attempts: u64,
    pub richcompare_dunder_attr_missing: u64,
    pub richcompare_dunder_callable_invocations: u64,
    pub richcompare_dunder_calls_owned: u64,
    pub richcompare_dunder_calls_external: u64,
    pub value_from_ptr_calls: u64,
    pub handle_from_ptr_calls: u64,
    pub handle_from_ptr_hits: u64,
    pub py_incref_calls: u64,
    pub py_incref_handle_hits: u64,
    pub py_decref_calls: u64,
    pub py_decref_handle_hits: u64,
}

static CAPIPERF_RICHCOMPARE_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_BOOL_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_SLOT_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_DUNDER_FALLBACK_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_DUNDER_ATTR_MISSING: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_DUNDER_CALLABLE_INVOCATIONS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_DUNDER_CALLS_OWNED: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_RICHCOMPARE_DUNDER_CALLS_EXTERNAL: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_VALUE_FROM_PTR_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_HANDLE_FROM_PTR_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_HANDLE_FROM_PTR_HITS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_PY_INCREF_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_PY_INCREF_HANDLE_HITS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_PY_DECREF_CALLS: AtomicU64 = AtomicU64::new(0);
static CAPIPERF_PY_DECREF_HANDLE_HITS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn capi_perf_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PYRS_CAPI_PERF").is_some())
}

#[inline]
fn capi_perf_inc(counter: &AtomicU64) {
    if capi_perf_enabled() {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_calls() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_bool_calls() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_BOOL_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_slot_attempts() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_SLOT_ATTEMPTS);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_dunder_fallback_attempts() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_DUNDER_FALLBACK_ATTEMPTS);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_dunder_attr_missing() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_DUNDER_ATTR_MISSING);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_dunder_callable_invocations() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_DUNDER_CALLABLE_INVOCATIONS);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_dunder_calls_owned() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_DUNDER_CALLS_OWNED);
}

#[inline]
pub(super) fn capi_perf_inc_richcompare_dunder_calls_external() {
    capi_perf_inc(&CAPIPERF_RICHCOMPARE_DUNDER_CALLS_EXTERNAL);
}

#[inline]
pub(super) fn capi_perf_inc_value_from_ptr_calls() {
    capi_perf_inc(&CAPIPERF_VALUE_FROM_PTR_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_handle_from_ptr_calls() {
    capi_perf_inc(&CAPIPERF_HANDLE_FROM_PTR_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_handle_from_ptr_hits() {
    capi_perf_inc(&CAPIPERF_HANDLE_FROM_PTR_HITS);
}

#[inline]
pub(super) fn capi_perf_inc_py_incref_calls() {
    capi_perf_inc(&CAPIPERF_PY_INCREF_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_py_incref_handle_hits() {
    capi_perf_inc(&CAPIPERF_PY_INCREF_HANDLE_HITS);
}

#[inline]
pub(super) fn capi_perf_inc_py_decref_calls() {
    capi_perf_inc(&CAPIPERF_PY_DECREF_CALLS);
}

#[inline]
pub(super) fn capi_perf_inc_py_decref_handle_hits() {
    capi_perf_inc(&CAPIPERF_PY_DECREF_HANDLE_HITS);
}

pub(crate) fn capi_perf_snapshot() -> Option<CapiPerfSnapshot> {
    if !capi_perf_enabled() {
        return None;
    }
    Some(CapiPerfSnapshot {
        richcompare_calls: CAPIPERF_RICHCOMPARE_CALLS.load(Ordering::Relaxed),
        richcompare_bool_calls: CAPIPERF_RICHCOMPARE_BOOL_CALLS.load(Ordering::Relaxed),
        richcompare_slot_attempts: CAPIPERF_RICHCOMPARE_SLOT_ATTEMPTS.load(Ordering::Relaxed),
        richcompare_dunder_fallback_attempts: CAPIPERF_RICHCOMPARE_DUNDER_FALLBACK_ATTEMPTS
            .load(Ordering::Relaxed),
        richcompare_dunder_attr_missing: CAPIPERF_RICHCOMPARE_DUNDER_ATTR_MISSING
            .load(Ordering::Relaxed),
        richcompare_dunder_callable_invocations: CAPIPERF_RICHCOMPARE_DUNDER_CALLABLE_INVOCATIONS
            .load(Ordering::Relaxed),
        richcompare_dunder_calls_owned: CAPIPERF_RICHCOMPARE_DUNDER_CALLS_OWNED
            .load(Ordering::Relaxed),
        richcompare_dunder_calls_external: CAPIPERF_RICHCOMPARE_DUNDER_CALLS_EXTERNAL
            .load(Ordering::Relaxed),
        value_from_ptr_calls: CAPIPERF_VALUE_FROM_PTR_CALLS.load(Ordering::Relaxed),
        handle_from_ptr_calls: CAPIPERF_HANDLE_FROM_PTR_CALLS.load(Ordering::Relaxed),
        handle_from_ptr_hits: CAPIPERF_HANDLE_FROM_PTR_HITS.load(Ordering::Relaxed),
        py_incref_calls: CAPIPERF_PY_INCREF_CALLS.load(Ordering::Relaxed),
        py_incref_handle_hits: CAPIPERF_PY_INCREF_HANDLE_HITS.load(Ordering::Relaxed),
        py_decref_calls: CAPIPERF_PY_DECREF_CALLS.load(Ordering::Relaxed),
        py_decref_handle_hits: CAPIPERF_PY_DECREF_HANDLE_HITS.load(Ordering::Relaxed),
    })
}
