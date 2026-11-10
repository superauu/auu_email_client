import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { colorTokens, metricRanges } from '../src/themes/contract';

// Single source of truth for runtime validation and editor schemas.
const base = { $schema: 'https://json-schema.org/draft/2020-12/schema' };
const string = (maxLength: number) => ({ type: 'string', minLength: 1, maxLength, pattern: '\\S' });
const manifest = {
  ...base, title: 'AuuMail skin manifest v1', type: 'object', additionalProperties: false,
  required: ['schemaVersion', 'id', 'name', 'version', 'author', 'description'],
  properties: {
    $schema: { type: 'string' }, schemaVersion: { const: 1 },
    id: { type: 'string', pattern: '^[a-z0-9][a-z0-9-]{1,63}$' },
    name: string(80), author: string(80), description: string(200),
    version: { type: 'string', pattern: '^\\d+\\.\\d+\\.\\d+$', maxLength: 32 },
  },
};
const tokens = {
  ...base, title: 'AuuMail skin tokens v1', type: 'object', additionalProperties: false,
  anyOf: [{ required: ['light'] }, { required: ['dark'] }],
  properties: { $schema: { type: 'string' }, light: { $ref: '#/$defs/palette' }, dark: { $ref: '#/$defs/palette' } },
  $defs: { palette: {
    type: 'object', additionalProperties: false,
    required: [...colorTokens, ...Object.keys(metricRanges)],
    properties: {
      ...Object.fromEntries(colorTokens.map(key => [key, { type: 'string', pattern: '^#[0-9a-fA-F]{6}$' }])),
      ...Object.fromEntries(Object.entries(metricRanges).map(([key, [minimum, maximum]]) => [key, { type: 'integer', minimum, maximum }])),
    },
  } },
};
await mkdir(new URL('../schemas/', import.meta.url), { recursive: true });
for (const [name, schema] of Object.entries({ 'skin-manifest': manifest, 'skin-tokens': tokens })) {
  const file = new URL(`../schemas/${name}.schema.json`, import.meta.url);
  const content = `${JSON.stringify(schema, null, 2)}\n`;
  if (process.argv.includes('--write')) await writeFile(file, content);
  else if (await readFile(file, 'utf8').catch(() => '') !== content) throw new Error(`Schema drift: run npm run skins:schema (${name})`);
}
