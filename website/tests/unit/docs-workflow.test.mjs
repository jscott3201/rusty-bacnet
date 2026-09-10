import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { parseDocument } from 'yaml';

const source = readFileSync(new URL('../../../.github/workflows/docs-pages.yml', import.meta.url), 'utf8');
const document = parseDocument(source, { uniqueKeys: true });
assert.deepEqual(document.errors, [], 'workflow must be valid YAML without duplicate keys');
const workflow = document.toJS();
const { validate, deploy } = workflow.jobs;
const step = (id) => {
  const matches = validate.steps.filter((entry) => entry.id === id);
  assert.equal(matches.length, 1, `exactly one ${id} step`);
  return matches[0];
};
// Compare GitHub expressions as contract text, not a home-grown interpreter.
const publishCondition = "github.event_name == 'workflow_dispatch' && inputs.publish == true && github.ref == 'refs/heads/dev' && github.repository == 'jscott3201/rusty-bacnet'";
const expression = (condition) => `\${{ ${condition} }}`;

test('docs run automatically only for relevant pull requests, and publish defaults off', () => {
  assert.deepEqual(Object.keys(workflow.on).sort(), ['pull_request', 'workflow_dispatch']);
  assert.deepEqual(workflow.on.pull_request, {
    branches: ['dev', 'main'],
    paths: ['website/**', '.github/workflows/docs-pages.yml'],
  });
  // The website/** filter covers this guard as well as the lockfile and site.
  assert.deepEqual(workflow.on.workflow_dispatch.inputs, {
    publish: {
      description: 'Publish the reviewed site from dev',
      type: 'boolean', default: false, required: true,
    },
    expected_sha: {
      description: 'Full reviewed dev commit SHA (required when publish is true)',
      type: 'string', default: '', required: false,
    },
  });
});

test('only the separate deployment job has Pages and OIDC permissions', () => {
  assert.equal(workflow.permissions, undefined, 'no global permissions');
  assert.deepEqual(Object.keys(workflow.jobs), ['validate', 'deploy']);
  assert.deepEqual(validate.permissions, { contents: 'read' });
  assert.deepEqual(deploy.permissions, { pages: 'write', 'id-token': 'write' });
  assert.equal(validate.environment, undefined);
  assert.deepEqual(deploy.environment, {
    name: 'github-pages', url: expression('steps.deployment.outputs.page_url'),
  });
});

test('validation uses the event SHA without persisted checkout credentials or ref inputs', () => {
  assert.deepEqual(step('checkout').with, {
    ref: expression('github.sha'), 'persist-credentials': false,
  });
  assert.equal(step('checkout').if, undefined);
  assert.equal(step('publication-request').if,
    expression("github.event_name == 'workflow_dispatch' && inputs.publish == true"));
  assert.deepEqual(step('publication-request').env, { EXPECTED_SHA: expression('inputs.expected_sha') });
});

test('native Ubuntu validation installs the lockfile, Chromium and runs the complete verify script', () => {
  assert.equal(validate.name, 'Docs validation');
  assert.equal(validate['runs-on'], 'ubuntu-latest');
  assert.equal(validate['timeout-minutes'], 20);
  assert.deepEqual(validate.defaults, { run: { 'working-directory': 'website', shell: 'bash' } });
  assert.deepEqual(validate.env, { DOCS_TEST_PORT: '46329' });
  assert.equal(validate.if, undefined);
  assert.equal(validate.container, undefined);
  assert.equal(validate.strategy, undefined);
  assert.deepEqual(step('node').with, {
    'node-version': '24', cache: 'npm', 'cache-dependency-path': 'website/package-lock.json',
  });
  assert.deepEqual(validate.steps.map((entry) => entry.id), [
    'checkout', 'publication-request', 'node', 'dependencies', 'browser', 'verify', 'reports', 'pages', 'provenance',
  ]);
  for (const [id, command] of [
    ['dependencies', 'npm ci'],
    ['browser', 'npx playwright install --with-deps chromium'],
    ['verify', 'npm run verify'],
  ]) {
    assert.equal(step(id).run, command);
    assert.equal(step(id).if, undefined, `${id} must not skip validation`);
    assert.equal(step(id).env, undefined, `${id} must not override the test configuration`);
  }
  for (const job of Object.values(workflow.jobs)) {
    assert.equal(job['continue-on-error'], undefined);
    for (const entry of job.steps) assert.equal(entry['continue-on-error'], undefined);
  }
  assert.deepEqual(validate.steps.filter((entry) => entry.run).map((entry) => entry.id),
    ['publication-request', 'dependencies', 'browser', 'verify', 'provenance']);
});

test('only a successful manual canonical dev publication uploads dist and deploys the same named artifact', () => {
  assert.equal(step('pages').if, expression(publishCondition));
  assert.deepEqual(step('pages').with, {
    name: 'github-pages', path: 'website/dist', 'retention-days': 1, 'include-hidden-files': false,
  });
  assert.equal(deploy.needs, 'validate');
  assert.equal(deploy.if, expression(`needs.validate.result == 'success' && ${publishCondition}`));
  assert.equal(deploy['runs-on'], 'ubuntu-latest');
  assert.equal(deploy['timeout-minutes'], 15);
  assert.equal(deploy.steps.length, 1, 'no checkout, fetch, rebuild or other artifact source in deploy');
  assert.equal(deploy.steps[0].id, 'deployment');
  assert.deepEqual(deploy.steps[0].with, { artifact_name: step('pages').with.name });
  assert.equal(deploy.steps[0].run, undefined);
  assert.equal(deploy.steps[0].if, undefined);
});

test('browser failure artifacts are bounded and separate from the public Pages artifact', () => {
  assert.equal(step('reports').if, expression('failure()'));
  assert.deepEqual(step('reports').with, {
    name: 'docs-browser-failure-${{ github.run_id }}-${{ github.run_attempt }}',
    path: 'website/playwright-report\nwebsite/test-results\n',
    'retention-days': 7, 'if-no-files-found': 'ignore', 'include-hidden-files': false,
  });
  assert.notEqual(step('reports').with.name, step('pages').with.name);
  assert.deepEqual(step('provenance').env, { PAGES_ARTIFACT_ID: expression('steps.pages.outputs.artifact_id') });
  assert.match(step('provenance').run, /"\$GITHUB_SHA" "\$PAGES_ARTIFACT_ID" >> "\$GITHUB_STEP_SUMMARY"/);
});

test('PR cancellation is isolated from the serialized manual publication lane', () => {
  assert.deepEqual(workflow.concurrency, {
    group: "docs-pages-${{ github.event_name == 'workflow_dispatch' && inputs.publish == true && 'publish' || format('{0}-{1}', github.event_name, github.ref) }}",
    'cancel-in-progress': expression("github.event_name == 'pull_request'"),
  });
  assert.equal(validate.concurrency, undefined);
  assert.equal(deploy.concurrency, undefined);
});

test('every external action is pinned to the reviewed immutable release', () => {
  const pins = [
    ['actions/checkout', '3d3c42e5aac5ba805825da76410c181273ba90b1', 'v7'],
    ['actions/setup-node', '249970729cb0ef3589644e2896645e5dc5ba9c38', 'v6'],
    ['actions/upload-artifact', '043fb46d1a93c77aae656e7c1c64a875d1fc6a0a', 'v7.0.1'],
    ['actions/upload-pages-artifact', 'fc324d3547104276b827a68afc52ff2a11cc49c9', 'v5'],
    ['actions/deploy-pages', '368f82528645a54fb793d4d04e342629a3f51346', 'v5'],
  ];
  const actions = Object.values(workflow.jobs).flatMap((job) => job.steps)
    .filter((entry) => entry.uses).map((entry) => entry.uses);
  assert.deepEqual(actions, pins.map(([action, sha]) => `${action}@${sha}`));
  for (const [action, sha, version] of pins) {
    assert.match(sha, /^[0-9a-f]{40}$/);
    assert.ok(source.includes(`uses: ${action}@${sha} # ${version}\n`));
  }
});

// Execute the real, side-effect-free Bash guard with environment fixtures.
// This tests the expected-SHA check, not GitHub's event/expression engine.
const reviewedSha = 'b455b8677be88966cb5d1452cd4cc587266468bd';
const publicationEnv = {
  GITHUB_REPOSITORY: 'jscott3201/rusty-bacnet', GITHUB_REF: 'refs/heads/dev',
  GITHUB_SHA: reviewedSha, EXPECTED_SHA: reviewedSha,
};
for (const [label, overrides, accepted] of [
  ['exact reviewed dev commit', {}, true],
  ['missing expected SHA', { EXPECTED_SHA: '' }, false],
  ['abbreviated SHA', { EXPECTED_SHA: reviewedSha.slice(0, 7) }, false],
  ['branch moved before dispatch', { GITHUB_SHA: '0'.repeat(40) }, false],
  ['other branch', { GITHUB_REF: 'refs/heads/main' }, false],
  ['tag named dev', { GITHUB_REF: 'refs/tags/dev' }, false],
  ['fork repository', { GITHUB_REPOSITORY: 'fork/rusty-bacnet' }, false],
  ['shell-shaped input', { EXPECTED_SHA: '$(exit 0)' }, false],
]) {
  test(`publication request guard: ${label}`, () => {
    const result = spawnSync('bash', ['--noprofile', '--norc', '-e', '-o', 'pipefail', '-c', step('publication-request').run], {
      env: { ...publicationEnv, ...overrides }, encoding: 'utf8', timeout: 5_000,
    });
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, accepted ? 0 : 1, result.stderr);
  });
}
