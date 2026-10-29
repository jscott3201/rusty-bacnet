import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { repo, splitDevSection } from './dev-section.mjs';

const root = new URL('../../', import.meta.url);
test('security page shows the exact release-pinned policy program and no dev section', async () => {
  const file = fileURLToPath(new URL('src/content/docs/integrate/security.mdx', root));
  const source = await readFile(file, 'utf8');
  const program = await readFile(new URL('examples/rust/sc-operator-policy/src/main.rs', root), 'utf8');
  const manifest = await readFile(new URL('examples/rust/sc-operator-policy/Cargo.toml', root), 'utf8');
  assert.match(source, /^import policySource from '\.\.\/\.\.\/\.\.\/\.\.\/examples\/rust\/sc-operator-policy\/src\/main\.rs\?raw';$/m);
  assert.match(source, /^import policyManifest from '\.\.\/\.\.\/\.\.\/\.\.\/examples\/rust\/sc-operator-policy\/Cargo\.toml\?raw';$/m);
  const body = await plainBody(frontmatter(source).body, file, []);
  const rust = [...body.matchAll(/^```rust\n([\s\S]*?)^```/gm)];
  const toml = [...body.matchAll(/^```toml\n([\s\S]*?)^```/gm)];
  assert.equal(rust.length, 1, 'one Rust sample, the tested program');
  assert.equal(toml.length, 1);
  assert.equal(rust[0][1], program, 'raw source bytes, including the final newline');
  assert.equal(toml[0][1], manifest, 'raw manifest bytes, including the final newline');
  for (const path of ['Cargo.toml', 'src/main.rs']) {
    assert.ok(body.includes(`https://jscott3201.github.io/rusty-bacnet/examples/rust/sc-operator-policy/${path}`), path);
  }
  for (const heading of [
    'One policy makes the server\'s decisions',
    'Know who sent the request',
    'Limit what reaches the server on BACnet/SC',
    'What Python exposes',
    'Record local access decisions',
    'Read the contracts',
    'Example: writes and DCC from one certificate name',
    'Next steps',
  ]) assert.ok(body.includes(`\n## ${heading}\n`), heading);
  // Everything is release-scoped: no dev section and no dev links.
  const { onDev } = splitDevSection(body, 'integrate/security');
  assert.equal(onDev, '', 'everything on the page is in the release');
  assert.ok(!body.includes('on-dev-after-the-release'));
  for (const doc of ['mutation-policy', 'dcc-policy', 'time-sync-policy', 'local-access-authorization']) {
    assert.ok(body.includes(`${repo}/blob/v${release}/docs/${doc}.md`), doc);
  }
  assert.ok(body.includes(`https://docs.rs/bacnet-server/${release}/bacnet_server/policy/index.html`));
  assert.ok(body.includes(`https://docs.rs/bacnet-transport/${release}/bacnet_transport/port/struct.TransportProvenance.html`));
  // The certificate accessors and the DCC decision are documented in the page body.
  for (const name of ['certificate()', 'subject_alt_names()', 'authorize_device_communication_control', 'sc_control_rate_limit', 'device_write_policy']) {
    assert.ok(body.includes(name), name);
  }
  // The manifest pins the site release, and the page says so.
  for (const crate of ['bacnet-transport', 'bacnet-types']) {
    assert.ok(manifest.includes(`${crate} = "=${release}"`), crate);
  }
  assert.ok(manifest.includes(`bacnet-server = { version = "=${release}", features = ["sc-tls"] }`));
  assert.ok(!manifest.includes('branch = '));
  assert.ok(body.includes(`The manifest pins **v${release}**, whose minimum Rust version is **1.95**.`));
  assert.ok(body.includes('`mutation_policy="permissive"`'));
  assert.ok(body.includes('Don\'t match by suffix.'));
});

