import { AlignLeft, ListTodo, PenLine, Settings, Sparkles, X } from 'lucide-react';
import { useEffect } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { Button } from '../../components/ui/Button';
import { useAiStore } from '../../stores/aiStore';
import { useWorkspaceStore } from '../../stores/workspaceStore';
import { selectMessages } from '../mail/selectors';
import { isDesktop } from '../../platform/desktop';
import { useI18n } from '../../i18n/react';
import type { AiTaskKind } from '../../domain/ai';
import type { MailMessage } from '../../domain/mail';

/** 演示版保持诚实的静态占位：不配置、不调用、不伪装。 */
function DemoAiPanel({ onClose }: { onClose: () => void }) {
  const t = useI18n();
  return <aside className="ai-panel" aria-label={t('ai.panelAria')}><header><span><Sparkles size={17} /> {t('ai.title')}</span><Button variant="ghost" iconOnly aria-label={t('ai.closeAria')} onClick={onClose}><X size={16} /></Button></header><div className="ai-panel-body"><div className="ai-mark"><Sparkles size={26} /></div><h2>{t('ai.heading')}</h2><p>{t('ai.intro')}</p><span className="label">{t('wizard.soonTag')}</span><p className="ai-privacy">{t('ai.privacy')}</p></div></aside>;
}

/** 桌面版 AI 面板：三种只读任务（摘要 / 回复草稿 / 提取待办），作用于当前阅读的邮件。 */
export function AiPanel({ onClose, onOpenSettings, onComposeWithText }: { onClose: () => void; onOpenSettings: () => void; onComposeWithText: (message: MailMessage, bodyText: string) => void }) {
  const t = useI18n();
  const { config, ensureConfig, running, results, error, run, clearError } = useAiStore(useShallow(s => ({
    config: s.config, ensureConfig: s.ensureConfig, running: s.running, results: s.results, error: s.error, run: s.run, clearError: s.clearError,
  })));
  const { messages, query, selectedId } = useWorkspaceStore(useShallow(s => ({ messages: s.messages, query: s.query, selectedId: s.selectedId })));
  useEffect(() => { void ensureConfig(); }, [ensureConfig]);
  const message = selectMessages(messages, { ...query, unreadOnly: false }).find(item => item.id === selectedId && !item.draft);
  async function execute(task: AiTaskKind) {
    if (!message) return;
    await run(task, message.accountId, [message.id]);
  }
  const tasks = [
    { kind: 'summarize' as const, Icon: AlignLeft, name: t('ai.summarize'), description: t('ai.summarizeDesc') },
    { kind: 'draft-reply' as const, Icon: PenLine, name: t('ai.draftReply'), description: t('ai.draftReplyDesc') },
    { kind: 'extract-events' as const, Icon: ListTodo, name: t('ai.extractEvents'), description: t('ai.extractEventsDesc') },
  ];
  const replyResult = results['draft-reply'];
  const extractResult = results['extract-events'];
  return <aside className="ai-panel" aria-label={t('ai.panelAria')}>
    <header><span><Sparkles size={17} /> {t('ai.title')}</span><Button variant="ghost" iconOnly aria-label={t('ai.closeAria')} onClick={onClose}><X size={16} /></Button></header>
    <div className="ai-panel-body">
      {!config ? <>
        <div className="ai-mark"><Sparkles size={26} /></div>
        <h2>{t('ai.needConfigTitle')}</h2>
        <p>{t('ai.needConfig')}</p>
        <Button variant="primary" onClick={onOpenSettings}><Settings size={15} />{t('ai.openSettings')}</Button>
      </> : <>
        <p className="muted">{message ? t('ai.targetMessage', { subject: message.subject }) : t('ai.emptySelection')}</p>
        <div className="ai-actions">{tasks.map(({ kind, Icon, name, description }) => <button key={kind} type="button" className={results[kind] ? 'ai-task has-result' : 'ai-task'} disabled={running !== null || !message} aria-label={name} onClick={() => void execute(kind)}>
          <Icon size={17} /><span><strong>{name}</strong><small>{running === kind ? t('ai.running') : description}</small></span>
        </button>)}</div>
        {error && <div className="inline-error" role="alert">{error}<Button variant="ghost" onClick={clearError}>{t('app.dismissToast')}</Button></div>}
        {results['summarize'] && <section className="ai-result" aria-label={t('ai.summarize')}><h3>{t('ai.resultHeading', { model: results['summarize']!.model })}</h3><p>{results['summarize']!.text}</p></section>}
        {replyResult && <section className="ai-result" aria-label={t('ai.draftReply')}><h3>{t('ai.resultHeading', { model: replyResult.model })}</h3><p>{replyResult.text}</p>{message && <Button variant="primary" onClick={() => onComposeWithText(message, replyResult.text)}><PenLine size={15} />{t('ai.useAsReply')}</Button>}<p className="muted">{t('ai.replyNote')}</p></section>}
        {extractResult && <section className="ai-result" aria-label={t('ai.extractEvents')}><h3>{t('ai.resultHeading', { model: extractResult.model })}</h3><p>{extractResult.text}</p><p className="muted">{t('ai.extractNote')}</p></section>}
      </>}
      <p className="ai-privacy">{t('ai.privacyDesktop')}</p>
    </div>
  </aside>;
}

export function AiPanelGate({ onClose, onOpenSettings, onComposeWithText }: { onClose: () => void; onOpenSettings: () => void; onComposeWithText: (message: MailMessage, bodyText: string) => void }) {
  if (!isDesktop()) return <DemoAiPanel onClose={onClose} />;
  return <AiPanel onClose={onClose} onOpenSettings={onOpenSettings} onComposeWithText={onComposeWithText} />;
}
