import { create } from 'zustand';
import type { TrustedEntry } from '../domain/trust';
import { isTrustedSender, validateTrustedEntries } from '../domain/trust';
import { desktopMail, isDesktop } from '../platform/desktop';
import { t } from '../i18n/core';

interface TrustState {
  /** 信任条目缓存；null = 尚未加载（首次 ensure 后必有值）。 */
  entries: TrustedEntry[] | null;
  ensure: () => Promise<void>;
  add: (kind: 'sender' | 'domain', value: string) => Promise<void>;
  remove: (kind: 'sender' | 'domain', value: string) => Promise<void>;
  /** 发件人当前是否被信任（基于已加载缓存；未加载视为否，由 ensure 先行）。 */
  isTrusted: (email: string) => boolean;
}

export const useTrustStore = create<TrustState>((set, get) => ({
  entries: null,
  async ensure() {
    if (!isDesktop()) return Promise.reject(new Error(t('store.trustDesktopOnly')));
    if (get().entries) return;
    const entries = validateTrustedEntries(await desktopMail.trustedSenders());
    set({ entries });
  },
  async add(kind, value) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.trustDesktopOnly')));
    const entry = validateTrustedEntries([await desktopMail.trustAdd(kind, value)])[0];
    set(state => ({ entries: [...(state.entries ?? []), entry] }));
  },
  async remove(kind, value) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.trustDesktopOnly')));
    await desktopMail.trustRemove(kind, value);
    set(state => ({ entries: (state.entries ?? []).filter(entry => !(entry.kind === kind && entry.value === value)) }));
  },
  isTrusted(email) {
    return isTrustedSender(email, get().entries);
  },
}));
