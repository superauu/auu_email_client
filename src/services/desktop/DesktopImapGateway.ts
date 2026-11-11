import type { ImapGateway } from '../../domain/imap';
import { desktopMail } from '../../platform/desktop';
export const desktopImapGateway: ImapGateway = {
  loadSettings: accountId => desktopMail.imapSettings(accountId),
  saveSettings: (accountId, settings) => desktopMail.saveImapSettings(accountId, settings),
};
