// The one rule for a page that shows a sample from website/examples (#2648):
// it gives the sample's note with <SampleNote sample="…" />, from
// src/lib/samples.mjs. Between releases the note says the sample follows the
// dev branch and links the same sample at the release's tag; at a release,
// that the manifest pins it.
import assert from 'node:assert/strict';
import { sampleManifests } from '../../scripts/content.mjs';
import { devSource, manifestRelease, noteMarkdown, sampleNote, samples, samplesRelease, tagUrl } from '../../src/lib/samples.mjs';
import { release } from '../../src/lib/site.mjs';

const site = 'https://jscott3201.github.io/rusty-bacnet/';

/** Check that a page's plain-text body gives each named sample's note once. */
export async function assertSampleNotes(body, names, label) {
  const manifests = await sampleManifests();
  const pinned = samplesRelease(manifests);
  for (const name of names) {
    const note = noteMarkdown(sampleNote(name, manifests)).replaceAll('](/rusty-bacnet/', `](${site}`);
    assert.equal(body.split(note).length - 1, 1, `${label}: ${name}'s note, once`);
    if (pinned !== null) {
      assert.ok(note.includes(`v${release}`), `${label}: ${name}'s note names the release`);
    } else if (samples[name].since === undefined) {
      assert.ok(note.includes('follows the `dev` branch') && note.includes(`](${tagUrl(samples[name])})`), `${label}: ${name} follows dev and links its tag`);
    } else if (samples[name].tagPage !== undefined) {
      assert.ok(note.includes('follows the `dev` branch') && note.includes(`/blob/v${release}/website/src/content/docs/${samples[name].tagPage})`),
        `${label}: ${name} follows dev and links the page at its tag`);
    } else {
      assert.ok(note.includes('follows the `dev` branch') && note.includes(`new since v${release}`), `${label}: ${name} is new`);
    }
  }
}

/** Check that a manifest takes every BACnet crate from dev, or pins the site's release. */
export function assertManifestSource(manifest, label) {
  const pinned = manifestRelease(manifest);
  const crates = manifest.split('\n').filter(line => line.startsWith('bacnet-'));
  assert.ok(crates.length > 0, label);
  for (const line of crates) {
    assert.ok(pinned === null ? line.includes(`{ ${devSource}`) : line.includes(`"=${release}"`), `${label}: ${line}`);
  }
}
