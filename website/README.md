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
tests or publish anything. The `Docs validation` CI job runs the same command on
native Ubuntu with Node 24 and Chromium. Local results do not prove that a GitHub
Actions run or Pages deployment succeeded.

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

## Docs CI and manual publication

`.github/workflows/docs-pages.yml` validates pull requests targeting `dev` or
`main` when `website/**` (including the workflow guard tests) or the workflow
itself changes. It also accepts manual dispatches. There is no push, schedule,
merge-triggered publication, or privileged pull-request workflow. The existing
Rust CI gates remain separate and unchanged; this job runs no BACnet, Python or
equipment tests. WebKit remains an optional local second-engine check.

The validation job has only `contents: read`, checks out the fixed event SHA
without persisting credentials, runs `npm ci`, installs Chromium with Linux
dependencies, and runs the entire `npm run verify`. `npm test` includes maintained
YAML contract guards and Bash expected-SHA cases; these and local `actionlint`
are static/local evidence, not a substitute for an actual Linux Actions run.
When editing the workflow, also run from the repository root:

```sh
actionlint .github/workflows/docs-pages.yml
```

Only a successful **manual** dispatch with `publish=true`, on `refs/heads/dev`,
in `jscott3201/rusty-bacnet` can publish. Its `expected_sha` must be the full
40-character lowercase reviewed `dev` SHA and must match the dispatch event SHA.
It is a freeze check, not a checkout ref: a branch move before dispatch fails
closed. A later branch move does not change the commit already being validated.
Review and delivery gates remain a maintainer responsibility; this input does
not prove approvals. PR validation uses GitHub's event (merge) SHA, not an
arbitrarily supplied branch/head.

The publish attempt packages **only `website/dist`**, after verification, as the
`github-pages` artifact with one-day retention. Validation-only runs do not
upload a Pages artifact. The separate deployment job requires successful
validation, alone receives `pages: write` and `id-token: write`, and deploys that
same run's named artifact with no checkout or rebuild. The pinned Pages actions
fail for missing artifacts; deployment also rejects ambiguous artifact names.
No source tree, dependencies, browser reports or private environments are sent
as the public artifact. Browser failure reports and screenshots are separate
Actions artifacts retained for seven days, with hidden files excluded.

### Maintainer sequence (not evidence of publication)

1. Review and merge this workflow only after the actual Linux `Docs validation`
   job and existing five lean CI gates pass. Do not infer live CI from local tests.
2. Once the workflow is on the default branch, dispatch **validation only** from
   `dev`. `publish` defaults to false; `expected_sha` can be omitted:

   ```sh
   gh workflow run docs-pages.yml --repo jscott3201/rusty-bacnet --ref dev -f publish=false
   ```

   Identify the new run in Actions, confirm its `headSha`/source summary, successful
   validation, skipped deployment and absence of a `github-pages` artifact.
3. Before the first publication, the repository owner must set Pages **Source**
   to **GitHub Actions** (`build_type=workflow`) and configure the `github-pages`
   environment for **Selected branches: `dev` only**, not tags or all protected
   branches. Leave the existing `release` environment unchanged. This workflow
   does not change settings, add reviewers/wait timers, or run `configure-pages`;
   Astro already has an explicit static `site` and project `base`.
4. After review and validation gates, record the full approved `dev` commit and
   dispatch with publication explicitly enabled (replace the placeholder):

   ```sh
   gh workflow run docs-pages.yml --repo jscott3201/rusty-bacnet --ref dev -f publish=true -f expected_sha=FULL_REVIEWED_DEV_SHA
   ```

   Verify the new run's `headSha` and summary equal the reviewed SHA. Record the
   run ID, `github-pages` artifact ID from the validation summary, artifact digest
   from the upload log, and deployment outcome/environment URL. Inspect the
   artifact to confirm it contains the built static site, not repository files.
   Confirm both jobs succeed; a green validation alone is not a publication.
5. Verify the **returned deployment URL** in a browser: homepage, direct nested
   route/reload, native search, theme/tabs, assets, raw Markdown, example download,
   original SVGs, sitemap/canonical URLs and 404 recovery under `/rusty-bacnet/`.
   Only after deployment and live checks may a **separate** follow-up add public
   repository README/package links or replace the unpublished status above.

Future deployments remain manual with the same sequence and reviewed SHA.
PR runs may cancel older checks for that PR, never a publication. Manual publish
runs share a non-cancelling concurrency group; validation-only dispatches have a
separate lane. GitHub concurrency is not FIFO and may replace pending runs: send
one publication at a time, inspect its outcome, and do not assume dispatch order
is deployment order. If validation fails, fix/review rather than bypass checks.
If the artifact expires or publication fails, inspect the failure and use a new
reviewed dispatch to rebuild and revalidate; do not substitute another run's
artifact or fetch a mutable branch in the deployment job.

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
- The docs workflow is approved for CI and manual publication only. Pages
  settings and actual publication remain owner-operated; the workflow's presence
  does not mean this site is live. Repository README/package public links remain
  deferred until verified deployment. Do not introduce a support-status database.
