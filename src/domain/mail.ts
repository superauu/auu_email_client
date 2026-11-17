/** UTC ISO-8601 at every persistence / IPC boundary; localize only when rendering. */
export type Timestamp = string;
export type ProviderKind = 'imap' | 'pop3' | 'microsoft' | 'gmail';
export type MailboxKind = 'inbox' | 'starred' | 'sent' | 'drafts' | 'archive' | 'trash' | 'other';

export interface MailCapabilities {
  folders: boolean;
  serverLabels: boolean;
  readStateSync: boolean;
  threads: boolean;
  calendar: boolean;
  /** SMTP 发送能力：仅收信账号、Microsoft（未授权 SMTP.Send）与演示账号为 false。 */
  send: boolean;
}

export interface MailAccount {
  oauth?: import('./oauth').OAuthConfig | null;
  authUser?: string;
  id: string;
  name: string;
  email: string;
  provider: ProviderKind;
  avatar: string;
  color: 'teal' | 'blue' | 'amber';
  status: 'demo' | 'connected' | 'offline' | 'reauthorization-required';
  capabilities: MailCapabilities;
}

export interface MailLabel {
  id: string;
  name: string;
  color: 'teal' | 'blue' | 'amber';
  scope: 'local' | 'server';
  accountId?: string;
  remoteId?: string;
}

export interface MailMessage {
  bodyDownload?: { state: 'queued' | 'failed'; error: string | null } | null;
  folderIds?: string[];
  attachments?: { index: number; name: string; size: number; mediaType: string }[];
  remoteMissing?: boolean;
  id: string;
  accountId: string;
  remoteId?: string;
  threadId: string;
  mailbox: Exclude<MailboxKind, 'starred'>;
  from: { name: string; email: string };
  to: string[];
  /** 抄送地址（收件侧缓存投影）；缺省 = 未解析或无抄送。 */
  cc?: string[];
  /** Reply-To 头地址；缺省 = 未提供，回复时退回发件人。 */
  replyTo?: string[];
  subject: string;
  preview: string;
  /** Plain text only. Remote HTML must go through a future isolated renderer. */
  bodyText: string;
  receivedAt: Timestamp;
  unread: boolean;
  starred: boolean;
  labelIds: string[];
  /** Present only for a locally editable draft; preserves empty subjects and reply metadata. */
  draft?: MailDraft;
  stateRevision?: number;
  changes?: MessageChange[];
  recovery?: RecoveryStatus | null;
}

export type RecoveryAction = 'check' | 'confirm' | 'retry-archive' | 'continue' | 'auto-continue' | 'fingerprint' | 'confirm-fingerprint';
export interface RecoveryStatus {
  autoContinue?: boolean;
  state: 'available' | 'queued' | 'checking' | 'scanning' | 'located' | 'source-present' | 'unresolved' | 'failed' | 'restored' | 'local' | 'stale';
  detail: string;
  location: { folder: string; kind: string; uidValidity: number; uid: number } | null;
  evidence: 'raw-sha256' | 'cached-fields' | null;
  progress?: { checked: number; total: number; matches: number; segments?: { completed: number; total: number; phase: 'collecting' | 'validating' } | null } | null;
  crossUidValidity?: boolean;
  fingerprint?: { eligible: boolean; reason: string } | null;
  operation?: 'recovery' | 'fingerprint';
}

export interface MessageChange {
  field: 'unread' | 'starred' | 'archive' | 'delete' | 'move';
  state: 'queued' | 'applying' | 'failed' | 'conflict' | 'unknown' | 'local';
  error: string | null;
}
export interface MessagePatch {
  unread?: boolean;
  starred?: boolean;
  archive?: true;
  /** 删除到废纸篓（IMAP UID MOVE；POP3 / 演示仅本机投影）。 */
  delete?: true;
  /** 移动到指定文件夹（folders.id）。 */
  moveTo?: string;
}

export interface MailDraft {
  id: string;
  accountId: string;
  to: string;
  cc?: string;
  bcc?: string;
  subject: string;
  bodyText: string;
  /** 富文本正文（编辑器产出，服务端消毒后持久化）；缺省 = 纯文本草稿。 */
  bodyHtml?: string;
  replyToId?: string;
  /** 撰写附件元数据；内容保存在 Rust 暂存区，保存草稿时落 SQLite，前端不回传字节。 */
  attachments?: DraftAttachment[];
  updatedAt: Timestamp;
  /** Persisted revision; omitted only for legacy drafts, which are treated as revision zero. */
  revision?: number;
}

/** 撰写附件元数据：id 稳定句柄（att-*）；sha256 与 size 在保存 / 发送时核对内容一致性。 */
export interface DraftAttachment {
  id: string;
  name: string;
  size: number;
  mediaType: string;
  sha256: string;
}

export interface DraftSaveOptions {
  /** Last persisted owner, required when moving an existing draft to another sender. */
  previousAccountId: string;
}

export interface MailEvent {
  id: string;
  accountId: string;
  messageId?: string;
  title: string;
  startsAt: Timestamp;
  timeZone: string;
  completed: boolean;
  source: 'local' | 'ics' | 'provider';
}

export interface WorkspaceSnapshot {
  folders?: MailFolder[];
  accounts: MailAccount[];
  messages: MailMessage[];
  labels: MailLabel[];
  events: MailEvent[];
  /** accountId → last successful sync time (ISO). Desktop gateway only; absent for demo. */
  syncStates?: Record<string, string>;
}

/**
 * 回复目标：普通回复优先 Reply-To（空或只含本账号时退回发件人）；
 * 回复全部合并发件人 + To + Cc，按地址去重并剔除本账号；To 全被剔除时降级到 Cc。
 */
export function replyTargets(
  message: Pick<MailMessage, 'from' | 'to' | 'cc' | 'replyTo'>,
  ownEmail: string,
  mode: 'reply' | 'reply-all',
): { to: string[]; cc: string[] } {
  const own = ownEmail.trim().toLowerCase();
  const seen = new Set<string>();
  const collect = (addresses: (string | undefined)[]) =>
    addresses.filter((address): address is string => !!address?.trim()).flatMap(address => address.split(',')).map(address => address.trim()).filter(address => {
      if (!address) return false;
      const key = address.toLowerCase();
      if (key === own || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  if (mode === 'reply') {
    const direct = collect(message.replyTo ?? []);
    return { to: direct.length ? direct : collect([message.from.email]), cc: [] };
  }
  const to = collect([message.from.email, ...message.to]);
  const cc = collect(message.cc ?? []);
  return to.length ? { to, cc } : { to: cc, cc: [] };
}

export interface MailQuery {
  /** UI-only distinction; local drafts have editable draft metadata, server copies do not. */
  draftSource?: 'local' | 'server' | null;
  unreadOnly?: boolean;
  folderId?: string | null;
  accountId: string | null;
  mailbox: MailboxKind;
  labelId: string | null;
  search: string;
}
export interface MailFolder { id: string; accountId: string; name: string; kind: string; available: boolean }

export interface MailGateway {
  requestBody(accountId: string, messageId: string, revision: number): Promise<MailMessage>;
  exportAttachment(accountId: string, messageId: string, index: number): Promise<boolean>;
  /** Sanitized sandbox srcdoc for the message HTML body; rejects when no verified raw cache exists. */
  messageHtml?(accountId: string, messageId: string, allowRemoteImages: boolean, mode: 'light' | 'dark'): Promise<string>;
  /** Persist event completion state (desktop: SQLite; demo: localStorage overlay). */
  updateEvent?(id: string, completed: boolean): Promise<void>;
  /** Sanitized HTML fragment for reply/forward quoting; rejects when no verified raw cache exists. */
  messageHtmlFragment?(accountId: string, messageId: string): Promise<string>;
  /** Register a persisted draft into the outbox queue (docs/SENDING.md); rejects honestly. */
  sendDraft?(draftId: string, revision: number): Promise<import('./outbox').OutboxEntry>;
  /** Delete a local draft after the user explicitly discards it; idempotent, rejects while queued in outbox. */
  deleteDraft?(draftId: string): Promise<void>;
  /**
   * 撰写附件选取：Rust 侧打开多选对话框、读入暂存区，仅返回新增附件元数据；
   * 取消选择返回空数组。演示环境不支持（诚实拒绝）。current 为草稿现有附件。
   */
  pickAttachments?(draftId: string, current: DraftAttachment[]): Promise<DraftAttachment[]>;
  /** 'demo': browser fixtures; 'desktop-local': SQLite cache with demo and synced accounts; 'live': direct provider gateway. */
  readonly mode: 'demo' | 'desktop-local' | 'live';
  loadWorkspace(signal?: AbortSignal): Promise<WorkspaceSnapshot>;
  saveDraft(draft: MailDraft, options?: DraftSaveOptions): Promise<MailMessage>;
  updateMessage(accountId: string, messageId: string, patch: MessagePatch): Promise<MailMessage>;
  keepMessageLocal(accountId: string, messageId: string): Promise<MailMessage>;
  requestRecovery(accountId: string, messageId: string, revision: number, action: RecoveryAction): Promise<MailMessage>;
  // Add sync/send through explicit capability interfaces when a provider is implemented.
}
