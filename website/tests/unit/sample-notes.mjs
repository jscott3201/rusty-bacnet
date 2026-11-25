// The one rule for a page that shows a sample from website/examples (#2648):
// it gives the sample's note with <SampleNote sample="…" />, from
// src/lib/samples.mjs. A sample changed since the release follows the dev
// branch, and its note says so and where the release's copy is; a sample that
// is the release's stays on it, and its note says how to get the release.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { sampleManifests } from '../../scripts/content.mjs';
import { devSource, manifestState, noteMarkdown, sampleNote, samples, tagUrl } from '../../src/lib/samples.mjs';
import { release } from '../../src/lib/site.mjs';

const site = 'https://jscott3201.github.io/rusty-bacnet/';
const repo = new URL('../../../', import.meta.url);

/** Whether a sample follows dev: a Rust sample's manifest says so, a Python sample's `follows` key. */
export function followsDev(name, manifests) {
  const sample = samples[name];
  return sample.language === 'rust' ? manifestState(manifests[sample.path]) === null : sample.follows === 'dev';
}

/** Check that a page's plain-text body gives each named sample's note once. */
export async function assertSampleNotes(body, names, label) {
  const manifests = await sampleManifests();
  for (const name of names) {
    const sample = samples[name];
    const note = noteMarkdown(sampleNote(name, manifests)).replaceAll('](/rusty-bacnet/', `](${site}`);
    assert.equal(body.split(note).length - 1, 1, `${label}: ${name}'s note, once`);
    if (!followsDev(name, manifests)) {
      assert.ok(note.includes(`v${release}`), `${label}: ${name}'s note names the release`);
      continue;
    }
    assert.ok(note.includes('follows the `dev` branch'), `${label}: ${name} follows dev`);
    const where = sample.tagPage !== undefined ? `/blob/v${release}/website/src/content/docs/${sample.tagPage})`
      : sample.since !== undefined ? `new since v${release}` : `](${tagUrl(sample)})`;
    assert.ok(note.includes(where), `${label}: ${name}'s note says where the release's copy is`);
  }
}

/** The version the workspace gives a crate: its own, or the workspace's. */
export async function workspaceVersion(crate) {
  const manifest = await readFile(new URL(`crates/${crate}/Cargo.toml`, repo), 'utf8');
  if (/^version\.workspace = true$/m.test(manifest)) {
    const root = await readFile(new URL('Cargo.toml', repo), 'utf8');
    return root.slice(root.indexOf('[workspace.package]')).match(/^version = "([^"]+)"$/m)[1];
  }
  return manifest.match(/^version = "([^"]+)"$/m)[1];
}

/** Check that a manifest takes every BACnet crate from dev, or pins each to its published version. */
export async function assertManifestSource(manifest, label) {
  const pinned = manifestState(manifest);
  const crates = manifest.split('\n').filter(line => line.startsWith('bacnet-'));
  assert.ok(crates.length > 0, label);
  if (pinned === null) {
    for (const line of crates) assert.ok(line.includes(`{ ${devSource}`), `${label}: ${line}`);
    return;
  }
  for (const [crate, version] of Object.entries(pinned)) {
    assert.equal(version, await workspaceVersion(crate), `${label}: ${crate} is pinned to its published version`);
  }
}
