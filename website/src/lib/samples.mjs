/**
 * The samples pages show from website/examples, and the note a page gives
 * each one with `<SampleNote sample="…" />` (#2648). The component and the
 * plain-text export (scripts/content.mjs) both take the note from here.
 *
 * Each sample is in one of two states. A sample whose files are the release's
 * stays on the release: a Rust manifest pins each BACnet crate to its
 * published version, and the note says how to get it from crates.io or PyPI.
 * A sample changed since the release follows the `dev` branch: its Rust
 * manifest takes the BACnet crates from `dev` with a git dependency, and the
 * note says so and links the release's copy. A Rust sample's state is its
 * manifest's; a Python sample's is its `follows` key in
 * src/data/samples.json. `scripts/release/website_samples.py --pin` and
 * `--follow-dev` set either, for one sample or all (docs/ci.md, "Release");
 * tests/unit/sample-notes.test.mjs checks each state against the tag.
 */
import { release, repository, sitePath } from './site.mjs';
import registry from '../data/samples.json' with { type: 'json' };

/**
 * @typedef {{ path: string, language: string, follows?: string, since?: string, tagPage?: string }} Sample
 * Every sample, by the name a page gives SampleNote (src/data/samples.json):
 * its path under website/examples and its language. `follows: "dev"` puts a
 * Python sample on the `dev` branch. `tagPage` names the page, under
 * src/content/docs at the release's tag, that showed the sample inline
 * before it was a file here; the release after it removes the key. `since`
 * names the release that will first ship a sample no release has in any form.
 * @type {Record<string, Sample>}
 */
export const samples = registry;

/** How a manifest names a BACnet crate from the `dev` branch, before any other key. */
export const devSource = `git = "${repository}", branch = "dev"`;

/** The pip line that installs `rusty-bacnet` from the `dev` branch. */
export const devInstall = `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"`;

/** The oldest Python `rusty-bacnet` supports: crates/rusty-bacnet/pyproject.toml's requires-python (a unit test keeps them equal). */
export const pythonMinimum = '3.11';

const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const pinnedPlain = /^"=(\d+\.\d+\.\d+)"$/;
const pinnedTable = /^\{ version = "=(\d+\.\d+\.\d+)"(?:, ([^{}]*))? \}$/;
const devTable = new RegExp(`^\\{ ${escape(devSource)}(?:, ([^{}]*))? \\}$`);
// Keys that would move a crate to another source; the rest (features) may follow.
const sourceKey = /(?:^|, )(?:version|git|branch|tag|rev|path|registry|package) =/;

/**
 * A manifest's state: null when its BACnet crates follow `dev`, otherwise
 * each crate's pinned version. Throws on a crate in neither form, on a mix,
 * on a BACnet crate outside a `bacnet-x = …` line of `[dependencies]`, and on
 * a manifest with none.
 * @param {string} text
 * @returns {Record<string, string> | null}
 */
export function manifestState(text) {
  /** @type {Record<string, string>} */
  const pinned = {};
  let onDev = 0;
  let section = '';
  for (const line of text.split('\n')) {
    if (line.startsWith('[')) section = line.trim();
    if (/^\[.*\bbacnet-/.test(line)) throw new Error(`${line}: name a BACnet crate on one line of [dependencies]`);
    if (!/^\s*"?bacnet-/.test(line)) continue;
    const match = line.match(/^(bacnet-[a-z0-9-]+) = (.+)$/);
    if (!match) throw new Error(`${line}: name a BACnet crate as bacnet-x = …`);
    const [, crate, value] = match;
    if (section !== '[dependencies]') throw new Error(`${crate} is in ${section}, not [dependencies]`);
    const plain = value.match(pinnedPlain);
    const table = value.match(pinnedTable);
    const dev = value.match(devTable);
    const version = plain?.[1] ?? table?.[1];
    if ((!version && !dev) || sourceKey.test(table?.[2] ?? dev?.[1] ?? '')) {
      throw new Error(`${crate} = ${value}: neither the dev branch ({ ${devSource} }) nor an exact release ("=x.y.z")`);
    }
    if (version) pinned[crate] = version;
    else onDev++;
  }
  const count = Object.keys(pinned).length;
  if (count + onDev === 0) throw new Error('the manifest names no BACnet crate');
  if (count && onDev) throw new Error(`the manifest pins ${count} BACnet crates and takes ${onDev} from dev`);
  return count ? pinned : null;
}

/** Whether release `a` comes after release `b`. */
function newer(a, b) {
  const [x, y] = [a, b].map(version => version.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

const msrvOf = text => text.match(/^rust-version = "([^"]+)"$/m)?.[1];

/** @typedef {{ text?: string, code?: string, strong?: string, href?: string, block?: string }} Run */

/**
 * One sample's note, as runs of `{ text }`, `{ code }`, `{ strong }`,
 * `{ text, href }` and `{ block }`, a shell command shown as its own block
 * (the text after it is a new paragraph). `manifests` maps every Rust
 * sample's path to its Cargo.toml: a Rust note names its own manifest's
 * minimum Rust version, and a Python note the newest of them, for pip's build.
 * @param {string} name
 * @param {Record<string, string>} manifests
 * @param {{ siteRelease?: string, registry?: Record<string, Sample> }} [options]
 * @returns {Run[]}
 */
export function sampleNote(name, manifests, { siteRelease = release, registry = samples } = {}) {
  const sample = registry[name];
  if (!sample) throw new Error(`Unknown sample ${name}; add it to src/data/samples.json`);
  const rust = sample.language === 'rust';
  let onDev;
  let msrv;
  if (rust) {
    const manifest = manifests[sample.path];
    if (manifest === undefined) throw new Error(`${sample.path}/Cargo.toml is missing`);
    if (sample.follows !== undefined) throw new Error(`${name}: a Rust sample's manifest says what it follows`);
    onDev = manifestState(manifest) === null;
    msrv = msrvOf(manifest);
  } else {
    if (![undefined, 'dev'].includes(sample.follows)) throw new Error(`${name}: follows is "dev" or absent`);
    onDev = sample.follows === 'dev';
    msrv = Object.values(manifests).map(msrvOf).filter(Boolean).sort((a, b) => newer(a, b) ? 1 : -1).at(-1);
  }
  if (!msrv) throw new Error(`${name}: no rust-version`);
  if (sample.since !== undefined) {
    if (!newer(sample.since, siteRelease)) throw new Error(`v${siteRelease} has ${name}: remove its since key`);
    if (!onDev || sample.tagPage !== undefined) throw new Error(`${name}: a sample no release has follows dev and has no tagPage`);
  }
  if (!onDev) {
    return rust
      ? [{ text: 'The manifest pins the ' }, { strong: `v${siteRelease}` }, { text: ' BACnet crates from crates.io, whose minimum Rust version is ' }, { strong: msrv }, { text: '.' }]
      : [{ text: `This program runs with the v${siteRelease} ` }, { code: 'rusty-bacnet' }, { text: ' package. Install it in a virtual environment:' },
        { block: `python -m pip install --only-binary=:all: rusty-bacnet==${siteRelease}` }];
  }
  const runs = rust
    ? [{ text: 'This sample follows the ' }, { code: 'dev' }, { text: ' branch: its manifest takes the BACnet crates from ' },
      { code: 'dev' }, { text: ' on GitHub, with Rust ' }, { strong: msrv }, { text: ' or later. ' }, { code: 'dev' },
      { text: ' moves: Cargo builds the commit it first fetches, which it records in ' }, { code: 'Cargo.lock' }, { text: ', and ' },
      { code: 'cargo update' }, { text: ' takes the newest one, which may no longer match the files shown here. ' }]
    : [{ text: 'This program follows the ' }, { code: 'dev' }, { text: ' branch. Install ' }, { code: 'rusty-bacnet' },
      { text: ' from ' }, { code: 'dev' }, { text: ` in a virtual environment. pip builds it from source, which takes several minutes and needs git, Python ${pythonMinimum} or later, ` },
      { text: 'Rust', href: 'https://rustup.rs' }, { text: ` ${msrv} or later, and CMake and a C toolchain for ` },
      { text: "BACnet/SC's TLS library", href: sitePath('integrate/security/#save-the-program-and-manifest') }, { text: ':' },
      { block: devInstall },
      { code: 'dev' }, { text: ' moves: each install builds its newest commit, which may no longer match the program shown here. ' }];
  const reader = rust ? `For the v${siteRelease} crates on crates.io, ` : `For the v${siteRelease} package from PyPI, `;
  if (sample.tagPage !== undefined) {
    return [...runs, { text: `${reader}follow ` },
      { text: `this page as published at v${siteRelease} (source)`, href: `${repository}/blob/v${siteRelease}/website/src/content/docs/${sample.tagPage}` },
      { text: ' instead.' }];
  }
  if (sample.since !== undefined) {
    return [...runs, { text: `It is new since v${siteRelease}, so no release has it yet.` }];
  }
  return [...runs, { text: `${reader}use ` },
    { text: `the same ${rust ? 'sample' : 'program'} at v${siteRelease}`, href: tagUrl(sample, siteRelease) },
    { text: '.' }];
}

/** The sample at a release's tag on GitHub: a Rust sample's directory, or a Python file. */
export function tagUrl(sample, version = release) {
  return `${repository}/${sample.language === 'rust' ? 'tree' : 'blob'}/v${version}/website/examples/${sample.path}`;
}

/**
 * A note's runs as Markdown, for the plain-text export.
 * @param {Run[]} runs
 */
export function noteMarkdown(runs) {
  return runs.map(run => run.block !== undefined ? `\n\n\`\`\`sh\n${run.block}\n\`\`\`\n\n`
    : run.href ? `[${run.text}](${run.href})` : run.code ? `\`${run.code}\`` : run.strong ? `**${run.strong}**` : run.text).join('');
}

/**
 * A note's runs as the paragraphs and command blocks the component renders.
 * @param {Run[]} runs
 * @returns {Array<{ block: string } | { inline: Run[] }>}
 */
export function noteParts(runs) {
  /** @type {Array<{ block: string } | { inline: Run[] }>} */
  const parts = [];
  for (const run of runs) {
    const last = parts.at(-1);
    if (run.block !== undefined) parts.push({ block: run.block });
    else if (last && 'inline' in last) last.inline.push(run);
    else parts.push({ inline: [run] });
  }
  return parts;
}
