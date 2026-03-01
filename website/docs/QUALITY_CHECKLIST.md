# Website Quality Checklist

Use this checklist before merging substantial website/docs UI changes.

## 1) Build and Structural Validation

Run from repo root:

```bash
pnpm --dir website build:check
```

Expected:
- Astro build succeeds.
- Internal links/assets resolve.
- Pages emit non-empty `title` + description metadata.
- Docs shell invariants pass (sidebar present, default-open state, nav links present).

## 2) Keyboard Navigation

Validate with keyboard only:
- `Tab` from top of page reaches the skip link first.
- Top nav actions are reachable and visibly focused.
- Docs sidebar links are reachable and visibly focused.
- Copy buttons for terminal/command blocks are reachable and activate with `Enter`/`Space`.
- Install platform tabs on home page switch with keyboard activation.

## 3) Mobile Behavior

At common breakpoints (at least ~1280, ~1024, ~760, ~390):
- No horizontal overflow or clipped content.
- Docs sidebar collapses on mobile and can be toggled open.
- Hero screenshots remain readable and not cut off.
- Command/code blocks remain horizontally scrollable without layout breakage.

## 4) Contrast and Readability

Manual pass:
- Body text remains readable on dark surfaces.
- Active and hover states are visually distinct.
- Sidebar and TOC active states are clearly visible.
- Code/command surfaces are distinct from surrounding page background.

## 5) Metadata and Crawl Hygiene

Validate:
- Public pages include canonical + social metadata.
- Internal utility pages (`404`, internal style-guide) are `noindex`.
- `robots.txt` and `sitemap.xml` are generated in build output.

## 6) Regression Notes

When fixing a production regression:
- Add a focused validation guard if feasible (script/assertion/check).
- Record the regression and guardrail in the PR description.
- Keep the fix isolated and commit as its own checkpoint.
