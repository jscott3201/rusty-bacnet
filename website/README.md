# Rusty BACnet documentation site

An **unpublished** Astro/Starlight site for **v0.11.0**. The development branch's
newer behavior is not the release contract; SC pages link explicitly labeled
development migration references. Root engineering docs and the canonical
conformance ledger remain authoritative for their own scopes.

## Local build and validation

Run from `website/` with **Node 24** (`.nvmrc`):

```sh
npm ci
npx playwright install chromium
npm run verify
```

`verify` runs Astro checking, Node unit tests, the production build, and local
Playwright/axe tests at 320, 390, 768, and 1440 CSS pixels. It does not run Python
tests or publish anything. These Node checks are local evidence, not an existing
repository CI job.

The build regenerates 19 plain Markdown exports, `llms.txt`, and the example
download, then forces a content-layer sync. This prevents cached Markdown HTML
from retaining an old code-renderer configuration. Astro's optional empty-i18n
and duplicate built-in/catch-all 404 warnings are currently nonfatal; production
tests check the actual custom 404 response and recovery link.

The production test server uses Astro's public preview API in the foreground,
rooted in this website. This avoids Astro 7's agent-detected background CLI mode.
Tests never reuse an existing server. Set `DOCS_TEST_PORT` to an unused port if
the default `46327` is occupied; do not stop unrelated processes. Playwright owns
server startup and shutdown.

For local review:

```sh
npm run preview -- --host 127.0.0.1 --port 46327
```

Open `http://127.0.0.1:46327/rusty-bacnet/`, not `/`. If Astro reports a background
preview, use `npm run preview -- status` and `npm run preview -- stop` from this
directory when finished. `npm run dev` is useful during editing but is not
production evidence.

Optional second-engine smoke:

```sh
npx playwright install webkit
DOCS_WEBKIT=1 npm run test:e2e -- --project=webkit
```

For separate local evidence, set `PLAYWRIGHT_BROWSERS_PATH`, `DOCS_TEST_OUTPUT`,
`PLAYWRIGHT_HTML_OUTPUT_DIR`, and `DOCS_SCREENSHOTS` to task-owned directories.
Use a fresh output/report directory for each run when preserving failures.
Screenshots are real production pages, not visual baselines to auto-approve.
Automated axe and browser tests do not establish full accessibility conformance;
screen-reader and physical-device checks remain separate lanes.

## Tutorial validation

`examples/python/loopback_lab.py` is the sole tutorial body: MDX imports it and
`prepare:content` copies it to the public download. Do not edit generated copies.
Use a fresh Python 3.11–3.13 environment with a provenance-verified **release**
native package, not a development wheel that happens to share version 0.11.0.

```sh
python -m unittest discover -s tests/python -v
python examples/python/loopback_lab.py
```

The tests use real loopback BACnet/IP, check the sample 72.5 value and Fahrenheit
unit 64, bound serve duration, and check shutdown. The fixture has no remote-host
option, discovery, writes, or COV. Separate native execution evidence must record
artifact/run/tag, interpreter, platform, hash and import origin. The source
handoff's earlier Linux CPython 3.13 results are not new macOS/Windows validation.
Its Linux CLI libpcap loader failure is not cleared by a test on another platform.

The integration run separately passed all eight tutorial tests and the standalone
lab on macOS 27.0 arm64 / CPython 3.12.13 using the cp312 release wheel from
workflow `34003656261`, artifact `9980479931`, tag `v0.11.0` at
`c9f36c48baa0444ce8c1c6f7889424a1630f2ca5`. Wheel SHA256:
`28b099856a75b8b7a31d45c41b30c5ab2d8b842b42630e1314a7b037173b1323`.
The release `bacnet-macos-arm64` executable also read value 72.5 and units 64
from a 30-second loopback server that then stopped. This is bounded tutorial
evidence, not PyPI, general wheel/platform, or physical-network qualification.

## Maintenance boundaries

- Preserve native Starlight navigation, Pagefind, tabs, theme selection and code
  copy. `DocsFooter.astro` wraps the native footer and makes the native theme
  selector available on mobile splash pages.
- `ec.config.mjs` applies static keyboard focusability to native code blocks;
  Sätteri wraps tables in focusable scroll regions without replacing table
  semantics. No client-side renderer or extra UI framework is used.
- Diagram enhancement leaves the original SVG and text available without JS.
  Current pages use each diagram once; add distinct IDs before repeating one
  diagram on a page.
- Keep `public/raw/`, `public/examples/`, `public/llms.txt`, `dist/`, `.astro/`,
  dependency trees, screenshots and browser reports untracked.
- No deployment workflow, Pages setting, repository README/package links, or
  support-status database is introduced here. Publication and CI integration
  require owner review and approval as a separate slice.
