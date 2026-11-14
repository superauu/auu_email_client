import { readPreference, writePreference } from '../../lib/preferences';
import type { MailEvent } from '../../domain/mail';

// 演示事件完成状态存 localStorage 覆盖层；桌面版走 mail_events 表（mail_update_event）。
const KEY = 'auumail.demo.events.v1';

function validateOverlay(value: unknown): Record<string, boolean> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const entries = Object.entries(value as Record<string, unknown>).filter(([, completed]) => typeof completed === 'boolean');
  return Object.fromEntries(entries) as Record<string, boolean>;
}

export function loadDemoEvents(events: MailEvent[]): MailEvent[] {
  const overlay = readPreference(KEY, validateOverlay) ?? {};
  return events.map(event => (event.id in overlay ? { ...event, completed: overlay[event.id] } : event));
}

export function persistDemoEvent(id: string, completed: boolean): void {
  const overlay = readPreference(KEY, validateOverlay) ?? {};
  overlay[id] = completed;
  writePreference(KEY, overlay);
}
