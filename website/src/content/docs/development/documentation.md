---
title: "Improve the documentation"
description: "Keep examples small, source-grounded, and useful to the next operator."
---

A good documentation change helps someone complete a real task and understand the result. Fix the example, the surrounding explanation, and any conflicting source guide together.

## Keep one task in focus

Start a guide with the intended outcome, prerequisites, and whether it sends traffic, creates remote state, or changes equipment behavior. Show the smallest useful command or program. Explain what success looks like and what the reader should check when it does not occur.

## Verify the interface

Check command help and implementation for CLI syntax; check published APIs, type stubs, and tests for language examples. Do not infer a server capability from an enum constant or a CLI feature from a Rust transport module.

Keep examples tied to the release the site describes, and mark anything that needs an unreleased source build. Use relative repository links for maintainer-only material and correctly based website links for public routes. Test the GitHub Pages project subpath rather than only a local root URL.

## Make visuals useful

Use SVG diagrams for transport relationships, addressing, and troubleshooting flow. Label illustrative data. Include meaningful alternative text and a nearby prose explanation. Keep essential instructions in text rather than only inside an image.

## Keep the reading experience accessible

Use descriptive headings, visible keyboard focus, copyable code, and clear link names. Do not rely on color alone for warnings or support status. Check mobile layouts and light/dark themes after editing a component.

## Preserve technical evidence

The website is the task-oriented entry point. Generated rustdoc, distributed Python stubs, command help, and the changelog retain their specific roles. Avoid creating a second handwritten copy of a large API table.

No private keys, real customer identifiers, or unredacted operational captures belong in site source or public issue attachments.

## Work in the project

Use [GitHub issues](https://github.com/jscott3201/rusty-bacnet/issues) to report problems and propose work. Preserve immutable release links and historical artifact provenance. Do not edit deployment workflows or publish the site as a side effect of a content change.

The [engineering docs map](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/README.md) identifies canonical contracts. Add integration guidance under `website/src/content/docs/integrate/`, data-link setup under `connect/` and operating guidance under `guides/`; keep install/lab examples tied to their validated revision. The release version lives in `website/src/lib/site.mjs`, and `npm test` names every page that still carries another one. Navigation drives route checks, and Markdown exports must preserve version scope and usable links.

## Validate a content change

From `website/`, use Node 24 and the locked dependencies, then run `DOCS_TEST_PORT=46329 npm run verify`. This checks Astro, unit tests, the static build and Chromium/axe at five widths (320 to 1440 CSS pixels). Inspect desktop and phone screenshots in both themes; exercise search, local links, code copy and keyboard navigation for the changed flow. See the [site maintenance guide](https://github.com/jscott3201/rusty-bacnet/blob/dev/website/README.md).

A documentation build does not execute Rust/Python snippets or requalify BACnet behavior. Reuse valid runtime evidence only with matching inputs, or run the relevant native check when behavior/examples change. The [merge-evidence policy](https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/ci.md) separates required local and hosted checks. Local verification never establishes publication.
