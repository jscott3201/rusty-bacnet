/**
 * The samples pages show from website/examples, and the note a page gives
 * each one with `<SampleNote sample="…" />` (#2648). The component and the
 * plain-text export (scripts/content.mjs) both take the note from here.
 *
 * Between releases the samples follow the `dev` branch: each Rust manifest
 * takes the BACnet crates from `dev` with a git dependency, so a page shows
 * what builds, and the note links the same sample at the last release's tag
 * for readers on crates.io and PyPI. A release pins every manifest to itself
 * with `scripts/release/website_samples.py --pin`, and the notes then say
 * so; `--follow-dev` restores the `dev` form after it (docs/ci.md, "Release").
 */
import { release, repository } from './site.mjs';

/**
 * Every sample, by the name a page gives SampleNote: its path under
 * website/examples and its language. `since` names the release that will
 * first ship a sample no release has yet: its note can't link the sample at
 * the tag, so it links `tagPage`, the page (under src/content/docs) that
 * showed the release's version inline, or else says the sample is new. The
 * release that ships it removes both keys.
 */
export const samples = {
  'bip-router': { path: 'rust/bip-router', language: 'rust' },
  'first-read': { path: 'rust/first-read', language: 'rust', since: '0.16.0', tagPage: 'start/rust.md' },
  'sc-operator-policy': { path: 'rust/sc-operator-policy', language: 'rust' },
  'server-device': { path: 'rust/server-device', language: 'rust' },
  'shared-endpoint': { path: 'rust/shared-endpoint', language: 'rust' },
  'bip_router.py': { path: 'python/bip_router.py', language: 'python' },
  'first_read.py': { path: 'python/first_read.py', language: 'python', since: '0.16.0', tagPage: 'start/python.md' },
  'loopback_lab.py': { path: 'python/loopback_lab.py', language: 'python' },
  'server_device.py': { path: 'python/server_device.py', language: 'python' },
};

/** The pip line that installs `rusty-bacnet` from the `dev` branch: pip builds the extension with Cargo. */
export const devInstall = `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"`;

/** How a manifest names a BACnet crate from the `dev` branch, before any other key. */
export const devSource = `git = "${repository}", branch = "dev"`;

const escape = text => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const pinnedPlain = /^"=(\d+\.\d+\.\d+)"$/;
const pinnedTable = /^\{ version = "=(\d+\.\d+\.\d+)"(?:, ([^{}]*))? \}$/;
const devTable = new RegExp(`^\\{ ${escape(devSource)}(?:, ([^{}]*))? \\}$`);
// Keys that would move a crate to another source; the rest (features) may follow.
const sourceKey = /(?:^|, )(?:version|git|branch|tag|rev|path|registry|package) =/;

/**
 * The release a manifest pins its BACnet crates to, or null when they follow
 * `dev`. Throws on a crate in neither form, on a mix, on a BACnet crate
 * outside `[dependencies]`, and on a manifest with none.
 */
export function manifestRelease(text) {
  const found = new Set();
  let section = '';
  for (const line of text.split('\n')) {
    if (line.startsWith('[')) section = line.trim();
    const match = line.match(/^(bacnet-[a-z0-9-]+) = (.+)$/);
    if (!match) continue;
    const [, crate, value] = match;
    if (section !== '[dependencies]') throw new Error(`${crate} is in ${section}, not [dependencies]`);
    const plain = value.match(pinnedPlain);
    const table = value.match(pinnedTable);
    const dev = value.match(devTable);
    const version = plain?.[1] ?? table?.[1];
    if ((!version && !dev) || sourceKey.test(table?.[2] ?? dev?.[1] ?? '')) {
      throw new Error(`${crate} = ${value}: neither the dev branch ({ ${devSource} }) nor an exact release ("=x.y.z")`);
    }
    found.add(version ?? null);
  }
  if (found.size === 0) throw new Error('the manifest names no BACnet crate');
  if (found.size > 1) throw new Error(`the manifest mixes ${[...found].map(form => form ?? 'dev').join(' and ')}`);
  return [...found][0];
}

/**
 * The one state of every Rust sample: the release their manifests pin, or
 * null when they follow `dev`. `manifests` maps each sample's path to its
 * Cargo.toml. Throws when they disagree, or pin a release other than the
 * site's: the release pins them all at once.
 */
export function samplesRelease(manifests, siteRelease = release) {
  const states = new Set(Object.entries(manifests).map(([path, text]) => {
    try {
      return manifestRelease(text);
    } catch (error) {
      throw new Error(`${path}/Cargo.toml: ${error.message}`);
    }
  }));
  if (states.size !== 1) throw new Error(`the sample manifests disagree: ${[...states].map(state => state ?? 'dev').join(', ')}`);
  const [state] = states;
  if (state !== null && state !== siteRelease) throw new Error(`the sample manifests pin ${state}, but the site describes ${siteRelease}`);
  return state;
}

/** Whether release `a` comes after release `b`. */
function newer(a, b) {
  const [x, y] = [a, b].map(version => version.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

/** @typedef {{ text?: string, code?: string, strong?: string, href?: string, block?: string }} Run */

/**
 * One sample's note, as runs of `{ text }`, `{ code }`, `{ strong }`,
 * `{ text, href }` and `{ block }`, a shell command shown as its own block
 * (the text after it is a new paragraph). `manifests` maps every Rust
 * sample's path to its Cargo.toml: the state is theirs together, and a Rust
 * note names its own manifest's minimum Rust version.
 * @param {string} name
 * @param {Record<string, string>} manifests
 * @param {{ siteRelease?: string, registry?: Record<string, { path: string, language: string, since?: string, tagPage?: string }> }} [options]
 * @returns {Run[]}
 */
export function sampleNote(name, manifests, { siteRelease = release, registry = samples } = {}) {
  const sample = registry[name];
  if (!sample) throw new Error(`Unknown sample ${name}; add it to src/lib/samples.mjs`);
  const pinned = samplesRelease(manifests, siteRelease);
  const rust = sample.language === 'rust';
  let msrv = '';
  if (rust) {
    msrv = manifests[sample.path]?.match(/^rust-version = "([^"]+)"$/m)?.[1];
    if (!msrv) throw new Error(`${sample.path}/Cargo.toml: no rust-version`);
  }
  if (sample.since !== undefined && !newer(sample.since, siteRelease)) {
    throw new Error(`v${siteRelease} has ${name}: remove its since and tagPage keys`);
  }
  if (sample.tagPage !== undefined && sample.since === undefined) {
    throw new Error(`${name}: tagPage goes with since`);
  }
  if (pinned !== null) {
    return rust
      ? [{ text: 'The manifest pins ' }, { strong: `v${pinned}` }, { text: ' of the BACnet crates on crates.io, whose minimum Rust version is ' }, { strong: msrv }, { text: '.' }]
      : [{ text: `This program runs with the v${pinned} ` }, { code: 'rusty-bacnet' }, { text: ' package. Install it in a virtual environment:' },
        { block: `python -m pip install rusty-bacnet==${pinned}` }];
  }
  const runs = rust
    ? [{ text: 'This sample follows the ' }, { code: 'dev' }, { text: ' branch: its manifest takes the BACnet crates from ' },
      { code: 'dev' }, { text: ' on GitHub, so these files build as shown, with Rust ' }, { strong: msrv }, { text: ' or later. ' }]
    : [{ text: 'This program follows the ' }, { code: 'dev' }, { text: ' branch. Install ' }, { code: 'rusty-bacnet' },
      { text: ' from ' }, { code: 'dev' }, { text: ' in a virtual environment; pip builds its native extension, so install ' },
      { text: 'Rust', href: 'https://rustup.rs' }, { text: ' first:' }, { block: devInstall }];
  const reader = rust ? `For the v${siteRelease} crates on crates.io, ` : `For the v${siteRelease} package from PyPI, `;
  if (sample.tagPage !== undefined) {
    return [...runs, { text: `${reader}follow ` },
      { text: `this page as v${siteRelease} published it`, href: `${repository}/blob/v${siteRelease}/website/src/content/docs/${sample.tagPage}` },
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
