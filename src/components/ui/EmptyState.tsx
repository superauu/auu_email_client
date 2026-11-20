import { Inbox } from 'lucide-react';
import type { ReactNode } from 'react';

/** heading：页面级空态用 h1（默认）；嵌在已有 h1 区域（列表 / 阅读区）内时传 h2，避免同级出现多个 h1。 */
export function EmptyState({ title, description, action, heading = 'h1' }: { title: string; description: string; action?: ReactNode; heading?: 'h1' | 'h2' }) {
  const Heading = heading;
  return <div className="empty-state"><span className="empty-icon"><Inbox size={28} /></span><Heading>{title}</Heading><p>{description}</p>{action}</div>;
}
