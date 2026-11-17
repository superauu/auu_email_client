// Outbox 发送队列（docs/SENDING.md）：状态机与出口语义以该文档为准。
import { t } from '../i18n/core';
export type OutboxState = 'queued' | 'sending' | 'sent' | 'failed' | 'unknown';

export interface OutboxEntry {
  id: string;
  draftId: string;
  accountId: string;
  messageId: string | null;
  state: OutboxState;
  attempts: number;
  lastError: string | null;
  queuedAt: string | null;
  updatedAt: string;
  resolvedBy: string | null;
}

export function outboxSummary(entry: OutboxEntry): string {
  switch (entry.state) {
    case 'queued': return t('outbox.stateQueued');
    case 'sending': return t('outbox.stateSending');
    case 'sent': return entry.resolvedBy === 'sent-folder' ? t('outbox.stateSentConfirmed') : t('outbox.stateSent');
    case 'unknown': return t('outbox.stateUnknown', { attempts: entry.attempts });
    case 'failed': return entry.resolvedBy === 'keep-local' ? t('outbox.stateKeepLocal') : t('outbox.stateFailed', { error: entry.lastError ?? t('outbox.retryHint') });
  }
}
