import { t } from '../i18n/core';
export const colorTokens = [
  'canvas', 'surface', 'surface-raised', 'surface-hover', 'surface-selected', 'rail',
  'text', 'text-muted', 'text-subtle', 'border', 'accent', 'accent-foreground', 'danger', 'success', 'warning',
] as const;
export const metricRanges = {
  'radius-sm': [0, 16], 'radius-md': [0, 24], 'radius-lg': [0, 32],
  'control-height': [30, 44], 'list-row-height': [68, 128],
} as const;

export type ColorToken = typeof colorTokens[number];
export type MetricToken = keyof typeof metricRanges;
export type SkinTokens = Record<ColorToken, string> & Record<MetricToken, number>;
export type ColorMode = 'light' | 'dark';
export type ThemeMode = ColorMode | 'system';
export interface SkinManifest {
  schemaVersion: 1;
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
}
export interface SkinPack {
  manifest: SkinManifest;
  tokens: Partial<Record<ColorMode, SkinTokens>>;
  source: 'builtin' | 'user';
}
export interface RawSkin { folderName: string; manifest: unknown; tokens: unknown }
export interface SkinCatalog { skins: RawSkin[]; warnings: string[] }

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(t('skin.errObject'));
  return value as Record<string, unknown>;
}
function exactKeys(value: Record<string, unknown>, allowed: readonly string[]) {
  for (const key of Object.keys(value)) if (!allowed.includes(key)) throw new Error(t('skin.errUnknownField', { key }));
}
function text(value: unknown, field: string, max: number): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max) throw new Error(t('skin.errFieldInvalid', { field }));
  return value;
}

export function validateSkin(raw: RawSkin, source: SkinPack['source']): SkinPack {
  const manifest = record(raw.manifest);
  exactKeys(manifest, ['$schema', 'schemaVersion', 'id', 'name', 'version', 'author', 'description']);
  if (manifest.schemaVersion !== 1) throw new Error(t('skin.errSchemaVersion'));
  const id = text(manifest.id, 'ID', 64);
  if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(id) || id !== raw.folderName) throw new Error(t('skin.errIdFolder'));
  const version = text(manifest.version, t('skin.fieldVersion'), 32);
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(t('skin.errVersionFormat'));
  const parsedManifest: SkinManifest = {
    schemaVersion: 1, id, version,
    name: text(manifest.name, t('skin.fieldName'), 80),
    author: text(manifest.author, t('skin.fieldAuthor'), 80),
    description: text(manifest.description, t('skin.fieldDescription'), 200),
  };
  const tokens = record(raw.tokens);
  exactKeys(tokens, ['$schema', 'light', 'dark']);
  const result: SkinPack['tokens'] = {};
  for (const mode of ['light', 'dark'] as const) {
    if (tokens[mode] === undefined) continue;
    const values = record(tokens[mode]);
    exactKeys(values, [...colorTokens, ...Object.keys(metricRanges)]);
    const colors = {} as Record<ColorToken, string>;
    for (const token of colorTokens) {
      const value = values[token];
      if (typeof value !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(value)) throw new Error(t('skin.errHexColor', { mode, token }));
      colors[token] = value;
    }
    const metrics = {} as Record<MetricToken, number>;
    for (const token of Object.keys(metricRanges) as MetricToken[]) {
      const value = values[token];
      const [min, max] = metricRanges[token];
      if (typeof value !== 'number' || !Number.isInteger(value) || value < min || value > max) throw new Error(t('skin.errMetricRange', { mode, token, min, max }));
      metrics[token] = value;
    }
    result[mode] = { ...colors, ...metrics };
  }
  if (!result.light && !result.dark) throw new Error(t('skin.errNoMode'));
  return { manifest: parsedManifest, tokens: result, source };
}

export function mergeCatalog(builtins: SkinPack[], catalog: SkinCatalog): { skins: SkinPack[]; warnings: string[] } {
  const skins = [...builtins];
  const ids = new Set(skins.map(skin => skin.manifest.id));
  const warnings = [...catalog.warnings];
  for (const raw of catalog.skins) {
    try {
      const skin = validateSkin(raw, 'user');
      if (ids.has(skin.manifest.id)) throw new Error(t('skin.errDuplicateId'));
      ids.add(skin.manifest.id);
      skins.push(skin);
    } catch (error) { warnings.push(`${raw.folderName}: ${error instanceof Error ? error.message : t('skin.errInvalidSkin')}`); }
  }
  return { skins, warnings };
}

export function resolveSkin(skins: SkinPack[], skinId: string, mode: ThemeMode, systemDark: boolean) {
  const skin = skins.find(item => item.manifest.id === skinId) ?? skins[0];
  if (!skin) throw new Error(t('skin.errMissingBuiltin'));
  const requested: ColorMode = mode === 'system' ? systemDark ? 'dark' : 'light' : mode;
  const resolvedMode: ColorMode = skin.tokens[requested] ? requested : requested === 'dark' ? 'light' : 'dark';
  const tokens = skin.tokens[resolvedMode];
  if (!tokens) throw new Error(t('skin.errNoTokens'));
  return { skin, mode: resolvedMode, tokens };
}
