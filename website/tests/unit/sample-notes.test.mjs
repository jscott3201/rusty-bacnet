// Each sample under website/examples stays on the release while its files are
// the release's, and follows the dev branch once they change; each page that
// shows one says which (#2648). src/data/samples.json lists them,
// src/lib/samples.mjs holds the note, components/SampleNote.astro renders it
// and scripts/content.mjs projects it into the plain export.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody, sampleManifests } from '../../scripts/content.mjs';
import { devInstall, devSource, manifestState, noteMarkdown, noteParts, pythonMinimum, sampleNote, samples, tagUrl } from '../../src/lib/samples.mjs';
import { release, repository } from '../../src/lib/site.mjs';
import { assertManifestSource, assertSampleNotes, followsDev, workspaceVersion } from './sample-notes.mjs';

const root = new URL('../../', import.meta.url);
const repo = fileURLToPath(new URL('../', root));
const docs = new URL('src/content/docs/', root);
const examples = new URL('examples/', root);
const byPath = Object.fromEntries(Object.entries(samples).map(([name, sample]) => [sample.path, name]));

test('the registry lists every sample under website/examples, by its file name, as the release script writes it', async () => {
  const rust = (await readdir(new URL('rust/', examples), { withFileTypes: true })).filter(entry => entry.isDirectory()).map(entry => `rust/${entry.name}`);
  // Python programs only: running the lab's tests leaves a __pycache__ beside them.
  const python = (await readdir(new URL('python/', examples))).filter(name => name.endsWith('.py')).map(name => `python/${name}`);
  assert.deepEqual(Object.keys(byPath).sort(), [...rust, ...python].sort());
  for (const [name, { path, language }] of Object.entries(samples)) {
    assert.equal(`${language}/${name}`, path, name);
  }
  // scripts/release/website_samples.py rewrites it with json.dumps(indent=2).
  const text = await readFile(new URL('src/data/samples.json', root), 'utf8');
  assert.equal(text, `${JSON.stringify(JSON.parse(text), null, 2)}\n`);
});

test('the pip build\'s minimums are the workspace\'s', async () => {
  const pyproject = await readFile(new URL('crates/rusty-bacnet/pyproject.toml', new URL('../', root)), 'utf8');
  assert.equal(pyproject.match(/^requires-python = ">=([0-9.]+)"$/m)?.[1], pythonMinimum);
  const workspace = await readFile(new URL('Cargo.toml', new URL('../', root)), 'utf8');
  const msrv = workspace.match(/^rust-version = "([^"]+)"$/m)[1];
  for (const [path, manifest] of Object.entries(await sampleManifests())) {
    assert.equal(manifest.match(/^rust-version = "([^"]+)"$/m)?.[1], msrv, path);
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

test('each page\'s plain export carries its notes and files, and each manifest follows dev or pins the release', async () => {
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
  for (const [path, manifest] of Object.entries(manifests)) await assertManifestSource(manifest, path);
});

// Its prose describes the script's calls, so it says they are newer than the
// release exactly while the script follows dev; a release that pins the
// script rewrites the sentence.
test('the Python start page says its calls are newer than the release exactly while its script follows dev', async () => {
  const page = await readFile(new URL('start/python.mdx', docs), 'utf8');
  assert.equal(page.includes('are newer than v{release}'), followsDev('first_read.py', await sampleManifests()),
    'start/python.mdx says its calls are newer than the release exactly while first_read.py follows dev: rewrite its "Sources and release scope" sentence');
});

test('a note follows its sample: the release, dev, or new since the release', () => {
  const dev = `[package]\nname = "x"\nrust-version = "1.95"\n\n[workspace]\n\n[dependencies]\nbacnet-types = { ${devSource} }\nbacnet-server = { ${devSource}, features = ["sc-tls"] }\ntokio = "1"\n`;
  const pinned = version => dev.replace(`{ ${devSource} }`, `"=${version}"`).replace(`{ ${devSource}, `, `{ version = "=${version}", `);
  const registry = {
    x: { path: 'rust/x', language: 'rust' },
    'x.py': { path: 'python/x.py', language: 'python', follows: 'dev' },
    'p.py': { path: 'python/p.py', language: 'python' },
    y: { path: 'rust/y', language: 'rust', since: '1.3.0' },
    // A file now, but a page showed the release's version inline.
    'y.py': { path: 'python/y.py', language: 'python', follows: 'dev', tagPage: 'start/y.md' },
    'w.py': { path: 'python/w.py', language: 'python', since: '1.3.0' },
    'v.py': { path: 'python/v.py', language: 'python', follows: 'main' },
  };
  const note = (name, manifests, siteRelease = '1.2.0') => noteMarkdown(sampleNote(name, manifests, { siteRelease, registry }));
  const onDev = { 'rust/x': dev, 'rust/y': dev };
  const rustDev = 'This sample follows the `dev` branch: its manifest takes the BACnet crates from `dev` on GitHub, with Rust **1.95** or later. '
    + '`dev` moves: Cargo builds the commit it first fetches, which it records in `Cargo.lock`, and `cargo update` takes the newest one, '
    + 'which may no longer match the files shown here. ';
  const pythonDev = 'This program follows the `dev` branch. Install `rusty-bacnet` from `dev` in a virtual environment. pip builds it from source, '
    + 'which takes several minutes and needs git, Python 3.11 or later, [Rust](https://rustup.rs) 1.95 or later, and CMake and a C toolchain for '
    + "[BACnet/SC's TLS library](/rusty-bacnet/integrate/security/#save-the-program-and-manifest):\n\n```sh\n"
    + `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"\n\`\`\`\n\n`
    + '`dev` moves: each install builds its newest commit, which may no longer match the program shown here. ';
  assert.equal(devInstall, `python -m pip install "rusty-bacnet @ git+${repository}@dev#subdirectory=crates/rusty-bacnet"`);
  assert.equal(manifestState(dev), null);
  assert.deepEqual(manifestState(pinned('1.2.0')), { 'bacnet-types': '1.2.0', 'bacnet-server': '1.2.0' });
  // Each crate keeps its own published version.
  assert.deepEqual(manifestState(pinned('1.2.0').replace('"=1.2.0"', '"=0.9.1"')), { 'bacnet-types': '0.9.1', 'bacnet-server': '1.2.0' });
  assert.equal(note('x', onDev), `${rustDev}For the v1.2.0 crates on crates.io, use `
    + `[the same sample at v1.2.0](${repository}/tree/v1.2.0/website/examples/rust/x).`);
  assert.equal(note('x.py', onDev), `${pythonDev}For the v1.2.0 package from PyPI, use `
    + `[the same program at v1.2.0](${repository}/blob/v1.2.0/website/examples/python/x.py).`);
  assert.equal(note('y', onDev), `${rustDev}It is new since v1.2.0, so no release has it yet.`);
  assert.equal(note('y.py', onDev), `${pythonDev}For the v1.2.0 package from PyPI, follow `
    + `[this page as published at v1.2.0 (source)](${repository}/blob/v1.2.0/website/src/content/docs/start/y.md) instead.`);
  // A sample that is the release's stays on it, one by one.
  const mixed = { 'rust/x': pinned('1.2.0'), 'rust/y': dev };
  assert.equal(note('x', mixed), 'The manifest pins the **v1.2.0** BACnet crates from crates.io, whose minimum Rust version is **1.95**.');
  assert.equal(note('p.py', mixed), 'This program runs with the v1.2.0 `rusty-bacnet` package. Install it in a virtual environment:'
    + '\n\n```sh\npython -m pip install --only-binary=:all: rusty-bacnet==1.2.0\n```\n\n');
  assert.ok(note('y', mixed).startsWith(rustDev));
  // A note is paragraphs and command blocks.
  assert.deepEqual(noteParts(sampleNote('p.py', mixed, { siteRelease: '1.2.0', registry })).map(part => Object.keys(part)[0]), ['inline', 'block']);
  assert.deepEqual(noteParts(sampleNote('y.py', onDev, { siteRelease: '1.2.0', registry })).map(part => Object.keys(part)[0]), ['inline', 'block', 'inline']);
  // The release that ships a new sample removes its `since`; until then it follows dev.
  assert.throws(() => note('y', { 'rust/y': dev }, '1.3.0'), /remove its since key/);
  assert.throws(() => note('w.py', onDev), /follows dev and has no tagPage/);
  assert.throws(() => note('y', { 'rust/y': pinned('1.2.0') }), /follows dev and has no tagPage/);
  assert.throws(() => note('v.py', onDev), /follows is "dev" or absent/);
  assert.throws(() => note('z', onDev), /Unknown sample z/);
  for (const [manifest, error] of [
    [dev.replace(`{ ${devSource} }`, '"=1.2.0"'), /pins 1 BACnet crates and takes 1 from dev/],
    [dev.replace(`{ ${devSource} }`, '"1.2"'), /neither the dev branch/],
    [dev.replace(`{ ${devSource} }`, `{ git = "${repository}", branch = "main" }`), /neither the dev branch/],
    [dev.replace(`{ ${devSource} }`, `{ ${devSource}, path = "../local" }`), /neither the dev branch/],
    [dev.replace('"sc-tls"] }', '"sc-tls"], rev = "0123456" }'), /neither the dev branch/],
    [`${dev}\n[patch.crates-io]\nbacnet-types = { path = "../local" }\n`, /not \[dependencies\]/],
    [`${dev}bacnet-client.workspace = true\n`, /name a BACnet crate as bacnet-x = /],
    [`${dev}\n[dependencies.bacnet-client]\nversion = "=1.2.0"\n`, /on one line of \[dependencies\]/],
    [dev.replace(/^bacnet-.*\n/gm, ''), /names no BACnet crate/],
  ]) {
    assert.throws(() => manifestState(manifest), error, manifest);
  }
});

const git = (...args) => spawnSync('git', args, { cwd: repo, encoding: 'utf8' });
const fence = (page, language) => [...page.matchAll(new RegExp(`^\`\`\`${language}\\n([\\s\\S]*?)^\`\`\``, 'gm'))].map(match => match[1]);

/**
 * Every tracked file of a sample, by its path under website/examples: a local
 * `cargo run` leaves a target/ and a Cargo.lock that are not the sample's.
 */
function sampleFiles(sample) {
  const listed = git('ls-files', '-z', '--', `website/examples/${sample.path}`);
  assert.equal(listed.status, 0, listed.stderr);
  return listed.stdout.split('\0').filter(Boolean).map(file => file.slice('website/examples/'.length)).sort();
}

/** A manifest as the release pins it: a crate from dev at the version the workspace gives it. */
async function releasedForm(manifest) {
  const lines = [];
  for (const line of manifest.split('\n')) {
    const match = line.match(new RegExp(`^(bacnet-[a-z0-9-]+) = \\{ ${devSource.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?:, (.*))? \\}$`));
    if (!match) {
      lines.push(line);
      continue;
    }
    const version = await workspaceVersion(match[1]);
    lines.push(`${match[1]} = ${match[2] ? `{ version = "=${version}", ${match[2]} }` : `"=${version}"`}`);
  }
  return lines.join('\n');
}

// The Docs workflow fetches the tag; a shallow local clone may lack it.
test('a sample on the release is its files at the tag; one on dev differs from them or is new', async t => {
  const manifests = await sampleManifests();
  const tag = `v${release}`;
  if (git('rev-parse', '--verify', '--quiet', `${tag}^{commit}`).status !== 0) {
    // A release PR pins every sample before its tag exists, and no note links
    // one. The Docs workflow fails when the tag exists but its fetch does, so
    // in CI this branch means the tag isn't made yet.
    if (Object.keys(samples).every(name => !followsDev(name, manifests))) {
      for (const [name, sample] of Object.entries(samples)) {
        assert.ok(sample.tagPage === undefined && sample.since === undefined,
          `${name}: v${release} ships it as a file, so remove its tagPage and since keys (website_samples.py --pin ${release} removes them)`);
      }
      return t.skip(`every sample is on ${release}, whose tag is not made yet`);
    }
    assert.ok(!process.env.CI, `${tag} is not in this checkout: if this is the release PR, put every sample on the release with `
      + `scripts/release/website_samples.py --pin ${release}; otherwise the Docs workflow must fetch ${tag} (.github/workflows/docs-pages.yml)`);
    return t.skip(`${tag} isn't in this clone; git fetch origin tag ${tag}`);
  }
  const atTag = path => {
    const shown = git('show', `${tag}:${path}`);
    return shown.status === 0 ? shown.stdout : undefined;
  };
  for (const [name, sample] of Object.entries(samples)) {
    assert.equal(tagUrl(sample), `${repository}/${sample.language === 'rust' ? 'tree' : 'blob'}/${tag}/website/examples/${sample.path}`);
    const onDev = followsDev(name, manifests);
    // A sample the release doesn't have as a file says where else it was, or that it is new;
    // otherwise its note's link to the release's copy would lead nowhere.
    const program = sample.language === 'rust' ? `${sample.path}/src/main.rs` : sample.path;
    assert.ok(atTag(`website/examples/${program}`) !== undefined || sample.tagPage !== undefined || sample.since !== undefined,
      `${name} is not at ${tag}: give it "since": "<the next release>" in src/data/samples.json`);
    const same = [];
    for (const file of sampleFiles(sample)) {
      const released = atTag(`website/examples/${file}`);
      assert.ok(released === undefined || (sample.tagPage === undefined && sample.since === undefined),
        `${name}: ${file} is at ${tag}, so remove its tagPage or since key`);
      // A manifest compares in the form the release gives it.
      let text = await readFile(new URL(file, examples), 'utf8');
      if (file.endsWith('/Cargo.toml')) text = await releasedForm(text);
      same.push(released === text);
    }
    if (sample.since !== undefined) continue;
    let unchanged = same.every(Boolean);
    if (sample.tagPage !== undefined) {
      // The release showed the program inline on that page.
      const page = atTag(`website/src/content/docs/${sample.tagPage}`);
      assert.ok(page !== undefined, `${name}: ${sample.tagPage} at ${tag}`);
      const shown = fence(page, sample.language);
      assert.equal(shown.length, 1, `${name}: one ${sample.language} block on ${sample.tagPage} at ${tag}`);
      // Only trailing whitespace may differ: a block ended with a blank line
      // before its closing fence, where a file ends with one newline.
      unchanged = shown[0].trimEnd() === (await readFile(new URL(program, examples), 'utf8')).trimEnd();
    }
    if (onDev) {
      assert.ok(!unchanged, `${name} is the release's: put it back on ${release} with scripts/release/website_samples.py --pin ${release} ${name}`);
    } else {
      assert.ok(unchanged, `${name} differs from ${tag}: run scripts/release/website_samples.py --follow-dev ${name}`);
    }
  }
});
