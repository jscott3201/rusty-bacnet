// Generate the Rust crate map from `cargo metadata` (#1989).
//
// Rewrites the region between the generated:start and generated:end markers
// of src/content/docs/reference/rust.mdx; the text around them is authored.
// The crates are the workspace members crates.io may take (`publish` not
// `[]`), with each one's description, features and workspace dependencies.
// docs.rs links take the release from src/lib/site.mjs at build time, so the
// page names no version itself.
//
//   node website/scripts/reference-rust.mjs          # rewrite the page
//   node website/scripts/reference-rust.mjs --check  # exit 1 if stale
//
// LAYERS places each crate. A published crate missing from it, or a name in
// it that isn't one, fails the run, so a new crate gets a
// deliberate place.
//
// A crate first published after the release in site.mjs sets
// `[package.metadata.website] first-release = "x.y.z"` in its Cargo.toml; it
// is listed as unreleased, linked to its source on dev, since docs.rs has no
// page for it yet.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { release } from '../src/lib/site.mjs';

const repo = fileURLToPath(new URL('../../', import.meta.url));
const page = fileURLToPath(new URL('../src/content/docs/reference/rust.mdx', import.meta.url));
const START = '{/* generated:start';
const END = '{/* generated:end */}';

/** The layers, bottom up: heading, what they hold, and their crates in page order. */
export const LAYERS = [
  ['Portable cores', '`no_std` crates that need no allocator: the clock, the network-layer vocabulary, the frame codecs and the sans-IO MS/TP, B/IP and router engines that firmware and the hosted transports share.',
    ['bacnet-time', 'bacnet-npdu', 'bacnet-mstp-codec', 'bacnet-bip-codec', 'bacnet-mstp-core', 'bacnet-bip-core', 'bacnet-router-core']],
  ['Application layer', 'Types, the APDU codec and the service requests and responses. They build with `std`, or `no_std` with or without an allocator.',
    ['bacnet-types', 'bacnet-encoding', 'bacnet-services']],
  ['Hosted network', 'The Tokio data links and the network layer with its router.',
    ['bacnet-transport', 'bacnet-network']],
  ['Objects and roles', 'The object database, the client and server roles, and the shared endpoint that runs both on one transport.',
    ['bacnet-objects', 'bacnet-endpoint-core', 'bacnet-client', 'bacnet-server', 'bacnet-endpoint']],
  ['Tools', 'The operations core, the MCP server for AI agent hosts, and the command-line tool.',
    ['bacnet-ops', 'bacnet-mcp', 'bacnet-cli']],
];

/** Whether version `a` (x.y.z) comes after `b`. */
export function newer(a, b) {
  const [x, y] = [a, b].map(version => version.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

/** Extra authored text for a crate's purpose cell. */
const NOTES = {
  'bacnet-cli': 'A program, not a library: its commands are in the [CLI reference](/rusty-bacnet/reference/cli/).',
};

function metadata() {
  const json = execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps', '--locked'], { cwd: repo, encoding: 'utf8', maxBuffer: 64 << 20 });
  return JSON.parse(json);
}

/** Escape text for an MDX table cell. */
const cell = text => text.replace(/\s+/g, ' ').replace(/[|{}<>]/g, char => `\\${char}`).trim();
/** End a description with a full stop. */
const sentence = text => text && !/[.!?]$/.test(text) ? `${text}.` : text;

export function render(meta) {
  const members = new Set(meta.workspace_members);
  const published = meta.packages.filter(pkg => members.has(pkg.id) && pkg.publish === null);
  const byName = new Map(published.map(pkg => [pkg.name, pkg]));
  const placed = LAYERS.flatMap(([, , crates]) => crates);
  const unplaced = [...byName.keys()].filter(name => !placed.includes(name));
  if (unplaced.length) throw new Error(`add ${unplaced.join(', ')} to LAYERS in reference-rust.mjs`);
  const unknown = placed.filter(name => !byName.has(name));
  if (unknown.length) throw new Error(`LAYERS names ${unknown.join(', ')}, not a published workspace crate`);

  const out = [];
  for (const [title, blurb, crates] of LAYERS) {
    out.push(`## ${title}`, '', blurb, '', '| Crate | Purpose | Features | Depends on |', '|---|---|---|---|');
    for (const name of crates) {
      const pkg = byName.get(name);
      const first = pkg.metadata?.website?.['first-release'];
      const unreleased = first !== undefined && newer(first, release);
      const source = relative(repo, pkg.manifest_path).replaceAll('\\', '/').replace(/\/Cargo\.toml$/, '');
      const library = pkg.targets.some(target => target.kind.includes('lib'));
      // A crate with its own version (not version.workspace) versions on its
      // own (#1768); its link opens the latest docs.rs build.
      const manifest = readFileSync(pkg.manifest_path, 'utf8');
      const own = !/^version\.workspace = true$/m.test(manifest);
      const link = unreleased
        ? `<a href={\`\${repository}/tree/dev/${source}\`}>${name}</a> (unreleased)`
        : library
          ? `<a href={\`https://docs.rs/${name}/${own ? 'latest' : '${release}'}\`}>${name}</a>`
          : `<a href={\`https://crates.io/crates/${name}\`}>${name}</a>`;
      const defaults = new Set(pkg.features.default ?? []);
      const features = Object.keys(pkg.features).filter(feature => feature !== 'default').sort()
        .map(feature => `\`${feature}\`${defaults.has(feature) ? ' (default)' : ''}`);
      const deps = [...new Set(pkg.dependencies.filter(dep => dep.kind === null && byName.has(dep.name)).map(dep => dep.name))].sort()
        .map(dep => `\`${dep}\``);
      const later = unreleased ? 'Merged on `dev` after the release, so not on crates.io or docs.rs yet; the link opens its source.' : '';
      const purpose = [sentence(cell(pkg.description ?? '')), NOTES[name], later].filter(Boolean).join(' ');
      out.push(`| ${link} | ${purpose} | ${features.join(', ') || '—'} | ${deps.join(', ') || '—'} |`);
    }
    out.push('');
  }
  while (out.at(-1) === '') out.pop();
  return out.join('\n');
}

export function splice(text, region) {
  const start = text.indexOf(START);
  if (start < 0) throw new Error(`${page}: no generated:start marker`);
  const opened = text.indexOf('*/}', start) + 3;
  const end = text.indexOf(END, opened);
  if (opened < 3 || end < 0) throw new Error(`${page}: no generated:end marker after generated:start`);
  return `${text.slice(0, opened)}\n\n${region}\n${text.slice(end)}`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const check = process.argv.includes('--check');
  const current = readFileSync(page, 'utf8');
  const fresh = splice(current, render(metadata()));
  if (check) {
    if (fresh !== current) {
      console.error('website/src/content/docs/reference/rust.mdx is stale; run node website/scripts/reference-rust.mjs');
      process.exit(1);
    }
    console.log('website/src/content/docs/reference/rust.mdx is current');
  } else if (fresh !== current) {
    writeFileSync(page, fresh);
    console.log('wrote website/src/content/docs/reference/rust.mdx');
  } else {
    console.log('website/src/content/docs/reference/rust.mdx is current');
  }
}
