import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sitePath } from '../../src/lib/site.mjs';
test('project prefix is preserved for pages and public assets', () => {
  assert.equal(sitePath(), '/rusty-bacnet/');
  assert.equal(sitePath('start/local-lab/'), '/rusty-bacnet/start/local-lab/');
  assert.equal(sitePath('/diagrams/discovery-path.svg'), '/rusty-bacnet/diagrams/discovery-path.svg');
});
test('local path helper rejects external URLs and traversal', () => {
  for (const value of ['https://example.com', '//example.com', '../private']) {
    assert.throws(() => sitePath(value));
  }
});
