import { test } from 'node:test';
import assert from 'node:assert/strict';
import { accessibleCode, accessibleTables } from '../../scripts/accessible-content.mjs';

test('native code wrapper retains its content and gains static keyboard scrolling', () => {
  const code = { type: 'element', tagName: 'code', properties: {}, children: [{ type: 'text', value: 'unchanged' }] };
  const pre = { type: 'element', tagName: 'pre', properties: { 'data-language': 'python' }, children: [code] };
  const blockAst = { type: 'element', tagName: 'figure', properties: {}, children: [pre] };
  accessibleCode.hooks.postprocessRenderedBlock({ renderData: { blockAst }, codeBlock: { language: 'python' } });
  assert.equal(pre.properties.tabIndex, 0);
  assert.equal(pre.properties.ariaLabel, 'python code example');
  assert.equal(pre.properties['data-language'], 'python');
  assert.equal(pre.children[0], code);
  assert.equal(code.children[0].value, 'unchanged');
});

test('table scroll wrapper is keyboard focusable without replacing table semantics', () => {
  const table = { type: 'element', tagName: 'table', properties: {}, children: [] };
  const wrapper = accessibleTables.element.visit(table);
  assert.equal(wrapper.tagName, 'div');
  assert.equal(wrapper.properties.tabindex, 0);
  assert.equal(wrapper.properties.role, 'region');
  assert.equal(wrapper.children[0], table);
  assert.equal(table.tagName, 'table');
  assert.deepEqual(table.properties, {});
});
