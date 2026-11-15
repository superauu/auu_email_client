import { create } from 'zustand';
import { navigationKey, validateNavigationPreferences, type NavigationPreferences, type NavigationTarget } from '../domain/navigation';
import { readPreference, writePreference } from '../lib/preferences';

const KEY = 'auumail.ui.navigation.v1';
export const useNavigationStore = create<NavigationPreferences & {
  error: string | null;
  setExpanded: (ids: string[]) => void;
  togglePinned: (target: NavigationTarget) => void;
}>((set, get) => {
  const persist = (changes: Partial<NavigationPreferences>) => {
    const next = { pinned: get().pinned, expanded: get().expanded, ...changes };
    set({ ...next, error: writePreference(KEY, next) ? null : '导航偏好暂未保存，当前会话仍可使用。' });
  };
  return {
    ...(readPreference(KEY, validateNavigationPreferences) ?? { pinned: [], expanded: null }), error: null,
    setExpanded: expanded => persist({ expanded }),
    togglePinned(target) {
      const key = navigationKey(target);
      const exists = get().pinned.some(item => navigationKey(item) === key);
      if (!exists && get().pinned.length >= 30) { set({ error: '常用最多固定 30 个入口，请先移除不再使用的入口。' }); return; }
      persist({ pinned: exists ? get().pinned.filter(item => navigationKey(item) !== key) : [...get().pinned, target] });
    },
  };
});
