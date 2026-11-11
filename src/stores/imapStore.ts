import { create } from 'zustand';
import type { ImapSettings } from '../domain/imap';
import { desktopImapGateway } from '../services/desktop/DesktopImapGateway';
import { isDesktop } from '../platform/desktop';
export const useImapStore = create<{
  load: (accountId: string) => Promise<ImapSettings>;
  save: (accountId: string, settings: ImapSettings) => Promise<void>;
}>(() => ({
  load(accountId) {
    if (!isDesktop()) return Promise.reject(new Error('IMAP 设置仅在桌面版可用。'));
    return desktopImapGateway.loadSettings(accountId);
  },
  save(accountId, settings) {
    if (!isDesktop()) return Promise.reject(new Error('IMAP 设置仅在桌面版可用。'));
    return desktopImapGateway.saveSettings(accountId, settings);
  },
}));
