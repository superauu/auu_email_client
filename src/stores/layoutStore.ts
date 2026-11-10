import { create } from 'zustand';
import { readPreference, writePreference } from '../lib/preferences';

const KEY = 'auumail.ui.layout.v1';
export const MIN_LIST_WIDTH = 280;
export const MAX_LIST_WIDTH = 440;
function validWidth(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error('Invalid width');
  return Math.max(MIN_LIST_WIDTH, Math.min(MAX_LIST_WIDTH, Math.round(value)));
}
export const useLayoutStore = create<{ listWidth: number; setListWidth: (value: number) => void }>(set => ({
  listWidth: readPreference(KEY, validWidth) ?? 340,
  setListWidth(value) {
    const listWidth = validWidth(value);
    writePreference(KEY, listWidth);
    set({ listWidth });
  },
}));
