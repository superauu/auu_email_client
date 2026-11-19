import { create } from 'zustand';
import { readPreference, writePreference } from '../lib/preferences';
import { parseSavedSearches, SAVED_SEARCHES_KEY, MAX_SAVED_SEARCHES, savedSearchNameError, type SavedSearch, type SavedQuery } from '../domain/savedSearch';
import { t } from '../i18n/core';

const saved = readPreference(SAVED_SEARCHES_KEY, raw => {
  if (!Array.isArray(raw)) throw new Error('Invalid saved searches');
  return parseSavedSearches(raw);
});

interface SavedSearchState {
  searches: SavedSearch[];
  /** 写入失败时的一次性用户可见提示（当前会话仍可使用）。 */
  warnings: string[];
  save: (name: string, query: SavedQuery) => Promise<string>;
  remove: (id: string) => void;
  dismissWarning: () => void;
}

export const useSavedSearchStore = create<SavedSearchState>((set, get) => ({
  searches: saved ?? [],
  warnings: [],
  async save(name, query) {
    const invalid = savedSearchNameError(name);
    if (invalid) throw new Error(invalid);
    if (get().searches.length >= MAX_SAVED_SEARCHES) throw new Error(t('searches.limit'));
    const entry: SavedSearch = { id: `search:${crypto.randomUUID()}`, name: name.trim(), query };
    const searches = [...get().searches, entry];
    set({ searches, ...(!writePreference(SAVED_SEARCHES_KEY, searches) ? { warnings: [t('searches.prefNotSaved')] } : {}) });
    return entry.id;
  },
  remove(id) {
    const searches = get().searches.filter(item => item.id !== id);
    set({ searches, ...(!writePreference(SAVED_SEARCHES_KEY, searches) ? { warnings: [t('searches.prefNotSaved')] } : {}) });
  },
  dismissWarning() { set({ warnings: [] }); },
}));
