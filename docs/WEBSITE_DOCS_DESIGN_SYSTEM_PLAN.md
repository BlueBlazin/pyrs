# Website and Docs Design System Plan

## Purpose
Establish a production-grade, reusable design and implementation system for the public PYRS website and user-facing documentation site.

This plan is intentionally milestone-based and execution-oriented. It defines what must be built, in what order, and how each milestone closes.

## Scope
In scope:
- Public website UX and visual system in `website/`.
- Docs information architecture, templates, and component system.
- Reusable UI primitives and style tokens.
- Content structure for user-facing docs/reference pages.
- Build/deploy integration for GitHub Pages.

Out of scope:
- Interpreter/runtime semantics work.
- Internal engineering docs migration from `docs/` to public site by default.
- Blog/news CMS.

## Current State Snapshot
Current pain points this plan addresses:
1. Inconsistent navigation behavior across home/docs routes.
2. Duplicated UI code (header/nav and page-level styles).
3. Ad-hoc page styling with weak component reuse.
4. Docs content and structure not aligned to modern documentation UX.
5. Uneven visual language between landing pages and docs pages.

## Goals
1. Build a coherent, reusable design system with component-level consistency.
2. Make `/docs/` the canonical user entry point with clear “Getting Started” flow.
3. Provide a clean docs IA with stable sidebar navigation.
4. Standardize tokens, typography, spacing, code blocks, tables, and callouts.
5. Keep docs pages fast, readable, and maintainable.
6. Preserve a distinct but compatible visual identity between landing and docs.

## Non-Goals
1. No framework migration away from Astro.
2. No React dependency unless a concrete interaction requires it.
3. No immediate migration of all internal `docs/*.md` to public docs pages.

## Technology Baseline
Approved baseline for this plan:
- `astro`
- `@astrojs/tailwind`
- `tailwindcss`
- `@tailwindcss/typography`
- `@astrojs/mdx`
- `clsx`
- `tailwind-merge`

Guidance:
- Default to Astro components + MDX.
- Use client-side JS only for localized interactions.
- Keep JS bundles minimal and avoid unnecessary client islands.

## Architecture Model
### 1) Routing and IA
Target top-level routes:
- `/` landing
- `/docs/` getting started
- `/docs/install/`
- `/docs/reference/`
- future docs sections under `/docs/...`

Legacy compatibility:
- `/reference/` redirects to `/docs/reference/` until old links are no longer used.

### 2) Shared Layout System
Required layouts:
- `SiteLayout` for landing-centric pages.
- `DocsLayout` for docs pages with sidebar shell.

Shared global UI primitives:
- `TopNav`
- `Footer`
- `SectionHeading`
- `ButtonLink`

Docs primitives:
- `DocsSidebar`
- `DocsPageHeader`
- `DocSection`
- `CodeBlock`
- `CommandBlock`
- `Callout`
- `DataTable`

### 3) Styling Strategy
- Tailwind utility-first styles for component construction.
- Single token source for color, spacing, typography, radii, elevation.
- Use semantic tokens rather than raw color literals at component surfaces.

Token families:
- Base surfaces: page background, elevated surface, subtle panel.
- Text: primary, secondary, muted, inverse.
- Border: subtle, default, strong, interactive.
- Accent: restrained brand accent for focus and active states only.
- Status: success, warning, error, info.

### 4) Content Model
- Use MDX for docs content pages.
- Keep metadata frontmatter minimal and explicit:
  - `title`
  - `description`
  - `sidebar_group`
  - `sidebar_order`
  - `slug` (optional when filesystem route is canonical)

Authoring rules:
- One concept per page.
- Command snippets runnable as-is.
- Use reference tables where behavior is contract-like (flags/env vars).
- Keep tone technical and concise.

## UX Standards
### Navigation
- Top nav must be stable in position and visual style across routes.
- Content of secondary nav action can vary by context:
  - home: `Docs`
  - docs: `Home`
- Sidebar must clearly indicate current page and section group.

### Docs Page Composition
Standard order:
1. Page title + concise description.
2. Task-oriented sections with practical examples.
3. Related links or next steps at page end.

### Code and Command UX
- Distinguish executable shell commands from generic code.
- Command blocks should support one-click copy.
- Copy payload must exclude prompt glyphs.
- Prompts should be visually present but non-selectable.
- Ensure strong contrast between code surfaces and page background.

### Visual Direction
- Docs theme: neutral dark, minimal accent usage.
- Avoid heavy panel stacking and decorative gradients in docs body.
- Keep hierarchy from typography and spacing, not decorative containers.

## Milestone Plan

## Milestone 0: Foundation Alignment
Objective:
- Confirm architecture, IA, and style constraints before refactor.

Deliverables:
- This design plan approved.
- Route map approved.
- Baseline dependency set installed.

Exit criteria:
- No open blockers on stack choice or IA ownership.

## Milestone 1: Global UI Infrastructure
Objective:
- Eliminate duplicated page chrome and set reusable foundations.

Deliverables:
- Shared `TopNav` component consumed by home/docs layouts.
- Shared root style/tokens setup wired for Tailwind.
- Base typography and spacing scales defined.

Exit criteria:
- Header/nav code no longer duplicated across pages.
- Visual/nav consistency verified across `/` and `/docs/*`.

## Milestone 2: Docs Shell and Navigation
Objective:
- Implement production-quality docs shell.

Deliverables:
- `DocsLayout` with stable left sidebar, content area, and page header.
- Sidebar group model and active-page highlighting.
- Mobile behavior for sidebar collapse/stacking.

Exit criteria:
- `/docs/`, `/docs/install/`, `/docs/reference/` share one shell.
- No route-specific drift in nav placement and style.

## Milestone 3: Component Library v1
Objective:
- Create reusable documentation primitives.

Deliverables:
- `DocSection`, `CodeBlock`, `CommandBlock`, `Callout`, `DataTable` components.
- `cn()` helper using `clsx` + `tailwind-merge` for class composition.
- Component usage docs in-code or in a small style guide page.

Exit criteria:
- Docs pages use components instead of page-local ad-hoc styles.
- Common UI patterns are no longer reimplemented per page.

## Milestone 4: Content Refactor v1 (Getting Started + Install + Reference)
Objective:
- Establish high-quality baseline docs set.

Deliverables:
- `Getting Started` task flow page.
- `Installation` page (Cargo, source build, Docker, nightly archives).
- `Reference` page for CLI modes, flags, and env vars.

Exit criteria:
- Commands validated against current runtime behavior.
- Reference tables map to actual implementation (`src/cli/mod.rs` and runtime wiring).

## Milestone 5: QA and Accessibility Hardening
Objective:
- Ensure docs UX quality and maintainability.

Deliverables:
- Accessibility pass (headings, focus states, color contrast, table semantics).
- Responsive pass for common breakpoints.
- Broken-link and route-integrity checks in build workflow.

Exit criteria:
- No major contrast/focus/semantic issues in docs shell and primitives.
- Build gate fails on broken docs links.

## Milestone 6: Performance and Build Hygiene
Objective:
- Keep docs fast and lightweight.

Deliverables:
- Audit client-side JS usage.
- Keep interaction scripts scoped to components that require them.
- Ensure no unnecessary framework client bundles.

Exit criteria:
- No global client framework hydration for static docs pages.
- Docs pages remain static-first and load quickly.

## Milestone 7: Deployment and Operations
Objective:
- Stabilize publishing flow for GitHub Pages.

Deliverables:
- Build pipeline from `website/` source into publish artifact branch.
- Clear ownership of generated output and deployment branch strategy.
- Rollback strategy for broken docs deploys.

Exit criteria:
- Deterministic docs deploy process documented and repeatable.
- Publish failures provide clear diagnostics.

## Milestone 8: Expansion Track
Objective:
- Extend docs depth after core quality baseline is stable.

Deliverables:
- Additional reference pages (REPL behavior, import/path behavior, diagnostics).
- Optional search integration.
- Optional changelog/release notes integration.

Exit criteria:
- New pages follow component and style conventions with no drift.

## Future Track: WASM REPL Integration (Deferred)
Purpose:
- Define how an in-browser PYRS REPL can be added later without rewriting the website architecture.

Status:
- Deferred until core docs/design-system milestones are stable.

Scope for this track:
- Browser-hosted REPL experience on the docs/landing site using WebAssembly.
- Client-side execution sandbox boundaries and runtime loading strategy.
- UX integration with docs examples (copy-to-REPL, quick-run snippets).

Out of scope for initial WASM track:
- Full parity with native runtime performance/features on day one.
- Arbitrary host filesystem/network access from browser execution.
- Replacing CLI/local runtime workflows.

### WASM Milestone W1: Runtime Packaging Strategy
Objective:
- Produce a deterministic WASM build target for the interpreter suitable for browser loading.

Deliverables:
- Documented `wasm32` build pipeline and artifacts.
- Versioned browser bundle contract (runtime wasm + loader JS + optional worker).
- Clear feature matrix for browser mode vs native mode.

Exit criteria:
- Repeatable local build and smoke run in browser shell.

### WASM Milestone W2: Web REPL Shell
Objective:
- Deliver a minimal production-quality web REPL UI integrated into the docs site.

Deliverables:
- Dedicated route (for example `/playground/`).
- Editor/input, output console, clear/reset controls.
- Execution lifecycle handling (busy/error states, cancel/reset semantics).

Exit criteria:
- Stable interactive session for core Python snippets in supported browser targets.

### WASM Milestone W3: Isolation and Performance
Objective:
- Keep browser execution safe and responsive.

Deliverables:
- Worker-based isolation model (avoid blocking main UI thread).
- Guardrails for long-running execution (timeouts/interrupt strategy where feasible).
- Basic telemetry hooks for load/execute timing and failure diagnostics.

Exit criteria:
- No main-thread lockup in normal usage; bounded failure behavior under stress snippets.

### WASM Milestone W4: Docs Integration
Objective:
- Connect documentation examples to the web REPL experience.

Deliverables:
- Reusable “Run in Browser REPL” action component.
- Example compatibility annotations when browser mode differs from native mode.
- Authoring guidelines for docs examples intended for browser execution.

Exit criteria:
- Docs examples can launch into REPL with predictable behavior and clear limitations.

### WASM Constraints and Design Rules
1. Keep REPL integration modular: no hard coupling that complicates non-WASM docs pages.
2. Preserve static-first site behavior; load WASM/worker code only on REPL routes.
3. Explicitly label browser limitations relative to native `pyrs`.
4. Treat security and isolation as first-class requirements, not polish tasks.

## Quality Gates
Every milestone closure should satisfy:
1. `pnpm --dir website build` passes.
2. No duplicated top-level chrome implementations.
3. No ad-hoc page-local color systems that bypass tokens.
4. Docs command snippets are runnable and copy-safe.
5. Any navigation or route change preserves existing external links (or adds redirects).

## Risk Register
1. Tailwind/Astro integration drift across versions.
   - Mitigation: pin compatible versions in lockfile and validate build on each config change.
2. Style drift from mixed page-local CSS and utility usage.
   - Mitigation: move repeated patterns into components; enforce component-first rule.
3. Documentation behavior drift from runtime reality.
   - Mitigation: source reference pages from actual CLI/runtime code and validate commands periodically.
4. Overuse of visual decoration reducing readability.
   - Mitigation: docs theme remains neutral with restrained accent usage.

## Implementation Rules
1. Component-first: if pattern appears twice, extract component.
2. Token-first: avoid raw color literals in page content files.
3. MDX-first for docs content; Astro components for structure and interaction.
4. Keep docs routes flat and predictable under `/docs/`.
5. Commit per subtask milestone checkpoint.

## Initial Execution Checklist
1. Configure Astro with Tailwind + MDX integrations.
2. Add Tailwind config and global stylesheet with token mapping.
3. Build `cn()` helper (`clsx` + `tailwind-merge`).
4. Build component primitives (TopNav, DocsShell, CommandBlock, Table).
5. Migrate `/docs/*` pages to the component system.
6. Add docs build and link-check gate in CI workflow.

## Ownership
- Canonical owner: repository maintainers for website/docs UX.
- Any contributor changing docs shell/components must update this plan if milestones or gates change.
