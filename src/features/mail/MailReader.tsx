import { Archive, ArrowLeft, CornerUpLeft, PenLine, Sparkles, Star } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import type { MailMessage } from '../../domain/mail';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../../components/ui/Button';
import { Avatar } from '../../components/ui/Avatar';
import { EmptyState } from '../../components/ui/EmptyState';
import { selectMessages } from './selectors';

export function MailReader({ onCompose, onAi, onBack }: { onCompose: (message: MailMessage, editing?: boolean) => void; onAi: () => void; onBack: () => void }) {
  const { messages, selectedId, accounts, labels, query, toggleStar, archiveMessage } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, selectedId: s.selectedId, accounts: s.accounts, labels: s.labels, query: s.query,
    toggleStar: s.toggleStar, archiveMessage: s.archiveMessage,
  })));
  const message = selectMessages(messages, query).find(m => m.id === selectedId);
  if (!message) return <section className="mail-reader"><EmptyState title="留一点空间，给重要的消息" description="从左侧选择一封邮件，开始阅读。" /></section>;
  const account = accounts.find(a => a.id === message.accountId);
  return <section className="mail-reader" aria-label="邮件阅读区">
    <div className="reader-toolbar"><div><Button variant="ghost" iconOnly className="reader-back" aria-label="返回邮件列表" onClick={onBack}><ArrowLeft size={17} /></Button><Button variant="ghost" iconOnly title="归档" aria-label="归档当前邮件" disabled={message.mailbox === 'archive' || message.mailbox === 'drafts'} onClick={() => archiveMessage(message.id)}><Archive size={17} /></Button><Button variant="ghost" iconOnly aria-label="切换当前邮件星标" aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={17} fill={message.starred ? 'currentColor' : 'none'} /></Button></div><Button variant="ghost" onClick={onAi}><Sparkles size={16} /> AI 助手</Button></div>
    <div className="reader-scroll"><div className="reader-content">
      <div className="reader-labels"><span className={`label tone-${account?.color ?? 'teal'}`}>{account?.name}</span>{message.labelIds.map(id => { const label = labels.find(l => l.id === id); return label && <span key={id} className={`label tone-${label.color}`}>{label.name}</span>; })}{message.mailbox === 'drafts' && <span className="label">本地草稿</span>}</div>
      <h2>{message.subject}</h2>
      <div className="message-sender"><Avatar text={message.from.name.slice(0, 1)} color={account?.color} /><div><strong>{message.from.name}</strong><span>{message.from.email}</span><span>收件人：{message.to.join(', ') || '尚未填写'}</span></div><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat('zh-CN', { month: 'long', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(message.receivedAt))}</time></div>
      <div className="message-body">{message.bodyText || '这封草稿还没有正文。'}</div>
      <div className="reply-card"><span className="reply-symbol"><CornerUpLeft size={20} /></span><div><strong>{message.mailbox === 'drafts' ? '继续写下你的想法' : '让交流继续'}</strong><p>{message.mailbox === 'drafts' ? '草稿已保存在这台设备上。' : `使用 ${account?.email} 回复`}</p></div><Button onClick={() => onCompose(message, message.mailbox === 'drafts')}>{message.mailbox === 'drafts' ? <PenLine size={15} /> : <CornerUpLeft size={15} />}{message.mailbox === 'drafts' ? '编辑草稿' : '回复邮件'}</Button></div>
    </div></div>
  </section>;
}
