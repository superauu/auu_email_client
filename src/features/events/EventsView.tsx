import { ArrowUpRight, CalendarDays, Check } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';

export function EventsView() {
  const { events, query, toggleEvent, openMessage } = useWorkspaceStore(useShallow(s => ({
    events: s.events, query: s.query, toggleEvent: s.toggleEvent, openMessage: s.openMessage,
  })));
  const visible = events.filter(event => !query.accountId || event.accountId === query.accountId);
  return <main className="events-view"><span className="eyebrow">A LITTLE MORE ORGANIZED</span><h1>重要的事，留在这里</h1><p className="muted">关联邮件的事件与待办。当前展示的是本机演示内容。</p>
    <div className="event-list">{visible.length ? visible.map(event => <article className="event-card" key={event.id}>
      <button className={`event-check ${event.completed ? 'is-completed' : ''}`} aria-label={`${event.completed ? '标记未完成' : '完成'}：${event.title}`} aria-pressed={event.completed} onClick={() => toggleEvent(event.id)}>{event.completed && <Check size={15} />}</button>
      <div><h2 className={event.completed ? 'completed-text' : ''}>{event.title}</h2><p><CalendarDays size={14} />{new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short', timeZone: event.timeZone }).format(new Date(event.startsAt))} · {event.timeZone}</p></div>
      {event.messageId && <Button variant="ghost" onClick={() => openMessage(event.messageId!)}>查看邮件<ArrowUpRight size={15} /></Button>}
    </article>) : <EmptyState title="这个账号还没有事件" description="与邮件关联的待办和提醒会显示在这里。" />}</div>
  </main>;
}
