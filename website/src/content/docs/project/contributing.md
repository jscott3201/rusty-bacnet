---
title: "Improve the documentation"
description: "Keep examples small, source-grounded, and useful to the next operator."
---

A good documentation change helps someone complete a real task and understand the result. Fix the example, the surrounding explanation, and any conflicting source guide together.

## Keep one task in focus

Start a guide with the intended outcome, prerequisites, and whether it sends traffic, creates remote state, or changes equipment behavior. Show the smallest useful command or program. Explain what success looks like and what the reader should check when it does not occur.

## Verify the interface

Check command help and implementation for CLI syntax; check published APIs, type stubs, and tests for language examples. Do not infer a server capability from an enum constant or a CLI feature from a Rust transport module.

Keep examples tied to a named release. Use relative repository links for maintainer-only material and correctly based website links for public routes. Test the GitHub Pages project subpath rather than only a local root URL.

## Make visuals useful

Use SVG diagrams for transport relationships, addressing, and troubleshooting flow. Label illustrative data. Include meaningful alternative text and a nearby prose explanation. Keep essential instructions in text rather than only inside an image.

## Keep the reading experience accessible

Use descriptive headings, visible keyboard focus, copyable code, and clear link names. Do not rely on color alone for warnings or support status. Check mobile layouts and light/dark themes after editing a component.

## Preserve technical evidence

The website is the task-oriented entry point. Generated rustdoc, distributed Python stubs, command help, the changelog, and conformance artifacts retain their specific roles. Avoid creating a second handwritten copy of a large API table or evidence ledger.

No private keys, real customer identifiers, or unredacted operational captures belong in site source or public issue attachments.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Repository](https://github.com/jscott3201/rusty-bacnet) · [Existing docs](https://github.com/jscott3201/rusty-bacnet/tree/v0.11.0/docs).
