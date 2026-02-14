# Extension Ecosystem Design (NumPy, SciPy, Pandas, Matplotlib)

Status: draft (active design baseline).

Owner: runtime/VM + packaging workstreams.

Target runtime: `pyrs` CPython 3.14 compatibility target.

## Purpose

Define an engineering-first plan to support the real extension-backed Python ecosystem, starting with:

- `numpy`
- `scipy`
- `pandas`
- `matplotlib`

This design is the source of truth for extension architecture and delivery gates.

Companion execution trackers:
- `docs/EXTENSION_CAPABILITY_MATRIX.md`
- `docs/EXTENSION_PACKAGING_CONTRACT.md`
- `docs/EXTENSION_CAPI_V1.md`
- `docs/NUMPY_BRINGUP_GATE.md`

## Scope and Compatibility Contract

### Primary Goal

Run the four target libraries with production-grade behavior and diagnostics on supported platforms, not just import-only demos.

### Non-Goals (Initial Delivery)

- Full CPython C-API coverage in one step.
- Arbitrary binary-wheel compatibility without explicit verification.
- GPU, GUI backend, and platform-specific optional acceleration closure in first pass.

### Explicit Compatibility Modes

The project will support two modes, in order:

1. `pyrs` extension compatibility mode (`pyrs314`): build extensions against `pyrs` headers/lib.
2. CPython ABI wheel-compat mode (`cp314` wheel reuse): only after measurable ABI conformance gates are green.

Rationale:

- This prevents unsafe over-claiming of binary compatibility.
- It enables real progress early while preserving a path to drop-in ecosystem adoption.

## Why These Libraries Require Native Extension Support

For these libraries, pure-Python compatibility is not enough:

- `numpy`: core ndarray/ufunc implementation is native extension code.
- `scipy`: large native stack (C/C++/Fortran) on top of NumPy runtime interfaces.
- `pandas`: Cython/native extensions and NumPy C-layer dependency.
- `matplotlib`: native extension components and backend/native image/font integrations.

Therefore, Milestone 15 must become a concrete extension ecosystem program, not a narrow placeholder.

## Architecture

## 1) Native Extension Loader Subsystem

Deliver a dedicated extension loader integrated with importlib semantics:

- discover/load shared objects (`.so`/`.dylib`/`.pyd`)
- support PEP 489 multi-phase initialization
- correctly bind module state and lifecycle
- deterministic, actionable import errors for unsupported symbols/features

Implementation boundary:

- new VM/stdlib import-layer module (separate from monolithic fallback paths)
- strict ownership in `docs/VM_ARCHITECTURE_MAP.md`

## 2) C Runtime Surface (`libpyrs-capi`)

Provide a C-facing runtime interface with explicit symbol/version control:

- `libpyrs-capi` shared/static artifact
- exported C API table and symbol manifest
- header set for extension compilation
- runtime capability query API for diagnostics

Design rule:

- no silent partial implementations; unsupported surface must fail with explicit error codes/messages

## 3) Object Model and Memory Interop Contract

Extension compatibility requires C-visible object semantics consistent with CPython expectations:

- `PyObject`/`PyVarObject` header compatibility where required
- reference counting semantics (`INCREF`/`DECREF`) with deterministic ownership rules
- exception indicator behavior and thread-local error state
- callable protocol and type slot behavior (`tp_call`, descriptors, method wrappers)

Safety approach:

- keep FFI boundary narrow and audited
- use explicit conversion APIs between C-visible handles and internal Rust values
- forbid ad-hoc pointer reinterpretation paths

## 4) GIL and Thread-State Interop

Support required extension thread APIs:

- `PyGILState_Ensure` / `PyGILState_Release`
- thread state attach/detach semantics
- safe `allow_threads` sections for native compute

Requirement:

- behavior must be testable under concurrent extension calls and callback-to-Python transitions

## 5) Buffer Protocol and Memory Views

High priority for NumPy and ecosystem interop:

- full producer/consumer buffer protocol behavior
- shape/strides/format/lifetime correctness
- safe export tracking and invalidation behavior

## 6) Packaging and Build Toolchain Integration

Need complete build/install flow, not only runtime loader:

- `sysconfig`/distutils-compatible build variables
- include/library paths for extension compilation
- wheel tagging policy (`pyrs314` first, `cp314` only after ABI gates)
- install/build workflows via modern PEP 517 backends

## Engineering Practices (Non-Negotiable)

## 1) Capability Matrix Before Claims

Maintain a symbol-level capability matrix for extension APIs:

- implemented
- unimplemented
- partially implemented
- intentionally unsupported

Every matrix row must include:

- owning module/file
- test coverage reference
- parity evidence link

## 2) No Shim-Only Compatibility Claims

No compatibility is considered shipped via ad-hoc Python-layer shims for native behavior.

Allowed temporary work must be:

- explicitly documented in `docs/STUB_ACCOUNTING.md` or `docs/ALGO_AUDIT_BACKLOG.md`
- bounded with owner + closure criteria

## 3) CPython-Referenced Implementation Discipline

For each C-API surface, implementation must cite CPython references:

- `Include/*.h`
- `Objects/*.c`
- `Python/*.c`
- `Modules/*.c`

No behavior invented without parity rationale.

## 4) Incremental, Gate-Driven Delivery

Each phase must be merged with:

- deterministic tests
- performance and leak checks for touched paths
- docs updates in same commit

## Delivery Plan

## Phase A: Foundational Extension Substrate

Exit criteria:

- extension loader operational for a minimal native test module
- C error/GIL/thread-state APIs working for baseline extension calls
- capability matrix file introduced and enforced

## Phase B: NumPy Bring-Up (First Real Ecosystem Target)

Exit criteria:

- `import numpy` green on supported platforms
- core ndarray/ufunc smoke suite green
- buffer protocol stress cases green

## Phase C: Pandas and Matplotlib Closure

Exit criteria:

- `import pandas` and core dataframe operation smokes green
- `import matplotlib` with non-GUI backend (`Agg`) render/save smoke green

## Phase D: SciPy Closure

Exit criteria:

- `import scipy` green
- selected core scientific smoke suites green (`linalg`, `fft`, selected stats paths)

## Phase E: Binary-Wheel Compatibility Expansion (Optional but Desired)

Exit criteria:

- CPython ABI conformance gate suite for claimed symbols green
- explicit wheel-compat matrix published
- `cp314` wheel reuse enabled only for verified subset

## Library-Specific Acceptance Gates

Each target library needs both import and functional gates.

## NumPy

- import gate: `import numpy as np`
- functional gates: ndarray creation, slicing/strides, dtype conversion, ufunc math
- reliability gates: refcount/leak checks under repeated operations

## Pandas

- import gate: `import pandas as pd`
- functional gates: DataFrame create/select/groupby/merge basics
- reliability gates: extension callback error propagation and stable exceptions

## Matplotlib

- import gate: `import matplotlib; matplotlib.use('Agg')`
- functional gates: plot -> save PNG
- reliability gates: backend loading diagnostics and deterministic failures when backend is unavailable

## SciPy

- import gate: `import scipy`
- functional gates: representative `linalg` and `fft` workloads
- reliability gates: thread/GIL transitions around native compute paths

## CI and Qualification Matrix

Required platform matrix for ecosystem gates:

- Linux `x86_64-unknown-linux-gnu`
- macOS `aarch64-apple-darwin`
- macOS `x86_64-apple-darwin`

Windows can be staged after initial closure unless moved to required set in release criteria.

Required lanes:

- extension smoke lane (minimal native test module)
- per-library import + functional smoke lane
- leak/stability lane for repeated extension lifecycle operations
- packaging lane (build from source using `pyrs` extension mode)

## Dependency Policy for This Program

This effort may require carefully scoped third-party crates for:

- dynamic loading
- platform ABI/tooling integration
- low-level FFI safety helpers

Policy:

- no new crate without explicit vet/approval
- each crate must have clear replacement/removal rationale if temporary
- security/license review required before adoption

## Risks and Mitigations

## Risk: Over-claiming binary compatibility

Mitigation:

- default to `pyrs314` mode until ABI evidence justifies `cp314` claims

## Risk: Memory safety regressions across FFI boundary

Mitigation:

- narrow audited unsafe regions
- mandatory leak/stress tests before closure

## Risk: Large hidden surface area in C-API

Mitigation:

- symbol-level matrix with strict ownership and closure gates
- prioritize by real target-library call paths, not theoretical completeness

## Risk: Build/packaging drift across platforms

Mitigation:

- CI packaging lanes on required target matrix
- lock and version build configuration outputs

## Definition of Done for "Support NumPy/SciPy/Pandas/Matplotlib"

All conditions must hold:

- import + functional smoke gates green for all four libraries on required platforms
- no open P0 blockers in extension capability matrix for exercised paths
- clear unsupported-surface diagnostics for out-of-scope features
- reproducible CI evidence and docs updated in same checkpoint
