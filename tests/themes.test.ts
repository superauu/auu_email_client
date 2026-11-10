import test from 'node:test';
import assert from 'node:assert/strict';
import { builtinSkins } from '../src/themes/builtins';
import { mergeCatalog, resolveSkin, validateSkin } from '../src/themes/contract';

const raw = () => ({ folderName: 'custom', manifest: { ...builtinSkins[0].manifest, id: 'custom' }, tokens: structuredClone(builtinSkins[0].tokens) });

test('builtins provide complete light and dark palettes', () => {
  for (const skin of builtinSkins) {
    assert.equal(resolveSkin([skin], skin.manifest.id, 'light', true).mode, 'light');
    assert.equal(resolveSkin([skin], skin.manifest.id, 'system', true).mode, 'dark');
  }
});

test('user pack installs without changing the builtin catalog', () => {
  const { skins, warnings } = mergeCatalog(builtinSkins, { skins: [raw()], warnings: [] });
  assert.equal(skins.length, 4);
  assert.equal(builtinSkins.length, 3);
  assert.equal(skins[3].source, 'user');
  assert.deepEqual(warnings, []);
});

test('a broken pack and an ID collision do not hide other user packs', () => {
  const duplicate = { ...raw(), folderName: 'paper', manifest: { ...raw().manifest, id: 'paper' } };
  const { skins, warnings } = mergeCatalog(builtinSkins, { skins: [{ ...raw(), tokens: {} }, duplicate, raw()], warnings: ['native warning'] });
  assert.equal(skins.length, 4);
  assert.equal(warnings.length, 3);
});

test('unsupported schemas, directory mismatch and CSS injection are rejected', () => {
  const unsupported = raw();
  assert.throws(() => validateSkin({ ...unsupported, manifest: { ...unsupported.manifest, schemaVersion: 2 } }, 'user'));
  assert.throws(() => validateSkin({ ...raw(), folderName: '../custom' }, 'user'));
  const injected = raw();
  injected.tokens.light!.accent = 'url(https://example.test/track)';
  assert.throws(() => validateSkin(injected, 'user'));
  assert.throws(() => validateSkin({ ...raw(), tokens: { light: { ...raw().tokens.light, 'font-family': 'external' } } }, 'user'));
});

test('out of range metrics and incomplete modes are rejected', () => {
  const pack = raw();
  pack.tokens.light!['control-height'] = 0;
  assert.throws(() => validateSkin(pack, 'user'));
  assert.throws(() => validateSkin({ ...raw(), tokens: { light: { accent: '#123456' } } }, 'user'));
});

test('a missing selected skin falls back, and a single-mode skin resolves safely', () => {
  assert.equal(resolveSkin(builtinSkins, 'removed-skin', 'dark', false).skin.manifest.id, 'paper');
  const pack = raw();
  delete pack.tokens.dark;
  const skin = validateSkin(pack, 'user');
  assert.equal(resolveSkin([skin], 'custom', 'dark', true).mode, 'light');
});
