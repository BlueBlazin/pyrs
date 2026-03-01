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

`build:check` validates:
- website build succeeds,
- internal links/assets resolve,
- per-page metadata (`title`, description) exists,
- docs shell invariants (sidebar container present, default-open sidebar state, and sidebar links exist).

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

## Deployment (GitHub Pages)

- Workflow: `.github/workflows/website-pages.yml`.
- Trigger: push to `main`/`master` when `website/**` changes (or manual dispatch).
- Publish target: `gh-pages` branch root from `website/dist`.
- The workflow sets:
  - `ASTRO_SITE` to `https://<repo>.github.io` for user/org pages, or `https://<owner>.github.io` for project pages.
  - `ASTRO_BASE` to `/` for user/org pages, or `/<repo>/` for project pages.
- Release gate for deploy is `pnpm --dir website build:check` (build + internal link/meta validation).

## Quality Checklist

- Pre-merge website/docs QA checklist:
  - `website/docs/QUALITY_CHECKLIST.md`
