# Website Agent Guide

This file is the implementation-aware guide for working in `website/`.

## Scope

- Public marketing site + docs + browser playground for PYRS.
- Framework stack:
  - Astro 5 (`.astro`, `.mdx`)
  - Tailwind CSS v4
  - Small client-side scripts where needed (playground, copy buttons, docs ToC behavior).
- Primary source roots:
  - `website/src` (app code)
  - `website/public` (static assets + worker + wasm bundle output)
  - `website/scripts` (site-specific checks/reporting)

## Key Commands

From repo root:

```bash
pnpm --dir website install
pnpm --dir website dev
pnpm --dir website build
pnpm --dir website check:links
pnpm --dir website build:check
pnpm --dir website report:size
pnpm --dir website dev:playground
pnpm --dir website build:playground
```

Notes:

- `dev:playground` and `build:playground` first run `wasm:build` (`../scripts/build_wasm_website_bundle.sh`).
- `build:check` is the standard release-quality gate for website edits.

## Folder Structure and Ownership

- `src/layouts/`
  - `SiteLayout.astro`: shared shell for home/playground pages.
  - `DocsLayout.astro`: docs shell with left sidebar + right ToC + pager.
- `src/components/`
  - Global chrome: `TopNav.astro`, `SiteHead.astro`, `Footer.astro`.
  - Home sections: `components/home/*`.
  - Docs primitives: `components/docs/*`.
  - Shared UI atoms: `components/ui/*`.
- `src/config/`
  - `docsNav.ts`: source of truth for docs sidebar and pager order.
  - `installCommands.ts`, `cliReference.ts`, `homeHighlights.ts`: content/config registries.
- `src/pages/`
  - `index.astro`: landing page.
  - `playground.astro`: browser REPL page (inline client logic).
  - `docs/**/*.mdx`: docs content pages.
  - `debug.astro`: debug-oriented runtime surface (not primary product path).
- `public/`
  - Static media + scripts.
  - `public/workers/playground-runtime-worker.js`: runtime worker bridge contract.
  - `public/wasm/*`: generated wasm bundle assets consumed by playground.
- `scripts/`
  - Site-local validation/reporting scripts (links, size report).

## Code Organization Rules

- Prefer existing components over new one-off markup.
- Keep page-level content in `src/pages`; keep shared logic/style in components/config.
- For docs pages:
  - Use docs primitives (`DocSection`, `CommandBlock`, `CodeBlock`, `DataTable`, `Callout`, etc.).
  - Keep docs navigation updates in sync by editing `src/config/docsNav.ts`.
- Keep nav behavior consistent via `TopNav` and layout props (`navContext`).
- Do not hardcode route lists in multiple places; use config files as source of truth.

## Playground REPL and Worker API (Important)

The website playground uses a dedicated module worker:

- Worker file: `public/workers/playground-runtime-worker.js`
- Page file: `src/pages/playground.astro`

### Message envelope

- Request: `{ requestId, action, ...payload }`
- Response: `{ requestId, ok, ...payload }`

### Supported actions

1. `load`
   - Request: `{ wasmEntrypoint }`
   - Success:
     - `runtimeInfo` (normalized from `wasm_runtime_info()`)
     - `prompt_continuation` (current prompt mode from runtime session)
2. `execute`
   - Request: `{ source }`
   - Success:
     - `result` (normalized execution result shape)
     - `prompt_continuation` (prompt mode after input is applied)
3. `reset`
   - Request: none
   - Success:
     - `{ ok: true, prompt_continuation }`

### Frontend behavior requirements

- Runtime loads automatically on first visit.
- Prompt mode (`>>>` vs `...`) must follow runtime-provided `prompt_continuation`, not UI guessing.
- Transcript input prompt rendering should reflect current continuation state when command is submitted.
- Worker errors/message decoding errors are treated as fatal for in-flight requests.

## WASM Build and Artifacts

- Builder: `scripts/build_wasm_website_bundle.sh` (repo root).
- Current build mode uses:
  - `--target wasm32-unknown-unknown`
  - `--profile release-wasm`
  - `--no-default-features --features wasm-vm-probe`
- Output written to `website/public/wasm/` (`pyrs.js`, `pyrs_bg.wasm`, `pyrs_env.js`, types).

Rules:

- Treat `public/wasm/*` as generated artifacts.
- Do not hand-edit generated wasm/js glue files.
- If runtime-facing contract changes, update docs and regenerate related artifacts/checks.

## Documentation and Contract Sync

When changing playground worker/runtime contracts, keep these in sync:

- `docs/WASM_API_CONTRACT.md`
- `docs/WASM_CLIENT_INTEGRATION_FLOW.md`
- `docs/REPL_SHARED_CORE_DESIGN.md` (if REPL architecture/ownership changes)

## Validation Checklist for Typical Changes

Website-only content/layout/component changes:

1. `pnpm --dir website build`
2. `pnpm --dir website check:links` (or `build:check`)

Playground/worker protocol or wasm-runtime integration changes:

1. `pnpm --dir website build`
2. Run relevant wasm checks from repo root (minimum):
   - `cargo check --target wasm32-unknown-unknown --no-default-features`
   - `cargo check --target wasm32-unknown-unknown --no-default-features --features wasm-vm-probe`
3. Prefer full gate before merge:
   - `scripts/check_wasm_branch.sh`

## UI/UX Guardrails

- Keep homepage and docs nav consistent; shared navigation must not drift by route.
- Favor clean spacing and readable typography over nested decorative containers.
- Reuse existing visual primitives (`HighlightChip`, command/code blocks, docs callouts/tables).
- Maintain dark-theme readability and responsive behavior (desktop + mobile).

## Deployment Notes

- GitHub Pages workflows build and publish `website/dist`.
- Base/site URL behavior is configured in workflows; avoid hardcoding absolute URLs in page code.
- Prefer `import.meta.env.BASE_URL` when wiring static/runtime asset paths.
