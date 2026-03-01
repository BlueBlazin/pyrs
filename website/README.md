# Website

Public website and docs app for PYRS (Astro + Tailwind v4 + MDX).

## Local Commands

From repo root:

```bash
pnpm --dir website install
pnpm --dir website dev
pnpm --dir website build
pnpm --dir website check:links
pnpm --dir website build:check
pnpm --dir website preview
```

## Directory Map

- `src/layouts/`
  - `SiteLayout.astro`: shared shell for landing-style pages.
  - `DocsLayout.astro`: shared docs shell with sidebar + pager.
- `src/components/`
  - `TopNav.astro`, `Footer.astro`, `SiteHead.astro`: shared global chrome.
  - `home/*`: landing page sections and install UI primitives.
  - `docs/*`: docs primitives (`DocSection`, `CommandBlock`, `DataTable`, etc).
- `src/pages/`
  - `index.astro`: landing page composition.
  - `docs/*.mdx`: docs content pages.
- `src/config/`
  - `docsNav.ts`: sidebar and docs page-order source of truth.
  - `installCommands.ts`: shared install snippet constants for home/docs.

## Docs Authoring

- Add/edit docs pages under `src/pages/docs/*.mdx`.
- Use `layout: ../../layouts/DocsLayout.astro` in frontmatter.
- Prefer docs primitives over ad-hoc HTML/CSS.
  - `DocSection` for section structure
  - `CommandBlock` for shell snippets
  - `CodeBlock` for code snippets
  - `DataTable` for reference contracts
  - `Callout` for emphasized context
- Example usage reference: `/docs/style-guide/`.

## Navigation and Pager

- Update `src/config/docsNav.ts` when adding/removing docs routes.
- Sidebar and previous/next pager are both generated from this config.

## Install Snippets

- Update install commands in `src/config/installCommands.ts`.
- Both landing install section and docs install/getting-started pages consume these constants.
