# WASM Capability Matrix (codex/wasm)

Status: branch-local draft for the isolated wasm track.

This matrix documents browser-mode capability contracts for the `codex/wasm`
branch. It is intentionally strict: unsupported behavior must fail explicitly.

## Capability Table

| Capability | Native Host | Wasm Host (current) | Contract |
| --- | --- | --- | --- |
| `filesystem_read` | supported | unsupported | No implicit host FS access in browser mode. |
| `filesystem_write` | supported | unsupported | No host FS mutation from browser mode. |
| `environment_read` | supported | unsupported | Browser mode does not read process env vars. |
| `process_args` | supported | supported (stubbed) | Browser mode currently reports a synthetic argv baseline. |
| `process_spawn` | supported | unsupported | No subprocess execution in browser mode. |
| `dynamic_library_load` | supported | unsupported | Native extension loading is disabled in browser mode. |
| `interactive_terminal` | supported | unsupported | Browser mode does not expose a terminal/TTY primitive. |
| `network_sockets` | supported | unsupported | Raw socket APIs are unavailable in browser mode. |

## Source of Truth

- Runtime enum: `src/host/mod.rs` (`HostCapability`)
- Host capability mapping:
  - `NativeHost::supports(...)`
  - `WasmHost::supports(...)`
- Browser bridge export: `src/wasm/mod.rs` (`wasm_capabilities()`)

## Error-Surface Policy

When a wasm-mode operation requires an unsupported capability:

1. Fail deterministically.
2. Return a structured error with a stable phase/message.
3. Do not silently degrade into native-specific assumptions.

## Change Control

Any capability change in `WasmHost::supports(...)` must update:

1. this matrix,
2. wasm API contract docs (`docs/WASM_EXECUTION_PLAN.md`),
3. wasm bridge behavior where applicable (`src/wasm/mod.rs`).
