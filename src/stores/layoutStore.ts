import { create } from 'zustand';
import { readPreference, writePreference } from '../lib/preferences';

const KEY = 'auumail.ui.layout.v1';
export const MIN_LIST_WIDTH = 280;
export const MAX_LIST_WIDTH = 440;
export const DEFAULT_LIST_WIDTH = 340;
export const MIN_SIDEBAR_WIDTH = 190;
export const MAX_SIDEBAR_WIDTH = 320;
export const DEFAULT_SIDEBAR_WIDTH = 222;
export type LayoutWidths = { listWidth: number; sidebarWidth: number };
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, Math.round(value)));
}
/** Accepts the current {listWidth, sidebarWidth} shape; a bare number is a pre-sidebar-width layout. */
function validWidths(value: unknown): LayoutWidths {
  if (typeof value === 'number' && Number.isFinite(value)) return { listWidth: clamp(value, MIN_LIST_WIDTH, MAX_LIST_WIDTH), sidebarWidth: DEFAULT_SIDEBAR_WIDTH };
  if (typeof value === 'object' && value !== null && 'listWidth' in value && 'sidebarWidth' in value) {
    const { listWidth, sidebarWidth } = value as Record<string, unknown>;
    if (typeof listWidth !== 'number' || typeof sidebarWidth !== 'number' || !Number.isFinite(listWidth) || !Number.isFinite(sidebarWidth)) throw new Error('Invalid widths');
    return { listWidth: clamp(listWidth, MIN_LIST_WIDTH, MAX_LIST_WIDTH), sidebarWidth: clamp(sidebarWidth, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH) };
  }
  throw new Error('Invalid widths');
}
export const useLayoutStore = create<LayoutWidths & { setListWidth: (value: number) => void; setSidebarWidth: (value: number) => void }>((set, get) => ({
  ...(readPreference(KEY, validWidths) ?? { listWidth: DEFAULT_LIST_WIDTH, sidebarWidth: DEFAULT_SIDEBAR_WIDTH }),
  setListWidth(value) {
    const listWidth = clamp(value, MIN_LIST_WIDTH, MAX_LIST_WIDTH);
    writePreference(KEY, { listWidth, sidebarWidth: get().sidebarWidth });
    set({ listWidth });
  },
  setSidebarWidth(value) {
    const sidebarWidth = clamp(value, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
    writePreference(KEY, { listWidth: get().listWidth, sidebarWidth });
    set({ sidebarWidth });
  },
}));
