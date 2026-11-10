import paperManifest from '../../skins/paper/manifest.json';
import paperTokens from '../../skins/paper/tokens.json';
import graphiteManifest from '../../skins/graphite/manifest.json';
import graphiteTokens from '../../skins/graphite/tokens.json';
import linenManifest from '../../skins/linen/manifest.json';
import linenTokens from '../../skins/linen/tokens.json';
import { validateSkin } from './contract';

export const builtinSkins = [
  { folderName: 'paper', manifest: paperManifest, tokens: paperTokens },
  { folderName: 'graphite', manifest: graphiteManifest, tokens: graphiteTokens },
  { folderName: 'linen', manifest: linenManifest, tokens: linenTokens },
].map(raw => validateSkin(raw, 'builtin'));
