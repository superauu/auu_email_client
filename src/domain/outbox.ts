// Outbox 发送队列（docs/SENDING.md）：状态机与出口语义以该文档为准。
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
    case 'queued': return '发送排队中';
    case 'sending': return '正在发送…';
    case 'sent': return entry.resolvedBy === 'sent-folder' ? '已发送（经 Sent 目录确认）' : '已发送';
    case 'unknown': return `结果不明（已尝试 ${entry.attempts} 次），不会自动重发`;
    case 'failed': return entry.resolvedBy === 'keep-local' ? '仅保留本机（投递结果未确认，不会再次发送）' : `发送失败：${entry.lastError ?? '请重试'}`;
  }
}
