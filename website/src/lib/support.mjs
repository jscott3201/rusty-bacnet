/**
 * The capability tables on project/conformance, built from
 * src/data/support.json, which a Rust generator writes from the code
 * (crates/bacnet-integration-tests/support_tables.rs, #1496). The page's
 * component renders these tables as HTML and scripts/content.mjs as Markdown,
 * so the two can't drift. Nothing here adds a fact: each cell comes from the
 * JSON.
 *
 * A table is { id, caption, columns, rows }. A cell is a string, or
 * { code } for text shown as code, or { text, code } for text followed by code.
 * Each function takes the parsed JSON: the component imports it through Vite,
 * and Node callers with an import attribute.
 */
/** "MULTI_STATE_INPUT" as "Multi State Input". */
export function typeLabel(name) {
  return name.toLowerCase().split('_').map(word => word.charAt(0).toUpperCase() + word.slice(1)).join(' ');
}

const yesNo = value => value ? 'Yes' : 'No';

function endpointCell(endpoint) {
  if (endpoint === 'always') return 'Always';
  if (endpoint) return { text: 'Opt-in: ', code: `${endpoint}()` };
  return 'No';
}

function cliCell(builds, targets) {
  const built = targets.filter(target => builds[target]);
  if (built.length === targets.length) return 'Every release target';
  if (built.length === 0) return 'No';
  return { code: built.join(', ') };
}

function services(data, kind) {
  const rows = data.services.filter(service => service.kind === kind);
  return {
    id: `services-${kind}`,
    caption: `${rows.length} ${kind} services, by surface`,
    columns: ['Service', 'Full server executes', 'Full server initiates', 'Client sends', 'Endpoint responder executes', 'Python client sends'],
    rows: rows.map(service => [
      service.label,
      yesNo(service.serverExecutes),
      yesNo(service.serverInitiates),
      yesNo(service.clientSends),
      endpointCell(service.endpoint),
      yesNo(service.pythonSends),
    ]),
  };
}

/** Every table, by id. */
export function supportTables(data) {
  const tables = [
    {
      id: 'objects',
      caption: `${data.objectTypes.length} constructible object types; not built: ${data.excludedObjectTypes.map(type => `${typeLabel(type.name)} (${type.id})`).join(', ') || 'none'}`,
      columns: ['Object type', 'Type number', 'CreateObject', 'DeleteObject'],
      rows: data.objectTypes.map(type => [typeLabel(type.name), String(type.id), yesNo(type.create), yesNo(type.delete)]),
    },
    services(data, 'confirmed'),
    services(data, 'unconfirmed'),
    {
      id: 'links',
      caption: `${data.dataLinks.length} data links, by artifact`,
      columns: ['Data link', 'Rust: bacnet-transport feature', 'Python wheel', 'CLI release executable'],
      rows: data.dataLinks.map(link => [
        link.label,
        link.feature ? { code: link.feature } : 'Always built',
        yesNo(link.python),
        cliCell(link.cli, data.cliTargets),
      ]),
    },
  ];
  return Object.fromEntries(tables.map(table => [table.id, table]));
}

/** One table by id; throws on an unknown id. */
export function supportTable(id, data) {
  const table = supportTables(data)[id];
  if (!table) throw new Error(`Unknown support table ${id}`);
  return table;
}

const markdownCell = cell => {
  const text = typeof cell === 'string' ? cell : `${cell.text ?? ''}\`${cell.code}\``;
  return text.replaceAll('|', '\\|');
};

/** The table as Markdown, for the plain-text export. */
export function supportTableMarkdown(id, data) {
  const table = supportTable(id, data);
  return [
    `*${table.caption}*`,
    '',
    `| ${table.columns.join(' | ')} |`,
    `|${table.columns.map(() => '---').join('|')}|`,
    ...table.rows.map(row => `| ${row.map(markdownCell).join(' | ')} |`),
  ].join('\n');
}
