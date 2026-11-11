import { Archive, ArrowLeft, CornerUpLeft, Mail, MailOpen, PenLine, Sparkles, Star } from 'lucide-react';
import { useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import type { MailMessage, RecoveryAction } from '../../domain/mail';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { Button } from '../../components/ui/Button';
import { Avatar } from '../../components/ui/Avatar';
import { EmptyState } from '../../components/ui/EmptyState';
import { selectMessages } from './selectors';
import { useSyncStore } from '../../stores/syncStore';

export function MailReader({ onCompose, onAi, onBack }: { onCompose: (message: MailMessage, editing?: boolean) => void; onAi: () => void; onBack: () => void }) {
  const { messages, selectedId, accounts, labels, query, savingIds, toggleStar, toggleUnread, keepMessageLocal, archiveMessage, requestRecovery } = useWorkspaceStore(useShallow(s => ({
    messages: s.messages, selectedId: s.selectedId, accounts: s.accounts, labels: s.labels, query: s.query,
    savingIds: s.savingIds, toggleStar: s.toggleStar, toggleUnread: s.toggleUnread, keepMessageLocal: s.keepMessageLocal, archiveMessage: s.archiveMessage, requestRecovery: s.requestRecovery,
  })));
  const { enqueue, jobs, submitting, syncError } = useSyncStore(useShallow(s => ({ enqueue: s.enqueue, jobs: s.jobs, submitting: s.submitting, syncError: s.error })));
  const message = selectMessages(messages, query).find(m => m.id === selectedId);
  if (!message) return <section className="mail-reader"><EmptyState title="留一点空间，给重要的消息" description="从左侧选择一封邮件，开始阅读。" /></section>;
  const account = accounts.find(a => a.id === message.accountId);
  const localDraft = Boolean(message.draft);
  const serverDraft = message.mailbox === 'drafts' && !localDraft;
  const saving = savingIds.includes(message.id);
  const changes = message.changes ?? [];
  const remote = changes.filter(change => change.state !== 'local');
  const needsReview = remote.some(change => ['failed', 'conflict', 'unknown'].includes(change.state));
  const active = submitting.includes(message.accountId) || jobs.some(job => job.accountId === message.accountId && ['queued', 'running'].includes(job.status));
  const recovery = message.recovery;
  const checking = recovery?.state === 'checking';
  const requestBody = useWorkspaceStore.getState().requestBody;
  async function recover(action: RecoveryAction) {
    if (!message) return;
    if (action === 'check' && message.recovery?.state === 'queued') { await enqueue([message.accountId]); return; }
    if (await requestRecovery(message.id, action)) await enqueue([message.accountId]);
  }
  return <section className="mail-reader" aria-label="邮件阅读区">
    <div className="reader-toolbar"><div><Button variant="ghost" iconOnly className="reader-back" aria-label="返回邮件列表" onClick={onBack}><ArrowLeft size={17} /></Button><Button variant="ghost" iconOnly title="归档" aria-label="归档当前邮件" disabled={saving || message.mailbox === 'archive' || localDraft} onClick={() => archiveMessage(message.id)}><Archive size={17} /></Button><Button variant="ghost" iconOnly aria-label="切换当前邮件星标" disabled={saving || localDraft} aria-pressed={message.starred} onClick={() => toggleStar(message.id)}><Star size={17} fill={message.starred ? 'currentColor' : 'none'} /></Button><Button variant="ghost" iconOnly disabled={saving || localDraft} aria-label={message.unread ? '标为已读' : '标为未读'} onClick={() => toggleUnread(message.id)}>{message.unread ? <MailOpen size={17} /> : <Mail size={17} />}</Button></div><Button variant="ghost" onClick={onAi}><Sparkles size={16} /> AI 助手</Button></div>
    <div className="reader-scroll"><div className="reader-content">
      {(saving || changes.length > 0 || recovery) && <div className="message-change-status" role="status" aria-live="polite">
        <span>{saving ? '正在保存到本机…' : remote.length ? needsReview ? '本机已保存 · 回写需要处理' : '本机已保存 · 等待同步此账号回写' : '已保存在本机'}</span>
        {remote.map(change => change.error).filter((error, index, all) => error && all.indexOf(error) === index).map(error => <p key={error}>{error}</p>)}
        {remote.length > 0 && <><Button variant="ghost" disabled={saving || active} onClick={() => void enqueue([message.accountId])}>{active ? '账号同步中' : '同步此账号'}</Button>{needsReview && <Button variant="ghost" disabled={saving || remote.some(change => change.state === 'applying')} onClick={() => keepMessageLocal(message.id)}>仅保留本机</Button>}</>}
        {syncError && remote.length > 0 && <p>{syncError}</p>}
        {recovery && <div>
          <p>{recovery.detail}</p>
          {recovery.progress && (recovery.progress.segments ? <p>分段{recovery.progress.segments.phase === 'collecting' ? '正文核对' : '集合复核'}：{recovery.progress.segments.completed} / {recovery.progress.segments.total}，已检查 {recovery.progress.checked} 个候选，已发现 {recovery.progress.matches} 个匹配。全段复核完成前不能确认绑定。</p> : <p>候选核对进度：{recovery.progress.checked} / {recovery.progress.total}，已发现 {recovery.progress.matches} 个匹配。完整扫描前不能确认绑定。</p>)}
          {recovery.fingerprint && <><p>{recovery.fingerprint.reason}</p><Button variant="ghost" disabled={!recovery.fingerprint.eligible || saving || active || checking || recovery.state === 'queued'} onClick={() => void recover('fingerprint')}>核对并补齐原文指纹</Button></>}
          {recovery.crossUidValidity && <p>远端 UIDVALIDITY 已变化，此次恢复将绑定经原文指纹验证的新 UID。</p>}
          {recovery.location && <p>核对位置：{recovery.location.folder} · UIDVALIDITY {recovery.location.uidValidity} · UID {recovery.location.uid}</p>}
          {recovery.state !== 'restored' && <Button variant="ghost" disabled={saving || active || checking} onClick={() => void recover('check')}>{active || checking ? '远端核对处理中' : recovery.state === 'queued' ? '开始已登记核对' : '核对远端定位'}</Button>}
          {recovery.state === 'located' && <Button variant="ghost" disabled={saving || active} onClick={() => void recover(recovery.operation === 'fingerprint' ? 'confirm-fingerprint' : 'confirm')}>{recovery.operation === 'fingerprint' ? '确认补齐完整原文指纹' : '确认恢复定位'}</Button>}
          {recovery.state === 'source-present' && <Button variant="ghost" disabled={saving || active} onClick={() => void recover('retry-archive')}>重新排队归档</Button>}
          {(recovery.state === 'scanning' || (recovery.state === 'failed' && recovery.progress)) && <Button variant="ghost" disabled={saving || active} onClick={() => void recover('continue')}>{recovery.progress?.segments ? recovery.progress.segments.phase === 'validating' ? '继续全段复核' : '继续分段核对' : '继续核对下一页'}</Button>}
          {(recovery.state === 'scanning' || (recovery.state === 'failed' && recovery.progress)) && <Button variant="ghost" disabled={saving || checking} onClick={() => void recover('auto-continue')}>自动核对剩余页</Button>}
          {recovery.autoContinue && recovery.state === 'queued' && <p>正在后台逐页核对；首次检查完成后仍需明确确认恢复。可在账号设置中停止同步。</p>}
          {!remote.length && recovery.state !== 'restored' && recovery.state !== 'local' && <Button variant="ghost" disabled={saving} onClick={() => keepMessageLocal(message.id)}>仅保留本机</Button>}
          {syncError && !remote.length && <p>{syncError}</p>}
        </div>}
      </div>}
      <div className="reader-labels"><span className={`label tone-${account?.color ?? 'teal'}`}>{account?.name}</span>{message.labelIds.map(id => { const label = labels.find(l => l.id === id); return label && <span key={id} className={`label tone-${label.color}`}>{label.name}</span>; })}{localDraft && <span className="label">本地草稿</span>}{serverDraft && <span className="label">服务器草稿副本</span>}</div>
      <h2>{message.subject}</h2>
      <div className="message-sender"><Avatar text={message.from.name.slice(0, 1)} color={account?.color} /><div><strong>{message.from.name}</strong><span>{message.from.email}</span><span>收件人：{message.to.join(', ') || '尚未填写'}</span></div><time dateTime={message.receivedAt}>{new Intl.DateTimeFormat('zh-CN', { month: 'long', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(new Date(message.receivedAt))}</time></div>
      <div className="message-body">{message.bodyText || (localDraft ? '这封草稿还没有正文。' : '这封邮件没有纯文本正文。')}</div>
      {account?.status !== 'demo' && !localDraft && <div className="message-change-status">
        <Button variant="ghost" disabled={saving || active || checking || message.remoteMissing} onClick={() => void (async () => {
          if (message.bodyDownload?.state === 'queued' || await requestBody(message.id)) await enqueue([message.accountId]);
        })()}>{active && message.bodyDownload?.state === 'queued' ? '正在下载原文…' : message.bodyDownload?.state === 'queued' ? '继续已登记原文下载' : '重新下载完整原文'}</Button>
        {message.bodyDownload?.error && <p role="status">{message.bodyDownload.error}</p>}
      </div>}
      {message.remoteMissing && <p className="message-change-status" role="status">远端原定位已不存在，本机内容和修改已保留。可核对远端定位。</p>}
      {!!message.attachments?.length && <AttachmentList key={message.id} message={message} />}
      <div className="reply-card"><span className="reply-symbol"><CornerUpLeft size={20} /></span><div><strong>{message.mailbox === 'drafts' ? '继续写下你的想法' : '让交流继续'}</strong><p>{localDraft ? '草稿已保存在这台设备上。' : serverDraft ? '复制后在本机编辑，不覆盖服务器草稿。' : `使用 ${account?.email} 回复`}</p></div><Button onClick={() => onCompose(message, message.mailbox === 'drafts')}>{message.mailbox === 'drafts' ? <PenLine size={15} /> : <CornerUpLeft size={15} />}{localDraft ? '编辑草稿' : serverDraft ? '复制为本地草稿' : '回复邮件'}</Button></div>
    </div></div>
  </section>;
}
function AttachmentList({ message }: { message: MailMessage }) {
  const exportAttachment = useWorkspaceStore(state => state.exportAttachment);
  const [busy, setBusy] = useState<number | null>(null);
  const [saved, setSaved] = useState('');
  async function save(index: number, name: string) {
    setBusy(index); setSaved('');
    if (await exportAttachment(message.id, index)) setSaved(`已保存附件：${name}`);
    setBusy(null);
  }
  return <section aria-label="邮件附件" className="message-change-status">
    <strong>附件</strong>
    {message.attachments?.map(attachment => <div key={attachment.index}>
      <span>{attachment.name} · {new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 }).format(attachment.size / 1024)} KB</span>
      <Button variant="ghost" disabled={busy !== null} aria-label={`保存附件 ${attachment.name}`} onClick={() => void save(attachment.index, attachment.name)}>{busy === attachment.index ? '正在保存…' : '另存为'}</Button>
    </div>)}
    <span role="status" aria-live="polite">{saved}</span>
  </section>;
}
