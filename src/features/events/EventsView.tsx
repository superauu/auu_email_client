import { ArrowUpRight, CalendarDays, Check } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';
import { isDesktop } from '../../platform/desktop';
import { currentLanguage, useI18n } from '../../i18n/react';
import type { MailEvent } from '../../domain/mail';

/** 事件时间格式化：floating（本机墙上时间）不带固定时区；非法时区回退查看者本地。 */
function formatEventTime(event: MailEvent): string {
  const options: Intl.DateTimeFormatOptions = { dateStyle: 'medium', timeStyle: 'short' };
  if (event.timeZone !== 'floating') options.timeZone = event.timeZone;
  try { return new Intl.DateTimeFormat(currentLanguage(), options).format(new Date(event.startsAt)); }
  catch { return new Intl.DateTimeFormat(currentLanguage(), { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(event.startsAt)); }
}

export function EventsView() {
  const t = useI18n();
  const { events, query, toggleEvent, openMessage } = useWorkspaceStore(useShallow(s => ({
    events: s.events, query: s.query, toggleEvent: s.toggleEvent, openMessage: s.openMessage,
  })));
  const visible = events.filter(event => !query.accountId || event.accountId === query.accountId);
  return <main className="events-view"><span className="eyebrow">A LITTLE MORE ORGANIZED</span><h1>{t('events.heading')}</h1><p className="muted">{isDesktop() ? t('events.desktopNote') : t('events.demoNote')}</p>
    <div className="event-list">{visible.length ? visible.map(event => <article className="event-card" key={event.id}>
      <button className={`event-check ${event.completed ? 'is-completed' : ''}`} aria-label={t('events.toggleAria', { action: event.completed ? t('events.markUncompleted') : t('events.complete'), title: event.title })} aria-pressed={event.completed} onClick={() => toggleEvent(event.id)}>{event.completed && <Check size={15} />}</button>
      <div><h2 className={event.completed ? 'completed-text' : ''}>{event.title}</h2><p><CalendarDays size={14} />{formatEventTime(event)} · {event.timeZone === 'floating' ? t('events.floatingZone') : event.timeZone}{event.source === 'ics' && <span className="event-source">{t('events.icsBadge')}</span>}</p></div>
      {event.messageId && <Button variant="ghost" onClick={() => openMessage(event.messageId!)}>{t('events.openMessage')}<ArrowUpRight size={15} /></Button>}
    </article>) : <EmptyState title={t('events.emptyTitle')} description={t('events.emptyBody')} />}</div>
  </main>;
}
