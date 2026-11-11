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
  field: 'unread' | 'starred' | 'archive';
  state: 'queued' | 'applying' | 'failed' | 'conflict' | 'unknown' | 'local';
  error: string | null;
}
export interface MessagePatch {
  unread?: boolean;
  starred?: boolean;
  archive?: true;
}

export interface MailDraft {
  id: string;
  accountId: string;
  to: string;
  subject: string;
  bodyText: string;
  replyToId?: string;
  updatedAt: Timestamp;
  /** Persisted revision; omitted only for legacy drafts, which are treated as revision zero. */
  revision?: number;
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

export interface MailQuery {
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
  /** 'demo': browser fixtures; 'desktop-local': SQLite cache with demo and synced accounts; 'live': direct provider gateway. */
  readonly mode: 'demo' | 'desktop-local' | 'live';
  loadWorkspace(signal?: AbortSignal): Promise<WorkspaceSnapshot>;
  saveDraft(draft: MailDraft, options?: DraftSaveOptions): Promise<MailMessage>;
  updateMessage(accountId: string, messageId: string, patch: MessagePatch): Promise<MailMessage>;
  keepMessageLocal(accountId: string, messageId: string): Promise<MailMessage>;
  requestRecovery(accountId: string, messageId: string, revision: number, action: RecoveryAction): Promise<MailMessage>;
  // Add sync/send through explicit capability interfaces when a provider is implemented.
}
