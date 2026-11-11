export interface ImapSettings {
  syncAllFolders: boolean;
  historyEnabled: boolean;
  autoSyncSeconds: number;
}
export interface ImapGateway {
  loadSettings(accountId: string): Promise<ImapSettings>;
  saveSettings(accountId: string, settings: ImapSettings): Promise<void>;
}
