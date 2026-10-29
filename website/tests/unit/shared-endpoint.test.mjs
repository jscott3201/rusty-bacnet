import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';

const root = new URL('../../', import.meta.url);
test('shared-endpoint MDX and raw export use the complete canonical program and manifest', async () => {
  const file = fileURLToPath(new URL('src/content/docs/integrate/shared-endpoints.mdx', root));
  const source = await readFile(file, 'utf8');
  const program = await readFile(new URL('examples/rust/shared-endpoint/src/main.rs', root), 'utf8');
  const manifest = await readFile(new URL('examples/rust/shared-endpoint/Cargo.toml', root), 'utf8');
  assert.match(source, /^import endpointSource from '\.\.\/\.\.\/\.\.\/\.\.\/examples\/rust\/shared-endpoint\/src\/main\.rs\?raw';$/m);
  assert.match(source, /^import endpointManifest from '\.\.\/\.\.\/\.\.\/\.\.\/examples\/rust\/shared-endpoint\/Cargo\.toml\?raw';$/m);
  assert.ok(manifest.includes(`bacnet-endpoint = "=${release}"`), 'manifest follows the site release');
  const body = await plainBody(frontmatter(source).body, file, []);
  assert.ok(body.includes(`These files target **v${release}**`));
  const rust = [...body.matchAll(/^```rust\n([\s\S]*?)^```/gm)];
  const toml = [...body.matchAll(/^```toml\n([\s\S]*?)^```/gm)];
  assert.equal(rust.length, 1);
  assert.equal(toml.length, 1);
  assert.equal(rust[0][1], program, 'raw source bytes, including the final newline');
  assert.equal(toml[0][1], manifest, 'raw manifest bytes, including the final newline');
  for (const heading of [
    'One session owns the connection',
    'Check requester and responder scope separately',
    'Add source READ or WRITE Audit reporting deliberately',
    'Close the owner, including after cancellation',
    'Next steps',
  ]) assert.ok(body.includes(`## ${heading}`), heading);
  assert.ok(body.includes('not a loopback-confined listener'));
  assert.ok(body.includes('does **not** exercise a BACnet request/reply'));
  assert.ok(body.includes('Rust 1.99.0 and a development test harness'));
  for (const path of ['Cargo.toml', 'src/main.rs']) {
    assert.ok(body.includes(`https://jscott3201.github.io/rusty-bacnet/examples/rust/shared-endpoint/${path}`));
  }
});
