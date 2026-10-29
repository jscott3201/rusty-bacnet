# Rusty BACnet documentation site

An Astro/Starlight task guide for the release named in `src/lib/site.mjs`:
installation, the local lab and first reads (`start/`), integration guides for
serving objects, routing, shared endpoints, Network Port/Number controls and
server authorization (`integrate/`), data-link setup for BACnet/IP, BACnet/SC
and MS/TP (`connect/`), contributor source builds
and documentation work (`development/`), and discovery, subscription and write
guides (`guides/`). [Engineering
docs](../docs/README.md) remain the contract authority.

The site is published to GitHub Pages by a manual dispatch of `docs-pages.yml`
from a reviewed `dev` commit. A successful local build or browser check does not
publish anything or show that the hosted site contains a change; publishing
requires separate explicit authority.

## Local build and validation

Run from `website/` with **Node 24** (`.nvmrc`):

```sh
npm ci
npx playwright install chromium
npm run verify
```

`verify` runs Astro checking, Node unit tests, the production build, and local
Playwright/axe tests at 320, 360, 768, 1024 and 1440 CSS pixels, each page
in the light and dark themes. It does not run Python
tests or publish anything. The `Docs validation` CI job runs the same command on
native Ubuntu with Node 24 and Chromium. Local results do not prove that a GitHub
Actions run or Pages deployment succeeded.

The build regenerates plain Markdown exports for every navigation guide, `llms.txt`, and the example
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
Use a fresh Python 3.11–3.14 environment with a provenance-verified **release**
native package, not a development wheel that happens to share the release
version. The script refuses any `rusty-bacnet` version but its `EXPECTED_VERSION`.

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
That evidence is for v0.11.0.

## Displayed Python first-read verification

The binding unittest suite also extracts the sole Python code block from
`src/content/docs/start/python.md` and executes those exact bytes in a subprocess.
It uses the installed extension and an ephemeral loopback server to check typed
values, instance selection, unknown-object errors and argument bounds. Missing,
ambiguous, empty or unclosed Python blocks fail instead of silently selecting a
different sample. Run this check from the repository root:

```sh
python -m unittest discover -s crates/rusty-bacnet/tests -p test_website_first_read.py -v
```

The existing binding CI jobs discover this test against their development
extension. To validate a published release, run the same test in a separate
virtual environment containing its published wheel and record the wheel's source,
hash and installation report. A development package can report the release's
version, so the version string alone is insufficient provenance. This check does
not execute other website snippets or qualify discovery, COV, writes or physical
networks. `npm run verify` remains separate from native Python execution.

## TUI screenshots

The `tui/` pages show real macOS Terminal screenshots of `bacnet tui`. They are
WebP files in `src/assets/tui/` (lossy, quality 92: sharp text at about a
twentieth of a PNG's size), which Astro's image pipeline resizes at build time. `src/assets/tui/manifest.json` lists every shot: its file,
alt text, terminal size, the command and keys that produced it, and whether it
is a live capture, an insta snapshot or a placeholder. A unit test checks that
the pages, the manifest and the capture script name the same shots.

`scripts/tui-screenshots/capture.py` regenerates them. It needs macOS, tmux
and cwebp (`brew install tmux webp`) and Rust. It shoots its own Terminal
window; when Terminal is in full screen, that window can't be resized, so the
script keeps the TUI at the shot's size in the window's top-left corner and
crops each shot to it.
From the repository root:

```sh
python3 website/scripts/tui-screenshots/capture.py --list    # the shot list
python3 website/scripts/tui-screenshots/capture.py           # build, then every shot
python3 website/scripts/tui-screenshots/capture.py --reuse --only watch-list,overview-help
```

The script builds `bacnet` (with `sc-tls` and `serial`) and the loopback lab
in `scripts/tui-screenshots/lab/`, a standalone crate outside the workspace
that uses the workspace crates by path. It starts the lab (a BBMD, a router,
three B/IP devices and a BACnet/SC hub, all on 127.0.0.1) and runs the TUI in
a tmux pane. It then opens a **new** Terminal window attached to that pane,
sends the keys through tmux, and photographs only that window with
`screencapture -l`. The window is closed at the end, also on failure or
Ctrl-C; your own Terminal windows and tabs are left as they were. Keep the
screen unlocked while it runs, about five minutes. Grant Terminal or your shell
the Screen Recording permission if macOS asks.

- **Paths.** `BACNET_CONFIG_DIR` is `/tmp/bacnet-docs`, a link to a fresh
  directory that is removed at the end, so no shot shows a home directory or
  user name.
- **MS/TP.** The script never joins a trunk. Without `--mstp-port`, the MS/TP
  shots show the TUI's insta snapshots (simulated stations, no colour).
  `--mstp-port /dev/cu.usbserial-XXXX` adds live monitor shots with
  `--mstp-monitor ... --passive`, which only listens; the join shots still
  come from snapshots.
- **Look.** The window uses Menlo 12 by default (`--font`, `--font-size`) and
  your default Terminal profile's colours (`--terminal-profile NAME` picks
  another). Shots are scaled to at most 1440 pixels wide (`--width`).
- **Checking the driving logic** without a Terminal window:
  `capture.py --reuse --tmux-only DIR` writes each shot's text to `DIR/NAME.txt`
  and leaves the images alone.

Rerun it after a TUI change that the pages show, and review the new WebP images before
committing them. A new shot needs an entry in `SHOTS` in `capture.py`, a step
in one of its scenarios, and a page that uses it.

**Known gap (#2110):** the scenarios match `dev`'s TUI again, and a full run
takes every shot in the manifest. The committed images still date from before
many TUI changes; they are to be retaken from the published 0.15.0 release
build, so the site shows what users install. When a wait times out, the script names the
scenario, the text it waited for and the last shot taken, then prints the
pane.

## Retained CI and publication reference

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

1. Follow the [CI and merge-evidence policy](../docs/ci.md) for delivery. PR
   validation is this workflow's `pull_request` run; publication is the
   manual dispatch below. Do not infer live CI from local tests.
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
  Expressive Code drops a hidden block's tabindex, so `src/scripts/tab-code-focus.js`
  restores it on the overflowing blocks of a tab panel when the panel is shown.
- A task shown in more than one language uses `<Tabs syncKey="lang">` with the
  labels `CLI`, `Python` and `Rust`; operating-system tabs use `syncKey="os"`.
  A reader's choice then follows them from page to page. The sample harness
  finds fences inside tabs as anywhere else. The axe test selects every tab
  of each page in both themes.
- Search: `src/components/DocsMarkdownContent.astro` lowers the Pagefind weight
  of every page with a generated reference region (`generatedSearchWeight` in
  `src/lib/site.mjs`), so task phrases rank the guides first while API names
  still find their reference page; `tests/e2e/site.spec.ts` checks both.
- Diagram enhancement leaves the original SVG and text available without JS.
  Current pages use each diagram once; add distinct IDs before repeating one
  diagram on a page.
- Keep `public/raw/`, `public/examples/`, `public/llms.txt`, `dist/`, `.astro/`,
  dependency trees, screenshots and browser reports untracked.
- The docs workflow is approved for CI and manual publication only. Pages
  settings and actual publication remain owner-operated; the workflow's presence
  does not mean a given change is live. Do not introduce a support-status database.

## Content organization

- `start/`, `connect/`, `integrate/` and `guides/`: release tasks for the version in
  `src/lib/site.mjs`, linked to the canonical repository docs at that release's
  tag. `development/build-from-source` and `development/documentation` cover repository contributions and unreleased work.
- `src/lib/site.mjs`: `release`, which MDX pages, the download list, the footer
  and the llms index read. To move to another release, change it and run
  `npm test`: `tests/unit/release.test.mjs` names every page, the lab script,
  the sidebar label and the home banner that still carry another version
  (the upgrade guide from it, `project/upgrading/v0-15.md`, and the lab's
  recorded evidence may name the previous one). It also checks the installation page's asset list against
  `.github/workflows/release.yml`.
- `tui/`: the terminal UI as it is on `dev`. It shipped in the release in
  `src/lib/site.mjs`; later changes land in the next release. Its screenshots come from `scripts/tui-screenshots/`
  (see "TUI screenshots" above).
- `reference/` and `project/`: reference entry points, support scope, and the
  upgrade hub (`project/upgrading`) with one guide per release under it.
- `reference/cli.md`, `reference/python.md` and `reference/rust.mdx` are
  generated between their `generated:start` and `generated:end` markers by
  `scripts/reference-cli.mjs` (the clap definitions, through a bacnet-cli
  test), `scripts/reference-python.py` (the `.pyi` stub) and
  `scripts/reference-rust.mjs` (`cargo metadata`); the text around the
  markers is authored. The Python script also writes one page per group under
  `reference/python/`, all generated. They track `dev`, so each page says so
  above its generated part. Each script takes `--check`. The release runs them
  (`docs/ci.md`, "Release"); PR CI doesn't, so a PR that adds an option or a
  method needn't regenerate them, but one that adds a `bacnet` command or a
  published crate must. `tests/unit/reference.test.mjs` checks that they
  parse and link correctly. The sidebar nests the Python pages in a
  collapsed group, so tests that walk `navigation.json` flatten an item's
  `items`.
- `src/data/support.json`: the capability tables on `project/conformance`
  (#1496), generated from the code by
  `crates/bacnet-integration-tests/support_tables.rs`; don't edit it. Run
  `cargo run -p bacnet-integration-tests --example support-tables` to
  regenerate it. A workspace test fails when it is stale, so a PR that adds
  an object type, a listed service or a transport feature regenerates it.
  `src/lib/support.mjs` turns it into the page's tables, for the
  `SupportTable` component and the plain-text export alike.
- `src/content/docs/project/sample-pics.md`: the sample PICS (#1496), a
  generated page; don't edit it. `crates/bacnet-integration-tests/sample_pics.rs`
  writes it, banner and all, with the same command, and the same workspace
  test fails when it is stale. A change to an object's property metadata or
  to the PICS generator's output regenerates it.
- `src/data/navigation.json`: sidebar and test route inventory.
- `src/data/legacy-routes.json`: retained routes and section destinations. Compatibility pages keep old bookmarks and raw URLs usable, stay out of Pagefind, and are omitted from the canonical llms guide list.
- `scripts/prepare.mjs`: source-derived raw Markdown and version-aware llms index.

Preserve the loopback lab's single source, generated download and recorded release
native provenance. Tests automatically sweep navigation routes and additionally
check integration/contributor routing, compatibility links, search, scope in exports and representative screenshots.
Do not turn a docs refresh into a new support database, dependency upgrade, site/base
change or publication workflow change.
