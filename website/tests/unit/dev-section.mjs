// The one rule for release-scoped pages with post-release additions: such a
// page keeps them, and every link into the repository at the dev branch, in
// one "On `dev` after the release" section. Everything outside that section,
// later sections included, describes the release.
import assert from 'node:assert/strict';

export const repo = 'https://github.com/jscott3201/rusty-bacnet';
export const devHeading = '\n## On `dev` after the release\n';
/** Any link into the repository at the dev branch: blob/, tree/, raw/, commits/ and so on. */
export const devRef = new RegExp(`${repo.replaceAll('.', '\\.')}/[a-z]+/dev/`);

/**
 * Split a plain-text page body into its release-scoped text and its dev
 * section (empty when the page has none), and check the release-scoped text
 * has no dev link. The dev section must be followed by another section.
 */
export function splitDevSection(body, label) {
  const parts = body.split(devHeading);
  assert.ok(parts.length <= 2, `${label}: at most one dev section`);
  let released = body;
  let onDev = '';
  if (parts.length === 2) {
    const next = parts[1].indexOf('\n## ');
    assert.notEqual(next, -1, `${label}: a section follows the dev one`);
    onDev = parts[1].slice(0, next);
    released = parts[0] + parts[1].slice(next);
  }
  assert.doesNotMatch(released, devRef, `${label}: dev links only in the dev section`);
  return { released, onDev };
}
