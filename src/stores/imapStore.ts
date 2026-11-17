import { create } from 'zustand';
import type { ImapSettings } from '../domain/imap';
import { desktopImapGateway } from '../services/desktop/DesktopImapGateway';
import { isDesktop } from '../platform/desktop';
import { t } from '../i18n/core';
export const useImapStore = create<{
  load: (accountId: string) => Promise<ImapSettings>;
  save: (accountId: string, settings: ImapSettings) => Promise<void>;
}>(() => ({
  load(accountId) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.imapOptionsDesktopOnly')));
    return desktopImapGateway.loadSettings(accountId);
  },
  save(accountId, settings) {
    if (!isDesktop()) return Promise.reject(new Error(t('store.imapOptionsDesktopOnly')));
    return desktopImapGateway.saveSettings(accountId, settings);
  },
}));
