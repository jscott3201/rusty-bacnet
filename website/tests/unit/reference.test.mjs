// The generated reference pages (#1989): reference/cli, reference/python
// (an index and one page per group) and reference/rust. Their generators
// (website/scripts/reference-*.mjs and reference-python.py) each have a
// --check mode, which the release runs (docs/ci.md); these tests don't
// compare the pages with fresh output, so a PR that adds an option or a stub
// method isn't failed for it. They check that each page has its generated
// region, parses, and links correctly, that every bacnet subcommand is on
// the CLI page, and that the crate map lists exactly the crates the release
// publishes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { cliDefinitions } from './cli-args.mjs';

const root = new URL('../../', import.meta.url);
const repo = new URL('../', root);
const docs = new URL('src/content/docs/', root);
const html = { start: '<!-- generated:start', end: '<!-- generated:end -->' };
const groups = ['client', 'server', 'router', 'endpoints', 'types', 'enums', 'errors', 'functions'];
const pages = {
  cli: { file: 'reference/cli.md', ...html },
  python: { file: 'reference/python.md', ...html },
  ...Object.fromEntries(groups.map(group => [`python/${group}`, { file: `reference/python/${group}.md`, ...html }])),
  rust: { file: 'reference/rust.mdx', start: '{/* generated:start', end: '{/* generated:end */}' },
};
const read = name => readFile(new URL(name, docs), 'utf8');
const regenerate = {
  cli: 'run node website/scripts/reference-cli.mjs',
  python: 'run python3 website/scripts/reference-python.py',
  rust: 'run node website/scripts/reference-rust.mjs',
};

/** The page and its generated region, checking the markers. */
async function load(key) {
  const { file, start, end } = pages[key];
  const text = await read(file);
  assert.equal(text.split(start).length, 2, `${file}: one generated:start marker`);
  assert.equal(text.split(end).length, 2, `${file}: one generated:end marker`);
  const region = text.slice(text.indexOf(start), text.indexOf(end));
  assert.ok(text.indexOf(start) < text.indexOf(end), `${file}: generated:start comes first`);
  assert.ok(region.split('\n').length > 20, `${file}: the generated region is filled in`);
  return { text, region };
}

// The heading slug Starlight gives a heading (github-slugger's rule), from
// its Markdown: a link counts by its text.
const slug = heading => heading.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1').trim().toLowerCase()
  .replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/\s/g, '-');
const headingsOf = body => new Set([...body.replace(/^```[^\n]*\n[\s\S]*?^```\s*$/gm, '')
  .matchAll(/^#{2,6} (.+)$/gm)].map(match => slug(match[1])));

const slugs = new Set((await readdir(docs, { recursive: true }))
  .filter(name => /\.mdx?$/.test(name))
  .map(name => name.replaceAll('\\', '/').replace(/\.mdx?$/, '').replace(/(^|\/)index$/, '')));

test('each reference page parses, exports and links to pages and headings that exist', async () => {
  const headings = new Map();
  for (const { file } of Object.values(pages)) {
    headings.set(file.replace(/\.mdx?$/, ''), headingsOf(frontmatter(await read(file)).body));
  }
  for (const [key, { file }] of Object.entries(pages)) {
    const { text } = await load(key);
    const page = frontmatter(text);
    assert.ok(page.title && page.description, `${file}: title and description`);
    const path = fileURLToPath(new URL(file, docs));
    const body = await plainBody(page.body, path, []);
    assert.doesNotMatch(body, /generated:(start|end)/, `${file}: the export leaves out the markers`);
    // Outside code, every site link names a page, and a fragment on a
    // reference page names one of its headings.
    const prose = page.body.replace(/^```[^\n]*\n[\s\S]*?^```\s*$/gm, '').replace(/`[^`\n]*`/g, '');
    for (const [, target, fragment] of prose.matchAll(/\]\(\/rusty-bacnet\/([^)#]*?)\/?(?:#([^)]*))?\)/g)) {
      assert.ok(slugs.has(target), `${file} links to /rusty-bacnet/${target}/, which is not a page`);
      if (fragment && headings.has(target)) {
        assert.ok(headings.get(target).has(fragment), `${file} links to ${target}#${fragment}, which is not a heading`);
      }
    }
    const own = headings.get(file.replace(/\.mdx?$/, ''));
    for (const [, fragment] of page.body.matchAll(/\]\(#([^)]+)\)/g)) {
      assert.ok(own.has(fragment), `${file} links to #${fragment}, which is not a heading`);
    }
  }
});

test('the generated regions name no version', async () => {
  for (const key of Object.keys(pages)) {
    const { region } = await load(key);
    const found = [...region.matchAll(/(?<![\w.])v?(0\.\d+\.\d+)(?!\.?\d)/g)].map(match => match[0]);
    assert.deepEqual(found, [], `${pages[key].file}'s generated region names a version`);
  }
});

test('each generated page says it tracks dev, not the release', async () => {
  for (const key of Object.keys(pages)) {
    const { text } = await load(key);
    const before = text.slice(0, text.indexOf(pages[key].start));
    assert.ok(before.includes(':::note[On `dev` after the release]'), `${pages[key].file}: the dev note comes before the generated region`);
  }
});

test('the CLI page has every bacnet subcommand, and marks the ones behind a feature', async () => {
  const { text, region } = await load('cli');
  for (const heading of ['## Exit status', '## Session behavior', '## Global options', '## Commands']) {
    assert.ok(text.includes(`\n${heading}\n`), `cli.md has ${heading}`);
  }
  const { commands } = await cliDefinitions();
  const sections = new Set([...region.matchAll(/^### bacnet (\S+)/gm)].map(match => match[1]));
  const aliases = new Set([...region.matchAll(/^Aliases: (.+)\.$/gm)].flatMap(match => [...match[1].matchAll(/`([^`]+)`/g)].map(m => m[1])));
  for (const command of commands) {
    assert.ok(sections.has(command) || aliases.has(command), `cli.md has no section for bacnet ${command}; ${regenerate.cli}`);
  }
  assert.ok(sections.size > 20);
  // A variant of `enum Command` behind #[cfg(feature = "...")] says so in its section.
  const args = await readFile(new URL('crates/bacnet-cli/src/args.rs', repo), 'utf8');
  const body = args.slice(args.indexOf('pub(crate) enum Command {'));
  const section = name => region.split(`### bacnet ${name}\n`)[1]?.split('\n### ')[0] ?? '';
  // Line by line: a cfg line, then any attributes, doc comments or blank
  // lines, then the variant.
  const gated = [];
  let pending;
  for (const line of body.split('\n')) {
    const trimmed = line.trim();
    const cfg = trimmed.match(/^#\[cfg\(feature = "([^"]+)"\)\]$/);
    if (cfg) {
      pending = cfg[1];
    } else if (pending !== undefined && !(trimmed === '' || trimmed.startsWith('#[') || trimmed.startsWith('///'))) {
      const variant = trimmed.match(/^([A-Z]\w*)/);
      if (variant) gated.push([pending, variant[1]]);
      pending = undefined;
    }
  }
  for (const [feature, variant] of gated) {
    const name = variant.replace(/(?<!^)[A-Z]/g, letter => `-${letter}`).toLowerCase();
    assert.ok(section(name).includes(`Needs the \`${feature}\` feature.`), `cli.md marks bacnet ${name} with the ${feature} feature; ${regenerate.cli}`);
  }
  for (const [name, feature] of [['tui', 'tui'], ['capture', 'pcap'], ['mstp-ports', 'serial']]) {
    assert.ok(section(name).includes(`Needs the \`${feature}\` feature.`), `cli.md marks bacnet ${name} with ${feature}`);
  }
});

test('the Python index links every group page, and each group page has its classes and signatures', async () => {
  const { region } = await load('python');
  for (const group of groups) {
    assert.match(region, new RegExp(`^## \\[[^\\]]+\\]\\(/rusty-bacnet/reference/python/${group}/\\)$`, 'm'), `python.md links ${group}`);
  }
  const expect = { client: 'BACnetClient', server: 'BACnetServer', router: 'BACnetRouter', endpoints: 'BipEndpoint', errors: 'BacnetError', enums: 'ObjectType', types: 'ObjectIdentifier' };
  for (const [group, name] of Object.entries(expect)) {
    const { region: page } = await load(`python/${group}`);
    assert.ok(page.includes(`\n## ${name}\n`), `python/${group}.md has ## ${name}`);
  }
  const { region: client } = await load('python/client');
  assert.match(client, /### BACnetClient\.read_property\n\n```python\ndef read_property\(/);
  const listed = (await readdir(new URL('reference/python/', docs))).filter(name => /\.mdx?$/.test(name)).sort();
  assert.deepEqual(listed, groups.map(group => `${group}.md`).sort(), `reference/python/ holds exactly the group pages; ${regenerate.python}`);
});

/** Whether version `a` (x.y.z) comes after `b`. */
const newer = (a, b) => {
  const [x, y] = [a, b].map(version => version.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
};

test('the crate map lists exactly the crates the release publishes, linked to their docs or source', async () => {
  const { text, region } = await load('rust');
  const manifest = await readFile(new URL('Cargo.toml', repo), 'utf8');
  const members = [...manifest.match(/^members = \[([\s\S]*?)^\]/m)[1].matchAll(/^\s*"([^"]+)"/gm)].map(match => match[1]);
  const published = [];
  for (const member of members) {
    const toml = await readFile(new URL(`${member}/Cargo.toml`, repo), 'utf8');
    if (/^publish = false$/m.test(toml)) continue;
    const name = toml.match(/^name = "([^"]+)"$/m)[1];
    const library = !/^\[\[bin\]\]$/m.test(toml) || /^\[lib\]$/m.test(toml);
    const first = toml.match(/^\[package\.metadata\.website\]\n(?:[^[\n][^\n]*\n)*?first-release = "([^"]+)"$/m)?.[1];
    published.push({ name, member, own: !/^version\.workspace = true$/m.test(toml), library, unreleased: first !== undefined && newer(first, release) });
  }
  const rows = [...region.matchAll(/^\| <a href=\{`([^`]+)`\}>([^<]+)<\/a>( \(unreleased\))? \|/gm)]
    .map(match => ({ href: match[1], name: match[2], unreleased: match[3] !== undefined }));
  assert.deepEqual(rows.map(row => row.name).sort(), published.map(crate => crate.name).sort(),
    `the crate map lists the published workspace crates; ${regenerate.rust}`);
  for (const crate of published) {
    const row = rows.find(row => row.name === crate.name);
    // docs.rs has no page for a crate the release didn't publish.
    const expected = crate.unreleased ? `\${repository}/tree/dev/${crate.member}`
      : !crate.library ? `https://crates.io/crates/${crate.name}`
        : `https://docs.rs/${crate.name}/${crate.own ? 'latest' : '${release}'}`;
    assert.equal(row.href, expected, `${crate.name}'s link; ${regenerate.rust}`);
    assert.equal(row.unreleased, crate.unreleased, `${crate.name} is marked unreleased exactly when it is; ${regenerate.rust}`);
    if (crate.own) assert.ok(text.includes(`\`${crate.name}\` versions on its own`), `rust.mdx says ${crate.name} versions on its own`);
  }
  assert.match(text, /^import \{ release, repository \} from '\.\.\/\.\.\/\.\.\/lib\/site\.mjs';$/m);
});
