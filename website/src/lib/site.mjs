export const base = '/rusty-bacnet/';
export const release = '0.11.0';
export const repository = 'https://github.com/jscott3201/rusty-bacnet';
/** Join a site-relative path. External links are intentionally not accepted. */
export function sitePath(path = '') {
  if (/^[a-z][a-z0-9+.-]*:/i.test(path) || path.startsWith('//') || path.includes('..')) {
    throw new Error('sitePath expects a local path without traversal');
  }
  return base + path.replace(/^\/+/, '');
}
