import type { AppError } from './errors';

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
export function syncJobSummary(job: SyncJob): string {
  if (job.status === 'queued') return '排队中';
  if (job.status === 'running') return job.cancelRequested ? '正在停止同步…' : '同步中…';
  if (job.status === 'cancelled') return job.startedAt ? '同步已停止，已保存批次保留' : '已取消排队';
  if (job.status === 'failed') return `同步失败：${job.error?.message ?? '请重试'}`;
  const outcome = job.outcome;
  if (outcome?.retryNeeded) return `部分文件夹未完成，已提交邮件保留；${outcome.warnings.join('；')}`;
  if (outcome?.listening) return `已同步 · 新邮件通知已启用${outcome.fetchedMessages ? `，拉取 ${outcome.fetchedMessages} 封邮件` : ''}`;
  return outcome ? `同步完成：${outcome.folders} 个文件夹，拉取 ${outcome.fetchedMessages} 封邮件${outcome.warnings.length ? `；${outcome.warnings.join('；')}` : ''}` : '同步完成';
}
