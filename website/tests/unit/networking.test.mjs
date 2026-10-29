import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { frontmatter, plainBody } from '../../scripts/content.mjs';
import { release } from '../../src/lib/site.mjs';
import { devRef, repo, splitDevSection } from './dev-section.mjs';
import { checkCliLine, cliDefinitions } from './cli-args.mjs';

const root = new URL('../../', import.meta.url);
// The networking pages describe the release, with no dev section. Each lists
// the names its body must document.
const pages = {
  'connect/across-subnets': { names: ['set_release_previous_bbmd', 'previous_release_refused'] },
  'connect/nat': { names: [] },
  'connect/multicast': { names: ['set_multicast_persist_path', 'multicast_persist_failures', 'BbmdChanges::multicast_group'] },
  'connect/ipv6': { names: [] },
};

async function page(slug) {
  const file = fileURLToPath(new URL(`src/content/docs/${slug}.mdx`, root));
  const source = await readFile(file, 'utf8');
  const { title, description, body } = frontmatter(source);
  return { title, description, body: await plainBody(body, file, []) };
}

test('networking pages are release-scoped, with no dev section', async () => {
  for (const [slug, { names }] of Object.entries(pages)) {
    const { title, description, body } = await page(slug);
    assert.ok(title && description, slug);
    assert.ok(body.includes(`The page describes **v${release}**.`), slug);
    assert.ok(body.includes('\n## Next steps\n'), slug);
    const { onDev } = splitDevSection(body, slug);
    assert.equal(onDev, '', `${slug}: no dev section`);
    assert.doesNotMatch(body, devRef, `${slug}: no dev links`);
    assert.ok(!body.includes('on-dev-after-the-release'), `${slug}: no pointer to a dev section`);
    assert.ok(body.includes(`${repo}/blob/v${release}/docs/`), `${slug}: release-pinned docs links`);
    for (const name of names) assert.ok(body.includes(name), `${slug}: ${name}`);
  }
});

test('every CLI option and subcommand in a networking sample is defined by the CLI', async () => {
  const cli = await cliDefinitions();
  // The parsers must see the options and commands the samples rely on.
  assert.equal(cli.options.get('foreign-bbmd'), true);
  assert.equal(cli.options.get('allow-write'), false);
  assert.ok(cli.commands.has('bdt') && cli.commands.has('whois-router') && cli.commands.has('whois'));
  for (const slug of Object.keys(pages)) {
    const { body } = await page(slug);
    const blocks = [...body.matchAll(/^```sh\n([\s\S]*?)^```/gm)].map(match => match[1]);
    for (const line of blocks.flatMap(block => block.split('\n')).filter(Boolean)) {
      checkCliLine(line, cli, slug);
    }
  }
});
