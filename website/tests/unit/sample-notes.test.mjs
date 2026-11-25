// The samples under website/examples follow the dev branch between releases,
// and each page that shows one says so (#2648): src/lib/samples.mjs holds the
// registry and the note, components/SampleNote.astro renders it and
// scripts/content.mjs projects it into the plain export.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody, sampleManifests } from '../../scripts/content.mjs';
import { devInstall, devSource, manifestRelease, noteMarkdown, noteParts, sampleNote, samples, samplesRelease, tagUrl } from '../../src/lib/samples.mjs';
import { release, repository } from '../../src/lib/site.mjs';
import { assertManifestSource, assertSampleNotes } from './sample-notes.mjs';

const root = new URL('../../', import.meta.url);
const repo = fileURLToPath(new URL('../', root));
const docs = new URL('src/content/docs/', root);
const examples = new URL('examples/', root);
const byPath = Object.fromEntries(Object.entries(samples).map(([name, sample]) => [sample.path, name]));

test('the registry lists every sample under website/examples, by its file name', async () => {
  const rust = (await readdir(new URL('rust/', examples), { withFileTypes: true })).filter(entry => entry.isDirectory()).map(entry => `rust/${entry.name}`);
  // Python programs only: running the lab's tests leaves a __pycache__ beside them.
  const python = (await readdir(new URL('python/', examples))).filter(name => name.endsWith('.py')).map(name => `python/${name}`);
  assert.deepEqual(Object.keys(byPath).sort(), [...rust, ...python].sort());
  for (const [name, { path, language }] of Object.entries(samples)) {
    assert.equal(`${language}/${name}`, path, name);
  }
});

/** Every MDX page that imports a sample file, with the samples it imports and the notes it gives. */
async function samplePages() {
  const names = (await readdir(docs, { recursive: true })).filter(name => /\.mdx?$/.test(name)).map(name => name.replaceAll('\\', '/'));
  const pages = [];
  for (const name of names) {
    const source = await readFile(new URL(name, docs), 'utf8');
    const shown = [...new Set([...source.matchAll(/^import \w+ from '(?:\.\.\/)+examples\/(rust\/[^/']+|python\/[^/?']+)[^']*\?raw';?$/gm)]
      .map(match => byPath[match[1]] ?? assert.fail(`${name} imports an unregistered sample, ${match[1]}`)))];
    // A note is a component on its own line; a code span may name the component.
    const notes = [...source.matchAll(/^<SampleNote\b.*$/gm)].map(match => match[0].match(/^<SampleNote sample="([^"]+)" \/>$/)?.[1] ?? assert.fail(`${name}: ${match[0]}`));
    if (!name.endsWith('.mdx')) assert.deepEqual([shown, notes], [[], []], `${name}: samples need MDX`);
    if (shown.length || notes.length) pages.push({ name, source, shown, notes });
  }
  return pages;
}

test('every page that shows a sample gives its note once, and no other', async () => {
  const pages = await samplePages();
  for (const { name, shown, notes } of pages) {
    assert.deepEqual([...notes].sort(), [...shown].sort(), name);
  }
  // Every sample is on a page, so every note is checked below.
  assert.deepEqual(pages.flatMap(page => page.shown).sort(), Object.keys(samples).sort());
});

test('each page\'s plain export carries its notes, and the manifests it shows follow dev or pin the release', async () => {
  for (const { name, source, notes } of await samplePages()) {
    const file = fileURLToPath(new URL(name, docs));
    const body = await plainBody(frontmatter(source).body, file, []);
    await assertSampleNotes(body, notes, name);
    // Each imported sample file is shown as it is.
    for (const [, path] of source.matchAll(/^import \w+ from '(?:\.\.\/)+(examples\/[^']+)\?raw';?$/gm)) {
      const text = await readFile(new URL(path, root), 'utf8');
      assert.ok(body.includes(`\n${text.trim()}\n\`\`\``), `${name}: ${path}`);
    }
  }
  const manifests = await sampleManifests();
  assert.deepEqual(Object.keys(manifests).sort(), Object.values(samples).filter(sample => sample.language === 'rust').map(sample => sample.path).sort());
  for (const [path, manifest] of Object.entries(manifests)) assertManifestSource(manifest, path);
  // All alike: the release script pins and unpins them together.
  samplesRelease(manifests);
});

test('a note follows the manifests: dev, the release, or new since the release', () => {
  const dev = `[package]\nname = "x"\nrust-version = "1.95"\n\n[workspace]\n\n[dependencies]\nbacnet-types = { ${devSource} }\nbacnet-server = { ${devSource}, features = ["sc-tls"] }\ntokio = "1"\n`;
  const pinned = version => dev.replace(`{ ${devSource} }`, `"=${version}"`).replace(`{ ${devSource}, `, `{ version = "=${version}", `);
  const registry = {
    x: { path: 'rust/x', language: 'rust' },
    'x.py': { path: 'python/x.py', language: 'python' },
    y: { path: 'rust/y', language: 'rust', since: '1.3.0' },
    // New as a file, but a page showed the release's version inline.
    'y.py': { path: 'python/y.py', language: 'python', since: '1.3.0', tagPage: 'start/y.md' },
    z: { path: 'python/z.py', language: 'python', tagPage: 'start/z.md' },
  };
  const note = (name, manifests, siteRelease = '1.2.0') => noteMarkdown(sampleNote(name, manifests, { siteRelease, registry }));
  const onDev = { 'rust/x': dev, 'rust/y': dev };
  const rustDev = 'This sample follows the `dev` branch: its manifest takes the BACnet crates from `dev` on GitHub, '
    + 'so these files build as shown, with Rust **1.95** or later. ';
  const pythonDev = 'This program follows the `dev` branch. Install `rusty-bacnet` from `dev` in a virtual environment; '
    + 'pip builds its native extension, so install [Rust](https://rustup.rs) first:\n\n```sh\n'
    + `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"\n\`\`\`\n\n`;
  assert.equal(devInstall, `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"`);
  assert.equal(manifestRelease(dev), null);
  assert.equal(manifestRelease(pinned('1.2.0')), '1.2.0');
  assert.equal(note('x', onDev), `${rustDev}For the v1.2.0 crates on crates.io, use `
    + `[the same sample at v1.2.0](${repository}/tree/v1.2.0/website/examples/rust/x).`);
  assert.equal(note('x.py', onDev), `${pythonDev}For the v1.2.0 package from PyPI, use `
    + `[the same program at v1.2.0](${repository}/blob/v1.2.0/website/examples/python/x.py).`);
  assert.equal(note('y', onDev), `${rustDev}It is new since v1.2.0, so no release has it yet.`);
  assert.equal(note('y.py', onDev), `${pythonDev}For the v1.2.0 package from PyPI, follow `
    + `[this page as v1.2.0 published it](${repository}/blob/v1.2.0/website/src/content/docs/start/y.md) instead.`);
  const atRelease = { 'rust/x': pinned('1.3.0') };
  assert.equal(note('x', atRelease, '1.3.0'), 'The manifest pins **v1.3.0** of the BACnet crates on crates.io, whose minimum Rust version is **1.95**.');
  assert.equal(note('x.py', atRelease, '1.3.0'), 'This program runs with the v1.3.0 `rusty-bacnet` package. Install it in a virtual environment:'
    + '\n\n```sh\npython -m pip install rusty-bacnet==1.3.0\n```\n\n');
  // A note is paragraphs and command blocks.
  assert.deepEqual(noteParts(sampleNote('x.py', atRelease, { siteRelease: '1.3.0', registry })).map(part => Object.keys(part)[0]), ['inline', 'block']);
  assert.deepEqual(noteParts(sampleNote('y.py', onDev, { siteRelease: '1.2.0', registry })).map(part => Object.keys(part)[0]), ['inline', 'block', 'inline']);
  // The release that ships a new sample removes its `since` and `tagPage`.
  assert.throws(() => note('y', { 'rust/y': pinned('1.3.0') }, '1.3.0'), /remove its since and tagPage keys/);
  assert.throws(() => note('z', onDev), /tagPage goes with since/);
  assert.throws(() => note('w', onDev), /Unknown sample w/);
  // The manifests move together, and only to the site's release.
  assert.throws(() => note('x', { 'rust/x': dev, 'rust/y': pinned('1.2.0') }), /disagree/);
  assert.throws(() => note('x', { 'rust/x': pinned('1.1.0') }), /pin 1\.1\.0, but the site describes 1\.2\.0/);
  for (const [manifest, error] of [
    [dev.replace(`{ ${devSource} }`, '"=1.2.0"'), /mixes/],
    [dev.replace(`{ ${devSource} }`, '"1.2"'), /neither the dev branch/],
    [dev.replace(`{ ${devSource} }`, `{ git = "${repository}", branch = "main" }`), /neither the dev branch/],
    [dev.replace(`{ ${devSource} }`, `{ ${devSource}, path = "../local" }`), /neither the dev branch/],
    [dev.replace('"sc-tls"] }', '"sc-tls"], rev = "0123456" }'), /neither the dev branch/],
    [`${dev}\n[patch.crates-io]\nbacnet-types = { path = "../local" }\n`, /not \[dependencies\]/],
    [dev.replace(/^bacnet-.*\n/gm, ''), /names no BACnet crate/],
  ]) {
    assert.throws(() => manifestRelease(manifest), error, manifest);
  }
});

const git = (...args) => spawnSync('git', args, { cwd: repo, encoding: 'utf8' });

// The Docs workflow fetches the tag; a shallow local clone may lack it.
test('each sample a note links is at the release tag, and its manifest there pins the release', async t => {
  if (samplesRelease(await sampleManifests()) !== null) return t.skip('the samples pin the release, so no note links a tag');
  const tag = `v${release}`;
  if (git('rev-parse', '--verify', '--quiet', `${tag}^{commit}`).status !== 0) {
    assert.ok(!process.env.CI, `CI must fetch ${tag} (.github/workflows/docs-pages.yml)`);
    return t.skip(`${tag} isn't in this clone; git fetch origin tag ${tag}`);
  }
  for (const [name, sample] of Object.entries(samples)) {
    assert.equal(tagUrl(sample), `${repository}/${sample.language === 'rust' ? 'tree' : 'blob'}/${tag}/website/examples/${sample.path}`);
    const files = sample.language === 'rust' ? [`${sample.path}/Cargo.toml`, `${sample.path}/src/main.rs`] : [sample.path];
    for (const file of files) {
      const there = git('cat-file', '-e', `${tag}:website/examples/${file}`).status === 0;
      assert.equal(there, sample.since === undefined, `${name}: ${file} ${there ? 'is' : 'is not'} at ${tag}`);
    }
    // A new sample's note links the page that showed the release's version.
    if (sample.tagPage !== undefined) {
      const page = `website/src/content/docs/${sample.tagPage}`;
      assert.equal(git('cat-file', '-e', `${tag}:${page}`).status, 0, `${name}: ${page} at ${tag}`);
    }
    if (sample.language === 'rust' && sample.since === undefined) {
      const manifest = git('show', `${tag}:website/examples/${sample.path}/Cargo.toml`);
      assert.equal(manifest.status, 0, manifest.stderr);
      assert.equal(manifestRelease(manifest.stdout), release, `${name}: the manifest at ${tag} pins it`);
    }
  }
});
