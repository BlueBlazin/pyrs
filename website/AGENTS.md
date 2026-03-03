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


# Concise rules for building accessible, fast, delightful UIs. Use MUST/SHOULD/NEVER to guide decisions.

## Interactions

### Keyboard

- MUST: Full keyboard support per [WAI-ARIA APG](https://www.w3.org/WAI/ARIA/apg/patterns/)
- MUST: Visible focus rings (`:focus-visible`; group with `:focus-within`)
- MUST: Manage focus (trap, move, return) per APG patterns
- NEVER: `outline: none` without visible focus replacement

### Targets & Input

- MUST: Hit target ≥24px (mobile ≥44px); if visual <24px, expand hit area
- MUST: Mobile `<input>` font-size ≥16px to prevent iOS zoom
- NEVER: Disable browser zoom (`user-scalable=no`, `maximum-scale=1`)
- MUST: `touch-action: manipulation` to prevent double-tap zoom
- SHOULD: Set `-webkit-tap-highlight-color` to match design

### Forms

- MUST: Hydration-safe inputs (no lost focus/value)
- NEVER: Block paste in `<input>`/`<textarea>`
- MUST: Loading buttons show spinner and keep original label
- MUST: Enter submits focused input; in `<textarea>`, ⌘/Ctrl+Enter submits
- MUST: Keep submit enabled until request starts; then disable with spinner
- MUST: Accept free text, validate after—don't block typing
- MUST: Allow incomplete form submission to surface validation
- MUST: Errors inline next to fields; on submit, focus first error
- MUST: `autocomplete` + meaningful `name`; correct `type` and `inputmode`
- SHOULD: Disable spellcheck for emails/codes/usernames
- SHOULD: Placeholders end with `…` and show example pattern
- MUST: Warn on unsaved changes before navigation
- MUST: Compatible with password managers & 2FA; allow pasting codes
- MUST: Trim values to handle text expansion trailing spaces
- MUST: No dead zones on checkboxes/radios; label+control share one hit target

### State & Navigation

- MUST: URL reflects state (deep-link filters/tabs/pagination/expanded panels)
- MUST: Back/Forward restores scroll position
- MUST: Links use `<a>`/`<Link>` for navigation (support Cmd/Ctrl/middle-click)
- NEVER: Use `<div onClick>` for navigation

### Feedback

- SHOULD: Optimistic UI; reconcile on response; on failure rollback or offer Undo
- MUST: Confirm destructive actions or provide Undo window
- MUST: Use polite `aria-live` for toasts/inline validation
- SHOULD: Ellipsis (`…`) for options opening follow-ups ("Rename…") and loading states ("Loading…")

### Touch & Drag

- MUST: Generous targets, clear affordances; avoid finicky interactions
- MUST: Delay first tooltip; subsequent peers instant
- MUST: `overscroll-behavior: contain` in modals/drawers
- MUST: During drag, disable text selection and set `inert` on dragged elements
- MUST: If it looks clickable, it must be clickable

### Autofocus

- SHOULD: Autofocus on desktop with single primary input; rarely on mobile

## Animation

- MUST: Honor `prefers-reduced-motion` (provide reduced variant or disable)
- SHOULD: Prefer CSS > Web Animations API > JS libraries
- MUST: Animate compositor-friendly props (`transform`, `opacity`) only
- NEVER: Animate layout props (`top`, `left`, `width`, `height`)
- NEVER: `transition: all`—list properties explicitly
- SHOULD: Animate only to clarify cause/effect or add deliberate delight
- SHOULD: Choose easing to match the change (size/distance/trigger)
- MUST: Animations interruptible and input-driven (no autoplay)
- MUST: Correct `transform-origin` (motion starts where it "physically" should)
- MUST: SVG transforms on `<g>` wrapper with `transform-box: fill-box`

## Layout

- SHOULD: Optical alignment; adjust ±1px when perception beats geometry
- MUST: Deliberate alignment to grid/baseline/edges—no accidental placement
- SHOULD: Balance icon/text lockups (weight/size/spacing/color)
- MUST: Verify mobile, laptop, ultra-wide (simulate ultra-wide at 50% zoom)
- MUST: Respect safe areas (`env(safe-area-inset-*)`)
- MUST: Avoid unwanted scrollbars; fix overflows
- SHOULD: Flex/grid over JS measurement for layout

## Content & Accessibility

- SHOULD: Inline help first; tooltips last resort
- MUST: Skeletons mirror final content to avoid layout shift
- MUST: `<title>` matches current context
- MUST: No dead ends; always offer next step/recovery
- MUST: Design empty/sparse/dense/error states
- SHOULD: Curly quotes (" "); avoid widows/orphans (`text-wrap: balance`)
- MUST: `font-variant-numeric: tabular-nums` for number comparisons
- MUST: Redundant status cues (not color-only); icons have text labels
- MUST: Accessible names exist even when visuals omit labels
- MUST: Use `…` character (not `...`)
- MUST: `scroll-margin-top` on headings; "Skip to content" link; hierarchical `<h1>`–`<h6>`
- MUST: Resilient to user-generated content (short/avg/very long)
- MUST: Locale-aware dates/times/numbers (`Intl.DateTimeFormat`, `Intl.NumberFormat`)
- MUST: Accurate `aria-label`; decorative elements `aria-hidden`
- MUST: Icon-only buttons have descriptive `aria-label`
- MUST: Prefer native semantics (`button`, `a`, `label`, `table`) before ARIA
- MUST: Non-breaking spaces: `10&nbsp;MB`, `⌘&nbsp;K`, brand names

## Content Handling

- MUST: Text containers handle long content (`truncate`, `line-clamp-*`, `break-words`)
- MUST: Flex children need `min-w-0` to allow truncation
- MUST: Handle empty states—no broken UI for empty strings/arrays

## Performance

- SHOULD: Test iOS Low Power Mode and macOS Safari
- MUST: Measure reliably (disable extensions that skew runtime)
- MUST: Track and minimize re-renders (React DevTools/React Scan)
- MUST: Profile with CPU/network throttling
- MUST: Batch layout reads/writes; avoid reflows/repaints
- MUST: Mutations (`POST`/`PATCH`/`DELETE`) target <500ms
- SHOULD: Prefer uncontrolled inputs; controlled inputs cheap per keystroke
- MUST: Virtualize large lists (>50 items)
- MUST: Preload above-fold images; lazy-load the rest
- MUST: Prevent CLS (explicit image dimensions)
- SHOULD: `<link rel="preconnect">` for CDN domains
- SHOULD: Critical fonts: `<link rel="preload" as="font">` with `font-display: swap`

## Dark Mode & Theming

- MUST: `color-scheme: dark` on `<html>` for dark themes
- SHOULD: `<meta name="theme-color">` matches page background
- MUST: Native `<select>`: explicit `background-color` and `color` (Windows fix)

## Hydration

- MUST: Inputs with `value` need `onChange` (or use `defaultValue`)
- SHOULD: Guard date/time rendering against hydration mismatch

## Design

- SHOULD: Layered shadows (ambient + direct)
- SHOULD: Crisp edges via semi-transparent borders + shadows
- SHOULD: Nested radii: child ≤ parent; concentric
- SHOULD: Hue consistency: tint borders/shadows/text toward bg hue
- MUST: Accessible charts (color-blind-friendly palettes)
- MUST: Meet contrast—prefer [APCA](https://apcacontrast.com/) over WCAG 2
- MUST: Increase contrast on `:hover`/`:active`/`:focus`
- SHOULD: Match browser UI to bg
- SHOULD: Avoid dark color gradient banding (use background images when needed)