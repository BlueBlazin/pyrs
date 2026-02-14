# Extension Capability Matrix (Milestone 15)

Status: active (source-of-truth for extension surface claims).

Scope: native-extension runtime path needed for NumPy/SciPy/Pandas/Matplotlib.

Legend:
- `DONE`: implemented and test-covered.
- `IN PROGRESS`: partial implementation in-tree.
- `PLANNED`: not implemented yet.
- `BLOCKED`: known blocker outside current substrate.

## Runtime and Loader Substrate

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| `.pyrs-ext` extension manifest discovery on import path | DONE | VM/import | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Scaffolding format for milestone bring-up; native `.so/.dylib/.pyd` loading remains planned. |
| Extension module loader dispatch (`pyrs.ExtensionFileLoader`) | DONE | VM/import | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Module metadata includes extension ABI + entrypoint markers. |
| Minimal extension entrypoint registry (`hello_ext`) | DONE | VM/extensions | `tests/extension_smoke.rs::imports_manifest_backed_hello_extension` | Smoke substrate only; not a user-facing compatibility claim. |
| Dynamic shared-library loader (`.so/.dylib/.pyd`) | PLANNED | VM/extensions | - | No loader crate added yet; explicit follow-up for real C-extension ingestion. |
| PEP 489 multi-phase init | PLANNED | VM/extensions | - | Required for production extension parity. |
| Extension module state lifecycle hooks | PLANNED | VM/extensions | - | Needs finalize/teardown semantics. |

## C Runtime Surface (`libpyrs-capi`)

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Exported C ABI artifact (`libpyrs-capi`) | PLANNED | runtime/ffi | - | Not started. |
| Header surface + versioned symbol manifest | PLANNED | runtime/ffi | - | Required before external extension builds. |
| `PyObject`/refcount ownership APIs | PLANNED | runtime/ffi | - | Must follow CPython semantics. |
| Exception indicator/thread-local error state APIs | PLANNED | runtime/ffi | - | Required for extension correctness. |
| GIL attach/detach APIs (`PyGILState_*`) | PLANNED | runtime/threading | - | Required for threaded native callers. |

## Interop Protocols

| Surface | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Buffer protocol producer/consumer parity | PLANNED | runtime/object-model | - | High-priority dependency for NumPy. |
| Capsule/callback interop primitives | PLANNED | runtime/ffi | - | Needed by ecosystem modules that pass opaque handles. |
| ABI capability introspection API | PLANNED | runtime/ffi | - | Needed for precise unsupported-surface diagnostics. |

## Ecosystem Gates

| Gate | Status | Owner | Evidence | Notes |
|---|---|---|---|---|
| Extension smoke gate (`hello_ext`) | DONE | VM/extensions | `tests/extension_smoke.rs` + CI `Extension smoke lane` | Baseline CI guard for extension import path. |
| NumPy import gate (`import numpy`) | IN PROGRESS | milestone-15 bring-up | `scripts/probe_numpy_gate.py` + `docs/NUMPY_BRINGUP_GATE.md` | Probe scaffold is landed; gate currently expected-red until C-extension substrate matures. |
| NumPy ndarray smoke (`np.array([...]).sum()`) | IN PROGRESS | milestone-15 bring-up | `scripts/probe_numpy_gate.py` + `docs/NUMPY_BRINGUP_GATE.md` | Same as above. |
| Pandas/matplotlib/scipy smoke gates | PLANNED | milestone-15 bring-up | - | Starts after NumPy substrate closure. |

## Policy

1. A surface cannot be marked `DONE` without deterministic test evidence.
2. Temporary scaffolding must be tracked in `docs/STUB_ACCOUNTING.md` with closure criteria.
3. This matrix must be updated in the same checkpoint as any extension-surface behavior change.
