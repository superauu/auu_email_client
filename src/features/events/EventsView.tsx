import { ArrowUpRight, CalendarDays, Check } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';
import { isDesktop } from '../../platform/desktop';
import { useI18n } from '../../i18n/react';

export function EventsView() {
  const t = useI18n();
  const { events, query, toggleEvent, openMessage } = useWorkspaceStore(useShallow(s => ({
    events: s.events, query: s.query, toggleEvent: s.toggleEvent, openMessage: s.openMessage,
  })));
  const visible = events.filter(event => !query.accountId || event.accountId === query.accountId);
  return <main className="events-view"><span className="eyebrow">A LITTLE MORE ORGANIZED</span><h1>{t('events.heading')}</h1><p className="muted">{isDesktop() ? t('events.desktopNote') : t('events.demoNote')}</p>
    <div className="event-list">{visible.length ? visible.map(event => <article className="event-card" key={event.id}>
      <button className={`event-check ${event.completed ? 'is-completed' : ''}`} aria-label={t('events.toggleAria', { action: event.completed ? t('events.markUncompleted') : t('events.complete'), title: event.title })} aria-pressed={event.completed} onClick={() => toggleEvent(event.id)}>{event.completed && <Check size={15} />}</button>
      <div><h2 className={event.completed ? 'completed-text' : ''}>{event.title}</h2><p><CalendarDays size={14} />{new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short', timeZone: event.timeZone }).format(new Date(event.startsAt))} · {event.timeZone}</p></div>
      {event.messageId && <Button variant="ghost" onClick={() => openMessage(event.messageId!)}>{t('events.openMessage')}<ArrowUpRight size={15} /></Button>}
    </article>) : <EmptyState title={t('events.emptyTitle')} description={t('events.emptyBody')} />}</div>
  </main>;
}
