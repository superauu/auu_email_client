/** UTC ISO-8601 at every persistence / IPC boundary; localize only when rendering. */
export type Timestamp = string;
export type ProviderKind = 'imap' | 'pop3' | 'microsoft' | 'gmail';
export type MailboxKind = 'inbox' | 'starred' | 'sent' | 'drafts' | 'archive' | 'trash';

export interface MailCapabilities {
  folders: boolean;
  serverLabels: boolean;
  readStateSync: boolean;
  threads: boolean;
  calendar: boolean;
}

export interface MailAccount {
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
}

export interface MailDraft {
  id: string;
  accountId: string;
  to: string;
  subject: string;
  bodyText: string;
  replyToId?: string;
  updatedAt: Timestamp;
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
  accounts: MailAccount[];
  messages: MailMessage[];
  labels: MailLabel[];
  events: MailEvent[];
}

export interface MailQuery {
  accountId: string | null;
  mailbox: MailboxKind;
  labelId: string | null;
  search: string;
}

export interface MailGateway {
  /** 'demo': in-memory demo data; 'desktop-local': real local SQLite via IPC, demo content; 'live': a connected provider. */
  readonly mode: 'demo' | 'desktop-local' | 'live';
  loadWorkspace(signal?: AbortSignal): Promise<WorkspaceSnapshot>;
  saveDraft(draft: MailDraft): Promise<MailMessage>;
  // Add sync/send through explicit capability interfaces when a provider is implemented.
}
