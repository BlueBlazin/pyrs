# WASM Module Support Policy (codex/wasm)

Status: branch-local draft for browser preflight behavior.

This document defines the canonical module-level blocker mapping used by:

- `src/wasm/mod.rs` (`WASM_MODULE_BLOCKER_POLICY`)
- `wasm_module_support(module_name)`
- `wasm_module_policy_entries()`

The policy is intentionally conservative and capability-driven.

## Policy Table

| Module | Blocker key | Why blocked in browser mode |
| --- | --- | --- |
| `_ctypes` | `dynamic_library_load` | Depends on dynamic library loading not available in wasm host. |
| `ctypes` | `dynamic_library_load` | High-level wrapper over `_ctypes`; same blocker applies. |
| `numpy` | `dynamic_library_load` | Standard distributions rely on native extension binaries. |
| `scipy` | `dynamic_library_load` | Standard distributions rely on native extension binaries. |
| `_socket` | `network_sockets` | Requires raw socket capability unavailable in wasm host. |
| `socket` | `network_sockets` | High-level wrapper over `_socket`; same blocker applies. |
| `_posixsubprocess` | `process_spawn` | Uses subprocess spawning primitives unavailable in wasm host. |
| `subprocess` | `process_spawn` | Depends on process spawning primitives. |
| `multiprocessing` | `process_spawn` | Requires process creation/spawn model unavailable in wasm host. |
| `readline` | `interactive_terminal` | Requires TTY/interactive terminal behavior unavailable in browser mode. |

## Design Notes

1. This is a preflight policy, not an exhaustive import-compatibility oracle.
2. Unknown modules default to "no known blocker" in the preflight API.
3. Blocking keys must match `HostCapability` keys in `src/host/mod.rs`.
4. If `WasmHost::supports(...)` changes, this policy must be revalidated.

## Contract Guardrails

- `tests/wasm_contract.rs` enforces:
  - blocker-key parity with wasm capability matrix,
  - stable mappings for representative module families,
  - stable shape for `wasm_module_policy_entries()`.
