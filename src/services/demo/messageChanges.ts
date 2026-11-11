import type { MailMessage, MessagePatch } from '../../domain/mail';
import { demoWorkspace } from './fixtures';

const KEY = 'auumail.demo.message-changes.v1';
type Saved = { id: string; unread?: boolean; starred?: boolean; archive?: true; revision: number };
function read(): Saved[] {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    if (!Array.isArray(raw)) return [];
    return raw.filter((value): value is Saved => typeof value === 'object' && value !== null &&
      typeof value.id === 'string' && demoWorkspace.messages.some(m => m.id === value.id) &&
      Number.isSafeInteger(value.revision) && value.revision >= 0 &&
      (value.unread === undefined || typeof value.unread === 'boolean') &&
      (value.starred === undefined || typeof value.starred === 'boolean') &&
      (value.archive === undefined || value.archive === true)).slice(0, demoWorkspace.messages.length);
  } catch { return []; }
}
function project(message: MailMessage, saved?: Saved): MailMessage {
  if (!saved) return message;
  return { ...message, ...(saved.unread !== undefined ? { unread: saved.unread } : {}),
    ...(saved.starred !== undefined ? { starred: saved.starred } : {}), ...(saved.archive ? { mailbox: 'archive' } : {}), stateRevision: saved.revision,
    changes: (['unread', 'starred', 'archive'] as const).filter(field => saved[field] !== undefined).map(field => ({ field, state: 'local', error: null })) };
}
export function loadDemoMessageChanges(messages: MailMessage[]) {
  const saved = read(); return messages.map(message => project(message, saved.find(value => value.id === message.id)));
}
export function persistDemoMessage(accountId: string, id: string, patch: MessagePatch): MailMessage {
  const message = demoWorkspace.messages.find(message => message.id === id && message.accountId === accountId);
  if (!message || message.mailbox === 'drafts') throw new Error('演示邮件不存在或不支持修改。');
  const saved = read(); const prior = saved.find(value => value.id === id);
  const next: Saved = { ...prior, ...patch, id, revision: (prior?.revision ?? 0) + 1 };
  try { localStorage.setItem(KEY, JSON.stringify([...saved.filter(value => value.id !== id), next])); }
  catch { throw new Error('本机存储不可用，邮件状态未保存，请重试。'); }
  return project(structuredClone(message), next);
}
