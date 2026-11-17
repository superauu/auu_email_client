import type { AppError } from './errors';
import { t } from '../i18n/core';

export interface SyncOutcome {
  refreshNeeded?: boolean;
  retryNeeded?: boolean;
  listening?: boolean;
  hasMore?: boolean;
  moreMail?: boolean;
  folders: number;
  fetchedMessages: number;
  syncedAt: string;
  warnings: string[];
}
export type SyncJobStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
export interface SyncJob {
  cancelRequested?: boolean;
  id: string;
  accountId: string;
  status: SyncJobStatus;
  queuedAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  outcome: SyncOutcome | null;
  error: AppError | null;
}
export interface SyncQueueSnapshot { revision: number; jobs: SyncJob[] }
export const isSyncActive = (job: SyncJob) => job.status === 'queued' || job.status === 'running';
/** 可同步账号：演示账号之外均可（IMAP / Gmail OAuth / POP3 走同一队列）。 */
export function syncableAccounts(accounts: { id: string; status: string; provider: string }[]): { id: string; provider: string }[] {
  return accounts.filter(account => account.status !== 'demo').map(account => ({ id: account.id, provider: account.provider }));
}
export function syncJobSummary(job: SyncJob): string {
  if (job.status === 'queued') return t('settings.queued');
  if (job.status === 'running') return job.cancelRequested ? t('syncSummary.stopping') : t('settings.syncing');
  if (job.status === 'cancelled') return job.startedAt ? t('syncSummary.stopped') : t('syncSummary.cancelledQueued');
  if (job.status === 'failed') return t('syncSummary.failed', { error: job.error?.message ?? t('outbox.retryHint') });
  const outcome = job.outcome;
  if (outcome?.retryNeeded) return t('syncSummary.retryNeeded', { warnings: outcome.warnings.join(t('syncSummary.listSeparator')) });
  if (outcome?.listening) return t('syncSummary.listening', { extra: outcome.fetchedMessages ? t('syncSummary.listeningFetched', { count: outcome.fetchedMessages }) : '' });
  return outcome ? t('syncSummary.done', { folders: outcome.folders, count: outcome.fetchedMessages, warnings: outcome.warnings.length ? `${t('syncSummary.listSeparator')}${outcome.warnings.join(t('syncSummary.listSeparator'))}` : '' }) : t('syncSummary.donePlain');
}
