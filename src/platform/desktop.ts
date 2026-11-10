import { isTauri, invoke } from '@tauri-apps/api/core';
import type { SkinCatalog } from '../themes/contract';
import type { MailAccount, MailEvent, MailLabel, MailMessage, ProviderKind, WorkspaceSnapshot } from '../domain/mail';

export interface AppInfo {
  version: string;
  dataDirectory: string;
  skinDirectory: string;
  databaseReady: boolean;
}
/** Workspace without messages: messages always arrive through the paged query. */
export interface DesktopSyncState {
  accountId: string;
  scope: string;
  updatedAt: string;
}
export interface DesktopWorkspaceBase {
  accounts: MailAccount[];
  labels: MailLabel[];
  events: MailEvent[];
  syncStates: DesktopSyncState[];
}
export interface SyncOutcome {
  folders: number;
  fetchedMessages: number;
  syncedAt: string;
  warnings: string[];
}
export interface DesktopMessagePage {
  items: MailMessage[];
  nextCursor: string | null;
}
export interface DesktopMessageQuery {
  accountId?: string | null;
  mailbox?: string | null;
  labelId?: string | null;
  search?: string | null;
  cursor?: string | null;
  limit?: number | null;
}

export const isDesktop = () => isTauri();
export const desktop = {
  async info(): Promise<AppInfo | null> { return isDesktop() ? invoke('get_app_info') : null; },
  async listSkins(): Promise<SkinCatalog> {
    return isDesktop() ? invoke('list_user_skins') : { skins: [], warnings: [] };
  },
  async openSkinDirectory(): Promise<void> {
    if (!isDesktop()) throw new Error('请在桌面版中打开皮肤目录。');
    await invoke('open_skin_directory');
  },
  async windowAction(action: 'minimize' | 'maximize' | 'close' | 'drag'): Promise<void> {
    if (!isDesktop()) return;
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const window = getCurrentWindow();
    if (action === 'minimize') await window.minimize();
    if (action === 'maximize') await window.toggleMaximize();
    if (action === 'close') await window.close();
    if (action === 'drag') await window.startDragging();
  },
};

export const desktopMail = {
  /** Idempotently writes the demo dataset into SQLite; leaves any other rows untouched. */
  async seed(snapshot: WorkspaceSnapshot): Promise<void> { await invoke('mail_seed_workspace', { seed: snapshot }); },
  async workspace(): Promise<DesktopWorkspaceBase> { return invoke('mail_load_workspace'); },
  async page(query: DesktopMessageQuery): Promise<DesktopMessagePage> { return invoke('mail_page_messages', { query }); },
  /** Incremental IMAP sync for a real account; the credential never leaves Rust. */
  async sync(accountId: string): Promise<SyncOutcome> { return invoke('mail_sync_account', { id: accountId }); },
};

/** OS credential storage. Secrets travel in only; nothing here ever returns one. */
export const desktopCredentials = {
  async status(provider: ProviderKind, accountId: string): Promise<boolean> { return invoke('credential_status', { provider, accountId }); },
  async save(provider: ProviderKind, accountId: string, secret: string): Promise<void> { await invoke('credential_save', { provider, accountId, secret }); },
  async delete(provider: ProviderKind, accountId: string): Promise<void> { await invoke('credential_delete', { provider, accountId }); },
};

export interface MailEndpoints {
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  authUser: string;
}
export interface MailProbeOutcome {
  ok: boolean;
  error: string | null;
  code: 'network' | 'auth' | null;
}
export interface MailVerifyReport {
  imap: MailProbeOutcome;
  smtp: MailProbeOutcome;
  folderCount: number | null;
}
export interface RealAccountInput {
  id?: string | null;
  name: string;
  email: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
  authUser: string;
  avatar: string;
  color: 'teal' | 'blue' | 'amber';
}

/** Real-account lifecycle: verify a connection, persist it, or remove it. */
export const desktopAccounts = {
  async verify(endpoints: MailEndpoints, secret: string): Promise<MailVerifyReport> { return invoke('mail_verify_account', { endpoints, secret }); },
  async save(account: RealAccountInput, secret: string): Promise<string> { return invoke('mail_save_account', { account, secret }); },
  async remove(id: string): Promise<void> { await invoke('mail_delete_account', { id }); },
};
