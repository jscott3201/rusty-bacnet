export const base = '/rusty-bacnet/';
/**
 * The release the start/ pages install. MDX pages, the download list, the
 * footer and the llms index read it from here; tests/unit/release.test.mjs
 * names every other place that must match it (plain Markdown pages and the
 * loopback lab script).
 */
export const release = '0.12.0';
export const repository = 'https://github.com/jscott3201/rusty-bacnet';
/** The download URL of one of the release's assets on its GitHub release copy. */
export function releaseAsset(file) {
  return `${repository}/releases/download/v${release}/${file}`;
}
/** Join a site-relative path. External links are intentionally not accepted. */
export function sitePath(path = '') {
  if (/^[a-z][a-z0-9+.-]*:/i.test(path) || path.startsWith('//') || path.includes('..')) {
    throw new Error('sitePath expects a local path without traversal');
  }
  return base + path.replace(/^\/+/, '');
}
