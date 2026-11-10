import { create } from 'zustand';
import { builtinSkins } from '../themes/builtins';
import { mergeCatalog, resolveSkin, type SkinPack, type ThemeMode } from '../themes/contract';
import { desktop } from '../platform/desktop';
import { readPreference, writePreference } from '../lib/preferences';
import { errorMessage } from '../domain/errors';

const KEY = 'auumail.ui.theme.v1';
const saved = readPreference(KEY, (raw) => {
  if (!raw || typeof raw !== 'object' || !('skinId' in raw) || !('mode' in raw) || typeof raw.skinId !== 'string' || !['light', 'dark', 'system'].includes(String(raw.mode))) throw new Error('Invalid theme');
  return { skinId: raw.skinId, mode: raw.mode as ThemeMode };
});
interface ThemeState {
  skinId: string;
  mode: ThemeMode;
  skins: SkinPack[];
  warnings: string[];
  loading: boolean;
  setAppearance: (skinId: string, mode: ThemeMode) => void;
  rescan: () => Promise<void>;
}
export const useThemeStore = create<ThemeState>((set, get) => ({
  skinId: saved?.skinId ?? 'paper', mode: saved?.mode ?? 'system',
  skins: builtinSkins, warnings: [], loading: false,
  setAppearance(skinId, mode) {
    const persisted = writePreference(KEY, { skinId, mode });
    set({ skinId, mode, ...(!persisted ? { warnings: [...get().warnings, '外观设置暂时无法保存，当前会话仍可使用。'] } : {}) });
  },
  async rescan() {
    if (get().loading) return;
    set({ loading: true });
    try { set(mergeCatalog(builtinSkins, await desktop.listSkins())); }
    catch (error) { set({ warnings: [errorMessage(error)] }); }
    finally { set({ loading: false }); }
  },
}));

export function applyAppearance(systemDark: boolean) {
  const state = useThemeStore.getState();
  const resolved = resolveSkin(state.skins, state.skinId, state.mode, systemDark);
  const root = document.documentElement;
  for (const [name, value] of Object.entries(resolved.tokens)) root.style.setProperty(`--${name}`, typeof value === 'number' ? `${value}px` : value);
  root.style.colorScheme = resolved.mode;
  root.dataset.mode = resolved.mode;
  root.dataset.skin = resolved.skin.manifest.id;
}

/** Subscribe once outside React so persisted appearance is applied before the first paint. */
export function initializeAppearance(): () => void {
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const update = () => applyAppearance(media.matches);
  update();
  const unsubscribe = useThemeStore.subscribe(update);
  media.addEventListener('change', update);
  void useThemeStore.getState().rescan();
  return () => { unsubscribe(); media.removeEventListener('change', update); };
}
