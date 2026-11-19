import { getCurrentWindow } from '@tauri-apps/api/window';
import { isTauri, invoke } from '@tauri-apps/api/core';
import type { SkinCatalog } from '../themes/contract';
import type { SyncQueueSnapshot, SyncOutcome } from '../domain/sync';
export type { SyncOutcome } from '../domain/sync';
import type { MailAccount, MailDraft, DraftSaveOptions, MailEvent, MailLabel, MailMessage, MessagePatch, PurgeOutcome, RecoveryAction, ProviderKind, WorkspaceSnapshot } from '../domain/mail';
import { t } from '../i18n/core';

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
  folders?: import('../domain/mail').MailFolder[];
  accounts: MailAccount[];
  labels: MailLabel[];
  events: MailEvent[];
  syncStates: DesktopSyncState[];
}
export interface DesktopMessagePage {
  items: MailMessage[];
  nextCursor: string | null;
}
export interface DesktopMessageQuery {
  accountId?: string | null;
  mailbox?: string | null;
  folderId?: string | null;
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
    if (!isDesktop()) throw new Error(t('app.openSkinDirDesktopOnly'));
    await invoke('open_skin_directory');
  },
  /** Open a mail link in the system browser / mail app; Rust enforces the scheme allowlist. */
  async openExternal(url: string): Promise<void> {
    if (!isDesktop()) throw new Error(t('app.openExternalDesktopOnly'));
    await invoke('open_external', { url });
  },
  async windowAction(action: 'minimize' | 'maximize' | 'close' | 'drag'): Promise<void> {
    if (!isDesktop()) return;
    const window = getCurrentWindow();
    if (action === 'minimize') await window.minimize();
    if (action === 'maximize') await window.toggleMaximize();
    if (action === 'close') await window.close();
    if (action === 'drag') await window.startDragging();
  },
  async guardWindowClose(shouldBlock: () => boolean, onBlocked: () => void): Promise<() => void> {
    if (!isDesktop()) return () => {};
    return getCurrentWindow().onCloseRequested(event => {
      if (shouldBlock()) { event.preventDefault(); onBlocked(); }
    });
  },
};

export const desktopMail = {
  async messageHtml(accountId: string, messageId: string, allowRemoteImages: boolean, mode: 'light' | 'dark'): Promise<string> { return invoke<{ html: string }>('mail_message_html', { accountId, messageId, allowRemoteImages, mode }).then(payload => payload.html); },
  async messageHtmlFragment(accountId: string, messageId: string): Promise<string> { return invoke('mail_message_html_fragment', { accountId, messageId }); },
  async sendDraft(draftId: string, revision: number): Promise<import('../domain/outbox').OutboxEntry> { return invoke('mail_send_draft', { draftId, revision }); },
  async deleteDraft(draftId: string): Promise<void> { return invoke('mail_delete_draft', { draftId }); },
  async outboxList(): Promise<import('../domain/outbox').OutboxEntry[]> { return invoke('mail_outbox_list'); },
  async cancelOutbox(id: string): Promise<import('../domain/outbox').OutboxEntry> { return invoke('mail_outbox_cancel', { id }); },
  async returnOutboxDraft(id: string): Promise<import('../domain/outbox').OutboxEntry> { return invoke('mail_outbox_return_draft', { id }); },
  async resolveOutbox(id: string, action: 'keep-local' | 'retry'): Promise<import('../domain/outbox').OutboxEntry> { return invoke('mail_outbox_resolve', { id, action }); },
  async retryOutbox(id: string): Promise<import('../domain/outbox').OutboxEntry> { return invoke('mail_retry_outbox', { id }); },
  async updateEvent(id: string, completed: boolean): Promise<void> { return invoke('mail_update_event', { id, completed }); },
  async requestBody(accountId: string, messageId: string, revision: number): Promise<MailMessage> { return invoke('mail_request_body', { accountId, messageId, revision }); },
  async imapSettings(accountId: string): Promise<import('../domain/imap').ImapSettings> { return invoke('mail_imap_settings', { accountId }); },
  async saveImapSettings(accountId: string, settings: import('../domain/imap').ImapSettings): Promise<void> { return invoke('mail_save_imap_settings', { accountId, settings }); },
  /** 每账号签名：读取（未设置返回默认值）与保存（Rust 侧 compose 消毒后入库）。 */
  async accountSignature(accountId: string): Promise<import('../domain/signature').AccountSignature> { return invoke('mail_account_signature', { accountId }); },
  async saveAccountSignature(accountId: string, signature: import('../domain/signature').AccountSignature): Promise<void> { return invoke('mail_save_account_signature', { accountId, signature }); },
  /** 签名图片选取：Rust 文件对话框，返回 data URI（取消返回 null）。 */
  async pickImage(): Promise<{ dataUri: string; name: string; size: number } | null> { return invoke('mail_compose_pick_image'); },
  /** 服务器草稿上传：IMAP APPEND 到草稿目录副本，返回字节数。 */
  async uploadDraft(accountId: string, draftId: string, revision: number): Promise<number> { return invoke('mail_upload_draft', { accountId, draftId, revision }); },
  /** 远程图片信任列表（v21）：读取 / 添加 / 移除（kind='sender'|'domain'，value 小写）。 */
  async trustedSenders(): Promise<unknown[]> { return invoke('mail_trusted_senders'); },
  async trustAdd(kind: 'sender' | 'domain', value: string): Promise<unknown> { return invoke('mail_trust_add', { kind, value }); },
  async trustRemove(kind: 'sender' | 'domain', value: string): Promise<void> { return invoke('mail_trust_remove', { kind, value }); },
  async exportAttachment(accountId: string, messageId: string, index: number): Promise<boolean> { return invoke('mail_export_attachment', { accountId, messageId, index }); },
  async requestRecovery(accountId: string, messageId: string, revision: number, action: RecoveryAction): Promise<MailMessage> { return invoke('mail_request_recovery', { accountId, messageId, revision, action }); },
  async updateMessage(accountId: string, messageId: string, patch: MessagePatch): Promise<MailMessage> { return invoke('mail_update_message', { accountId, messageId, patch }); },
  /** 废纸篓永久删除：IMAP 排队 EXPUNGE（queued 返回更新行），POP3 / 演示立即本机移除（removed）。 */
  async purgeMessage(accountId: string, messageId: string): Promise<PurgeOutcome> { return invoke('mail_purge_message', { accountId, messageId }); },
  async keepMessageLocal(accountId: string, messageId: string): Promise<MailMessage> { return invoke('mail_keep_message_local', { accountId, messageId }); },
  /** Idempotently writes the demo dataset into SQLite; leaves any other rows untouched. */
  async seed(snapshot: WorkspaceSnapshot): Promise<void> { await invoke('mail_seed_workspace', { seed: snapshot }); },
  async workspace(): Promise<DesktopWorkspaceBase> { return invoke('mail_load_workspace'); },
  async page(query: DesktopMessageQuery): Promise<DesktopMessagePage> { return invoke('mail_page_messages', { query }); },
  /** 侧栏视图计数：与 mailbox 投影一致的 total/unread 聚合，大邮箱无需加载全部邮件。 */
  async folderCounts(): Promise<import('../domain/mail').ViewCountEntry[]> { return invoke('mail_folder_counts'); },
  /** 撰写收件人建议：最近邮件的发件人与收件人（去重保序，Rust 侧限量）。 */
  async contactSuggestions(accountId: string): Promise<import('../domain/mail').ContactSuggestion[]> { return invoke('mail_contact_suggestions', { accountId }); },
  /** Incremental IMAP sync for a real account; the credential never leaves Rust. */
  async sync(accountId: string): Promise<SyncOutcome> { return invoke('mail_sync_account', { id: accountId }); },
  async enqueueSync(ids: string[]): Promise<SyncQueueSnapshot> { return invoke('mail_enqueue_sync', { ids }); },
  async syncJobs(): Promise<SyncQueueSnapshot> { return invoke('mail_sync_jobs'); },
  async cancelSync(jobId: string): Promise<SyncQueueSnapshot> { return invoke('mail_cancel_sync', { jobId }); },
  async drafts(): Promise<MailMessage[]> { return invoke('mail_list_drafts'); },
  async saveDraft(draft: MailDraft, options?: DraftSaveOptions): Promise<MailMessage> {
    return invoke('mail_save_draft', { draft, previousAccountId: options?.previousAccountId ?? draft.accountId });
  },
  /** Rust 侧多选文件对话框 + 暂存；仅元数据回传，路径与字节不出 Rust。 */
  async pickAttachments(draftId: string, current: import('../domain/mail').DraftAttachment[]): Promise<import('../domain/mail').DraftAttachment[]> {
    return invoke('mail_compose_pick_attachments', {
      draftId,
      currentTotalBytes: current.reduce((total, item) => total + item.size, 0),
      currentCount: current.length,
    });
  },
  async importLegacyDrafts(drafts: MailDraft[]): Promise<void> { await invoke('mail_import_legacy_drafts', { drafts }); },
};

/** OS credential storage. Secrets travel in only; nothing here ever returns one. */
export const desktopCredentials = {
  async status(provider: ProviderKind, accountId: string): Promise<boolean> { return invoke('credential_status', { provider, accountId }); },
  async save(provider: ProviderKind, accountId: string, secret: string): Promise<void> { await invoke('credential_save', { provider, accountId, secret }); },
  async delete(provider: ProviderKind, accountId: string): Promise<void> { await invoke('credential_delete', { provider, accountId }); },
};

export interface MailEndpoints {
  /** 'imap'（默认）或 'pop3'：收件探针选择。 */
  provider?: string;
  receiveOnly?: boolean;
  imapHost: string;
  imapPort: number;
  imapSecurity?: 'tls' | 'starttls';
  smtpHost: string;
  smtpPort: number;
  authUser: string;
}
export interface MailProbeOutcome {
  skipped?: boolean;
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
  /** 'imap'（默认）或 'pop3'：决定保存的账号协议与同步分支。 */
  provider?: string;
  receiveOnly?: boolean;
  id?: string | null;
  name: string;
  email: string;
  imapHost: string;
  imapPort: number;
  imapSecurity?: 'tls' | 'starttls';
  smtpHost: string;
  smtpPort: number;
  authUser: string;
  avatar: string;
  color: 'teal' | 'blue' | 'amber';
}

/** 远端目录 CRUD（仅 IMAP 自定义目录）；返回该账号最新目录列表。 */
export const desktopFolders = {
  async create(accountId: string, name: string): Promise<import('../domain/mail').MailFolder[]> { return invoke('mail_create_folder', { accountId, name }); },
  async rename(accountId: string, folderId: string, newName: string): Promise<import('../domain/mail').MailFolder[]> { return invoke('mail_rename_folder', { accountId, folderId, newName }); },
  async remove(accountId: string, folderId: string): Promise<import('../domain/mail').MailFolder[]> { return invoke('mail_delete_folder', { accountId, folderId }); },
};

/** Real-account lifecycle: verify a connection, persist it, or remove it. */
export const desktopAccounts = {
  async configuration(id: string): Promise<RealAccountInput> { return invoke('mail_account_config', { id }); },
  async startOAuth(account: RealAccountInput, config: import('../domain/oauth').OAuthConfig): Promise<string> { return invoke('mail_oauth_start', { account, config }); },
  async oauthStatus(flowId: string): Promise<import('../domain/oauth').OAuthFlow> { return invoke('mail_oauth_status', { flowId }); },
  async cancelOAuth(flowId: string): Promise<import('../domain/oauth').OAuthFlow> { return invoke('mail_oauth_cancel', { flowId }); },
  async verify(endpoints: MailEndpoints, secret: string): Promise<MailVerifyReport> { return invoke('mail_verify_account', { endpoints, secret }); },
  async save(account: RealAccountInput, secret: string): Promise<string> { return invoke('mail_save_account', { account, secret }); },
  async remove(id: string): Promise<void> { await invoke('mail_delete_account', { id }); },
};
